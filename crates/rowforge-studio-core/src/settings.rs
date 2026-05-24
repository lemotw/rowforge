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
    pub default_workers: Option<u32>,
    pub max_concurrent_runs: Option<u32>,
    pub telemetry_opt_in: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            schema_version: CURRENT_SCHEMA_VERSION,
            workspace_root: None,
            default_workers: None,
            max_concurrent_runs: None,
            telemetry_opt_in: false,
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
    }
}
