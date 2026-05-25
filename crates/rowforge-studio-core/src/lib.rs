//! rowforge-studio-core — GUI-only extension of rowforge-core.
//!
//! See `docs/spec/studio/part-1-overview.md` for principles and
//! `docs/spec/studio/part-5-api.md` for the public surface.

pub mod aggregator;
pub mod attempt_detail;
pub mod cache;
pub mod error;
pub mod events;
pub mod exec_detail;
pub mod exec_view;
pub mod failed;
pub mod ids;
pub mod manifest;
pub mod rollup;
pub mod row_history;
pub mod run;
pub mod run_handle;
pub mod session;
pub mod settings;
pub mod workspace;

use crate::cache::{Cache, ExecListKey, DEFAULT_TTL};

pub use aggregator::{ProgressAggregator, ProgressSnapshot};
pub use attempt_detail::{AttemptDetail, AttemptPaths, HandlerInstanceView};
pub use error::{BusyScope, UiError};
pub use events::{AbortReason, Phase, ProgressEvent, RunReport, WorkerCrashRecord};
pub use exec_detail::{AttemptSummary, ExecDetail, FieldMapping, HandlerBindingView, InputFormat};
pub use exec_view::{AttemptCountsStub, ExecSummary, ListFilter};
pub use failed::{FailedPageQuery, FailedRow, FailedRowPage, RowOutcomeKind};
pub use ids::{AttemptId, ExecutionId};
pub use manifest::{Manifest, ManifestError, ManifestReport, ManifestSource, ManifestWarning, validate_manifest};
pub use row_history::RowHistory;
pub use rollup::ExecRollup;
pub use run::{RunOpts, RunRollupTick, RunStartedHandle, RunStream};
pub use run_handle::{CancelMode, RunHandle, RunStatus};
pub use session::{BusyReason, Session, SessionRegistry};
pub use settings::Settings;
pub use workspace::{OpenOpts, Workspace};
// Re-export export types so the Tauri shell can import them from this crate
// without needing a direct rowforge-core dependency.
pub use rowforge_core::export::{ExportFormat, ExportOpts, ExportReport, ExportWarning};

// StartExecArgs is defined below (inline in lib.rs) and exported here.
// Re-export is done at the bottom of the `pub use` section for discoverability.

// ---------------------------------------------------------------------------
// StartExecArgs (spec §5.2)
// ---------------------------------------------------------------------------

/// Arguments for `StudioCore::start_exec`.
///
/// `#[non_exhaustive]` so that new optional fields (e.g. field_mapping,
/// config_overrides) can be added without a breaking API change.
///
/// Use `StartExecArgs::new(input_path, name)` to construct; optional fields
/// can be set via the builder-style setters.
#[non_exhaustive]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StartExecArgs {
    /// Local filesystem path to the input file (csv/jsonl/ndjson).
    pub input_path: std::path::PathBuf,
    /// Human-readable name for this execution; must be unique in the workspace.
    pub name: String,
    /// Optional logical CSV id for pre-registered CSVs. Defaults to
    /// `"csv_unregistered"` when absent.
    pub csv_id: Option<String>,
    /// If set, pins the execution to a specific handler instance id.
    pub pinned_handler_instance: Option<String>,
}

impl StartExecArgs {
    /// Construct with required fields; optional fields default to `None`.
    pub fn new(input_path: impl Into<std::path::PathBuf>, name: impl Into<String>) -> Self {
        Self {
            input_path: input_path.into(),
            name: name.into(),
            csv_id: None,
            pinned_handler_instance: None,
        }
    }

    /// Set the logical CSV id.
    pub fn with_csv_id(mut self, id: impl Into<String>) -> Self {
        self.csv_id = Some(id.into());
        self
    }

    /// Pin to a specific handler instance.
    pub fn with_pinned_handler(mut self, id: impl Into<String>) -> Self {
        self.pinned_handler_instance = Some(id.into());
        self
    }
}

// ---------------------------------------------------------------------------
// Orphan recovery (spec §3.7)
// ---------------------------------------------------------------------------

/// Threshold beyond which a non-terminal attempt is considered orphaned.
const ORPHAN_MTIME_THRESHOLD: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// Scan all attempts whose state is `running`; mark those whose
/// `outcomes.jsonl` mtime (or `started_at` when the file is absent)
/// is more than `ORPHAN_MTIME_THRESHOLD` ago as `aborted`.
///
/// Returns the count of attempts marked. Never fails open — callers
/// should warn-and-continue if this returns an error.
fn scan_for_orphans(
    store: &mut rowforge_core::execution_store::ExecutionStore,
    _workspace_root: &std::path::Path,
) -> Result<u32, rowforge_core::error::CoreError> {
    use rowforge_core::execution_store::{AttemptState, FinishAttempt};
    use std::time::SystemTime;

    let executions = store.list_executions()?;
    let mut marked = 0u32;
    let now = SystemTime::now();

    for exec in executions {
        let attempts = store.list_attempts_for_execution(&exec.id)?;
        for attempt in attempts {
            // Only non-terminal (running) attempts need checking.
            if attempt.state != AttemptState::Running {
                continue;
            }

            // Derive staleness from outcomes.jsonl mtime, falling back to
            // started_at when the file has not been written yet.
            let outcomes_path = exec
                .dir
                .join("attempts")
                .join(&attempt.id)
                .join("outcomes.jsonl");

            let stale = match outcomes_path.metadata().and_then(|m| m.modified()) {
                Ok(mtime) => now
                    .duration_since(mtime)
                    .map(|d| d > ORPHAN_MTIME_THRESHOLD)
                    .unwrap_or(false),
                Err(_) => {
                    // File absent — use started_at as the fallback clock.
                    let started_sys = std::time::UNIX_EPOCH
                        + std::time::Duration::from_secs(
                            attempt.started_at.timestamp() as u64,
                        );
                    now.duration_since(started_sys)
                        .map(|d| d > ORPHAN_MTIME_THRESHOLD)
                        .unwrap_or(false)
                }
            };

            if stale {
                store.finish_attempt(
                    &attempt.id,
                    FinishAttempt {
                        success_count: 0,
                        failed_count: 0,
                        aborted: true,
                        aborted_reason: Some("orphaned_on_restart".into()),
                    },
                )?;
                marked += 1;
                tracing::warn!(
                    attempt_id = %attempt.id,
                    execution_id = %exec.id,
                    "marked orphan attempt as aborted (mtime > 5 min)"
                );
            }
        }
    }

    Ok(marked)
}

/// Top-level handle returned by `StudioCore::open`.
///
/// Plan 1 ships only `open` and `list`. Later plans add `show`, `attempt`,
/// `start_run`, `cancel`, `subscribe`, `start_exec`, `export`, plus the
/// handler-authoring surface (Part 8).
pub struct StudioCore {
    workspace: Workspace,
    pub(crate) store: std::sync::Arc<std::sync::Mutex<rowforge_core::execution_store::ExecutionStore>>,
    exec_list_cache: Cache<ExecListKey, Vec<ExecSummary>>,
    pub(crate) sessions: std::sync::Arc<crate::session::SessionRegistry>,
}

impl Drop for StudioCore {
    fn drop(&mut self) {
        // Soft-cancel all active sessions. Spec §3.6.
        //
        // Tauri shutdown hooks handle the actual graceful drain via
        // wait-loops; here we only signal cancellation. Tasks owning the
        // cancel_token will observe cancellation and emit Aborted events
        // before exiting their tokio spawn.
        for handle in self.sessions.handles() {
            if let Some(session) = self.sessions.get(&handle) {
                session.cancel_token.cancel();
                let _ = session.tick_stop.send(true);
            }
        }
    }
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
            root: root.clone(),
            schema_version: store.schema_version(),
        };
        let store = std::sync::Arc::new(std::sync::Mutex::new(store));

        // Orphan recovery: mark stale running attempts as aborted.
        // Never fails open — log and continue on error.
        {
            let mut store_guard = store.lock().unwrap_or_else(|p| p.into_inner());
            if let Err(e) = scan_for_orphans(&mut store_guard, &root) {
                tracing::warn!("orphan scan failed: {e}");
            }
        }

        // Plan 6 T9: workspace_limit sourced from Settings via OpenOpts;
        // per_exec_limit stays hard-coded to spec default (§3.4). The Tauri
        // workspace_open command loads Settings and threads max_concurrent_runs
        // through; studio-core stays filesystem-policy-free.
        let workspace_limit = opts.max_concurrent_runs.unwrap_or(3);
        let sessions = std::sync::Arc::new(crate::session::SessionRegistry::new(workspace_limit, 1));

        Ok(Self {
            workspace,
            store,
            exec_list_cache: Cache::new(DEFAULT_TTL),
            sessions,
        })
    }

    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    /// Return the Arc-wrapped session registry for this workspace.
    ///
    /// Used by the Tauri event bridge to spawn `forward_active_runs` with only
    /// a `SessionRegistry` handle, avoiding the need to hold a `StudioCore`
    /// reference across async task boundaries.
    pub fn sessions(&self) -> std::sync::Arc<crate::session::SessionRegistry> {
        self.sessions.clone()
    }

    /// Return detail for a single execution by id.
    ///
    /// Returns `UiError::NotFound` if no execution with that id exists.
    pub fn show(&self, id: &ExecutionId) -> Result<ExecDetail, UiError> {
        use crate::exec_detail::{AttemptSummary, HandlerBindingView, InputFormat};

        let store = self.store.lock().unwrap_or_else(|p| p.into_inner());

        let exec = store
            .get_execution(id.as_str())
            .map_err(|e| UiError::Internal(e.to_string()))?
            .ok_or_else(|| UiError::NotFound(format!("execution {} not found", id)))?;

        let summary = ExecSummary::from_execution(&exec, &store)
            .map_err(|e| UiError::Internal(e.to_string()))?;

        let attempts_raw = store
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

        let store = self.store.lock().unwrap_or_else(|p| p.into_inner());

        let exec = store
            .get_execution(e.as_str())
            .map_err(|err| UiError::Internal(err.to_string()))?
            .ok_or_else(|| UiError::NotFound(format!("execution {} not found", e)))?;

        let attempts = store
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
        let store = self.store.lock().unwrap_or_else(|p| p.into_inner());

        // Validate existence first to return a clean NotFound.
        let _exec = store
            .get_execution(id.as_str())
            .map_err(|e| UiError::Internal(e.to_string()))?
            .ok_or_else(|| UiError::NotFound(format!("execution {} not found", id)))?;

        // Call the full compute_resolution because we need by_error_code (which
        // is a sibling field, not inside ResolutionCounts).
        let res = rowforge_core::row_resolution::compute_resolution(
            &store,
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
    /// Returns `UiError::NotFound` only when the execution does not exist.
    /// When the attempt's `outcomes.jsonl` is missing (attempt created but
    /// never ran, handshake failed before any outcome, replay-in-progress,
    /// etc.) returns an empty page — UI treats it as "no failed rows yet".
    pub fn failed_page(&self, q: FailedPageQuery) -> Result<FailedRowPage, UiError> {
        let store = self.store.lock().unwrap_or_else(|p| p.into_inner());
        let exec = store
            .get_execution(q.execution_id.as_str())
            .map_err(|e| UiError::Internal(e.to_string()))?
            .ok_or_else(|| {
                UiError::NotFound(format!("execution {} not found", q.execution_id))
            })?;
        drop(store);

        let outcomes = exec
            .dir
            .join("attempts")
            .join(q.attempt_id.as_str())
            .join("outcomes.jsonl");

        if !outcomes.exists() {
            return Ok(FailedRowPage {
                rows: Vec::new(),
                next_offset: None,
                total_known: None,
            });
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
        let store = self.store.lock().unwrap_or_else(|p| p.into_inner());

        let exec = store
            .get_execution(e.as_str())
            .map_err(|err| UiError::Internal(err.to_string()))?
            .ok_or_else(|| UiError::NotFound(format!("execution {} not found", e)))?;

        let attempts = store
            .list_attempts_for_execution(e.as_str())
            .map_err(|err| UiError::Internal(err.to_string()))?;
        drop(store);

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

    /// Validate the `rowforge.yaml` inside `source`.
    ///
    /// Delegates to `rowforge_core::manifest::Manifest::load_from_dir`,
    /// then adds PATH-probing of `entry.cmd[0]` and `entry.build[0]`
    /// for first tokens that aren't path-shaped.
    ///
    /// Returns a structured `ManifestReport`. Errors block exec_start /
    /// run_start; warnings (e.g. PATH miss) are informational.
    pub fn validate_manifest(&self, source: ManifestSource) -> Result<ManifestReport, UiError> {
        Ok(crate::manifest::validate_manifest(&source))
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
        let store = self.store.lock().unwrap_or_else(|p| p.into_inner());
        let executions = store
            .list_executions()
            .map_err(|e| UiError::Internal(e.to_string()))?;
        let summaries: Vec<ExecSummary> = executions
            .iter()
            .map(|e| ExecSummary::from_execution(e, &store))
            .collect::<Result<_, _>>()
            .map_err(|e: rowforge_core::error::CoreError| UiError::Internal(e.to_string()))?;
        drop(store);
        self.exec_list_cache.put(ExecListKey, summaries.clone(), &db_path);
        Ok(summaries)
    }

    /// Export an execution to files.
    ///
    /// Thin wrapper over `rowforge_core::export::export_execution`.
    /// Parses the `export_incomplete:<N>` sentinel from the core into
    /// `UiError::ExportIncomplete { missing_count }` so the React layer can
    /// surface a precise message. All other errors become `UiError::Internal`.
    pub fn export(
        &self,
        id: &ExecutionId,
        opts: rowforge_core::export::ExportOpts,
    ) -> Result<rowforge_core::export::ExportReport, UiError> {
        let store = self.store.lock().unwrap_or_else(|p| p.into_inner());
        match rowforge_core::export::export_execution(&store, id.as_str(), &opts) {
            Ok(report) => Ok(report),
            Err(e) => {
                let msg = e.to_string();
                // CoreError::Store wraps the sentinel as "store: export_incomplete:N"
                let sentinel_haystack = msg
                    .strip_prefix("store: ")
                    .unwrap_or(&msg);
                if let Some(rest) = sentinel_haystack.strip_prefix("export_incomplete:") {
                    let missing: u64 = rest.parse().unwrap_or(0);
                    Err(UiError::ExportIncomplete { missing_count: missing })
                } else {
                    Err(UiError::Internal(msg))
                }
            }
        }
    }

    /// Create a new execution from a local input file.
    ///
    /// Spec §5.2. Does:
    /// 1. Input validation: file must exist and have a csv/jsonl/ndjson extension.
    /// 2. Workspace-scoped duplicate name check via `store.list_executions()`.
    /// 3. Delegates to `rowforge_core::ExecutionStore::create_execution`.
    ///
    /// Returns the new `ExecutionId` on success.
    pub fn start_exec(&self, args: StartExecArgs) -> Result<ExecutionId, UiError> {
        // 1. Input validation — file must exist.
        if !args.input_path.is_file() {
            return Err(UiError::InvalidInput {
                reason: format!(
                    "input not found or not a file: {}",
                    args.input_path.display()
                ),
            });
        }
        // Format sniff by extension.
        let ext = args
            .input_path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        if !matches!(ext.as_deref(), Some("csv") | Some("jsonl") | Some("ndjson")) {
            return Err(UiError::InvalidInput {
                reason: "unsupported input format — must be csv/jsonl/ndjson".into(),
            });
        }

        // 2. Duplicate name check (workspace-scoped).
        // NOTE: list_executions() is the actual method on ExecutionStore — returns
        //       Vec<Execution>, each with an `id: String` and `name: Option<String>`.
        let mut store = self.store.lock().unwrap_or_else(|p| p.into_inner());
        let existing = store
            .list_executions()
            .map_err(|e| UiError::Internal(e.to_string()))?;
        if existing.iter().any(|e| e.name.as_deref() == Some(&args.name)) {
            return Err(UiError::DuplicateExecName { name: args.name });
        }

        // 3. Delegate to core store.
        let new = rowforge_core::execution_store::NewExecution {
            name: Some(args.name.clone()),
            input_csv_id: args
                .csv_id
                .unwrap_or_else(|| "csv_unregistered".into()),
            input_csv_path: args.input_path,
            current_handler_instance_id: args.pinned_handler_instance,
        };
        let exec = store
            .create_execution(new)
            .map_err(|e| UiError::Internal(e.to_string()))?;
        Ok(ExecutionId::new(exec.id))
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

// ---------------------------------------------------------------------------
// T8 unit test — Drop cancels active sessions (spec §3.6)
// ---------------------------------------------------------------------------
//
// Lives here (unit test, not integration test) because it needs access to
// `pub(crate) sessions` on StudioCore. Integration tests in tests/ compile
// the crate without cfg(test) so pub(crate) items are inaccessible there.

#[cfg(test)]
mod drop_tests {
    use super::*;
    use crate::workspace::OpenOpts;
    use crate::run::RunOpts;
    use crate::ids::ExecutionId;

    fn empty_workspace() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let _store = rowforge_core::execution_store::ExecutionStore::open(tmp.path()).unwrap();
        tmp
    }

    /// Build a minimal handler dir with a valid rowforge.yaml whose `cmd`
    /// points to a nonexistent binary. The manifest loads; workers fail to
    /// start → run eventually aborts. Good enough for Drop testing.
    fn minimal_handler_dir(base: &tempfile::TempDir) -> std::path::PathBuf {
        let handler = base.path().join("handler");
        std::fs::create_dir_all(&handler).unwrap();
        std::fs::write(
            handler.join("rowforge.yaml"),
            "name: test-handler\nversion: 0.1.0\nentry:\n  cmd: [\"/nonexistent-binary\"]\n",
        )
        .unwrap();
        handler
    }

    #[tokio::test]
    async fn drop_cancels_active_sessions() {
        let tmp = empty_workspace();
        let csv = tmp.path().join("input.csv");
        std::fs::write(&csv, "x\n1\n").unwrap();
        let handler = minimal_handler_dir(&tmp);

        let exec_id = {
            let mut store =
                rowforge_core::execution_store::ExecutionStore::open(tmp.path()).unwrap();
            store
                .create_execution(rowforge_core::execution_store::NewExecution {
                    name: Some("drop-test".into()),
                    input_csv_id: "csv1".into(),
                    input_csv_path: csv,
                    current_handler_instance_id: None,
                })
                .unwrap()
                .id
        };

        // Open core, start a run, capture the cancel_token via pub(crate) sessions.
        let session_token = {
            let core = StudioCore::open(
                OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
            )
            .unwrap();

            let opts = RunOpts::new(handler);
            let started = core
                .start_run(&ExecutionId::new(exec_id), opts)
                .unwrap();
            let handle = started.handle;

            // Grab the token reference via pub(crate) sessions so we can check
            // after drop.
            let session = core.sessions.get(&handle).unwrap();
            let token = session.cancel_token.clone();
            token
            // `core` drops here at end of block.
        };

        // After drop, the token should be cancelled.
        assert!(
            session_token.is_cancelled(),
            "Drop should have cancelled the active session's token"
        );
    }
}
