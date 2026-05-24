//! rowforge-studio-core — GUI-only extension of rowforge-core.
//!
//! See `docs/spec/studio/part-1-overview.md` for principles and
//! `docs/spec/studio/part-5-api.md` for the public surface.

pub mod error;
pub mod exec_view;
pub mod settings;
pub mod workspace;

pub use error::UiError;
pub use exec_view::{ExecSummary, ListFilter};
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
}

impl StudioCore {
    /// Open a workspace. If `opts.workspace` is None, falls back to
    /// `rowforge_core::workspace::default_workspace_root()`.
    pub fn open(opts: OpenOpts) -> Result<Self, UiError> {
        let root = match opts.workspace {
            Some(p) => p,
            None => rowforge_core::workspace::default_workspace_root()
                .ok_or_else(|| {
                    UiError::WorkspaceUnavailable(
                        "no home directory available".into(),
                    )
                })?,
        };
        let store = rowforge_core::execution_store::ExecutionStore::open(&root)
            .map_err(|e| UiError::WorkspaceUnavailable(e.to_string()))?;
        let workspace = Workspace {
            root,
            schema_version: store.schema_version(),
        };
        Ok(Self { workspace, store })
    }

    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    /// List all executions in this workspace, newest first.
    ///
    /// Plan 1 emits one DB call per invocation (no caching). Plan 3
    /// adds the warm-tier mtime probe per spec part-4 §4.3.
    pub fn list(&self, _filter: ListFilter) -> Result<Vec<ExecSummary>, UiError> {
        let executions = self
            .store
            .list_executions()
            .map_err(|e| UiError::Internal(e.to_string()))?;
        Ok(executions.iter().map(ExecSummary::from).collect())
    }
}
