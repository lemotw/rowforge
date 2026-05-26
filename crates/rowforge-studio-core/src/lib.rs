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
pub mod handler;
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
pub use handler::{
    HandlerSummary, HandlerDetail, SourceFileSummary,
    ManifestStatus, ScaffoldArgs, ScaffoldTemplate,
};
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
// Re-export build types (Plan 8 T7) so the Tauri shell and ipc_contract tests
// can reference BuildOutcome without a direct rowforge-core dependency.
pub use rowforge_core::build::BuildOutcome;
// Re-export handler log types (Plan 9 T6) so the Tauri shell and ipc_contract
// tests can reference HandlerLogLine without a direct rowforge-core dependency.
pub use rowforge_core::handler_log::HandlerLogLine;

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
// Plan 10 — bulk-delete result types
// ---------------------------------------------------------------------------

/// Outcome of a [`StudioCore::execution_delete_bulk`] call.
///
/// `#[non_exhaustive]` so that future fields (e.g. `skipped`) can be added
/// without a breaking change. Cross-crate code that needs to construct this
/// (e.g. Tauri ipc_contract tests) should round-trip through `serde_json`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct ExecDeleteBulkResult {
    /// Ids of executions that were successfully deleted, in input order.
    pub deleted: Vec<String>,
    /// Per-item failure descriptors for executions that could not be deleted,
    /// in input order relative to the original slice.
    pub failed: Vec<ExecDeleteFailure>,
}

/// A single per-item failure within an [`ExecDeleteBulkResult`].
///
/// `#[non_exhaustive]` for the same future-compatibility reason as the parent.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct ExecDeleteFailure {
    /// The execution id that could not be deleted.
    pub exec_id: String,
    /// Human-readable reason string derived from the underlying `UiError`
    /// (`format!("{}", err)`). Compatible with `uiErrorMessage` in the TS
    /// layer.
    pub reason: String,
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
    /// Plan 7: caller-supplied editor command for handler_open_editor.
    /// Sourced from `OpenOpts.preferred_editor` which the Tauri layer
    /// loads from Settings before calling `open()`. None → resolver
    /// falls through to $VISUAL / $EDITOR / probes.
    preferred_editor: Option<String>,
    /// Plan 9 T5: mirrors `Settings.handler_log_capture_raw_stdout`.
    /// Read once per attempt at start_run time; threaded into
    /// `RunRequest.capture_raw_stdout`. Updated by `set_handler_log_capture_raw_stdout`
    /// after each `workspace_settings_save` in the Tauri layer.
    capture_raw_stdout: bool,
    /// Plan 8 T6: in-memory build cache. Keys are handler names; values are
    /// the most recent BuildOutcome (success OR failure). Dies on Drop.
    /// Lock is held only briefly — never across the subprocess spawn.
    build_cache: std::sync::Mutex<std::collections::HashMap<String, rowforge_core::build::BuildOutcome>>,
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
            // Plan 7 T15: sourced from Settings.preferred_editor via OpenOpts.
            preferred_editor: opts.preferred_editor,
            // Plan 8 T6: empty build cache; populated by handler_build.
            build_cache: std::sync::Mutex::new(std::collections::HashMap::new()),
            // Plan 9 T5: sourced from Settings.handler_log_capture_raw_stdout via OpenOpts.
            capture_raw_stdout: opts.handler_log_capture_raw_stdout,
        })
    }

    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    /// Plan 7 T15: update the preferred editor in-place after a settings_save
    /// so the next handler_open_editor call uses the new value without
    /// requiring a workspace re-open.
    pub fn set_preferred_editor(&mut self, editor: Option<String>) {
        self.preferred_editor = editor;
    }

    /// Plan 9 T5: update the raw-stdout capture flag in-place after a
    /// settings_save so the next start_run call picks up the new value.
    /// Changes don't affect already-running attempts (intentional — the
    /// flag is snapshotted into `RunRequest` at attempt-start).
    pub fn set_handler_log_capture_raw_stdout(&mut self, enabled: bool) {
        self.capture_raw_stdout = enabled;
    }

    /// Return the current value of the raw-stdout capture flag.
    ///
    /// Used by tests and callers that need to verify the flag was applied
    /// without requiring a full run.
    pub fn capture_raw_stdout(&self) -> bool {
        self.capture_raw_stdout
    }

    /// Plan 7 T3: list all handlers under `<workspace>/handlers/`.
    /// Returns empty list when the dir doesn't exist (not an error).
    pub fn handler_list(&self) -> Result<Vec<HandlerSummary>, UiError> {
        crate::handler::list(self.workspace.root.as_path())
    }

    /// Plan 7 T3: load a single handler's detail (manifest + source files).
    /// Injects the cached BuildOutcome (if any) into `detail.last_build`.
    /// Errors: `InvalidHandlerName` (regex fail), `HandlerNotFound` (dir missing).
    pub fn handler_show(&self, name: &str) -> Result<HandlerDetail, UiError> {
        let mut detail = crate::handler::show(self.workspace.root.as_path(), name)?;
        detail.last_build = self.build_cache.lock().unwrap().get(name).cloned();
        Ok(detail)
    }

    /// Plan 8 T6: build a handler by running its `entry.build` command.
    ///
    /// Always force-builds (does not call `needs_build`). On success or on
    /// a `BuildFailed` result, the `BuildOutcome` is written to the in-memory
    /// cache so `handler_show` can surface it. On `ToolchainMissing` the cache
    /// is left untouched (no outcome to show).
    ///
    /// Errors:
    /// - `NoBuildCommand`   — manifest has no `entry.build`
    /// - `BuildFailed`      — process exited non-zero (outcome still cached)
    /// - `ToolchainMissing` — first token of build command not on PATH
    /// - `Io`               — manifest load or spawn failure
    pub fn handler_build(&self, name: &str) -> Result<rowforge_core::build::BuildOutcome, UiError> {
        // Validate name BEFORE any path construction — defense in depth.
        // Prevents out-of-workspace manifest reads from paths like ../etc/passwd.
        if !crate::handler::validate_name(name) {
            return Err(UiError::InvalidHandlerName { name: name.to_string() });
        }
        // Pre-flight: load manifest & check entry.build before invoking
        // build_raw, so we surface NoBuildCommand cleanly.
        let dir = self.workspace.root.as_path().join("handlers").join(name);
        let (manifest, _) = rowforge_core::manifest::Manifest::load_from_dir(&dir)
            .map_err(|e| UiError::Io(format!("manifest load: {}", e)))?;
        if manifest.entry.build.is_none() {
            return Err(UiError::NoBuildCommand { name: name.to_string() });
        }

        // build_raw does the heavy lifting; we own cache + UiError mapping.
        match crate::handler::build_raw(self.workspace.root.as_path(), name) {
            Ok(outcome) => {
                self.build_cache
                    .lock()
                    .unwrap()
                    .insert(name.to_string(), outcome.clone());
                Ok(outcome)
            }
            Err(rowforge_core::build::BuildError::BuildFailed { exit_code, outcome, .. }) => {
                // Cache the failed outcome so the UI can inspect the log.
                self.build_cache
                    .lock()
                    .unwrap()
                    .insert(name.to_string(), outcome);
                Err(UiError::BuildFailed { name: name.to_string(), exit_code })
            }
            Err(rowforge_core::build::BuildError::ToolchainMissing { tool }) => {
                // No outcome to cache; the UI shows a "tool not found" message.
                Err(UiError::ToolchainMissing { name: name.to_string(), tool })
            }
            Err(rowforge_core::build::BuildError::NoBuildCommand) => {
                Err(UiError::NoBuildCommand { name: name.to_string() })
            }
            Err(rowforge_core::build::BuildError::Io(e)) => Err(UiError::Io(e)),
        }
    }

    /// Plan 7 T4: open the handler dir in the user's preferred external editor.
    /// 4-tier resolution per spec 8.4.1. Errors: InvalidHandlerName,
    /// HandlerNotFound, EditorNotFound, InvalidArg (shell-parse failure),
    /// Io (spawn failure).
    pub fn handler_open_editor(&self, name: &str) -> Result<(), UiError> {
        crate::handler::open_editor(
            self.workspace.root.as_path(),
            name,
            self.preferred_editor.as_deref(),
        )
    }

    /// Plan 7 T4: return the handler dir path for the Tauri layer to pass
    /// to `shell::open()`. Errors: InvalidHandlerName, HandlerNotFound.
    pub fn handler_reveal_path(&self, name: &str) -> Result<std::path::PathBuf, UiError> {
        crate::handler::reveal_path(self.workspace.root.as_path(), name)
    }

    /// Plan 7 T6: scaffold a new handler from a template. Errors:
    /// `InvalidHandlerName` (regex fail), `HandlerExists` (destination
    /// taken), `Io` (filesystem write failure).
    pub fn handler_scaffold(&self, args: ScaffoldArgs) -> Result<String, UiError> {
        crate::handler::scaffold(self.workspace.root.as_path(), args)
    }

    /// Plan 7 T7: delete a handler directory. Three-layer defense against
    /// path traversal (regex / canonicalize / starts_with). Errors:
    /// `InvalidHandlerName`, `HandlerNotFound`, `InvalidArg` (resolved
    /// outside workspace), `Io` (filesystem op failure).
    pub fn handler_delete(&self, name: &str) -> Result<(), UiError> {
        crate::handler::delete(self.workspace.root.as_path(), name)
    }

    /// Plan 7 T8: rename a handler directory. Lazy on sqlite — past
    /// `handler_instances.source_snapshot_dir` rows continue to reference
    /// the old path (handler_instance is content-addressed; the path field
    /// is informational). Errors: InvalidHandlerName, HandlerNotFound,
    /// HandlerExists, Io.
    pub fn handler_rename(&self, old: &str, new: &str) -> Result<(), UiError> {
        crate::handler::rename(self.workspace.root.as_path(), old, new)
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

    /// Return the deduplicated, sorted-ascending `Vec<u64>` of `seq` values
    /// from `outcomes.jsonl` where the row outcome type is `"error"` or
    /// `"crash"`. Used by Plan 11's Re-run failed flow.
    ///
    /// The `outcomes.jsonl` format is `BatchOutcome` lines; each line has an
    /// `"outcomes"` array whose elements carry a `"type"` tag (`"error"` /
    /// `"crash"` / `"success"`) and a `"seq"` field (u64).
    ///
    /// # ID validation
    ///
    /// Both `exec_id` and `attempt_id` are validated via `is_valid_id_component`
    /// **before** any path construction — traversal IDs like `"../etc"` are
    /// rejected with `UiError::Io`.
    ///
    /// # File missing
    ///
    /// If `outcomes.jsonl` does not exist, returns `Ok(vec![])`.
    ///
    /// # Malformed lines
    ///
    /// JSON lines that fail to parse are silently skipped.
    pub fn attempt_failed_row_ids(
        &self,
        exec_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<u64>, UiError> {
        // Validate BOTH ids before any path construction.
        if !is_valid_id_component(exec_id) {
            return Err(UiError::Io(format!("invalid exec_id: {}", exec_id)));
        }
        if !is_valid_id_component(attempt_id) {
            return Err(UiError::Io(format!("invalid attempt_id: {}", attempt_id)));
        }

        let outcomes_path = self.workspace.root.as_path()
            .join("executions")
            .join(exec_id)
            .join("attempts")
            .join(attempt_id)
            .join("outcomes.jsonl");

        // File missing → empty (attempt created but not yet run, etc.).
        if !outcomes_path.exists() {
            return Ok(vec![]);
        }

        use std::collections::BTreeSet;
        use std::io::{BufRead, BufReader};

        let f = std::fs::File::open(&outcomes_path)
            .map_err(|e| UiError::Io(e.to_string()))?;
        let reader = BufReader::new(f);

        let mut seqs: BTreeSet<u64> = BTreeSet::new();

        for line_res in reader.lines() {
            let line = match line_res {
                Ok(l) => l,
                Err(_) => continue,
            };
            let v: serde_json::Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue, // skip malformed lines silently
            };
            // Each line is a BatchOutcome: {"outcomes": [...], ...}
            let outcomes = match v.get("outcomes").and_then(|o| o.as_array()) {
                Some(arr) => arr,
                None => continue,
            };
            for outcome in outcomes {
                let kind = outcome
                    .get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("");
                if kind != "error" && kind != "crash" {
                    continue;
                }
                if let Some(seq) = outcome.get("seq").and_then(|s| s.as_u64()) {
                    seqs.insert(seq);
                }
            }
        }

        // BTreeSet already gives sorted, deduplicated output.
        Ok(seqs.into_iter().collect())
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
        let mut summaries: Vec<ExecSummary> = executions
            .iter()
            .map(|e| ExecSummary::from_execution(e, &store))
            .collect::<Result<_, _>>()
            .map_err(|e: rowforge_core::error::CoreError| UiError::Internal(e.to_string()))?;
        drop(store);
        // Populate size_bytes lazily by walking each execution directory.
        let exec_root = self.workspace.root.as_path().join("executions");
        for summary in &mut summaries {
            let dir = exec_root.join(summary.id.as_str());
            summary.size_bytes = crate::exec_view::dir_size_bytes(&dir);
        }
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

    // -------------------------------------------------------------------------
    // Plan 10 — execution delete
    // -------------------------------------------------------------------------

    /// Hard-delete a single execution by id.
    ///
    /// Operations in order:
    /// 1. Validate `exec_id` format via `is_valid_id_component` (traversal defense).
    /// 2. Refuse with `UiError::ExecutionInUse` if `SessionRegistry` reports an
    ///    active run for this execution.
    /// 3. Sqlite cascade in a transaction via `ExecutionStore::delete_execution`:
    ///    DELETE attempts → DELETE executions (manual cascade; schema has no
    ///    `ON DELETE CASCADE` on `attempts.execution_id`).
    ///    Returns `UiError::NotFound` when the execution row didn't exist.
    /// 4. `fs::remove_dir_all` on `<workspace>/executions/<exec_id>/` — best-effort.
    ///    Missing directory is silently ignored. Any other I/O error is logged via
    ///    `tracing::warn` but does **not** propagate — sqlite is authoritative.
    pub fn execution_delete(&self, exec_id: &str) -> Result<(), UiError> {
        // Step 1: reject malformed IDs before any fs or db operation.
        if !is_valid_id_component(exec_id) {
            return Err(UiError::Io(format!("invalid exec_id: {}", exec_id)));
        }

        // Step 2a: cross-process active-attempt gate (sqlite is source of truth).
        // A CLI process opens a fresh StudioCore with an empty in-memory
        // SessionRegistry; without this check it would silently bypass the
        // in-process gate below and delete an exec that is running in Studio.
        {
            let store = self.store.lock().unwrap_or_else(|p| p.into_inner());
            let sqlite_active = store
                .has_active_attempt(exec_id)
                .map_err(|e| UiError::Internal(format!("active-attempt check: {}", e)))?;
            if sqlite_active {
                return Err(UiError::ExecutionInUse { exec_id: exec_id.to_string() });
            }
        }

        // Step 2b: in-process active-run gate (catches the brief window
        // between attempt-start and the sqlite state being committed).
        if self.sessions.has_active_run_for_exec(exec_id) {
            return Err(UiError::ExecutionInUse { exec_id: exec_id.to_string() });
        }

        // Step 3: sqlite cascade (manual; no ON DELETE CASCADE in schema).
        let found = {
            let mut store = self.store.lock().unwrap_or_else(|p| p.into_inner());
            store
                .delete_execution(exec_id)
                .map_err(|e| UiError::Internal(e.to_string()))?
        };
        if !found {
            return Err(UiError::NotFound(format!("execution '{}' not found", exec_id)));
        }

        // Step 4: best-effort dir removal; never propagate this error.
        let exec_dir = self.workspace.root.as_path()
            .join("executions")
            .join(exec_id);
        if exec_dir.exists() {
            if let Err(e) = std::fs::remove_dir_all(&exec_dir) {
                tracing::warn!(
                    exec_id = %exec_id,
                    error = %e,
                    "execution_delete: fs::remove_dir_all failed (dir orphaned; sqlite already authoritative)"
                );
            }
        }

        Ok(())
    }

    /// Serial bulk-delete wrapper over [`execution_delete`].
    ///
    /// Iterates `exec_ids` in order, calling `execution_delete` for each.
    /// Never aborts on individual failures — every item produces either a
    /// `deleted` or `failed` entry. Input order is preserved within each
    /// output vector.
    ///
    /// Returns an [`ExecDeleteBulkResult`] containing the ids of all
    /// successfully deleted executions and per-item failure descriptors for
    /// those that could not be deleted (e.g. active run, not found, traversal
    /// attempt).
    pub fn execution_delete_bulk(&self, exec_ids: &[String]) -> ExecDeleteBulkResult {
        let mut deleted = Vec::new();
        let mut failed = Vec::new();
        for id in exec_ids {
            match self.execution_delete(id) {
                Ok(()) => deleted.push(id.clone()),
                Err(e) => failed.push(ExecDeleteFailure {
                    exec_id: id.clone(),
                    reason: format!("{}", e),
                }),
            }
        }
        ExecDeleteBulkResult { deleted, failed }
    }

    // -------------------------------------------------------------------------
    // Plan 9 — handler log API
    // -------------------------------------------------------------------------

    /// Tail the on-disk `handler_log.log` for a completed (or live) attempt.
    ///
    /// Returns up to `max_lines` parsed lines in chronological order (oldest
    /// first). Returns an empty `Vec` when the file doesn't exist — e.g. the
    /// attempt predates Plan 9 or hasn't started running yet.
    ///
    /// # Errors
    ///
    /// Returns `UiError::Io` for read failures or path-traversal attempts.
    pub fn handler_log_tail(
        &self,
        exec_id: &str,
        attempt_id: &str,
        max_lines: usize,
    ) -> Result<Vec<rowforge_core::handler_log::HandlerLogLine>, UiError> {
        use rowforge_core::handler_log::{handler_log_path, parse_line};

        // BLOCKER fix: validate IDs BEFORE any path construction so that
        // user-controlled strings like "../etc/passwd" never trigger a
        // filesystem probe (even path.exists() counts as a probe).
        if !is_valid_id_component(exec_id) {
            return Err(UiError::Io(format!("invalid exec_id: {}", exec_id)));
        }
        if !is_valid_id_component(attempt_id) {
            return Err(UiError::Io(format!("invalid attempt_id: {}", attempt_id)));
        }

        let attempt_dir = self.workspace.root.as_path()
            .join("executions").join(exec_id)
            .join("attempts").join(attempt_id);

        let path = handler_log_path(&attempt_dir);

        // Fast path: file absent (common for pre-Plan-9 attempts or first run).
        if !path.exists() {
            return Ok(vec![]);
        }

        // Boundary check: belt-and-suspenders canonicalize check in addition
        // to the ID validation above.
        let workspace_root = self.workspace.root.as_path()
            .canonicalize()
            .map_err(|e| UiError::Io(format!("canonicalize workspace: {}", e)))?;
        if let Ok(canon) = attempt_dir.canonicalize() {
            if !canon.starts_with(&workspace_root) {
                return Err(UiError::Io("attempt path outside workspace".into()));
            }
        }

        // MINOR fix: byte-seek to the tail of large files rather than reading
        // the entire file into memory before trimming.
        let raw_lines = read_tail_lines(&path, max_lines)
            .map_err(|e| UiError::Io(format!("read handler log: {}", e)))?;
        let lines: Vec<rowforge_core::handler_log::HandlerLogLine> = raw_lines
            .iter()
            .filter_map(|l| parse_line(l))
            .collect();

        Ok(lines)
    }

    /// Subscribe to the live handler-log broadcast for a currently-active attempt.
    ///
    /// Returns a `broadcast::Receiver` that yields `HandlerLogLine`s in real
    /// time as the pipeline emits them. The channel capacity is 4096; the oldest
    /// lines are silently dropped if the consumer falls behind.
    ///
    /// Returns `Err(UiError::Io)` when the attempt is not currently active (not
    /// in the `SessionRegistry`). The UI should fall back to
    /// `handler_log_tail` for the static file view in that case.
    pub fn handler_log_subscribe(
        &self,
        attempt_id: &str,
    ) -> Result<tokio::sync::broadcast::Receiver<rowforge_core::handler_log::HandlerLogLine>, UiError>
    {
        // Defense in depth: validate the attempt_id before looking it up in
        // SessionRegistry, even though subscribe only does an in-memory lookup.
        if !is_valid_id_component(attempt_id) {
            return Err(UiError::Io(format!("invalid attempt_id: {}", attempt_id)));
        }
        self.sessions
            .handler_log_subscribe(attempt_id)
            .ok_or_else(|| UiError::Io(format!(
                "attempt {} is not active (subscribe to a running attempt or use handler_log_tail)",
                attempt_id,
            )))
    }
}

// ---------------------------------------------------------------------------
// Plan 9 — ID validation + tail helpers
// ---------------------------------------------------------------------------

/// Return `true` iff `s` is a safe path-component for exec/attempt IDs.
///
/// Rejects: empty, containing `/`, `\`, or the substring `..`. Only
/// ASCII alphanumerics, `_`, and `-` are permitted. This is intentionally
/// strict — real IDs are `e_<ULID>` / `r_<ULID>` / `a_<ULID>` which
/// trivially pass. The check runs BEFORE any path join so that a caller
/// passing `"../etc/passwd"` never touches the filesystem at all.
fn is_valid_id_component(s: &str) -> bool {
    !s.is_empty()
        && !s.contains('/')
        && !s.contains('\\')
        && !s.contains("..")
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Read the last `max_lines` lines from `path` using a byte-seek heuristic.
///
/// Assumes an average line length of 200 bytes (handler log lines are
/// typically 100–300 bytes). Seeks to `max_lines * 1000` bytes from the
/// end (capped at file length) to load only the relevant tail, then drops
/// a potentially-partial first line when the seek didn't land at offset 0.
fn read_tail_lines(path: &std::path::Path, max_lines: usize) -> std::io::Result<Vec<String>> {
    use std::io::{Read, Seek, SeekFrom};

    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    // Heuristic: 1000 bytes per line covers the realistic maximum.
    let target = (max_lines as u64).saturating_mul(1000).min(len);
    file.seek(SeekFrom::Start(len.saturating_sub(target)))?;
    let mut buf = Vec::with_capacity(target as usize);
    file.read_to_end(&mut buf)?;
    let s = String::from_utf8_lossy(&buf);
    let mut lines: Vec<String> = s.lines().map(|l| l.to_string()).collect();
    // If we started mid-file the very first element may be a partial line — drop it.
    if target < len && !lines.is_empty() {
        lines.remove(0);
    }
    if lines.len() > max_lines {
        let start = lines.len() - max_lines;
        lines.drain(..start);
    }
    Ok(lines)
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
