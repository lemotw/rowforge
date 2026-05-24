//! Export logic shared between rowforge-cli and rowforge-studio-core.
//!
//! See spec docs/spec/cli/part-2-model.md for resolution semantics.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Csv,
    Jsonl,
    Both,
}

#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExportOpts {
    /// None = auto-pick `<exec_dir>/exports/<UTC-timestamp>/`.
    pub output_dir: Option<PathBuf>,
    pub format: ExportFormat,
    /// If true and any rows are `NeverAttempted` (or any attempt is aborted),
    /// return an incomplete-export error before any file is written.
    pub require_complete: bool,
}

#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportReport {
    pub output_dir: PathBuf,
    pub written_files: Vec<PathBuf>,
    pub success_count: u64,
    pub failed_count: u64,
    pub warnings: Vec<ExportWarning>,
}

#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportWarning {
    pub code: String,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn export_format_serializes_snake_case() {
        assert_eq!(serde_json::to_value(ExportFormat::Csv).unwrap(), json!("csv"));
        assert_eq!(serde_json::to_value(ExportFormat::Jsonl).unwrap(), json!("jsonl"));
        assert_eq!(serde_json::to_value(ExportFormat::Both).unwrap(), json!("both"));
    }

    #[test]
    fn export_opts_round_trip() {
        let opts = ExportOpts {
            output_dir: Some(PathBuf::from("/tmp/x")),
            format: ExportFormat::Both,
            require_complete: true,
        };
        let s = serde_json::to_string(&opts).unwrap();
        let back: ExportOpts = serde_json::from_str(&s).unwrap();
        assert_eq!(opts, back);
    }
}
