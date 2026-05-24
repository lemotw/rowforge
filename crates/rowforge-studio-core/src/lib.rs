//! rowforge-studio-core — GUI-only extension of rowforge-core.
//!
//! See `docs/spec/studio/part-1-overview.md` for principles and
//! `docs/spec/studio/part-5-api.md` for the public surface.

pub mod attempt_detail;
pub mod cache;
pub mod error;
pub mod events;
pub mod exec_detail;
pub mod exec_view;
pub mod failed;
pub mod ids;
pub mod rollup;
pub mod row_history;
pub mod run_handle;
pub mod settings;
pub mod workspace;

use crate::cache::{Cache, ExecListKey, DEFAULT_TTL};

pub use attempt_detail::{AttemptDetail, AttemptPaths, HandlerInstanceView};
pub use error::UiError;
pub use events::{AbortReason, Phase, ProgressEvent, RunReport, WorkerCrashRecord};
pub use exec_detail::{AttemptSummary, ExecDetail, FieldMapping, HandlerBindingView, InputFormat};
pub use exec_view::{AttemptCountsStub, ExecSummary, ListFilter};
pub use failed::{FailedPageQuery, FailedRow, FailedRowPage, RowOutcomeKind};
pub use ids::{AttemptId, ExecutionId};
pub use row_history::RowHistory;
pub use rollup::ExecRollup;
pub use run_handle::{CancelMode, RunHandle, RunStatus};
pub use settings::Settings;
pub use workspace::{OpenOpts, Workspace};

/// Top-level handle returned by `StudioCore::open`.
///
/// Plan 1 ships only `open` and `list`. Later plans add `show`, `attempt`,
/// `start_run`, `cancel`, `subscribe`, `start_exec`, `export`, plus the
/// handler-authoring surface (Part 8).
pub struct StudioCore {
    workspace: Workspace,
    store: rowforge_core::execution_store::ExecutionStore,
    exec_list_cache: Cache<ExecListKey, Vec<ExecSummary>>,
}

impl StudioCore {
    /// Open a workspace. If `opts.workspace` is None, falls back to
    /// `rowforge_core::workspace::default_workspace_root()`.
    pub fn open(opts: OpenOpts) -> Result<Self, UiError> {
        let root = match opts.workspace {
            Some(p) => p,
            None => rowforge_core::workspace::default_workspace_root()
                .ok_or_else(|| {
                    UiError::WorkspaceLocked(
                        "no home directory available".into(),
                    )
                })?,
        };
        let store = rowforge_core::execution_store::ExecutionStore::open(&root)
            .map_err(|e| UiError::WorkspaceLocked(e.to_string()))?;
        let workspace = Workspace {
            root,
            schema_version: store.schema_version(),
        };
        Ok(Self {
            workspace,
            store,
            exec_list_cache: Cache::new(DEFAULT_TTL),
        })
    }

    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    /// Return detail for a single execution by id.
    ///
    /// Returns `UiError::NotFound` if no execution with that id exists.
    pub fn show(&self, id: &ExecutionId) -> Result<ExecDetail, UiError> {
        use crate::exec_detail::{AttemptSummary, HandlerBindingView, InputFormat};

        let exec = self
            .store
            .get_execution(id.as_str())
            .map_err(|e| UiError::Internal(e.to_string()))?
            .ok_or_else(|| UiError::NotFound(format!("execution {} not found", id)))?;

        let summary = ExecSummary::from_execution(&exec, &self.store)
            .map_err(|e| UiError::Internal(e.to_string()))?;

        let attempts_raw = self
            .store
            .list_attempts_for_execution(id.as_str())
            .map_err(|e| UiError::Internal(e.to_string()))?;

        let attempts: Vec<AttemptSummary> = attempts_raw
            .into_iter()
            .map(|a| AttemptSummary {
                id: AttemptId::new(a.id),
                state: serde_json::to_value(&a.state)
                    .ok()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_else(|| format!("{:?}", a.state).to_lowercase()),
                started_at: a.started_at,
                finished_at: a.ended_at,
                run_type: serde_json::to_value(&a.run_type)
                    .ok()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_else(|| format!("{:?}", a.run_type).to_lowercase()),
                stats: None, // backfilled in attempt() detail call (Task 9)
            })
            .collect();

        Ok(ExecDetail {
            summary,
            input_path_snapshot: exec.dir.join("input.csv"),
            input_format: InputFormat::Csv,
            handler_binding: HandlerBindingView {
                handler_id: None,
                handler_instance_id: exec.current_handler_instance_id.clone(),
                version: None,
            },
            attempts,
            field_mapping: None,
            config_overrides: Default::default(),
        })
    }

    /// Return detail for a single attempt.
    ///
    /// Returns `UiError::NotFound` if the execution or attempt does not exist.
    /// meta.json is read best-effort; missing/malformed → zero counts.
    pub fn attempt(
        &self,
        e: &ExecutionId,
        r: &AttemptId,
    ) -> Result<AttemptDetail, UiError> {
        use crate::attempt_detail::{AttemptPaths, HandlerInstanceView};

        let exec = self
            .store
            .get_execution(e.as_str())
            .map_err(|err| UiError::Internal(err.to_string()))?
            .ok_or_else(|| UiError::NotFound(format!("execution {} not found", e)))?;

        let attempts = self
            .store
            .list_attempts_for_execution(e.as_str())
            .map_err(|err| UiError::Internal(err.to_string()))?;

        let attempt = attempts
            .into_iter()
            .find(|a| a.id == r.as_str())
            .ok_or_else(|| UiError::NotFound(format!("attempt {} not found", r)))?;

        let attempt_dir = exec.dir.join("attempts").join(&attempt.id);
        let meta_path = attempt_dir.join("meta.json");
        let (stats, by_error_code) = read_meta_full(&meta_path).unwrap_or_default();

        let state_str = serde_json::to_value(&attempt.state)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| format!("{:?}", attempt.state).to_lowercase());
        let is_terminal =
            matches!(state_str.as_str(), "done" | "completed" | "aborted" | "crashed");

        let run_type_str = serde_json::to_value(&attempt.run_type)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| format!("{:?}", attempt.run_type).to_lowercase());

        Ok(AttemptDetail {
            id: AttemptId::new(attempt.id),
            execution_id: e.clone(),
            state: state_str,
            run_type: run_type_str,
            started_at: attempt.started_at,
            finished_at: attempt.ended_at,
            stats,
            by_error_code,
            handler_instance: HandlerInstanceView {
                id: exec.current_handler_instance_id.clone(),
                handler_id: None,
                version: None,
            },
            paths: AttemptPaths {
                meta_json: meta_path,
                outcomes_jsonl: attempt_dir.join("outcomes.jsonl"),
                handler_stderr_log: attempt_dir.join("handler.stderr.log"),
            },
            is_terminal,
        })
    }

    /// Return a cold rollup of row-resolution counts for an execution.
    ///
    /// Uses the full `compute_resolution` path (not counts_only) because
    /// `by_error_code` is a sibling field on `RowResolution`, not inside
    /// `ResolutionCounts`. See task T10 context.
    pub fn rollup(&self, id: &ExecutionId) -> Result<ExecRollup, UiError> {
        // Validate existence first to return a clean NotFound.
        let _exec = self
            .store
            .get_execution(id.as_str())
            .map_err(|e| UiError::Internal(e.to_string()))?
            .ok_or_else(|| UiError::NotFound(format!("execution {} not found", id)))?;

        // Call the full compute_resolution because we need by_error_code (which
        // is a sibling field, not inside ResolutionCounts).
        let res = rowforge_core::row_resolution::compute_resolution(
            &self.store,
            id.as_str(),
        )
        .map_err(|e| UiError::Internal(e.to_string()))?;

        Ok(ExecRollup {
            resolved: res.counts.resolved,
            failed_last: res.counts.failed_last,
            crashed_last: res.counts.crashed_last,
            cancelled_last: res.counts.cancelled_last,
            too_large: res.counts.too_large,
            never_attempted: res.counts.never_attempted,
            by_error_code: res.by_error_code,
        })
    }

    /// Return a paged list of failed rows for one attempt.
    ///
    /// Reads `outcomes.jsonl` linearly, collecting `error` and `crash` rows.
    /// Pagination is cursor-based: `query.offset` is the count of failed rows
    /// to skip; `next_offset` in the response is the resume cursor.
    ///
    /// Returns `UiError::NotFound` when the execution or attempt does not
    /// exist, or when `outcomes.jsonl` has not been created yet.
    pub fn failed_page(&self, q: FailedPageQuery) -> Result<FailedRowPage, UiError> {
        let exec = self
            .store
            .get_execution(q.execution_id.as_str())
            .map_err(|e| UiError::Internal(e.to_string()))?
            .ok_or_else(|| {
                UiError::NotFound(format!("execution {} not found", q.execution_id))
            })?;

        let outcomes = exec
            .dir
            .join("attempts")
            .join(q.attempt_id.as_str())
            .join("outcomes.jsonl");

        if !outcomes.exists() {
            return Err(UiError::NotFound(format!(
                "attempt {} has no outcomes.jsonl",
                q.attempt_id
            )));
        }

        crate::failed::read_failed_page(&outcomes, &q)
            .map_err(|e| UiError::Io(e.to_string()))
    }

    /// Return the per-attempt history of a single row identified by `seq`.
    ///
    /// Walks all attempts for the execution in order; for each attempt reads
    /// `outcomes.jsonl` to find the outcome for `seq`. Failure outcomes are
    /// accumulated in `rows`; the first Success short-circuits and sets
    /// `resolved_at`.
    pub fn row_history(&self, e: &ExecutionId, seq: u64) -> Result<RowHistory, UiError> {
        let exec = self
            .store
            .get_execution(e.as_str())
            .map_err(|err| UiError::Internal(err.to_string()))?
            .ok_or_else(|| UiError::NotFound(format!("execution {} not found", e)))?;

        let attempts = self
            .store
            .list_attempts_for_execution(e.as_str())
            .map_err(|err| UiError::Internal(err.to_string()))?;

        let mut rows = Vec::new();
        let mut resolved_at: Option<AttemptId> = None;

        for attempt in attempts {
            let outcomes_path = exec
                .dir
                .join("attempts")
                .join(&attempt.id)
                .join("outcomes.jsonl");
            if !outcomes_path.exists() {
                continue;
            }
            let outcome_for_seq =
                read_outcome_for_seq(&outcomes_path, seq).map_err(UiError::from)?;
            if let Some(kind_and_code) = outcome_for_seq {
                match kind_and_code {
                    OutcomeForSeq::Success => {
                        if resolved_at.is_none() {
                            resolved_at = Some(AttemptId::new(attempt.id.clone()));
                        }
                        // First success short-circuits per-attempt collection.
                        break;
                    }
                    OutcomeForSeq::Failure(kind, code) => {
                        rows.push((AttemptId::new(attempt.id.clone()), kind, code));
                    }
                }
            }
        }

        Ok(RowHistory {
            seq,
            rows,
            resolved_at,
        })
    }

    /// List all executions in this workspace, newest first.
    ///
    /// Uses a warm-tier mtime probe per spec part-4 §4.3: cache is valid
    /// iff the DB file mtime is unchanged AND we are within TTL.
    pub fn list(&self, _filter: ListFilter) -> Result<Vec<ExecSummary>, UiError> {
        let db_path = self.workspace.root.join("executions.db");
        if let Some(cached) = self.exec_list_cache.get_if_fresh(&ExecListKey, &db_path) {
            return Ok(cached);
        }
        let executions = self
            .store
            .list_executions()
            .map_err(|e| UiError::Internal(e.to_string()))?;
        let summaries: Vec<ExecSummary> = executions
            .iter()
            .map(|e| ExecSummary::from_execution(e, &self.store))
            .collect::<Result<_, _>>()
            .map_err(|e: rowforge_core::error::CoreError| UiError::Internal(e.to_string()))?;
        self.exec_list_cache.put(ExecListKey, summaries.clone(), &db_path);
        Ok(summaries)
    }
}

// ---------------------------------------------------------------------------
// row_history helpers
// ---------------------------------------------------------------------------

enum OutcomeForSeq {
    Success,
    Failure(crate::failed::RowOutcomeKind, Option<String>),
}

fn read_outcome_for_seq(
    outcomes_jsonl: &std::path::Path,
    seq: u64,
) -> Result<Option<OutcomeForSeq>, std::io::Error> {
    use std::io::{BufRead, BufReader};

    use crate::failed::RowOutcomeKind;

    let f = std::fs::File::open(outcomes_jsonl)?;
    let reader = BufReader::new(f);

    for line_res in reader.lines() {
        let line = line_res?;
        let v: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue, // skip malformed lines silently
        };
        // Batched: iterate outcomes[] inside the BatchOutcome line.
        let outcomes = v.get("outcomes").and_then(|o| o.as_array());
        let Some(outcomes) = outcomes else {
            continue;
        };
        for outcome in outcomes {
            let s = outcome
                .get("seq")
                .and_then(|s| s.as_u64())
                .unwrap_or(u64::MAX);
            if s != seq {
                continue;
            }
            let kind = outcome
                .get("type")
                .and_then(|t| t.as_str())
                .unwrap_or("");
            return Ok(Some(match kind {
                "success" => OutcomeForSeq::Success,
                "error" => OutcomeForSeq::Failure(
                    RowOutcomeKind::Error,
                    outcome
                        .get("code")
                        .and_then(|c| c.as_str())
                        .map(String::from),
                ),
                "crash" => OutcomeForSeq::Failure(RowOutcomeKind::Crash, None),
                _ => return Ok(None), // unknown type
            }));
        }
    }
    Ok(None)
}

/// Read the full meta.json for an attempt — best-effort.
///
/// Returns `(AttemptCountsStub, by_error_code)` or `None` if the file is
/// absent, unreadable, or malformed.
fn read_meta_full(
    path: &std::path::Path,
) -> Option<(AttemptCountsStub, std::collections::BTreeMap<String, u64>)> {
    let bytes = std::fs::read(path).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let stats = v.get("stats").cloned().unwrap_or_default();
    let counts = AttemptCountsStub {
        success: stats.get("success").and_then(|x| x.as_u64()).unwrap_or(0),
        failed: stats.get("failed").and_then(|x| x.as_u64()).unwrap_or(0),
        crashed: stats.get("crashed").and_then(|x| x.as_u64()).unwrap_or(0),
    };
    let by_code = v
        .get("by_error_code")
        .and_then(|m| m.as_object())
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_u64()?)))
                .collect()
        })
        .unwrap_or_default();
    Some((counts, by_code))
}
