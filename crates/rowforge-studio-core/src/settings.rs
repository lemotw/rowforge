//! User settings type — filesystem-policy-free.
//!
//! `Settings::load_from(impl Read)` / `Settings::save_to(impl Write)` take
//! arbitrary streams so this crate never depends on a specific file
//! location. The Tauri layer resolves `<app_data_dir>/rowforge-studio/
//! settings.json` and feeds the bytes through.
//!
//! Spec: `docs/spec/studio/part-2-model.md` §2.2.9,
//!       `docs/spec/studio/part-5-api.md` §5.6.

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::PathBuf;

use crate::UiError;

const CURRENT_SCHEMA_VERSION: u8 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct Settings {
    pub schema_version: u8,
    pub workspace_root: Option<PathBuf>,
    pub max_concurrent_runs: Option<u32>,
    pub telemetry_opt_in: bool,
    /// Plan 7 T15: user-supplied editor command for handler_open_editor.
    /// When `Some`, overrides $VISUAL / $EDITOR / probe fallback chain.
    /// The value is shell-split (shlex) at call time so "code --wait"
    /// works. `None` means fall through to the 4-tier resolver.
    #[serde(default)]
    pub preferred_editor: Option<String>,
    /// Plan 9 T5: when true, valid outcome JSON stdout lines are duplicated
    /// into `handler_log.log` in addition to `outcomes.jsonl`. Default false:
    /// outcomes go only to outcomes.jsonl; turn on to debug protocol issues.
    /// Read once at attempt-start into `StreamingPoolConfig.capture_raw_stdout`;
    /// changes mid-run don't affect in-flight attempts (intentional).
    #[serde(default)]
    pub handler_log_capture_raw_stdout: bool,
    /// Plan 13: default row count in the smoke test UI.
    /// Clamped to 1..=100 by handler_smoke_run.
    pub smoke_default_rows: usize,
    /// Plan 13: per-row timeout for smoke runs (seconds).
    /// 0 means no timeout.
    pub smoke_timeout_per_row_secs: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            schema_version: CURRENT_SCHEMA_VERSION,
            workspace_root: None,
            max_concurrent_runs: None,
            telemetry_opt_in: false,
            preferred_editor: None,
            handler_log_capture_raw_stdout: false,
            smoke_default_rows: 5,
            smoke_timeout_per_row_secs: 30,
        }
    }
}

impl Settings {
    pub fn load_from<R: Read>(reader: R) -> Result<Self, UiError> {
        serde_json::from_reader(reader)
            .map_err(|e| UiError::Io(format!("settings parse: {e}")))
    }

    pub fn save_to<W: Write>(&self, writer: W) -> Result<(), UiError> {
        serde_json::to_writer_pretty(writer, self)
            .map_err(|e| UiError::Io(format!("settings write: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_schema_version_1() {
        assert_eq!(Settings::default().schema_version, 1);
    }

    #[test]
    fn roundtrip_preserves_workspace_root() {
        let mut s = Settings::default();
        s.workspace_root = Some(PathBuf::from("/tmp/ws"));
        let mut buf = Vec::new();
        s.save_to(&mut buf).unwrap();
        let parsed = Settings::load_from(buf.as_slice()).unwrap();
        assert_eq!(parsed.workspace_root, Some(PathBuf::from("/tmp/ws")));
    }

    #[test]
    fn tolerant_to_missing_fields() {
        let json = br#"{"schema_version": 1}"#;
        let parsed = Settings::load_from(json.as_slice()).unwrap();
        assert_eq!(parsed.schema_version, 1);
        assert_eq!(parsed.workspace_root, None);
        assert!(!parsed.telemetry_opt_in);
        assert_eq!(parsed.preferred_editor, None);
        assert!(!parsed.handler_log_capture_raw_stdout);
    }

    #[test]
    fn roundtrip_handler_log_capture_raw_stdout() {
        let mut s = Settings::default();
        s.handler_log_capture_raw_stdout = true;
        let mut buf = Vec::new();
        s.save_to(&mut buf).unwrap();
        let parsed = Settings::load_from(buf.as_slice()).unwrap();
        assert!(parsed.handler_log_capture_raw_stdout);
    }

    #[test]
    fn handler_log_capture_raw_stdout_defaults_false_on_old_json() {
        // Settings files written before Plan 9 T5 omit the field — must
        // deserialize to false (not an error).
        let json = br#"{"schema_version": 1, "workspace_root": null}"#;
        let parsed = Settings::load_from(json.as_slice()).unwrap();
        assert!(!parsed.handler_log_capture_raw_stdout);
    }

    #[test]
    fn roundtrip_preferred_editor_some() {
        let mut s = Settings::default();
        s.preferred_editor = Some("code --wait".into());
        let mut buf = Vec::new();
        s.save_to(&mut buf).unwrap();
        let parsed = Settings::load_from(buf.as_slice()).unwrap();
        assert_eq!(parsed.preferred_editor, Some("code --wait".into()));
    }

    #[test]
    fn roundtrip_preferred_editor_none_survives_json() {
        // Older settings files without the field should deserialize to None.
        let json = br#"{"schema_version": 1, "workspace_root": null}"#;
        let parsed = Settings::load_from(json.as_slice()).unwrap();
        assert_eq!(parsed.preferred_editor, None);
    }

    #[test]
    fn smoke_defaults() {
        let s = Settings::default();
        assert_eq!(s.smoke_default_rows, 5);
        assert_eq!(s.smoke_timeout_per_row_secs, 30);
    }

    #[test]
    fn smoke_fields_tolerant_to_missing() {
        let json = br#"{"schema_version": 1}"#;
        let parsed = Settings::load_from(json.as_slice()).unwrap();
        assert_eq!(parsed.smoke_default_rows, 5);
        assert_eq!(parsed.smoke_timeout_per_row_secs, 30);
    }

    #[test]
    fn smoke_fields_roundtrip() {
        let mut s = Settings::default();
        s.smoke_default_rows = 12;
        s.smoke_timeout_per_row_secs = 90;
        let mut buf = Vec::new();
        s.save_to(&mut buf).unwrap();
        let parsed = Settings::load_from(buf.as_slice()).unwrap();
        assert_eq!(parsed.smoke_default_rows, 12);
        assert_eq!(parsed.smoke_timeout_per_row_secs, 90);
    }
}
