//! `rowforge handler build [name] [--force]` — Explicit handler build.
//!
//! Builds one or all handlers under `<workspace>/handlers/`.
//! Respects staleness by default; `--force` bypasses it.
//!
//! Exit code = number of failures (capped at 125).

use anyhow::Result;
use rowforge_core::build::{needs_build, run_build, BuildError};
use rowforge_core::manifest::Manifest;
use std::path::{Path, PathBuf};

pub fn run(workspace: &Path, name: Option<String>, force: bool) -> Result<i32> {
    let handlers_dir = workspace.join("handlers");
    if !handlers_dir.is_dir() {
        eprintln!("[rowforge] no handlers/ directory in workspace");
        return Ok(0);
    }

    let targets: Vec<PathBuf> = match name {
        Some(ref n) => {
            let d = handlers_dir.join(n);
            if !d.is_dir() {
                eprintln!("[rowforge] handler '{}' not found", n);
                return Ok(1);
            }
            vec![d]
        }
        None => {
            let mut dirs: Vec<PathBuf> = std::fs::read_dir(&handlers_dir)?
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .map(|e| e.path())
                .collect();
            dirs.sort();
            dirs
        }
    };

    let mut failed = 0usize;
    for dir in targets {
        let handler_name = dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let manifest = match load_manifest(&dir) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("[{}] skipped (invalid manifest: {})", handler_name, e);
                continue;
            }
        };

        if manifest.entry.build.is_none() {
            eprintln!("[{}] skipped (no entry.build)", handler_name);
            continue;
        }

        if !force && !needs_build(&dir, &manifest) {
            eprintln!("[{}] up to date", handler_name);
            continue;
        }

        match run_build(&dir, &manifest) {
            Ok(outcome) => {
                let dur = outcome
                    .finished_at
                    .signed_duration_since(outcome.started_at)
                    .num_milliseconds();
                eprintln!("[{}] ok ({} ms)", handler_name, dur);
            }
            Err(BuildError::BuildFailed {
                exit_code, outcome, ..
            }) => {
                eprintln!("[{}] failed (exit {})", handler_name, exit_code);
                if !outcome.stdout.is_empty() {
                    eprint!("{}", outcome.stdout);
                }
                if !outcome.stderr.is_empty() {
                    eprint!("{}", outcome.stderr);
                }
                failed += 1;
            }
            Err(BuildError::ToolchainMissing { tool }) => {
                eprintln!("[{}] toolchain missing: {}", handler_name, tool);
                failed += 1;
            }
            Err(BuildError::NoBuildCommand) => {
                // should not happen — we checked above, but handle defensively
                eprintln!("[{}] skipped (no entry.build)", handler_name);
            }
            Err(e) => {
                eprintln!("[{}] error: {}", handler_name, e);
                failed += 1;
            }
        }
    }

    Ok(failed.min(125) as i32)
}

fn load_manifest(dir: &Path) -> anyhow::Result<Manifest> {
    let (manifest, _path) = Manifest::load_from_dir(dir)?;
    Ok(manifest)
}
