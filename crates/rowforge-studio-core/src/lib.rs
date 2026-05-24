//! rowforge-studio-core — GUI-only extension of rowforge-core.
//!
//! See `docs/spec/studio/part-1-overview.md` for principles and
//! `docs/spec/studio/part-5-api.md` for the public surface.

pub mod cache;
pub mod error;
pub mod exec_view;
pub mod ids;
pub mod settings;
pub mod workspace;

use crate::cache::{Cache, ExecListKey, DEFAULT_TTL};

pub use error::UiError;
pub use exec_view::{ExecSummary, ListFilter};
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
