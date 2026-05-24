//! rowforge-studio-core — GUI-only extension of rowforge-core.
//!
//! See `docs/spec/studio/part-1-overview.md` for principles and
//! `docs/spec/studio/part-5-api.md` for the public surface.

pub mod error;
pub mod exec_view;
pub mod workspace;

pub use error::UiError;
pub use exec_view::{ExecSummary, ListFilter};
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
}
