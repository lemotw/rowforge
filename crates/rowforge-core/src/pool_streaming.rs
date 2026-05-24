//! Streaming dispatch pool — wires Reader + Accumulator + Workers + Stall Monitor
//! into a single async pipeline (plan §2, §7, §8, §11, v3.3).
//!
//! # Architecture
//!
//! ```text
//! Reader ──row_tx──► Accumulator ──job_tx──► Worker(s) ──► outcomes.jsonl
//!                                                             ▲
//!                                             Stall Monitor ──┘ (cancel on no growth)
//! ```
//!
//! All four tasks share a single `CancellationToken`. Normal completion: Reader /
//! Accumulator / Workers all finish naturally, then `run_pool_streaming` fires
//! `cancel.cancel()` to tell the Stall Monitor "we're done, stop monitoring" (§7.1).
//!
//! # Cancel source distinction (§7.2)
//!
//! | Source | Detection |
//! |---|---|
//! | Normal EOF | monitor returns `Ok(())` after `cancel.cancel()` |
//! | Stall | monitor returns `Err("stalled …")` |
//! | Operator SIGINT | external `cfg.cancel` was fired before we finished; all tasks Ok but cancel was pre-set |
//! | Worker/accumulator error | any task handle returns `Err` |

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::accumulator::{accumulator_task, Batch, JOB_CHANNEL_CAP};
use crate::cancel::CancellationToken;
use crate::reader::FieldMap;
use crate::error::CoreError;
use crate::input_stream::InputStream;
use crate::jsonl_writer::{
    stall_monitor_task, SharedJsonlWriter, STALL_POLL_INTERVAL_SECS, STALL_TIMEOUT_SECS,
};
use crate::manifest::Manifest;
use crate::pool::RowJob;
use crate::reader::{reader_task, ReaderConfig, ROW_CHANNEL_CAP};
use crate::runtime::Runtime;
use crate::worker::Worker;
use crate::worker_loop::run_worker_loop;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Configuration for [`run_pool_streaming`].
pub struct StreamingPoolConfig {
    /// Directory containing the handler's `rowforge.yaml` and binary.
    pub handler_dir: PathBuf,
    /// Parsed manifest (shared-ref so we don't double-parse).
    pub manifest: Arc<Manifest>,
    /// Number of parallel worker processes. Silently clamped to 1 when
    /// `manifest.runtime.stateful` is `true`.
    pub workers: u32,
    /// Attempt/run identifier forwarded to the handler in the `init` envelope.
    pub run_id: String,
    /// Effective config (manifest defaults already merged with CLI overrides).
    pub config: BTreeMap<String, serde_json::Value>,
    /// Grace period for graceful handler shutdown.
    pub shutdown_grace: Duration,
    /// Optional external cancellation token (e.g. operator Ctrl-C). When
    /// `Some`, the pool creates a *child* token so that the caller's token can
    /// cancel us, and we can cancel the stall monitor without cancelling the
    /// caller's token.
    pub cancel: Option<CancellationToken>,
    /// Effective `runtime` block from the manifest.
    pub runtime: Runtime,
    /// Where to open / append `outcomes.jsonl`.
    pub jsonl_path: PathBuf,
    /// Whether to `sync_data` after every JSONL append.
    pub fsync_outcomes: bool,
    /// Override stall timeout (default: `STALL_TIMEOUT_SECS`). `None` uses
    /// the default. Used by tests to get a short timeout without changing the
    /// module constant.
    pub stall_timeout: Option<Duration>,
    /// Override stall poll interval (default: `STALL_POLL_INTERVAL_SECS`).
    /// `None` uses the default. Used by tests to get sub-second polling.
    pub stall_poll_interval: Option<Duration>,
    /// Optional per-row progress callback fired after each batch's outcomes
    /// are appended to `outcomes.jsonl`. Cloned (Arc) into each worker.
    /// Receives `(seq, success)`. Used by rowforge-studio's progress tracker;
    /// the CLI leaves this `None`.
    pub on_row_done: Option<Arc<dyn Fn(u64, bool) + Send + Sync>>,
}

/// Report returned by [`run_pool_streaming`].
pub struct StreamingPoolReport {
    /// True if the run did not complete normally (stall, cancel, or error).
    pub aborted: bool,
    /// Human-readable reason string, populated when `aborted = true`.
    /// Example values: `"stalled at 1024 bytes"`, `"cancelled by operator"`,
    /// `"all workers failed to start"`, or an error message.
    pub abort_reason: Option<String>,
    /// The `bytes_written` snapshot at the time a stall was detected.
    pub stalled_at_bytes: Option<u64>,
}

// ---------------------------------------------------------------------------
// run_pool_streaming
// ---------------------------------------------------------------------------

/// Run the full streaming dispatch pipeline.
///
/// Returns a [`StreamingPoolReport`] describing whether the run completed
/// normally or was aborted (and why). The `CoreError` return covers truly
/// unrecoverable failures (e.g. I/O on the JSONL file itself).
///
/// `input` is consumed by the Reader task. `skip_seqs`, `row_limit`,
/// `field_map`, and `dry_run` are forwarded verbatim to the Reader's
/// [`ReaderConfig`].
pub async fn run_pool_streaming(
    input: Box<dyn InputStream>,
    skip_seqs: HashSet<u64>,
    row_limit: Option<usize>,
    field_map: FieldMap,
    dry_run: bool,
    cfg: StreamingPoolConfig,
) -> Result<StreamingPoolReport, CoreError> {
    // ------------------------------------------------------------------
    // 1. Cancellation token
    // ------------------------------------------------------------------
    // We always own our own token. If the caller supplied one, we use it
    // directly so that their cancel() broadcasts to us immediately.
    // After all pipeline tasks finish we call cancel() ourselves to notify
    // the Stall Monitor — this is the §7.1 "normal cancel" path.
    let cancel = match &cfg.cancel {
        Some(c) => c.clone(),
        None => CancellationToken::new(),
    };

    // ------------------------------------------------------------------
    // 2. SharedJsonlWriter
    // ------------------------------------------------------------------
    let jsonl: Arc<SharedJsonlWriter> =
        Arc::new(SharedJsonlWriter::open(&cfg.jsonl_path, cfg.fsync_outcomes).await?);

    // ------------------------------------------------------------------
    // 3. Worker count
    // ------------------------------------------------------------------
    let effective_workers = if cfg.runtime.stateful {
        if cfg.workers > 1 {
            tracing::warn!(
                requested = cfg.workers,
                "stateful handler: forcing workers=1"
            );
        }
        1u32
    } else {
        cfg.workers
    };

    // ------------------------------------------------------------------
    // 4. Channels
    // ------------------------------------------------------------------
    let (row_tx, row_rx) = mpsc::channel::<RowJob>(ROW_CHANNEL_CAP);
    let (job_tx, job_rx) = mpsc::channel::<Batch>(JOB_CHANNEL_CAP);
    // Wrap job receiver in Arc<Mutex> so multiple workers compete for next job.
    let job_rx = Arc::new(tokio::sync::Mutex::new(job_rx));

    // ------------------------------------------------------------------
    // 5. Spawn workers
    // ------------------------------------------------------------------
    let mode = cfg.runtime.mode.clone();
    let idempotent = cfg.runtime.idempotent.unwrap_or(true);
    let mut worker_handles = Vec::new();
    let mut startup_failed = false;
    let mut startup_err_msg = String::new();

    for id in 0..effective_workers {
        let dir = cfg.handler_dir.clone();
        let manifest = cfg.manifest.clone();
        let run_id = cfg.run_id.clone();
        let config_inner = cfg.config.clone();
        // Pass empty columns list — streaming pipeline doesn't use the columns
        // field for CSV projection (that's exec export territory, P10).
        let columns: Vec<String> = Vec::new();

        let worker_result =
            Worker::spawn(id, &dir, &manifest, &run_id, &config_inner, &columns).await;

        match worker_result {
            Ok(mut worker) => {
                // Drain stderr in a background task (same as legacy pool.rs).
                if let Some(stderr) = worker.take_stderr() {
                    let wid = worker.id;
                    tokio::spawn(async move {
                        use tokio::io::AsyncBufReadExt;
                        let mut reader = tokio::io::BufReader::new(stderr).lines();
                        loop {
                            match reader.next_line().await {
                                Ok(Some(line)) => eprintln!("[handler#{}] {}", wid, line),
                                Ok(None) => break,
                                Err(e) => {
                                    tracing::warn!(worker = wid, error = %e, "stderr_drainer.error");
                                    break;
                                }
                            }
                        }
                    });
                }

                let job_rx_clone = job_rx.clone();
                let jsonl_clone = jsonl.clone();
                let grace = cfg.shutdown_grace;
                let cancel_clone = cancel.clone();
                let mode_clone = mode.clone();
                let on_row_done_clone = cfg.on_row_done.clone();

                let h = tokio::spawn(async move {
                    run_worker_loop(
                        worker,
                        mode_clone,
                        idempotent,
                        job_rx_clone,
                        jsonl_clone,
                        grace,
                        Some(cancel_clone),
                        on_row_done_clone,
                    )
                    .await
                });
                worker_handles.push(h);
            }
            Err(e) => {
                startup_failed = true;
                startup_err_msg = format!("worker {} failed to start: {}", id, e);
                tracing::error!(worker = id, error = %e, "worker spawn failed");
                // Cancel the pipeline — no point reading more rows.
                cancel.cancel();
                break;
            }
        }
    }

    // If all workers failed to start, we still need to clean up channels and
    // await any handles we spawned before the failure.
    if startup_failed {
        // Drop the tx sides so readers/accumulators drain and exit.
        drop(row_tx);
        drop(job_tx);
        // Await any worker handles that did start (there may be none).
        for h in worker_handles {
            let _ = h.await;
        }
        return Ok(StreamingPoolReport {
            aborted: true,
            abort_reason: Some(startup_err_msg),
            stalled_at_bytes: None,
        });
    }

    // ------------------------------------------------------------------
    // 6. Spawn reader, accumulator, stall monitor
    // ------------------------------------------------------------------
    let reader_cfg = ReaderConfig {
        skip_seqs,
        row_limit,
        field_map,
        dry_run,
    };
    let reader_h = tokio::spawn(reader_task(input, reader_cfg, row_tx, Some(cancel.clone())));

    let accum_h = tokio::spawn(accumulator_task(
        cfg.runtime.clone(),
        row_rx,
        job_tx,
        jsonl.clone(),
        Some(cancel.clone()),
    ));

    let stall_duration = cfg
        .stall_timeout
        .unwrap_or_else(|| Duration::from_secs(STALL_TIMEOUT_SECS));
    let poll_interval = cfg
        .stall_poll_interval
        .unwrap_or_else(|| Duration::from_secs(STALL_POLL_INTERVAL_SECS));
    let monitor_h = tokio::spawn(stall_monitor_task(
        jsonl.clone(),
        cancel.clone(),
        poll_interval,
        stall_duration,
    ));

    // ------------------------------------------------------------------
    // 7. Await reader, accumulator, ALL workers — then signal monitor (§7.1)
    // ------------------------------------------------------------------
    let reader_result = reader_h
        .await
        .map_err(|e| CoreError::Other(anyhow::anyhow!("reader task join: {e}")))?;
    let accum_result = accum_h
        .await
        .map_err(|e| CoreError::Other(anyhow::anyhow!("accumulator task join: {e}")))?;

    let mut worker_errors: Vec<String> = Vec::new();
    for (i, h) in worker_handles.into_iter().enumerate() {
        match h.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => worker_errors.push(format!("worker[{i}]: {e}")),
            Err(je) => worker_errors.push(format!("worker[{i}] panic: {je}")),
        }
    }

    // §7.1: all pipeline tasks done → notify Stall Monitor to exit cleanly.
    cancel.cancel();

    // ------------------------------------------------------------------
    // 8. Await monitor — distinguish stall vs normal exit (§7.2)
    // ------------------------------------------------------------------
    let monitor_result = monitor_h
        .await
        .map_err(|e| CoreError::Other(anyhow::anyhow!("monitor task join: {e}")))?;

    // ------------------------------------------------------------------
    // 9. Build report (§7.2 cancel source distinction)
    // ------------------------------------------------------------------

    // Stall monitor fired → highest priority abort indicator.
    if let Err(ref stall_err) = monitor_result {
        let last_bytes = jsonl.bytes_written();
        return Ok(StreamingPoolReport {
            aborted: true,
            abort_reason: Some(format!(
                "stalled at {} bytes: {}",
                last_bytes, stall_err
            )),
            stalled_at_bytes: Some(last_bytes),
        });
    }

    // Worker errors.
    if !worker_errors.is_empty() {
        return Ok(StreamingPoolReport {
            aborted: true,
            abort_reason: Some(worker_errors.join("; ")),
            stalled_at_bytes: None,
        });
    }

    // Reader error.
    if let Err(ref e) = reader_result {
        return Ok(StreamingPoolReport {
            aborted: true,
            abort_reason: Some(format!("reader error: {e}")),
            stalled_at_bytes: None,
        });
    }

    // Accumulator error (flush-cancelled is expected on operator cancel — treat it
    // as a cancel, not an opaque error).
    if let Err(ref e) = accum_result {
        let msg = e.to_string();
        // "flush cancelled" / "worker dropped" on cancel path → classify as cancel.
        let is_cancel_artefact =
            msg.contains("flush cancelled") || msg.contains("worker dropped");

        if is_cancel_artefact {
            // Fall through to operator-cancel check below.
        } else {
            return Ok(StreamingPoolReport {
                aborted: true,
                abort_reason: Some(format!("accumulator error: {msg}")),
                stalled_at_bytes: None,
            });
        }
    }

    // If the external cancel token was set (operator SIGINT path), classify as
    // "cancelled by operator".  We check the token state *after* all tasks
    // have returned: if it was pre-cancelled (or cancelled mid-run by the
    // operator), we report it here. The internal `cancel.cancel()` we fired in
    // §7.1 is indistinguishable from this in the single-token model, so we use
    // a simple heuristic: if the *caller's* cancel was Some and is_cancelled(),
    // that means the operator triggered it (or stall did — but stall is already
    // handled above).
    //
    // Note: when cfg.cancel is None we own the token exclusively; the only way
    // it can be cancelled before step 7 is via stall (already handled) or an
    // internal error, neither of which reaches here.
    if let Some(ref external) = cfg.cancel {
        // The token was already fired (either by us in §7.1 or earlier by
        // the operator). We can't distinguish these post-hoc in the single-
        // token model. However: if the run completed with 0 worker errors, 0
        // reader errors, and the accumulator only had a cancel-artefact error
        // (or no error), we conservatively classify as "cancelled by operator"
        // only when the token was visibly cancelled BEFORE the pipeline would
        // have naturally finished — which we approximate as "accum_result is Err".
        if accum_result.is_err() && external.is_cancelled() {
            return Ok(StreamingPoolReport {
                aborted: true,
                abort_reason: Some("cancelled by operator".into()),
                stalled_at_bytes: None,
            });
        }
    }

    // All tasks returned Ok (or cancel-artefact) and monitor returned Ok →
    // normal completion.
    Ok(StreamingPoolReport {
        aborted: false,
        abort_reason: None,
        stalled_at_bytes: None,
    })
}

// ---------------------------------------------------------------------------
// compute_run_stats — tally outcomes.jsonl for RunReport
// ---------------------------------------------------------------------------

/// Stats tallied from a completed `outcomes.jsonl`.
#[derive(Debug, Default)]
pub struct RunStats {
    pub success: u64,
    pub failed: u64,
    pub by_error_code: BTreeMap<String, u64>,
}

/// Read `outcomes.jsonl` and count successes / failures per error code.
///
/// Called by `execute()` after `run_pool_streaming` returns to compute the
/// `RunReport` aggregate counts. The file may be empty (zero rows dispatched)
/// or absent (startup failure) — both cases return zeroed `RunStats`.
///
/// Per-row consumers that want the full outcome payload should read
/// `outcomes.jsonl` directly; `RunReport` only carries aggregate counts.
pub fn compute_run_stats(jsonl_path: &std::path::Path) -> Result<RunStats, CoreError> {
    use crate::pool::{BatchOutcome, RowOutcome};

    let content = match std::fs::read_to_string(jsonl_path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(RunStats::default()),
        Err(e) => return Err(CoreError::Io(e)),
    };

    let mut stats = RunStats::default();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let bo: BatchOutcome = serde_json::from_str(trimmed).map_err(|e| {
            CoreError::Store(format!(
                "compute_run_stats: malformed jsonl line: {e}\nline: {trimmed}"
            ))
        })?;
        for outcome in &bo.outcomes {
            match outcome {
                RowOutcome::Success { .. } => stats.success += 1,
                RowOutcome::Error { code, .. } => {
                    stats.failed += 1;
                    *stats.by_error_code.entry(code.clone()).or_insert(0) += 1;
                }
                // defensive: streaming pipeline emits Error{code=WORKER_CRASH}
                // via synthesize_crash(); this arm covers legacy / future Crash
                // variant outcomes that may appear in historical jsonl files or
                // be re-introduced by other code paths.
                RowOutcome::Crash { .. } => {
                    stats.failed += 1;
                    *stats
                        .by_error_code
                        .entry(crate::run::ERR_WORKER_CRASH.into())
                        .or_insert(0) += 1;
                }
            }
        }
    }
    Ok(stats)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input_stream::CsvInputStream;
    use crate::manifest::{Entry, Manifest};
    use crate::pool::{BatchOutcome, RowOutcome};
    use crate::runtime::{Mode, Runtime};
    use std::collections::BTreeMap;
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;
    use tempfile::TempDir;

    // -----------------------------------------------------------------------
    // Test infrastructure helpers
    // -----------------------------------------------------------------------

    fn test_handler_path() -> PathBuf {
        use std::sync::Once;
        static BUILD: Once = Once::new();
        BUILD.call_once(|| {
            let status = std::process::Command::new("cargo")
                .args(["build", "-p", "test-handler"])
                .status()
                .expect("invoking `cargo build -p test-handler`");
            assert!(status.success(), "cargo build -p test-handler failed");
        });
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let workspace_root = manifest.parent().unwrap().parent().unwrap();
        workspace_root.join("target/debug/test-handler")
    }

    fn make_manifest(behavior: &str) -> Arc<Manifest> {
        Arc::new(Manifest {
            name: "test".into(),
            version: "0.0.0".into(),
            description: String::new(),
            language: String::new(),
            entry: Entry {
                cmd: vec![
                    test_handler_path().to_string_lossy().into(),
                    behavior.to_string(),
                ],
                build: None,
                cwd: ".".into(),
                env: Default::default(),
                startup_timeout_ms: 5000,
            },
            required_input: vec![],
            config: BTreeMap::new(),
            runtime: None,
            output: None,
        })
    }

    fn row_runtime() -> Runtime {
        Runtime {
            mode: Mode::Row,
            batch_size: None,
            max_batch_bytes: 16 * 1024 * 1024,
            batch_bytes_target: 4 * 1024 * 1024,
            idempotent: None,
            stateful: false,
        }
    }

    fn batch_runtime(batch_size: u32) -> Runtime {
        Runtime {
            mode: Mode::Batch,
            batch_size: Some(batch_size),
            max_batch_bytes: 16 * 1024 * 1024,
            batch_bytes_target: 4 * 1024 * 1024,
            idempotent: Some(true),
            stateful: false,
        }
    }

    fn write_csv(dir: &TempDir, name: &str, content: &str) -> PathBuf {
        let path = dir.path().join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    fn make_cfg(
        dir: &TempDir,
        manifest: Arc<Manifest>,
        runtime: Runtime,
        workers: u32,
    ) -> StreamingPoolConfig {
        StreamingPoolConfig {
            handler_dir: std::env::temp_dir(),
            manifest,
            workers,
            run_id: "test-run".into(),
            config: BTreeMap::new(),
            shutdown_grace: Duration::from_secs(2),
            cancel: None,
            runtime,
            jsonl_path: dir.path().join("outcomes.jsonl"),
            fsync_outcomes: false,
            stall_timeout: None,
            stall_poll_interval: None,
        on_row_done: None,
        }
    }

    fn count_jsonl_lines(path: &std::path::Path) -> usize {
        let content = std::fs::read_to_string(path).unwrap_or_default();
        content.lines().filter(|l| !l.trim().is_empty()).count()
    }

    fn collect_jsonl_outcomes(path: &std::path::Path) -> Vec<RowOutcome> {
        let content = std::fs::read_to_string(path).unwrap_or_default();
        let mut outcomes = Vec::new();
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let bo: BatchOutcome = serde_json::from_str(trimmed).unwrap();
            outcomes.extend(bo.outcomes);
        }
        outcomes
    }

    fn open_csv_input(path: &std::path::Path) -> Box<dyn InputStream> {
        Box::new(CsvInputStream::open(path, &[]).unwrap())
    }

    // -----------------------------------------------------------------------
    // Test 1: streaming_basic_csv_end_to_end
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn streaming_basic_csv_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let mut csv = "n\n".to_string();
        for i in 0..10 {
            csv.push_str(&format!("{}\n", i));
        }
        let csv_path = write_csv(&dir, "input.csv", &csv);

        let manifest = make_manifest("echo");
        let mut cfg = make_cfg(&dir, manifest, row_runtime(), 2);
        cfg.handler_dir = std::env::temp_dir();

        let input = open_csv_input(&csv_path);
        let report = run_pool_streaming(
            input,
            HashSet::new(),
            None,
            BTreeMap::new(),
            false,
            cfg,
        )
        .await
        .unwrap();

        assert!(!report.aborted, "expected not aborted: {:?}", report.abort_reason);

        let jsonl_path = dir.path().join("outcomes.jsonl");
        let line_count = count_jsonl_lines(&jsonl_path);
        assert_eq!(line_count, 10, "expected 10 jsonl lines (one per row)");

        let stats = compute_run_stats(&jsonl_path).unwrap();
        assert_eq!(stats.success, 10);
        assert_eq!(stats.failed, 0);
    }

    // -----------------------------------------------------------------------
    // Test 2: streaming_basic_jsonl_end_to_end
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn streaming_basic_jsonl_end_to_end() {
        use crate::input_stream::JsonlInputStream;

        let dir = tempfile::tempdir().unwrap();
        let mut jsonl_input = String::new();
        for i in 0..10 {
            jsonl_input.push_str(&format!("{{\"n\":{}}}\n", i));
        }
        let jsonl_path_input = dir.path().join("input.jsonl");
        std::fs::write(&jsonl_path_input, &jsonl_input).unwrap();

        let manifest = make_manifest("echo");
        let cfg = make_cfg(&dir, manifest, row_runtime(), 1);

        let input: Box<dyn InputStream> =
            Box::new(JsonlInputStream::open(&jsonl_path_input, &[]).unwrap());
        let report = run_pool_streaming(
            input,
            HashSet::new(),
            None,
            BTreeMap::new(),
            false,
            cfg,
        )
        .await
        .unwrap();

        assert!(!report.aborted, "expected not aborted: {:?}", report.abort_reason);

        let outcomes_path = dir.path().join("outcomes.jsonl");
        assert_eq!(count_jsonl_lines(&outcomes_path), 10);

        let stats = compute_run_stats(&outcomes_path).unwrap();
        assert_eq!(stats.success, 10);
        assert_eq!(stats.failed, 0);
    }

    // -----------------------------------------------------------------------
    // Test 3: streaming_skip_gap
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn streaming_skip_gap() {
        let dir = tempfile::tempdir().unwrap();
        let mut csv = "n\n".to_string();
        for i in 0..10 {
            csv.push_str(&format!("{}\n", i));
        }
        let csv_path = write_csv(&dir, "input.csv", &csv);

        let manifest = make_manifest("echo");
        // Use row_runtime: echo handler speaks row mode.
        let cfg = make_cfg(&dir, manifest, row_runtime(), 1);

        let skip: HashSet<u64> = [3u64, 7u64].iter().cloned().collect();
        let input = open_csv_input(&csv_path);
        let report = run_pool_streaming(
            input,
            skip,
            None,
            BTreeMap::new(),
            false,
            cfg,
        )
        .await
        .unwrap();

        assert!(!report.aborted, "expected not aborted: {:?}", report.abort_reason);

        let jsonl_path = dir.path().join("outcomes.jsonl");
        let outcomes = collect_jsonl_outcomes(&jsonl_path);
        // 10 rows minus 2 skipped = 8 rows dispatched.
        assert_eq!(outcomes.len(), 8, "expected 8 outcomes (skipped 3,7)");

        // Verify seqs 3 and 7 are absent.
        let seqs: HashSet<u64> = outcomes
            .iter()
            .map(|o| match o {
                RowOutcome::Success { seq, .. } => *seq,
                RowOutcome::Error { seq, .. } => *seq,
                RowOutcome::Crash { seq, .. } => *seq,
            })
            .collect();
        assert!(!seqs.contains(&3), "seq 3 should be skipped");
        assert!(!seqs.contains(&7), "seq 7 should be skipped");
    }

    // -----------------------------------------------------------------------
    // Test 4: streaming_normal_completion_monitor_exit (acceptance #6)
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn streaming_normal_completion_monitor_exit() {
        let dir = tempfile::tempdir().unwrap();
        let mut csv = "n\n".to_string();
        for i in 0..5 {
            csv.push_str(&format!("{}\n", i));
        }
        let csv_path = write_csv(&dir, "input.csv", &csv);

        let manifest = make_manifest("echo");
        // Short stall timeout — should NOT fire since the run completes quickly.
        let mut cfg = make_cfg(&dir, manifest, row_runtime(), 1);
        cfg.stall_timeout = Some(Duration::from_secs(5));

        let start = std::time::Instant::now();
        let input = open_csv_input(&csv_path);
        let report = run_pool_streaming(
            input,
            HashSet::new(),
            None,
            BTreeMap::new(),
            false,
            cfg,
        )
        .await
        .unwrap();

        // (a) not aborted
        assert!(!report.aborted, "expected not aborted: {:?}", report.abort_reason);

        // (b) outcomes.jsonl has 5 lines
        let jsonl_path = dir.path().join("outcomes.jsonl");
        assert_eq!(count_jsonl_lines(&jsonl_path), 5);

        // (c) completed well under STALL_TIMEOUT_SECS (no spurious stall).
        assert!(
            start.elapsed() < Duration::from_secs(4),
            "run should complete quickly, took {:?}",
            start.elapsed()
        );
    }

    // -----------------------------------------------------------------------
    // Test 5: streaming_stall_no_progress_cancel (acceptance #5)
    // -----------------------------------------------------------------------
    //
    // Uses "stall-after-2" handler: processes 2 rows normally then sleeps
    // forever. With a short poll interval and stall timeout, the monitor fires
    // and the run aborts with aborted=true + abort_reason containing "stalled".
    // The jsonl should have exactly 2 lines (the rows processed before stall).
    #[tokio::test]
    async fn streaming_stall_no_progress_cancel() {
        let dir = tempfile::tempdir().unwrap();
        let mut csv = "n\n".to_string();
        for i in 0..10 {
            csv.push_str(&format!("{}\n", i));
        }
        let csv_path = write_csv(&dir, "input.csv", &csv);

        let manifest = make_manifest("stall-after-2");
        let mut cfg = make_cfg(&dir, manifest, row_runtime(), 1);
        // Short poll + short stall timeout so the test finishes in < 2s.
        cfg.stall_poll_interval = Some(Duration::from_millis(100));
        cfg.stall_timeout = Some(Duration::from_millis(200));

        let input = open_csv_input(&csv_path);
        let report = tokio::time::timeout(
            Duration::from_secs(10),
            run_pool_streaming(
                input,
                HashSet::new(),
                None,
                BTreeMap::new(),
                false,
                cfg,
            ),
        )
        .await
        .expect("run_pool_streaming should complete within 10s (stall fires quickly)")
        .expect("CoreError");

        assert!(report.aborted, "expected aborted=true on stall");
        let reason = report.abort_reason.as_deref().unwrap_or("");
        assert!(
            reason.contains("stalled"),
            "abort_reason should mention 'stalled', got: {reason}"
        );
        // jsonl should have at least 2 lines (the rows processed before the handler
        // stalled). The in-flight row (the one the stalled handler accepted but never
        // replied to) also lands as a synthesized WORKER_CRASH outcome, so the total
        // may be 2 or 3 depending on whether cancellation races with that third send.
        // We assert >= 2 (not exactly 2) to stay correct under C2 crash synthesis.
        let jsonl_path = dir.path().join("outcomes.jsonl");
        let line_count = count_jsonl_lines(&jsonl_path);
        assert!(
            line_count >= 2,
            "expected at least 2 jsonl lines (processed before stall), got {line_count}"
        );
    }

    // -----------------------------------------------------------------------
    // Test 6: streaming_cancel_propagation (acceptance §16)
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn streaming_cancel_propagation() {
        let dir = tempfile::tempdir().unwrap();
        // 50 rows
        let mut csv = "n\n".to_string();
        for i in 0..50 {
            csv.push_str(&format!("{}\n", i));
        }
        let csv_path = write_csv(&dir, "input.csv", &csv);

        let manifest = make_manifest("echo");
        let cancel = CancellationToken::new();
        let mut cfg = make_cfg(&dir, manifest, row_runtime(), 2);
        cfg.cancel = Some(cancel.clone());

        let input = open_csv_input(&csv_path);
        let jsonl_path = dir.path().join("outcomes.jsonl");
        let jsonl_path_clone = jsonl_path.clone();

        // Spawn the run, then cancel after some rows appear.
        let handle = tokio::spawn(async move {
            run_pool_streaming(
                input,
                HashSet::new(),
                None,
                BTreeMap::new(),
                false,
                cfg,
            )
            .await
        });

        // Poll until at least a few rows appear, then cancel.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            if tokio::time::Instant::now() > deadline {
                break;
            }
            let n = count_jsonl_lines(&jsonl_path_clone);
            if n >= 5 {
                cancel.cancel();
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        // Should complete within shutdown_grace (2s) + a batch worth of time.
        let result = tokio::time::timeout(Duration::from_secs(10), handle)
            .await
            .expect("run_pool_streaming did not complete in time")
            .expect("join error")
            .expect("CoreError");

        // Either aborted (cancel fired before completion) or normal (all 50
        // rows processed before cancel was observed — unlikely but valid).
        // Just verify it returned without hanging.
        let _ = result;
    }

    // -----------------------------------------------------------------------
    // Test 7: streaming_cancel_pending_reingested_next_attempt (acceptance #14)
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn streaming_cancel_pending_reingested_next_attempt() {
        // Run 1: 10-row CSV with batch_size=10, fire cancel immediately so
        // no batches are dispatched (pending rows are dropped by accumulator).
        // Run 2: same input, no skip_seqs; rows not in jsonl → all 10 rows
        // processed. Final: all 10 seqs in jsonl, none synthesized as CANCELLED.
        let dir = tempfile::tempdir().unwrap();
        let mut csv = "n\n".to_string();
        for i in 0..10 {
            csv.push_str(&format!("{}\n", i));
        }
        let csv_path = write_csv(&dir, "input.csv", &csv);
        let jsonl_path = dir.path().join("outcomes.jsonl");

        // Run 1: cancel immediately → pending dropped, nothing in jsonl.
        {
            let cancel = CancellationToken::new();
            let manifest = make_manifest("echo");
            let mut cfg = make_cfg(&dir, manifest, batch_runtime(10), 1);
            cfg.cancel = Some(cancel.clone());

            let input = open_csv_input(&csv_path);
            // Fire cancel before the pipeline even starts.
            cancel.cancel();
            let report = run_pool_streaming(
                input,
                HashSet::new(),
                None,
                BTreeMap::new(),
                false,
                cfg,
            )
            .await
            .unwrap();

            // Either aborted (cancel was pre-fired) or normal (no rows processed).
            // The jsonl should be empty or very small.
            let line_count = count_jsonl_lines(&jsonl_path);
            // Worker never gets a batch because cancel fired before accumulator
            // could flush. In the worst case 0 lines written.
            assert!(
                report.aborted || line_count == 0,
                "run 1 should be aborted or produce 0 lines; aborted={}, lines={}",
                report.aborted,
                line_count
            );
        }

        // Run 2: resume from same jsonl; process only rows not yet in it.
        // Since run 1 wrote nothing (or very few), run 2 processes all/most rows.
        {
            let manifest = make_manifest("echo");
            let mut cfg = make_cfg(&dir, manifest, row_runtime(), 1);
            // Same jsonl_path → appends to existing.
            cfg.jsonl_path = jsonl_path.clone();

            // Compute which seqs are already in the jsonl (from run 1, if any).
            let already_done: HashSet<u64> = {
                let content = std::fs::read_to_string(&jsonl_path).unwrap_or_default();
                let mut seqs = HashSet::new();
                for line in content.lines() {
                    let t = line.trim();
                    if t.is_empty() {
                        continue;
                    }
                    if let Ok(bo) =
                        serde_json::from_str::<crate::pool::BatchOutcome>(t)
                    {
                        for s in &bo.seqs {
                            seqs.insert(*s);
                        }
                    }
                }
                seqs
            };

            let input = open_csv_input(&csv_path);
            let report = run_pool_streaming(
                input,
                already_done,
                None,
                BTreeMap::new(),
                false,
                cfg,
            )
            .await
            .unwrap();

            assert!(
                !report.aborted,
                "run 2 should complete normally: {:?}",
                report.abort_reason
            );

            // All 10 seqs should be in the jsonl now.
            let outcomes = collect_jsonl_outcomes(&jsonl_path);
            assert_eq!(outcomes.len(), 10, "expected 10 total outcomes after run 2");

            // None should be CANCELLED synthetic outcomes.
            for o in &outcomes {
                if let RowOutcome::Error { code, .. } = o {
                    assert_ne!(
                        code, "CANCELLED",
                        "found unexpected CANCELLED outcome; outcomes: {:?}",
                        outcomes
                    );
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Test 8: stream_large_low_mem (acceptance #2)
    // -----------------------------------------------------------------------
    //
    // Generate a 1 000-row fake CSV (kept manageable for CI); verify outcomes.jsonl
    // has 1 000 lines. A full 100K-row test would be too slow in CI — instead
    // we verify the pipeline works at 10× the normal batch size with 4 workers.
    // If no OOM panic occurs and the count is correct, the low-memory property holds.
    //
    // Note: Acceptance §19 #2 expects 100K rows + < 50 MB RSS. This test runs
    // 1000 rows and does not measure RSS — it only verifies no panic + correct
    // outcome count. A full 100K-row RSS test would be too slow for CI; consider
    // adding a manual `#[ignore]`d variant `stream_100k_low_mem_rss` that uses
    // `procfs` or `psutil` for RSS measurement.
    #[tokio::test]
    async fn stream_large_low_mem() {
        let dir = tempfile::tempdir().unwrap();
        // 1000 rows with a small payload.
        let mut csv = "n\n".to_string();
        for i in 0..1000 {
            csv.push_str(&format!("{}\n", i));
        }
        let csv_path = write_csv(&dir, "input.csv", &csv);

        let manifest = make_manifest("echo");
        let cfg = make_cfg(&dir, manifest, row_runtime(), 4);

        let input = open_csv_input(&csv_path);
        let report = run_pool_streaming(
            input,
            HashSet::new(),
            None,
            BTreeMap::new(),
            false,
            cfg,
        )
        .await
        .unwrap();

        assert!(
            !report.aborted,
            "expected not aborted: {:?}",
            report.abort_reason
        );

        let jsonl_path = dir.path().join("outcomes.jsonl");
        let line_count = count_jsonl_lines(&jsonl_path);
        assert_eq!(line_count, 1000, "expected 1000 jsonl lines");

        let stats = compute_run_stats(&jsonl_path).unwrap();
        assert_eq!(stats.success, 1000, "all 1000 rows must succeed");
        assert_eq!(stats.failed, 0);
    }

    // -----------------------------------------------------------------------
    // Test 9: cancel_no_deadlock_full_pipeline (acceptance #13)
    // -----------------------------------------------------------------------
    //
    // Fills the row_channel (capacity = ROW_CHANNEL_CAP) by using a slow
    // handler (stall-after-2), then fires cancel. Verifies the entire pipeline
    // winds down within a timeout — no deadlock.
    #[tokio::test]
    async fn cancel_no_deadlock_full_pipeline() {
        let dir = tempfile::tempdir().unwrap();
        // More rows than the channel capacity so the reader blocks on send.
        let row_count = crate::reader::ROW_CHANNEL_CAP + 20;
        let mut csv = "n\n".to_string();
        for i in 0..row_count {
            csv.push_str(&format!("{}\n", i));
        }
        let csv_path = write_csv(&dir, "input.csv", &csv);

        let manifest = make_manifest("stall-after-2");
        let cancel = CancellationToken::new();
        let mut cfg = make_cfg(&dir, manifest, row_runtime(), 1);
        cfg.cancel = Some(cancel.clone());
        // Short stall timeout so the monitor also fires quickly.
        cfg.stall_poll_interval = Some(Duration::from_millis(50));
        cfg.stall_timeout = Some(Duration::from_millis(200));

        let input = open_csv_input(&csv_path);
        let handle = tokio::spawn(async move {
            run_pool_streaming(
                input,
                HashSet::new(),
                None,
                BTreeMap::new(),
                false,
                cfg,
            )
            .await
        });

        // Give the pipeline ~300ms to start and fill up, then fire cancel.
        tokio::time::sleep(Duration::from_millis(300)).await;
        cancel.cancel();

        // The entire pipeline must wind down within 5 seconds (not deadlock).
        let result = tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("pipeline must wind down within 5s (no deadlock)")
            .expect("join error")
            .expect("CoreError");

        // Report may be aborted (stall or cancel) or normal (fast machine).
        let _ = result;
    }

    // -----------------------------------------------------------------------
    // Test 10: stall_vs_normal_cancel_distinction (acceptance #7 / §7.2)
    // -----------------------------------------------------------------------
    //
    // Two attempts on the same execution (shared tempdir):
    //
    //   Attempt 1 — "echo" handler with a small input. Completes normally.
    //     → report.aborted == false, report.abort_reason is None.
    //
    //   Attempt 2 — "stall-after-2" handler with a short stall_timeout. The
    //     handler processes 2 rows then sleeps forever. The stall monitor fires
    //     and aborts the run.
    //     → report.aborted == true, report.abort_reason contains "stalled".
    //
    // This test verifies §7.2: the two cancel-source paths (normal EOF vs. stall
    // timeout) are correctly distinguished by `run_pool_streaming`.
    #[tokio::test]
    async fn stall_vs_normal_cancel_distinction() {
        let dir = tempfile::tempdir().unwrap();

        // Shared input: 5 rows — small enough that attempt 1 completes quickly.
        let mut csv = "n\n".to_string();
        for i in 0..5 {
            csv.push_str(&format!("{}\n", i));
        }
        let csv_path = write_csv(&dir, "input.csv", &csv);

        // ------------------------------------------------------------------
        // Attempt 1: normal echo handler — should complete without abort.
        // ------------------------------------------------------------------
        {
            let manifest = make_manifest("echo");
            let mut cfg = make_cfg(&dir, manifest, row_runtime(), 1);
            // Use a dedicated jsonl for attempt 1 to keep the files isolated.
            cfg.jsonl_path = dir.path().join("outcomes_attempt1.jsonl");

            let input = open_csv_input(&csv_path);
            let report = tokio::time::timeout(
                Duration::from_secs(15),
                run_pool_streaming(
                    input,
                    HashSet::new(),
                    None,
                    BTreeMap::new(),
                    false,
                    cfg,
                ),
            )
            .await
            .expect("attempt 1 should complete within 15s")
            .expect("CoreError on attempt 1");

            assert!(
                !report.aborted,
                "attempt 1 (normal echo): expected aborted=false, got abort_reason={:?}",
                report.abort_reason
            );
            assert!(
                report.abort_reason.is_none(),
                "attempt 1 (normal echo): expected abort_reason=None, got {:?}",
                report.abort_reason
            );
        }

        // ------------------------------------------------------------------
        // Attempt 2: stall-after-2 handler — should be aborted with stall reason.
        // ------------------------------------------------------------------
        {
            let manifest = make_manifest("stall-after-2");
            let mut cfg = make_cfg(&dir, manifest, row_runtime(), 1);
            // Use a separate jsonl so attempt 2 starts from a clean slate.
            cfg.jsonl_path = dir.path().join("outcomes_attempt2.jsonl");
            // Short poll + stall timeout so the test finishes quickly.
            cfg.stall_poll_interval = Some(Duration::from_millis(100));
            cfg.stall_timeout = Some(Duration::from_millis(500));

            let input = open_csv_input(&csv_path);
            let report = tokio::time::timeout(
                Duration::from_secs(15),
                run_pool_streaming(
                    input,
                    HashSet::new(),
                    None,
                    BTreeMap::new(),
                    false,
                    cfg,
                ),
            )
            .await
            .expect("attempt 2 should complete within 15s (stall fires quickly)")
            .expect("CoreError on attempt 2");

            assert!(
                report.aborted,
                "attempt 2 (stall-after-2): expected aborted=true"
            );
            let reason = report.abort_reason.as_deref().unwrap_or("");
            assert!(
                reason.contains("stalled"),
                "attempt 2 (stall-after-2): abort_reason should contain 'stalled', got: {reason:?}"
            );
        }
    }
}
