//! UI-facing error type.
//!
//! Surface aligned with spec `docs/spec/studio/part-5-api.md` §5.3.
//! Plan 3 lands the spec-named variants; the previous `WorkspaceUnavailable`
//! becomes `WorkspaceLocked`. Plan 4 adds `RunAborted`, `RunBusy`,
//! `UnknownHandle`; Plan 6 adds `HandlerBusy`, `EditorNotFound`, etc.

use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error, Serialize)]
#[serde(tag = "kind", content = "message", rename_all = "snake_case")]
#[non_exhaustive]
pub enum UiError {
    /// Workspace cannot be opened: missing $HOME, incompatible schema, or
    /// SQLite failure.
    #[error("workspace locked or incompatible: {0}")]
    WorkspaceLocked(String),

    /// Entity not found. Message describes what (`"execution e1"`, etc.).
    #[error("{0}")]
    NotFound(String),

    /// Caller-supplied argument is invalid.
    #[error("invalid argument: {0}")]
    InvalidArg(String),

    /// I/O failure reading workspace artefacts.
    #[error("io error: {0}")]
    Io(String),

    /// Internal failure. Future plans should classify instead.
    #[error("internal: {0}")]
    Internal(String),

    /// A run was aborted (e.g. by cancel request or signal).
    #[error("run aborted: {0}")]
    RunAborted(String),

    /// A run cannot start because the execution or scope is already busy.
    #[error("run cannot start: {0}")]
    RunBusy(String),

    /// The provided run handle is expired or unknown.
    #[error("handle expired or unknown: {0}")]
    UnknownHandle(String),
}

impl From<std::io::Error> for UiError {
    fn from(e: std::io::Error) -> Self {
        UiError::Io(e.to_string())
    }
}

impl From<rowforge_core::error::CoreError> for UiError {
    fn from(e: rowforge_core::error::CoreError) -> Self {
        UiError::Internal(e.to_string())
    }
}
