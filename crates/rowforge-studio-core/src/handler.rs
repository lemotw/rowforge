//! Handler management — discover, view, scaffold, rename, delete handlers
//! under `<workspace>/handlers/*`. Spec part 8 §8.3 / §8.5.
//!
//! This module ships the **static surface** in Plan 7:
//! list / show / open_editor / reveal / scaffold / delete / rename.
//!
//! Build runtime (`manifest.build` subprocess + stderr stream) is Plan 8.
//! Smoke-test runtime (handshake + per-row dispatch, ≤ 100 rows) is Plan 9.
//! In-Studio code editor is an explicit non-goal per spec 8.1.

use crate::UiError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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

// ---------------------------------------------------------------------------
// Plan 7 T3 — list + show
// ---------------------------------------------------------------------------

/// List all handler directories under `<workspace_root>/handlers/`.
/// Returns empty vec when the `handlers/` dir doesn't exist (not an error).
pub fn list(workspace_root: &Path) -> Result<Vec<HandlerSummary>, UiError> {
    let handlers_dir = workspace_root.join("handlers");
    if !handlers_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&handlers_dir).map_err(|e| UiError::Io(e.to_string()))? {
        let entry = entry.map_err(|e| UiError::Io(e.to_string()))?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => continue,
        };
        out.push(build_summary(&path, name)?);
    }
    Ok(out)
}

fn build_summary(path: &Path, name: String) -> Result<HandlerSummary, UiError> {
    let manifest_path = path.join("rowforge.yaml");
    let (status, manifest_opt) = if !manifest_path.is_file() {
        (ManifestStatus::Missing, None)
    } else {
        match rowforge_core::manifest::Manifest::load_from_dir(path) {
            Ok((m, _)) => (ManifestStatus::Valid, Some(m)),
            Err(_) => (ManifestStatus::Invalid, None),
        }
    };
    let metadata = std::fs::metadata(path).map_err(|e| UiError::Io(e.to_string()))?;
    let last_modified: chrono::DateTime<chrono::Utc> = metadata
        .modified()
        .map_err(|e| UiError::Io(e.to_string()))?
        .into();
    Ok(HandlerSummary {
        name,
        path: path.to_path_buf(),
        manifest_status: status,
        last_modified,
        version: manifest_opt.as_ref().map(|m| m.version.clone()),
        language: manifest_opt.as_ref().and_then(|m| {
            if m.language.is_empty() {
                None
            } else {
                Some(m.language.clone())
            }
        }),
    })
}

/// Load a single handler's detail (manifest report + source files).
///
/// Errors:
/// - `UiError::InvalidHandlerName` — name fails `[a-z0-9-]+` regex
/// - `UiError::HandlerNotFound`    — directory doesn't exist
pub fn show(workspace_root: &Path, name: &str) -> Result<HandlerDetail, UiError> {
    if !validate_name(name) {
        return Err(UiError::InvalidHandlerName {
            name: name.to_string(),
        });
    }
    let path = workspace_root.join("handlers").join(name);
    if !path.is_dir() {
        return Err(UiError::HandlerNotFound {
            name: name.to_string(),
        });
    }
    let summary = build_summary(&path, name.to_string())?;

    // Reuse Plan 5's validate_manifest for structured errors/warnings.
    let report = crate::manifest::validate_manifest(&crate::manifest::ManifestSource::Path {
        path: path.clone(),
    });

    // `HandlerDetail.manifest` is typed as the raw rowforge_core manifest.
    // Load it directly when the status is Valid (report.manifest is the
    // studio-core projection, not the core type).
    let raw_manifest = if report.errors.is_empty() {
        rowforge_core::manifest::Manifest::load_from_dir(&path)
            .ok()
            .map(|(m, _)| m)
    } else {
        None
    };

    // Source files: top-level entries, excluding rowforge.yaml itself
    // (the manifest gets its own panel in the UI).
    let mut source_files = Vec::new();
    for entry in std::fs::read_dir(&path).map_err(|e| UiError::Io(e.to_string()))? {
        let entry = entry.map_err(|e| UiError::Io(e.to_string()))?;
        let entry_path = entry.path();
        let entry_name = match entry_path.file_name().and_then(|n| n.to_str()) {
            Some(s) if !s.is_empty() && s != "rowforge.yaml" => s.to_string(),
            _ => continue,
        };
        let metadata = std::fs::metadata(&entry_path).map_err(|e| UiError::Io(e.to_string()))?;
        source_files.push(SourceFileSummary {
            name: entry_name,
            size_bytes: metadata.len(),
            is_directory: metadata.is_dir(),
        });
    }
    source_files.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(HandlerDetail {
        summary,
        manifest: raw_manifest,
        manifest_errors: report.errors,
        manifest_warnings: report.warnings,
        source_files,
        has_fixtures_dir: path.join("fixtures").is_dir(),
    })
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
