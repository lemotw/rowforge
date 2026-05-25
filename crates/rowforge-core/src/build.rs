use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::SystemTime;

use crate::manifest::Manifest;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BuildOutcome {
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub exit_code: i32,
    pub command: Vec<String>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("no build command in manifest")]
    NoBuildCommand,
    #[error("build tool {tool:?} not found in PATH")]
    ToolchainMissing { tool: String },
    #[error("build failed (exit {exit_code})")]
    BuildFailed {
        exit_code: i32,
        stderr_tail: String,
        outcome: BuildOutcome,
    },
    #[error("io: {0}")]
    Io(String),
}

pub fn needs_build(handler_dir: &Path, manifest: &Manifest) -> bool {
    // No build command → never build.
    if manifest.entry.build.is_none() {
        return false;
    }
    let cmd = match manifest.entry.cmd.first() {
        Some(s) => s.as_str(),
        None => return false,
    };

    // Absolute path → not a relative binary, no staleness concept here.
    if cmd.starts_with('/') {
        return false;
    }

    // Bare name resolvable via PATH (interpreter like `python3`, `node`) → no binary.
    if !cmd.contains('/') && !cmd.starts_with('.') && which::which(cmd).is_ok() {
        return false;
    }

    // Treat as relative-path binary in handler_dir.
    let bin_rel = cmd.trim_start_matches("./");
    let bin = handler_dir.join(bin_rel);
    let bin_mtime = match bin.metadata().and_then(|m| m.modified()) {
        Ok(t) => t,
        Err(_) => return true, // binary missing → must build
    };

    max_source_mtime(handler_dir)
        .map(|src| src > bin_mtime)
        .unwrap_or(false)
}

fn max_source_mtime(dir: &Path) -> Option<SystemTime> {
    const EXTS: &[&str] = &[
        "go", "rs", "py", "js", "ts", "mjs", "java", "c", "cpp", "h", "hpp",
    ];
    let mut max: Option<SystemTime> = None;
    let entries = std::fs::read_dir(dir).ok()?;
    for e in entries.flatten() {
        let ft = match e.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if !ft.is_file() {
            continue;
        }
        let path = e.path();
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        if !EXTS.contains(&ext) {
            continue;
        }
        if let Ok(m) = e.metadata().and_then(|m| m.modified()) {
            max = Some(max.map_or(m, |cur| cur.max(m)));
        }
    }
    max
}

pub fn run_build(handler_dir: &Path, manifest: &Manifest) -> Result<BuildOutcome, BuildError> {
    let cmd = manifest
        .entry
        .build
        .as_ref()
        .ok_or(BuildError::NoBuildCommand)?;
    if cmd.is_empty() {
        return Err(BuildError::NoBuildCommand);
    }
    let tool = &cmd[0];
    // Resolve PATH up-front so ENOENT becomes a friendlier error.
    if which::which(tool).is_err() {
        return Err(BuildError::ToolchainMissing { tool: tool.clone() });
    }

    let started_at = Utc::now();
    let output = std::process::Command::new(tool)
        .args(&cmd[1..])
        .current_dir(handler_dir)
        .output()
        .map_err(|e| BuildError::Io(e.to_string()))?;
    let finished_at = Utc::now();

    let exit_code = output.status.code().unwrap_or(-1);
    let outcome = BuildOutcome {
        started_at,
        finished_at,
        exit_code,
        command: cmd.clone(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    };

    if exit_code != 0 {
        let tail_start = outcome.stderr.len().saturating_sub(500);
        return Err(BuildError::BuildFailed {
            exit_code,
            stderr_tail: outcome.stderr[tail_start..].to_string(),
            outcome,
        });
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{Entry, Manifest};
    use std::collections::BTreeMap;
    use tempfile::TempDir;

    fn mk_manifest(cmd: Vec<&str>, build: Option<Vec<&str>>) -> Manifest {
        Manifest {
            name: "t".into(),
            version: "0.1.0".into(),
            description: String::new(),
            language: String::new(),
            entry: Entry {
                cmd: cmd.into_iter().map(String::from).collect(),
                build: build.map(|v| v.into_iter().map(String::from).collect()),
                cwd: ".".into(),
                env: BTreeMap::new(),
                startup_timeout_ms: 30_000,
            },
            required_input: vec![],
            config: BTreeMap::new(),
            runtime: None,
            output: None,
        }
    }

    #[test]
    fn needs_build_false_when_no_build_command() {
        let tmp = TempDir::new().unwrap();
        let m = mk_manifest(vec!["./handler"], None);
        assert!(!needs_build(tmp.path(), &m));
    }

    #[test]
    fn needs_build_false_when_cmd_is_absolute_path() {
        let tmp = TempDir::new().unwrap();
        let m = mk_manifest(vec!["/usr/bin/python3"], Some(vec!["go", "build"]));
        assert!(!needs_build(tmp.path(), &m));
    }

    #[test]
    fn needs_build_false_when_cmd_is_path_executable() {
        // Use a tool that's universally available on POSIX builders.
        let tmp = TempDir::new().unwrap();
        let m = mk_manifest(vec!["sh"], Some(vec!["go", "build"]));
        assert!(!needs_build(tmp.path(), &m));
    }

    #[test]
    fn needs_build_true_when_binary_missing() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("handler.go"), "package main\nfunc main(){}").unwrap();
        let m = mk_manifest(vec!["./handler"], Some(vec!["go", "build"]));
        assert!(needs_build(tmp.path(), &m));
    }

    #[test]
    fn needs_build_true_when_source_newer_than_binary() {
        use filetime::{set_file_mtime, FileTime};
        let tmp = TempDir::new().unwrap();
        let bin = tmp.path().join("handler");
        let src = tmp.path().join("handler.go");
        std::fs::write(&bin, "binary").unwrap();
        std::fs::write(&src, "source").unwrap();
        // Force binary mtime older than source.
        set_file_mtime(&bin, FileTime::from_unix_time(1_000_000, 0)).unwrap();
        set_file_mtime(&src, FileTime::from_unix_time(2_000_000, 0)).unwrap();
        let m = mk_manifest(vec!["./handler"], Some(vec!["go", "build"]));
        assert!(needs_build(tmp.path(), &m));
    }

    #[test]
    fn needs_build_false_when_binary_newer_than_source() {
        use filetime::{set_file_mtime, FileTime};
        let tmp = TempDir::new().unwrap();
        let bin = tmp.path().join("handler");
        let src = tmp.path().join("handler.go");
        std::fs::write(&bin, "binary").unwrap();
        std::fs::write(&src, "source").unwrap();
        set_file_mtime(&src, FileTime::from_unix_time(1_000_000, 0)).unwrap();
        set_file_mtime(&bin, FileTime::from_unix_time(2_000_000, 0)).unwrap();
        let m = mk_manifest(vec!["./handler"], Some(vec!["go", "build"]));
        assert!(!needs_build(tmp.path(), &m));
    }

    #[test]
    fn run_build_returns_no_build_command_when_none() {
        let tmp = TempDir::new().unwrap();
        let m = mk_manifest(vec!["./handler"], None);
        assert!(matches!(run_build(tmp.path(), &m), Err(BuildError::NoBuildCommand)));
    }

    #[test]
    fn run_build_returns_toolchain_missing_when_tool_not_in_path() {
        let tmp = TempDir::new().unwrap();
        let m = mk_manifest(
            vec!["./handler"],
            Some(vec!["this-tool-definitely-does-not-exist-xyz"]),
        );
        assert!(matches!(
            run_build(tmp.path(), &m),
            Err(BuildError::ToolchainMissing { .. })
        ));
    }

    #[test]
    fn run_build_success_returns_outcome() {
        let tmp = TempDir::new().unwrap();
        let m = mk_manifest(vec!["./handler"], Some(vec!["sh", "-c", "echo hi"]));
        let outcome = run_build(tmp.path(), &m).expect("build ok");
        assert_eq!(outcome.exit_code, 0);
        assert!(outcome.stdout.contains("hi"));
    }

    #[test]
    fn run_build_failure_returns_build_failed_with_outcome() {
        let tmp = TempDir::new().unwrap();
        let m = mk_manifest(
            vec!["./handler"],
            Some(vec!["sh", "-c", "echo oops >&2; exit 3"]),
        );
        match run_build(tmp.path(), &m) {
            Err(BuildError::BuildFailed { exit_code, outcome, .. }) => {
                assert_eq!(exit_code, 3);
                assert!(outcome.stderr.contains("oops"));
            }
            other => panic!("expected BuildFailed, got {:?}", other),
        }
    }
}
