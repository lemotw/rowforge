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

// ---------------------------------------------------------------------------
// Plan 7 T4 — 4-tier editor resolver + open_editor + reveal_path
// ---------------------------------------------------------------------------

/// 4-tier editor resolution per spec 8.4.1.
///
/// `preferred` is `Settings.preferred_editor` (caller-loaded; T15 wires
/// this via `StudioCore.preferred_editor`). `visual` and `editor` are the
/// `$VISUAL` / `$EDITOR` env vars (production callers pass
/// `std::env::var("VISUAL").ok().as_deref()`). `probes` are the
/// well-known tool names tried in order via `which::which`.
///
/// Returns the parsed argv (first element = command, rest = args).
/// Empty / whitespace-only strings in tiers 1-3 are treated as `None`
/// and fall through.
///
/// `UiError::EditorNotFound` when all 4 tiers exhausted.
/// `UiError::InvalidArg` when `shell_words::split` fails (e.g. unclosed
///   quotes in a user-supplied preferred_editor).
pub(crate) fn resolve_editor(
    preferred: Option<&str>,
    visual: Option<&str>,
    editor: Option<&str>,
    probes: &[&str],
) -> Result<Vec<String>, crate::UiError> {
    // Tier 1: caller-supplied preferred (skip if blank).
    if let Some(cmd) = preferred {
        if !cmd.trim().is_empty() {
            return parse_argv(cmd);
        }
    }
    // Tier 2: $VISUAL.
    if let Some(cmd) = visual {
        if !cmd.trim().is_empty() {
            return parse_argv(cmd);
        }
    }
    // Tier 3: $EDITOR.
    if let Some(cmd) = editor {
        if !cmd.trim().is_empty() {
            return parse_argv(cmd);
        }
    }
    // Tier 4: probe well-known tools via PATH.
    for name in probes {
        if which::which(name).is_ok() {
            return Ok(vec![(*name).to_string()]);
        }
    }
    Err(crate::UiError::EditorNotFound)
}

fn parse_argv(cmd: &str) -> Result<Vec<String>, crate::UiError> {
    shell_words::split(cmd).map_err(|e| {
        crate::UiError::InvalidArg(format!("invalid editor command '{}': {}", cmd, e))
    })
}

/// Plan 7 T4: spawn the resolved editor at the handler dir.
/// Detached — no waiting, no process tracking.
pub fn open_editor(
    workspace_root: &Path,
    name: &str,
    settings_preferred: Option<&str>,
) -> Result<(), crate::UiError> {
    if !validate_name(name) {
        return Err(crate::UiError::InvalidHandlerName {
            name: name.to_string(),
        });
    }
    let handler_dir = workspace_root.join("handlers").join(name);
    if !handler_dir.is_dir() {
        return Err(crate::UiError::HandlerNotFound {
            name: name.to_string(),
        });
    }
    let visual = std::env::var("VISUAL").ok();
    let editor = std::env::var("EDITOR").ok();
    let argv = resolve_editor(
        settings_preferred,
        visual.as_deref(),
        editor.as_deref(),
        &["code", "cursor", "subl", "zed"],
    )?;
    let (cmd, args) = argv
        .split_first()
        .ok_or(crate::UiError::EditorNotFound)?;
    std::process::Command::new(cmd)
        .args(args)
        .arg(&handler_dir)
        .spawn()
        .map_err(|e| crate::UiError::Io(format!("spawn editor '{}': {}", cmd, e)))?;
    Ok(())
}

/// Plan 7 T4: return the handler's dir path. Tauri layer wraps with
/// `shell::open()` to launch the OS file manager. Keeps studio-core
/// OS-policy-free.
pub fn reveal_path(workspace_root: &Path, name: &str) -> Result<PathBuf, crate::UiError> {
    if !validate_name(name) {
        return Err(crate::UiError::InvalidHandlerName {
            name: name.to_string(),
        });
    }
    let handler_dir = workspace_root.join("handlers").join(name);
    if !handler_dir.is_dir() {
        return Err(crate::UiError::HandlerNotFound {
            name: name.to_string(),
        });
    }
    Ok(handler_dir)
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

    #[test]
    fn resolver_uses_preferred_editor_first() {
        let r = resolve_editor(
            Some("/usr/bin/true"),     // tier 1: preferred (absolute path bypasses PATH probe)
            Some("/bin/echo"),         // tier 2: VISUAL (ignored — tier 1 wins)
            Some("/bin/cat"),          // tier 3: EDITOR (ignored)
            &[],                       // tier 4: empty probe list
        );
        assert_eq!(r.unwrap(), vec!["/usr/bin/true".to_string()]);
    }

    #[test]
    fn resolver_falls_back_to_visual_then_editor() {
        // No preferred → VISUAL takes over.
        let r = resolve_editor(None, Some("/bin/echo arg1"), Some("/bin/cat"), &[]);
        assert_eq!(r.unwrap(), vec!["/bin/echo".to_string(), "arg1".to_string()]);

        // No preferred, no VISUAL → EDITOR.
        let r = resolve_editor(None, None, Some("/bin/cat"), &[]);
        assert_eq!(r.unwrap(), vec!["/bin/cat".to_string()]);
    }

    #[test]
    fn resolver_falls_back_to_probes_when_envs_empty() {
        // No preferred / VISUAL / EDITOR; probe finds a known tool.
        // `sh` is on PATH on every reasonable Unix machine.
        let r = resolve_editor(None, None, None, &["sh"]);
        let argv = r.unwrap();
        assert_eq!(argv.len(), 1);
        assert!(argv[0].ends_with("sh") || argv[0] == "sh",
            "probe should return the tool name or its absolute path; got {}", argv[0]);
    }

    #[test]
    fn resolver_errors_when_all_tiers_miss() {
        let r = resolve_editor(None, None, None, &["__no_such_tool_xyz_123__"]);
        assert!(matches!(r, Err(crate::UiError::EditorNotFound)));
    }

    #[test]
    fn resolver_skips_blank_tier_values() {
        // Empty string in preferred should be treated as None (fall through).
        let r = resolve_editor(Some(""), Some("/bin/echo"), None, &[]);
        assert_eq!(r.unwrap(), vec!["/bin/echo".to_string()]);

        let r = resolve_editor(Some("   "), None, None, &["__nope__"]);
        assert!(matches!(r, Err(crate::UiError::EditorNotFound)));
    }

    #[test]
    fn resolver_errors_on_unparseable_command() {
        // shell_words::split returns Err on unclosed quotes.
        let r = resolve_editor(Some("code -w 'unclosed"), None, None, &[]);
        assert!(matches!(r, Err(crate::UiError::InvalidArg(_))));
    }
}
