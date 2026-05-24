//! Derive per-row state across all attempts of an execution.
//!
//! Pure compute layer. Reads the SQLite attempts list via ExecutionStore,
//! then walks each completed or aborted attempt's `outcomes.jsonl` to
//! collapse outcomes per the rules in `docs/plan/2026-05-16-streaming-dispatch.md`
//! § 13 Row Resolution Semantics Change.

use crate::error::CoreError;
use crate::execution_store::{AttemptState, ExecutionStore};
use crate::pool::{BatchOutcome, RowOutcome};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::BufRead;
use tracing;

type Result<T> = std::result::Result<T, CoreError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionState {
    NeverAttempted,
    Resolved,
    FailedLast,
    CrashedLast,
    CancelledLast,
    TooLarge,
}

/// Outcome of one seq in one attempt.
#[derive(Debug, Clone)]
pub struct OutcomeRecord {
    pub attempt_id: String,
    pub seq: u64,
    /// "success" | "failed"
    pub kind: OutcomeKind,
    /// Raw CSV row synthesized from the JSONL outcome data.
    /// Kept in CSV form so that downstream `exec_cmd.rs` consumers that
    /// write merged success/failed CSVs continue to work without change (P10
    /// will replace that path wholesale).
    pub raw: csv::StringRecord,
    pub headers: Vec<String>,
    /// For failed rows: error code.
    pub code: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeKind {
    Success,
    Failed,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ResolutionCounts {
    pub resolved: u64,
    pub failed_last: u64,
    pub crashed_last: u64,
    pub cancelled_last: u64,
    pub too_large: u64,
    pub never_attempted: u64,
}

#[derive(Debug, Clone)]
pub struct PerSeq {
    pub state: ResolutionState,
    /// First SUCCESS for the seq (canonical), if any.
    pub canonical_success: Option<OutcomeRecord>,
    /// Latest non-success outcome (for failed.csv export), if state ≠ Resolved.
    pub latest_failure: Option<OutcomeRecord>,
}

#[derive(Debug, Clone)]
pub struct RowResolution {
    pub execution_id: String,
    pub input_row_count: u64,
    pub per_seq: BTreeMap<u64, PerSeq>,
    pub counts: ResolutionCounts,
    pub by_error_code: BTreeMap<String, u64>,
    pub merged_from_attempts: Vec<String>,
    /// Attempts that were still Running and intentionally skipped (stale
    /// in-progress attempts that were never finished or aborted).
    pub skipped_running: Vec<String>,
}

impl RowResolution {
    pub fn resolved_seqs(&self) -> std::collections::HashSet<u64> {
        self.per_seq
            .iter()
            .filter(|(_, p)| p.state == ResolutionState::Resolved)
            .map(|(s, _)| *s)
            .collect()
    }

    /// All seqs that have ANY outcome from a prior attempt — i.e. anything
    /// except `NeverAttempted`. Used by `--skip-attempted` to deduplicate
    /// sampling across attempts (each row gets one shot at the handler).
    pub fn attempted_seqs(&self) -> std::collections::HashSet<u64> {
        self.per_seq
            .iter()
            .filter(|(_, p)| p.state != ResolutionState::NeverAttempted)
            .map(|(s, _)| *s)
            .collect()
    }

    /// Seqs whose latest outcome is a non-success — FailedLast, CrashedLast,
    /// CancelledLast, TooLarge. Excludes Resolved (success-absorbing wins)
    /// and NeverAttempted (no outcome to retry). Used by `--retry-failed`
    /// to target only failures without dragging in fresh rows.
    pub fn failed_seqs(&self) -> std::collections::HashSet<u64> {
        self.per_seq
            .iter()
            .filter(|(_, p)| matches!(
                p.state,
                ResolutionState::FailedLast
                    | ResolutionState::CrashedLast
                    | ResolutionState::CancelledLast
                    | ResolutionState::TooLarge
            ))
            .map(|(s, _)| *s)
            .collect()
    }
}

pub fn compute_resolution(store: &ExecutionStore, exec_id: &str) -> Result<RowResolution> {
    let exec = store
        .get_execution(exec_id)?
        .ok_or_else(|| CoreError::Store(format!("execution not found: {exec_id}")))?;
    let attempts = store.list_attempts_for_execution(&exec.id)?;

    let mut per_seq: BTreeMap<u64, PerSeq> = BTreeMap::new();
    let mut merged_from = Vec::new();
    let mut skipped_running = Vec::new();
    let mut by_code: BTreeMap<String, u64> = BTreeMap::new();

    // Initialize all seqs as NeverAttempted.
    for s in 0..exec.input_row_count {
        per_seq.insert(s, PerSeq {
            state: ResolutionState::NeverAttempted,
            canonical_success: None,
            latest_failure: None,
        });
    }

    // Walk attempts in chronological order (list_attempts_for_execution
    // already returns ASC by started_at).
    for at in &attempts {
        match at.state {
            // Both Completed and Aborted attempts are ingested — Aborted
            // attempts may have written partial outcomes.jsonl before the
            // cancellation/stall fired, and those partial results are valid.
            AttemptState::Completed | AttemptState::Aborted => {}
            // Running attempts are stale in-progress rows; skip them.
            AttemptState::Running => {
                skipped_running.push(at.id.clone());
                continue;
            }
        }
        merged_from.push(at.id.clone());
        ingest_jsonl(&at.dir.join("outcomes.jsonl"), &at.id, &mut per_seq)?;
    }

    // Derive counts + by_error_code (over LATEST failure of unresolved).
    let mut counts = ResolutionCounts::default();
    for (_, p) in per_seq.iter() {
        match p.state {
            ResolutionState::Resolved => counts.resolved += 1,
            ResolutionState::FailedLast => {
                counts.failed_last += 1;
                if let Some(f) = &p.latest_failure {
                    if let Some(code) = &f.code {
                        *by_code.entry(code.clone()).or_insert(0) += 1;
                    }
                }
            }
            ResolutionState::CrashedLast => counts.crashed_last += 1,
            ResolutionState::CancelledLast => counts.cancelled_last += 1,
            ResolutionState::TooLarge => counts.too_large += 1,
            ResolutionState::NeverAttempted => counts.never_attempted += 1,
        }
    }

    Ok(RowResolution {
        execution_id: exec.id,
        input_row_count: exec.input_row_count,
        per_seq,
        counts,
        by_error_code: by_code,
        merged_from_attempts: merged_from,
        skipped_running,
    })
}

/// Counts-only entry point for cross-attempt resolution. Currently
/// implemented as a wrapper around `compute_resolution` that discards
/// the canonical-success map — same cost, less interface surface.
///
/// A future revision can specialize the inner loop to skip the map
/// allocation when only counts are needed. This is gated behind a real
/// performance need (Studio's ExecRollup is gated by user click and
/// shown with a spinner; current implementation is acceptable).
///
/// Spec: `docs/spec/studio/part-5-api.md` §5.1 lift list.
pub fn compute_resolution_counts_only(
    store: &ExecutionStore,
    exec_id: &str,
) -> Result<ResolutionCounts> {
    Ok(compute_resolution(store, exec_id)?.counts)
}

/// Read `outcomes.jsonl` for one attempt and merge outcomes into `per_seq`.
///
/// - File missing → `Ok(())`: legitimate for an empty Aborted attempt that
///   crashed before writing any outcomes.
/// - Parse error → fail-fast per §13.1 with a `CoreError::Store` message
///   that identifies the file, line number, and attempt.
fn ingest_jsonl(
    path: &std::path::Path,
    attempt_id: &str,
    per_seq: &mut BTreeMap<u64, PerSeq>,
) -> Result<()> {
    if !path.is_file() {
        return Ok(());
    }
    let file = std::fs::File::open(path)
        .map_err(|e| CoreError::Store(format!("open {}: {e}", path.display())))?;
    let reader = std::io::BufReader::new(file);
    for (line_num, line_res) in reader.lines().enumerate() {
        let line = line_res
            .map_err(|e| CoreError::Store(format!("read {}: {e}", path.display())))?;
        if line.trim().is_empty() {
            continue;
        }
        let bo: BatchOutcome = serde_json::from_str(&line).map_err(|e| {
            tracing::error!(
                path = %path.display(),
                line = line_num + 1,
                attempt_id = %attempt_id,
                error = %e,
                "outcomes.jsonl corruption: ingest aborted",
            );
            CoreError::Store(format!(
                "outcomes.jsonl corruption at {}:{}: {e}; attempt {} ingest aborted",
                path.display(),
                line_num + 1,
                attempt_id
            ))
        })?;
        for row_outcome in bo.outcomes {
            apply_row_outcome(row_outcome, attempt_id, per_seq);
        }
    }
    Ok(())
}

/// Apply one `RowOutcome` to the per-seq map.
fn apply_row_outcome(
    outcome: RowOutcome,
    attempt_id: &str,
    per_seq: &mut BTreeMap<u64, PerSeq>,
) {
    use crate::run::ERR_WORKER_CRASH;

    match outcome {
        RowOutcome::Success { seq, data, .. } => {
            let entry = per_seq.entry(seq).or_insert(PerSeq {
                state: ResolutionState::NeverAttempted,
                canonical_success: None,
                latest_failure: None,
            });
            // SUCCESS is absorbing — first wins; later successes ignored.
            if entry.canonical_success.is_none() {
                entry.canonical_success = Some(build_success_record(attempt_id, seq, &data));
            }
            entry.state = ResolutionState::Resolved;
        }
        RowOutcome::Error { seq, code, message, data, .. } => {
            let entry = per_seq.entry(seq).or_insert(PerSeq {
                state: ResolutionState::NeverAttempted,
                canonical_success: None,
                latest_failure: None,
            });
            // Monotonicity: don't downgrade RESOLVED to FailedLast.
            if entry.state == ResolutionState::Resolved {
                return;
            }
            let new_state = classify_failure(Some(code.as_str()));
            entry.state = new_state;
            entry.latest_failure = Some(build_error_record(
                attempt_id,
                seq,
                &code,
                &message,
                data.as_ref(),
            ));
        }
        RowOutcome::Crash { seq, worker_id, crash_at_seq } => {
            let entry = per_seq.entry(seq).or_insert(PerSeq {
                state: ResolutionState::NeverAttempted,
                canonical_success: None,
                latest_failure: None,
            });
            // Monotonicity: don't downgrade RESOLVED.
            if entry.state == ResolutionState::Resolved {
                return;
            }
            let code = ERR_WORKER_CRASH.to_string();
            let message = format!(
                "worker {worker_id} crashed at seq {crash_at_seq}"
            );
            entry.state = ResolutionState::CrashedLast;
            entry.latest_failure = Some(build_error_record(
                attempt_id,
                seq,
                &code,
                &message,
                None,
            ));
        }
    }
}

/// Synthesize a CSV-backed `OutcomeRecord` for a Success outcome.
///
/// headers = ["seqid", ...sorted(data keys)]
/// raw     = [seqid_decimal, ...values in same order]
fn build_success_record(
    attempt_id: &str,
    seq: u64,
    data: &serde_json::Map<String, serde_json::Value>,
) -> OutcomeRecord {
    let mut sorted_keys: Vec<&String> = data.keys().collect();
    sorted_keys.sort();
    let mut headers = vec!["seqid".to_string()];
    headers.extend(sorted_keys.iter().map(|k| (*k).clone()));
    let mut values: Vec<String> = vec![seq.to_string()];
    for k in &sorted_keys {
        values.push(json_value_to_csv(data.get(*k).unwrap()));
    }
    let raw = csv::StringRecord::from(values);
    OutcomeRecord {
        attempt_id: attempt_id.to_string(),
        seq,
        kind: OutcomeKind::Success,
        raw,
        headers,
        code: None,
    }
}

/// Synthesize a CSV-backed `OutcomeRecord` for an Error or Crash outcome.
///
/// headers = ["seqid", "errcode", "errmessage", ...sorted(data keys when Some)]
/// raw     = corresponding values
fn build_error_record(
    attempt_id: &str,
    seq: u64,
    code: &str,
    message: &str,
    data: Option<&serde_json::Map<String, serde_json::Value>>,
) -> OutcomeRecord {
    let mut headers = vec![
        "seqid".to_string(),
        "errcode".to_string(),
        "errmessage".to_string(),
    ];
    let mut values = vec![seq.to_string(), code.to_string(), message.to_string()];
    if let Some(d) = data {
        let mut sorted_keys: Vec<&String> = d.keys().collect();
        sorted_keys.sort();
        for k in &sorted_keys {
            headers.push((*k).clone());
            values.push(json_value_to_csv(d.get(*k).unwrap()));
        }
    }
    let raw = csv::StringRecord::from(values);
    OutcomeRecord {
        attempt_id: attempt_id.to_string(),
        seq,
        kind: OutcomeKind::Failed,
        raw,
        headers,
        code: Some(code.to_string()),
    }
}

/// Convert a JSON value to a CSV cell string (simple, lossless for scalar types).
fn json_value_to_csv(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn classify_failure(code: Option<&str>) -> ResolutionState {
    use crate::run::*;
    match code {
        Some(c) if c == ERR_WORKER_CRASH || c == ERR_WORKER_CRASH_UNSAFE => ResolutionState::CrashedLast,
        Some(c) if c == ERR_CANCELLED => ResolutionState::CancelledLast,
        Some(c) if c == ERR_ROW_TOO_LARGE => ResolutionState::TooLarge,
        _ => ResolutionState::FailedLast,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution_store::{
        ExecutionStore, NewExecution, NewHandlerInstance, NewAttempt, FinishAttempt,
        Source, Simulation, RunType,
    };
    use crate::pool::{BatchOutcome, RowOutcome};
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn make_csv(p: &std::path::Path, rows: usize) {
        let mut s = String::from("billid\n");
        for i in 0..rows {
            s.push_str(&format!("{i}\n"));
        }
        std::fs::write(p, s).unwrap();
    }

    /// Write an `outcomes.jsonl` for one attempt.
    /// `successes`: list of seq ids that succeeded (data = {})
    /// `failures`: list of (seq, errcode) pairs
    fn write_outcomes_jsonl(
        dir: &std::path::Path,
        successes: &[u64],
        failures: &[(u64, &str)],
    ) {
        let path = dir.join("outcomes.jsonl");
        let mut lines = Vec::new();

        // Emit one BatchOutcome per (success seq) and one per (failure pair)
        // for simplicity; the resolution logic handles multiple batches fine.
        for &seq in successes {
            let bo = BatchOutcome {
                first_seq: seq,
                seqs: vec![seq],
                outcomes: vec![RowOutcome::Success {
                    seq,
                    data: serde_json::Map::new(),
                    dur_ms: 1,
                }],
            };
            lines.push(serde_json::to_string(&bo).unwrap());
        }
        for &(seq, code) in failures {
            let bo = BatchOutcome {
                first_seq: seq,
                seqs: vec![seq],
                outcomes: vec![RowOutcome::Error {
                    seq,
                    code: code.to_string(),
                    message: "x".to_string(),
                    dur_ms: 1,
                    data: None,
                }],
            };
            lines.push(serde_json::to_string(&bo).unwrap());
        }
        std::fs::write(path, lines.join("\n")).unwrap();
    }

    fn fixture(rows: usize) -> (tempfile::TempDir, ExecutionStore, String, String) {
        let home = tempdir().unwrap();
        let src = tempdir().unwrap();
        let csv = src.path().join("in.csv");
        make_csv(&csv, rows);
        let mut store = ExecutionStore::open(home.path()).unwrap();
        let exec = store.create_execution(NewExecution {
            name: None,
            input_csv_id: "c".into(),
            input_csv_path: csv,
            current_handler_instance_id: None,
        }).unwrap();
        let hi = store.register_handler_instance(NewHandlerInstance {
            handler_id: "h".into(),
            manifest_hash: "sha256:m".into(),
            source_snapshot_dir: PathBuf::from("/tmp/snap"),
            binary_hash: None,
        }).unwrap();
        (home, store, exec.id, hi.id)
    }

    fn run_attempt(
        store: &mut ExecutionStore,
        exec_id: &str,
        hi_id: &str,
        successes: &[u64],
        failures: &[(u64, &str)],
    ) -> String {
        let at = store.create_attempt(NewAttempt {
            execution_id: exec_id.to_string(),
            handler_instance_id: hi_id.to_string(),
            parent_attempt_id: None,
            run_type: RunType { source: Source::Full, simulation: Simulation::Real },
        }).unwrap();
        write_outcomes_jsonl(&at.dir, successes, failures);
        store.finish_attempt(&at.id, FinishAttempt {
            success_count: successes.len() as u64,
            failed_count: failures.len() as u64,
            aborted: false,
            aborted_reason: None,
        }).unwrap();
        at.id
    }

    // ── Rewritten original tests ─────────────────────────────────────────────

    #[test]
    fn never_attempted_when_no_attempts() {
        let (_h, store, exec_id, _) = fixture(3);
        let r = compute_resolution(&store, &exec_id).unwrap();
        assert_eq!(r.counts.never_attempted, 3);
        assert_eq!(r.counts.resolved, 0);
    }

    #[test]
    fn success_is_absorbing_across_attempts() {
        let (_h, mut store, exec_id, hi) = fixture(3);
        // Attempt 1: succeed seq 0, fail seq 1.
        run_attempt(&mut store, &exec_id, &hi, &[0], &[(1, "INVALID")]);
        // Attempt 2: succeed seq 1 (was failed), fail seq 0 (was succeeded).
        run_attempt(&mut store, &exec_id, &hi, &[1], &[(0, "INVALID")]);
        let r = compute_resolution(&store, &exec_id).unwrap();
        assert_eq!(r.counts.resolved, 2, "0 and 1 both resolved (SUCCESS absorbing)");
        assert_eq!(r.counts.never_attempted, 1, "seq 2 untouched");
    }

    #[test]
    fn failure_classification_by_code() {
        let (_h, mut store, exec_id, hi) = fixture(4);
        run_attempt(&mut store, &exec_id, &hi, &[],
                    &[(0, "INVALID"), (1, "WORKER_CRASH"),
                      (2, "CANCELLED"), (3, "ROW_TOO_LARGE")]);
        let r = compute_resolution(&store, &exec_id).unwrap();
        assert_eq!(r.counts.failed_last, 1);
        assert_eq!(r.counts.crashed_last, 1);
        assert_eq!(r.counts.cancelled_last, 1);
        assert_eq!(r.counts.too_large, 1);
        assert_eq!(*r.by_error_code.get("INVALID").unwrap(), 1);
        assert!(!r.by_error_code.contains_key("WORKER_CRASH"),
                "by_error_code is only for FailedLast bucket");
    }

    #[test]
    fn failed_seqs_excludes_resolved_and_never_attempted() {
        let (_h, mut store, exec_id, hi) = fixture(6);
        // seq 0 resolved; seqs 1-4 each fail with a different code; seq 5 untouched.
        run_attempt(
            &mut store,
            &exec_id,
            &hi,
            &[0],
            &[
                (1, "INVALID"),         // → FailedLast
                (2, "WORKER_CRASH"),    // → CrashedLast
                (3, "CANCELLED"),       // → CancelledLast
                (4, "ROW_TOO_LARGE"),   // → TooLarge
            ],
        );
        let r = compute_resolution(&store, &exec_id).unwrap();

        let failed = r.failed_seqs();
        // Exactly the four failure-class seqs — Resolved (0) and NeverAttempted (5) are out.
        assert_eq!(failed, [1u64, 2, 3, 4].into_iter().collect());
        assert!(!failed.contains(&0), "Resolved must not be in failed_seqs");
        assert!(!failed.contains(&5), "NeverAttempted must not be in failed_seqs");
    }

    #[test]
    fn attempted_seqs_excludes_only_never_attempted() {
        let (_h, mut store, exec_id, hi) = fixture(5);
        // Attempt 1: seq 0 success, seq 1 failed, seq 2 crashed; seqs 3-4 never touched.
        run_attempt(
            &mut store,
            &exec_id,
            &hi,
            &[0],
            &[(1, "INVALID"), (2, "WORKER_CRASH")],
        );
        let r = compute_resolution(&store, &exec_id).unwrap();

        let resolved = r.resolved_seqs();
        let attempted = r.attempted_seqs();

        // resolved_seqs() = SUCCESS only
        assert_eq!(resolved, [0u64].into_iter().collect());
        // attempted_seqs() = everything except NeverAttempted
        assert_eq!(attempted, [0u64, 1, 2].into_iter().collect());
        // Seqs 3 & 4 (NeverAttempted) MUST be absent from both — they're the
        // "fresh sample pool" that --skip-attempted relies on.
        assert!(!attempted.contains(&3));
        assert!(!attempted.contains(&4));
    }

    /// Renamed from `aborted_attempt_skipped` — v3.3 semantic shift: Aborted
    /// attempts ARE now ingested, not skipped.
    #[test]
    fn aborted_attempt_now_ingested() {
        let (_h, mut store, exec_id, hi) = fixture(2);
        // Create an attempt, write outcomes.jsonl with seq 0 succeeded, then
        // mark it as Aborted (simulating a cancel/stall mid-run).
        let at = store.create_attempt(NewAttempt {
            execution_id: exec_id.clone(),
            handler_instance_id: hi.clone(),
            parent_attempt_id: None,
            run_type: RunType { source: Source::Full, simulation: Simulation::Real },
        }).unwrap();
        write_outcomes_jsonl(&at.dir, &[0], &[]);
        store.finish_attempt(&at.id, FinishAttempt {
            success_count: 1,
            failed_count: 0,
            aborted: true,
            aborted_reason: Some("test".into()),
        }).unwrap();
        let r = compute_resolution(&store, &exec_id).unwrap();
        // v3.3 change: the partial result from the aborted attempt IS counted.
        assert_eq!(r.counts.resolved, 1, "aborted attempt's success IS ingested now");
        assert_eq!(r.counts.never_attempted, 1, "seq 1 was never touched");
        assert_eq!(r.skipped_running.len(), 0, "no Running attempts — nothing skipped");
        assert_eq!(r.merged_from_attempts.len(), 1, "aborted attempt IS in merged_from");
    }

    // ── New acceptance tests ─────────────────────────────────────────────────

    /// §16 acceptance #4: stall recovery via jsonl.
    ///
    /// Attempt 1 (Aborted/stalled): writes outcomes for seqs 0-4 only.
    /// Attempt 2 (Completed):       writes outcomes for seqs 5-9.
    /// Resolution: all 10 seqs Resolved, zero NeverAttempted.
    #[test]
    fn stall_recovery_via_jsonl() {
        let (_h, mut store, exec_id, hi) = fixture(10);

        // Stall attempt — covers first half, aborted before finishing.
        let at1 = store.create_attempt(NewAttempt {
            execution_id: exec_id.clone(),
            handler_instance_id: hi.clone(),
            parent_attempt_id: None,
            run_type: RunType { source: Source::Full, simulation: Simulation::Real },
        }).unwrap();
        write_outcomes_jsonl(&at1.dir, &[0, 1, 2, 3, 4], &[]);
        store.finish_attempt(&at1.id, FinishAttempt {
            success_count: 5,
            failed_count: 0,
            aborted: true,
            aborted_reason: Some("stall".into()),
        }).unwrap();

        // Recovery attempt — covers the remaining seqs.
        run_attempt(&mut store, &exec_id, &hi, &[5, 6, 7, 8, 9], &[]);

        let r = compute_resolution(&store, &exec_id).unwrap();
        assert_eq!(r.counts.resolved, 10, "all 10 seqs resolved across the two attempts");
        assert_eq!(r.counts.never_attempted, 0, "no seqs left unresolved");
        assert_eq!(r.merged_from_attempts.len(), 2, "both attempts merged");
    }

    /// P0 review follow-up: single BatchOutcome with 3 seqs (mixed Success + Error).
    ///
    /// Verifies that resolution correctly processes a multi-seq batch where seqs
    /// are packed into a single BatchOutcome (as the accumulator emits in batch mode).
    #[test]
    fn ingest_jsonl_multi_seq_batch() {
        use serde_json::json;

        let (_h, mut store, exec_id, hi) = fixture(3);

        let at = store.create_attempt(NewAttempt {
            execution_id: exec_id.clone(),
            handler_instance_id: hi.clone(),
            parent_attempt_id: None,
            run_type: RunType { source: Source::Full, simulation: Simulation::Real },
        }).unwrap();

        // A single BatchOutcome with 3 seqs: Success, Error, Success.
        let bo = BatchOutcome {
            first_seq: 0,
            seqs: vec![0, 1, 2],
            outcomes: vec![
                RowOutcome::Success {
                    seq: 0,
                    data: serde_json::Map::from_iter([("k".to_string(), json!("v0"))]),
                    dur_ms: 1,
                },
                RowOutcome::Error {
                    seq: 1,
                    code: "INVALID".to_string(),
                    message: "bad value".to_string(),
                    dur_ms: 2,
                    data: Some(serde_json::Map::from_iter([("detail".to_string(), json!("x"))])),
                },
                RowOutcome::Success {
                    seq: 2,
                    data: serde_json::Map::from_iter([("k".to_string(), json!("v2"))]),
                    dur_ms: 3,
                },
            ],
        };
        let line = serde_json::to_string(&bo).unwrap();
        std::fs::write(at.dir.join("outcomes.jsonl"), format!("{line}\n")).unwrap();

        store.finish_attempt(&at.id, FinishAttempt {
            success_count: 2,
            failed_count: 1,
            aborted: false,
            aborted_reason: None,
        }).unwrap();

        let r = compute_resolution(&store, &exec_id).unwrap();

        // Seqs 0 and 2 are Success → Resolved.
        // Seq 1 is Error(INVALID) → FailedLast.
        assert_eq!(r.counts.resolved, 2, "seqs 0 and 2 resolved");
        assert_eq!(r.counts.failed_last, 1, "seq 1 is FailedLast");
        assert_eq!(r.counts.never_attempted, 0, "all 3 seqs ingested");
        assert_eq!(
            *r.by_error_code.get("INVALID").unwrap(), 1,
            "INVALID appears in by_error_code"
        );
    }

    #[test]
    fn counts_only_matches_full_computation_counts() {
        let (_h, mut store, exec_id, hi) = fixture(5);
        // seq 0 resolved; seqs 1-2 fail with different codes; seq 3 crashed; seq 4 never touched.
        run_attempt(
            &mut store,
            &exec_id,
            &hi,
            &[0],
            &[(1, "INVALID"), (2, "INVALID"), (3, "WORKER_CRASH")],
        );
        let full = compute_resolution(&store, &exec_id).unwrap();
        let counts = compute_resolution_counts_only(&store, &exec_id).unwrap();
        assert_eq!(counts.resolved, full.counts.resolved);
        assert_eq!(counts.failed_last, full.counts.failed_last);
        assert_eq!(counts.crashed_last, full.counts.crashed_last);
        assert_eq!(counts.cancelled_last, full.counts.cancelled_last);
        assert_eq!(counts.too_large, full.counts.too_large);
        assert_eq!(counts.never_attempted, full.counts.never_attempted);
    }

    /// §13.1 fail-fast on corrupt outcomes.jsonl.
    ///
    /// Write 2 valid BatchOutcome lines, then 1 corrupt line.
    /// `compute_resolution` must return `CoreError::Store` with a message
    /// containing "corruption at" and the line number (3).
    #[test]
    fn jsonl_corruption_fail_fast() {
        let (_h, mut store, exec_id, hi) = fixture(5);

        let at = store.create_attempt(NewAttempt {
            execution_id: exec_id.clone(),
            handler_instance_id: hi.clone(),
            parent_attempt_id: None,
            run_type: RunType { source: Source::Full, simulation: Simulation::Real },
        }).unwrap();

        // Two valid lines, then a corrupt one.
        let good1 = serde_json::to_string(&BatchOutcome {
            first_seq: 0,
            seqs: vec![0],
            outcomes: vec![RowOutcome::Success { seq: 0, data: serde_json::Map::new(), dur_ms: 1 }],
        }).unwrap();
        let good2 = serde_json::to_string(&BatchOutcome {
            first_seq: 1,
            seqs: vec![1],
            outcomes: vec![RowOutcome::Success { seq: 1, data: serde_json::Map::new(), dur_ms: 1 }],
        }).unwrap();
        let corrupt = r#"{"this_is": "not a BatchOutcome", "garbage": true}"#;
        let contents = format!("{good1}\n{good2}\n{corrupt}\n");
        std::fs::write(at.dir.join("outcomes.jsonl"), contents).unwrap();

        store.finish_attempt(&at.id, FinishAttempt {
            success_count: 0,
            failed_count: 0,
            aborted: false,
            aborted_reason: None,
        }).unwrap();

        let err = compute_resolution(&store, &exec_id).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("corruption at"), "message should say 'corruption at': {msg}");
        assert!(msg.contains(":3"), "corruption should be at line 3: {msg}");
    }
}
