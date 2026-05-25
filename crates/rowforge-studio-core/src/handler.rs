//! Handler management — discover, view, scaffold, rename, delete handlers
//! under `<workspace>/handlers/*`. Spec part 8 §8.3 / §8.5.
//!
//! This module ships the **static surface** in Plan 7:
//! list / show / open_editor / reveal / scaffold / delete / rename.
//!
//! Build runtime (`manifest.build` subprocess + stderr stream) is Plan 8.
//! Smoke-test runtime (handshake + per-row dispatch, ≤ 100 rows) is Plan 9.
//! In-Studio code editor is an explicit non-goal per spec 8.1.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandlerSummary {
    pub name: String,
    pub path: PathBuf,
    pub manifest_status: ManifestStatus,
    pub last_modified: chrono::DateTime<chrono::Utc>,
    pub version: Option<String>,
    pub language: Option<String>,
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManifestStatus {
    Valid,
    Invalid,
    Missing,
}

#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandlerDetail {
    pub summary: HandlerSummary,
    /// Parsed manifest if status == Valid. None otherwise.
    pub manifest: Option<rowforge_core::manifest::Manifest>,
    pub manifest_errors: Vec<crate::manifest::ManifestError>,
    pub manifest_warnings: Vec<crate::manifest::ManifestWarning>,
    pub source_files: Vec<SourceFileSummary>,
    pub has_fixtures_dir: bool,
}

#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceFileSummary {
    pub name: String,
    pub size_bytes: u64,
    pub is_directory: bool,
}

#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScaffoldArgs {
    pub name: String,
    pub template: ScaffoldTemplate,
    /// Input column the example handler reads. e.g. "email", "order_id".
    pub primary_field: String,
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScaffoldTemplate {
    GoStdio,
    GoBatch,
    Empty,
}

/// Validates that `name` matches `[a-z0-9-]+`. Called by every fs-touching
/// op (scaffold / delete / rename) BEFORE any path operation, so a
/// malicious `name` like `../etc` is rejected before it can join into a
/// path. First line of three-layer defense (canonicalize + parent-check
/// follow in T7).
pub(crate) fn validate_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_validator_accepts_canonical() {
        assert!(validate_name("golang-billing-channel"));
        assert!(validate_name("my-handler-v2"));
        assert!(validate_name("h1"));
        assert!(validate_name("a"));
    }

    #[test]
    fn name_validator_rejects_uppercase_path_chars_and_empty() {
        assert!(!validate_name(""));
        assert!(!validate_name("UpperCase"));
        assert!(!validate_name("with space"));
        assert!(!validate_name("../etc"));
        assert!(!validate_name("a/b"));
        assert!(!validate_name("with_underscore"));
        assert!(!validate_name("foo.bar"));
    }
}
