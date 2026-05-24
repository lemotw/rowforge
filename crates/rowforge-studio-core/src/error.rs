//! UI-facing error type.
//!
//! Surface aligned with spec `docs/spec/studio/part-5-api.md` §5.3.
//! Plan 3 lands the spec-named variants; the previous `WorkspaceUnavailable`
//! becomes `WorkspaceLocked`. Plan 4 adds `RunAborted`, `RunBusy`,
//! `UnknownHandle`; Plan 5 refactors tuple variants to struct payloads and
//! adds `InvalidInput`, `DuplicateExecName`, `ExportIncomplete`,
//! `ManifestInvalid`, `ToolchainMissing`.

use serde::Serialize;
use thiserror::Error;

use crate::events::AbortReason;

#[non_exhaustive]
#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BusyScope {
    PerExec,
    PerWorkspace,
}

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

    /// Run aborted. Payload is the structured AbortReason (spec §6.1).
    /// Flattened so `message` serializes as the AbortReason object directly,
    /// e.g. `{"kind":"run_aborted","message":{"kind":"user_cancelled"}}`.
    #[error("run aborted")]
    RunAborted {
        #[serde(flatten)]
        reason: AbortReason,
    },

    /// Run cannot start because the execution or workspace scope is busy.
    #[error("run busy: limit {limit} reached for scope {scope:?}")]
    RunBusy {
        execution_id: String,
        limit: u32,
        scope: BusyScope,
    },

    /// The provided run handle is expired or unknown.
    #[error("handle expired or unknown: {0}")]
    UnknownHandle(String),

    /// Caller supplied invalid input (path missing, format undetectable, etc).
    #[error("invalid input: {reason}")]
    InvalidInput { reason: String },

    /// Execution name conflicts with an existing exec in this workspace.
    #[error("duplicate exec name: {name}")]
    DuplicateExecName { name: String },

    /// Export refused because require_complete=true and rows remain unresolved.
    #[error("export incomplete: {missing_count} rows unresolved")]
    ExportIncomplete { missing_count: u64 },

    /// Handler manifest validation failed. errors block exec_start / run_start.
    #[error("manifest invalid")]
    ManifestInvalid {
        // ManifestError lives in crate::manifest (added in Task 5). Use the
        // typed list once that module lands. For now, accept Vec<String> as
        // a placeholder shape so we don't block Task 5.
        errors: Vec<String>,
    },

    /// Manifest references a binary not found on PATH (used by future plans).
    #[error("toolchain missing: {token}")]
    ToolchainMissing { token: String },
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use crate::events::AbortReason;

    #[test]
    fn run_aborted_serializes_with_reason_struct() {
        let e = UiError::RunAborted {
            reason: AbortReason::UserCancelled,
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["kind"], json!("run_aborted"));
        assert_eq!(v["message"]["kind"], json!("user_cancelled"));
    }

    #[test]
    fn run_busy_serializes_with_struct_fields() {
        let e = UiError::RunBusy {
            execution_id: "e_01ABC".into(),
            limit: 3,
            scope: BusyScope::PerWorkspace,
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["kind"], json!("run_busy"));
        assert_eq!(v["message"]["execution_id"], json!("e_01ABC"));
        assert_eq!(v["message"]["limit"], json!(3));
        assert_eq!(v["message"]["scope"], json!("per_workspace"));
    }

    #[test]
    fn invalid_input_serializes() {
        let e = UiError::InvalidInput { reason: "no such file".into() };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["kind"], json!("invalid_input"));
        assert_eq!(v["message"]["reason"], json!("no such file"));
    }

    #[test]
    fn duplicate_exec_name_serializes() {
        let e = UiError::DuplicateExecName { name: "foo".into() };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["kind"], json!("duplicate_exec_name"));
        assert_eq!(v["message"]["name"], json!("foo"));
    }

    #[test]
    fn export_incomplete_serializes() {
        let e = UiError::ExportIncomplete { missing_count: 42 };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["kind"], json!("export_incomplete"));
        assert_eq!(v["message"]["missing_count"], json!(42));
    }
}
