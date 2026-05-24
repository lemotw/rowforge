//! rowforge-studio-core — GUI-only extension of rowforge-core.
//!
//! See `docs/spec/studio/part-1-overview.md` for principles and
//! `docs/spec/studio/part-5-api.md` for the public surface.

pub mod attempt_detail;
pub mod cache;
pub mod error;
pub mod exec_detail;
pub mod exec_view;
pub mod ids;
pub mod settings;
pub mod workspace;

use crate::cache::{Cache, ExecListKey, DEFAULT_TTL};

pub use attempt_detail::{AttemptDetail, AttemptPaths, HandlerInstanceView};
pub use error::UiError;
pub use exec_detail::{AttemptSummary, ExecDetail, FieldMapping, HandlerBindingView, InputFormat};
pub use exec_view::{AttemptCountsStub, ExecSummary, ListFilter};
pub use ids::{AttemptId, ExecutionId};
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
