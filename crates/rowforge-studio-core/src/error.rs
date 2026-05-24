//! UI-facing error type.
//!
//! Surface is intentionally narrow in Plan 1 (open + list paths only).
//! Later plans extend with `RunAborted`, `RunBusy`, `HandlerBusy`, etc.
//! Spec: `docs/spec/studio/part-5-api.md` §5.3.

use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum UiError {
    /// Workspace cannot be located (no `$HOME` and no explicit override) or
    /// the SQLite store could not be opened.
    #[error("workspace unavailable: {0}")]
    WorkspaceUnavailable(String),

    /// I/O failure reading or scanning workspace artefacts.
    #[error("io error: {0}")]
    Io(String),

    /// Unclassifiable internal failure. Future plans should classify
    /// instead of reaching for this.
    #[error("internal: {0}")]
    Internal(String),
}

impl From<std::io::Error> for UiError {
    fn from(e: std::io::Error) -> Self {
        UiError::Io(e.to_string())
    }
}

impl From<rowforge_core::error::CoreError> for UiError {
    fn from(e: rowforge_core::error::CoreError) -> Self {
        // CoreError lacks variant-level discrimination today; treat as
        // workspace-unavailable when surfaced from store open paths,
        // internal otherwise. Plan 3 revisits when we classify more
        // narrowly per call site.
        UiError::Internal(e.to_string())
    }
}
