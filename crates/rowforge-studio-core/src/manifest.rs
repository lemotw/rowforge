//! Handler manifest validation.
//!
//! Delegates to `rowforge_core::Manifest::load_from_dir` (reads
//! `rowforge.yaml`). Adds:
//! - Structured error variants (file missing vs. parse failure vs. required
//!   field missing) so the UI can render specific messages.
//! - PATH-probing of the first token of `entry.cmd` and `entry.build` via
//!   the `which` crate. A miss is a **warning**, not an error — `PATH`
//!   differs across machines.
//!
//! Note: spec part 8 §8.2 describes a TOML manifest with `build`/`run`
//! string fields. That was a proposed extension; the real on-disk format
//! is `rowforge.yaml` with `entry.cmd: Vec<String>` and
//! `entry.build: Option<Vec<String>>`. This validator follows the real
//! format.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ManifestSource {
    Path { path: PathBuf },
}

/// UI-projected view of `rowforge_core::Manifest`. Carries just the fields
/// the wizard surfaces; the full manifest stays inside core.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    /// Free-form language tag (e.g. "go", "python"). May be empty.
    pub language: String,
    /// Argv of the run command. `validate_manifest` rejects an empty
    /// vector with `ManifestError::ParseFailed`; downstream worker
    /// spawn would otherwise fail late with "entry.cmd empty".
    pub entry_cmd: Vec<String>,
    /// Argv of the optional pre-spawn build command.
    pub entry_build: Option<Vec<String>>,
}

#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ManifestError {
    /// rowforge.yaml does not exist in the handler dir.
    ManifestMissing { path: PathBuf },
    /// rowforge.yaml exists but failed to parse (YAML invalid or missing
    /// required schema fields like `entry.cmd`).
    ParseFailed { message: String },
}

#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ManifestWarning {
    /// First token of an argv (entry.cmd[0] or entry.build[0]) is bare —
    /// no path separator — but not present on the current `PATH`.
    /// Relative tokens like `./bin/x` or `bin/x` skip the probe; they
    /// resolve via cwd at spawn time.
    PathLookupFailed { field: String, token: String },
}

#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestReport {
    pub manifest: Option<Manifest>,
    pub errors: Vec<ManifestError>,
    pub warnings: Vec<ManifestWarning>,
}

pub fn validate_manifest(source: &ManifestSource) -> ManifestReport {
    match source {
        ManifestSource::Path { path } => validate_at(path),
    }
}

fn validate_at(handler_dir: &Path) -> ManifestReport {
    let mut errors: Vec<ManifestError> = Vec::new();
    let mut warnings: Vec<ManifestWarning> = Vec::new();

    let manifest_path = handler_dir.join("rowforge.yaml");
    if !manifest_path.is_file() {
        errors.push(ManifestError::ManifestMissing { path: manifest_path });
        return ManifestReport { manifest: None, errors, warnings };
    }

    let core_manifest = match rowforge_core::manifest::Manifest::load_from_dir(handler_dir) {
        Ok((m, _path)) => m,
        Err(e) => {
            errors.push(ManifestError::ParseFailed { message: e.to_string() });
            return ManifestReport { manifest: None, errors, warnings };
        }
    };

    // rowforge-core's serde happily accepts `entry.cmd: []` — empty vec
    // passes type-check. The worker spawner then fails at run time with
    // "entry.cmd empty" Protocol error. Reject up front so the Wizard /
    // Run launcher can show a useful message before exec_start.
    if core_manifest.entry.cmd.is_empty() {
        errors.push(ManifestError::ParseFailed {
            message: "entry.cmd must be non-empty (no run command defined)".into(),
        });
        return ManifestReport { manifest: None, errors, warnings };
    }

    // PATH-probe the first token of cmd and build (if relative-but-bare).
    if let Some(first) = core_manifest.entry.cmd.first() {
        probe_path_token(first, "entry.cmd", &mut warnings);
    }
    if let Some(build) = &core_manifest.entry.build {
        if let Some(first) = build.first() {
            probe_path_token(first, "entry.build", &mut warnings);
        }
    }

    let manifest = Manifest {
        name: core_manifest.name,
        version: core_manifest.version,
        language: core_manifest.language,
        entry_cmd: core_manifest.entry.cmd,
        entry_build: core_manifest.entry.build,
    };

    ManifestReport { manifest: Some(manifest), errors, warnings }
}

fn probe_path_token(token: &str, field: &str, warnings: &mut Vec<ManifestWarning>) {
    // Skip probe for any path-shaped token: leading `./`, `../`, `/`, or
    // anything containing a path separator. Those resolve at spawn-time
    // via the handler dir as cwd.
    if token.contains('/') || token.contains('\\') {
        return;
    }
    if which::which(token).is_err() {
        warnings.push(ManifestWarning::PathLookupFailed {
            field: field.into(),
            token: token.to_string(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmpdir(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "rfs-plan5-mtest-{}-{}",
            name,
            ulid::Ulid::new()
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn write_yaml(dir: &Path, body: &str) {
        fs::write(dir.join("rowforge.yaml"), body).unwrap();
    }

    #[test]
    fn missing_manifest_reports_error() {
        let dir = tmpdir("missing");
        let report = validate_manifest(&ManifestSource::Path { path: dir.clone() });
        assert!(report.manifest.is_none());
        assert!(matches!(report.errors[0], ManifestError::ManifestMissing { .. }));
    }

    #[test]
    fn parse_failure_reports_error() {
        let dir = tmpdir("bad-yaml");
        write_yaml(&dir, "this: is: not: valid: yaml: :::");
        let report = validate_manifest(&ManifestSource::Path { path: dir });
        assert!(report.manifest.is_none());
        assert!(matches!(report.errors[0], ManifestError::ParseFailed { .. }));
    }

    #[test]
    fn empty_entry_cmd_reports_parse_failure() {
        // serde happily accepts `entry.cmd: []` (Vec<String> deserialize
        // is content-blind), but worker spawn would later fail with
        // "entry.cmd empty". validate_manifest must reject up front.
        let dir = tmpdir("no-cmd");
        write_yaml(&dir, "name: x\nversion: 0.1.0\nentry:\n  cmd: []\n");
        let report = validate_manifest(&ManifestSource::Path { path: dir });
        assert!(report.manifest.is_none(), "empty entry.cmd should reject");
        assert!(report.errors.iter().any(|e| matches!(
            e,
            ManifestError::ParseFailed { message } if message.contains("entry.cmd")
        )));
    }

    #[test]
    fn missing_binary_emits_path_warning_not_error() {
        let dir = tmpdir("missing-bin");
        write_yaml(
            &dir,
            "name: x\nversion: 0.1.0\nentry:\n  cmd: [\"this-binary-definitely-not-on-path-xyz123\"]\n",
        );
        let report = validate_manifest(&ManifestSource::Path { path: dir });
        assert!(report.errors.is_empty());
        assert!(report.warnings.iter().any(|w| matches!(
            w,
            ManifestWarning::PathLookupFailed { field, .. } if field == "entry.cmd"
        )));
        assert!(report.manifest.is_some(), "warnings don't block manifest parse");
    }

    #[test]
    fn relative_cmd_not_path_probed() {
        let dir = tmpdir("rel-bin");
        write_yaml(
            &dir,
            "name: x\nversion: 0.1.0\nentry:\n  cmd: [\"./bin/handler\"]\n",
        );
        let report = validate_manifest(&ManifestSource::Path { path: dir });
        assert!(report.errors.is_empty());
        assert!(report.warnings.is_empty());
        assert!(report.manifest.is_some());
    }

    #[test]
    fn build_first_token_path_probed() {
        let dir = tmpdir("build-probe");
        write_yaml(
            &dir,
            "name: x\nversion: 0.1.0\nentry:\n  cmd: [\"./bin/handler\"]\n  build: [\"nonexistent-build-tool-xyz\", \"--flag\"]\n",
        );
        let report = validate_manifest(&ManifestSource::Path { path: dir });
        assert!(report.errors.is_empty());
        assert!(report.warnings.iter().any(|w| matches!(
            w,
            ManifestWarning::PathLookupFailed { field, .. } if field == "entry.build"
        )));
    }
}
