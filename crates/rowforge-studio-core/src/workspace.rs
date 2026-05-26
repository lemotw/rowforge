//! Workspace projection and open options.
//!
//! Spec: `docs/spec/studio/part-2-model.md` §2.2.1.

use serde::Serialize;
use std::path::PathBuf;

/// Options for `StudioCore::open`. None ⇒ use the platform default
/// (`rowforge_core::workspace::default_workspace_root()`).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct OpenOpts {
    pub workspace: Option<PathBuf>,
    /// Maximum concurrent runs allowed across the whole workspace.
    /// When `None`, `StudioCore::open` falls back to the spec default (3).
    /// Plan 6 T9: threaded from `Settings.max_concurrent_runs` by the
    /// Tauri `workspace_open` command; studio-core stays filesystem-policy-free.
    pub max_concurrent_runs: Option<u32>,
    /// Plan 7 T15: user-supplied editor command sourced from
    /// `Settings.preferred_editor`. When `Some`, overrides the
    /// $VISUAL / $EDITOR / probe fallback in `resolve_editor`.
    /// `None` means fall through to the 4-tier resolver.
    pub preferred_editor: Option<String>,
    /// Plan 9 T5: initial value of `Settings.handler_log_capture_raw_stdout`.
    /// Threaded by the Tauri `workspace_open` command; studio-core stays
    /// filesystem-policy-free.
    pub handler_log_capture_raw_stdout: bool,
    /// Plan 13: clamped to 1..=100 at smoke-run time.
    pub smoke_default_rows: usize,
    /// Plan 13: per-row timeout for smoke runs (seconds).
    /// 0 is treated as a 1-hour ceiling (effectively no timeout).
    pub smoke_timeout_per_row_secs: u64,
}

impl Default for OpenOpts {
    fn default() -> Self {
        OpenOpts {
            workspace: None,
            max_concurrent_runs: None,
            preferred_editor: None,
            handler_log_capture_raw_stdout: false,
            smoke_default_rows: 5,
            smoke_timeout_per_row_secs: 30,
        }
    }
}

impl OpenOpts {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_workspace(mut self, p: PathBuf) -> Self {
        self.workspace = Some(p);
        self
    }
    /// Set the workspace-level concurrency limit for `SessionRegistry`.
    pub fn with_max_concurrent_runs(mut self, n: Option<u32>) -> Self {
        self.max_concurrent_runs = n;
        self
    }
    /// Set the preferred editor command for `handler_open_editor`.
    pub fn with_preferred_editor(mut self, editor: Option<String>) -> Self {
        self.preferred_editor = editor;
        self
    }
    /// Set the initial `handler_log_capture_raw_stdout` flag.
    pub fn with_handler_log_capture_raw_stdout(mut self, enabled: bool) -> Self {
        self.handler_log_capture_raw_stdout = enabled;
        self
    }
    /// Set the default smoke row count sourced from Settings.
    pub fn with_smoke_default_rows(mut self, rows: usize) -> Self {
        self.smoke_default_rows = rows;
        self
    }
    /// Set the per-row smoke timeout sourced from Settings.
    pub fn with_smoke_timeout_per_row_secs(mut self, secs: u64) -> Self {
        self.smoke_timeout_per_row_secs = secs;
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
