//! Handler manifest validation per spec part 8 §8.2.
//!
//! `Manifest.run` is required; `Manifest.build` is optional. Both must
//! parse via shell-words. First token of each is PATH-probed; a miss is
//! a warning (PATH may differ across machines), not an error.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ManifestSource {
    Path { path: PathBuf },
}

#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub name: Option<String>,
    pub version: Option<String>,
    pub language: Option<String>,
    pub build: Option<String>,
    pub run: String,
}

#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ManifestError {
    ManifestMissing { path: PathBuf },
    ParseFailed { message: String },
    MissingRequired { field: String },
    ShellParseFailed { field: String, message: String },
}

#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ManifestWarning {
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
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut manifest: Option<Manifest> = None;

    let manifest_path = handler_dir.join("manifest.toml");
    let text = match std::fs::read_to_string(&manifest_path) {
        Ok(s) => s,
        Err(_) => {
            errors.push(ManifestError::ManifestMissing { path: manifest_path });
            return ManifestReport { manifest: None, errors, warnings };
        }
    };

    let m: Manifest = match toml::from_str(&text) {
        Ok(m) => m,
        Err(e) => {
            errors.push(ManifestError::ParseFailed { message: e.to_string() });
            return ManifestReport { manifest: None, errors, warnings };
        }
    };

    if m.run.trim().is_empty() {
        errors.push(ManifestError::MissingRequired { field: "run".into() });
    }
    if let Some(build) = &m.build {
        check_shell_token(build, "build", &mut errors, &mut warnings);
    }
    if !m.run.trim().is_empty() {
        check_shell_token(&m.run, "run", &mut errors, &mut warnings);
    }
    if errors.is_empty() {
        manifest = Some(m);
    }
    ManifestReport { manifest, errors, warnings }
}

fn check_shell_token(
    cmd: &str,
    field: &str,
    errors: &mut Vec<ManifestError>,
    warnings: &mut Vec<ManifestWarning>,
) {
    let tokens = match shell_words::split(cmd) {
        Ok(t) => t,
        Err(e) => {
            errors.push(ManifestError::ShellParseFailed {
                field: field.into(),
                message: e.to_string(),
            });
            return;
        }
    };
    if let Some(first) = tokens.first() {
        if !first.contains('/') && !first.contains('\\') {
            if which::which(first).is_err() {
                warnings.push(ManifestWarning::PathLookupFailed {
                    field: field.into(),
                    token: first.clone(),
                });
            }
        }
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

    #[test]
    fn missing_manifest_reports_error() {
        let dir = tmpdir("missing");
        let report = validate_manifest(&ManifestSource::Path { path: dir.clone() });
        assert!(report.manifest.is_none());
        assert!(matches!(report.errors[0], ManifestError::ManifestMissing { .. }));
    }

    #[test]
    fn parse_failure_reports_error() {
        let dir = tmpdir("bad-toml");
        fs::write(dir.join("manifest.toml"), "not = valid = toml").unwrap();
        let report = validate_manifest(&ManifestSource::Path { path: dir });
        assert!(report.manifest.is_none());
        assert!(matches!(report.errors[0], ManifestError::ParseFailed { .. }));
    }

    #[test]
    fn missing_run_field_reports_error() {
        let dir = tmpdir("no-run");
        fs::write(dir.join("manifest.toml"), "version = \"1.0\"\nrun = \"\"\n").unwrap();
        let report = validate_manifest(&ManifestSource::Path { path: dir });
        assert!(report.errors.iter().any(|e|
            matches!(e, ManifestError::MissingRequired { field } if field == "run")));
    }

    #[test]
    fn missing_binary_emits_path_warning_not_error() {
        let dir = tmpdir("missing-bin");
        fs::write(
            dir.join("manifest.toml"),
            "run = \"this-binary-definitely-not-on-path-xyz123\"\n",
        ).unwrap();
        let report = validate_manifest(&ManifestSource::Path { path: dir });
        assert!(report.errors.is_empty());
        assert!(report.warnings.iter().any(|w|
            matches!(w, ManifestWarning::PathLookupFailed { field, .. } if field == "run")));
        assert!(report.manifest.is_some());
    }

    #[test]
    fn relative_path_run_not_path_probed() {
        let dir = tmpdir("rel-bin");
        fs::write(dir.join("manifest.toml"), "run = \"bin/handler\"\n").unwrap();
        let report = validate_manifest(&ManifestSource::Path { path: dir });
        assert!(report.errors.is_empty());
        assert!(report.warnings.is_empty());
    }
}
