//! Workspace projection and open options.
//!
//! Spec: `docs/spec/studio/part-2-model.md` §2.2.1.

use serde::Serialize;
use std::path::PathBuf;

/// Options for `StudioCore::open`. None ⇒ use the platform default
/// (`rowforge_core::workspace::default_workspace_root()`).
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct OpenOpts {
    pub workspace: Option<PathBuf>,
}

impl OpenOpts {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_workspace(mut self, p: PathBuf) -> Self {
        self.workspace = Some(p);
        self
    }
}

/// A handle to the on-disk workspace identity. The `schema_version` is
/// captured at open time and never refreshed during a session.
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct Workspace {
    pub root: PathBuf,
    /// SQLite `schema_version` recorded at the moment we opened the
    /// store. Plan 3 starts enforcing a hard pin here; Plan 1 just
    /// records the value.
    pub schema_version: u8,
}
