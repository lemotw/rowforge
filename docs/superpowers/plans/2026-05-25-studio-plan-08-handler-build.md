# Plan 8 — Handler Build + Validate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make handlers buildable end-to-end. CLI auto-builds before `exec run` when binary is stale; Studio's handler detail page shows a Build button + Last build log section. Manifest validation gains two PATH-resolution warnings.

**Architecture:** New `rowforge-core::build` module (sync subprocess + staleness check) is shared by CLI and Studio. CLI gates `exec run` on `needs_build`; Studio always forces and keeps an in-memory `BuildOutcome` cache on `StudioCore` for the session.

**Tech Stack:** Rust (rowforge-core, rowforge-cli, rowforge-studio-core, Tauri 2), React 19 + Vite 6 + TanStack Query v5, Tailwind + shadcn/ui.

**Design spec:** `docs/superpowers/specs/2026-05-25-studio-plan-08-handler-build-design.md`

---

## Task 1: rowforge-core::build module

**Files:**
- Create: `crates/rowforge-core/src/build.rs`
- Modify: `crates/rowforge-core/src/lib.rs` (add `pub mod build;`)
- Modify: `crates/rowforge-core/Cargo.toml` (verify `which`, `chrono`, `serde`, `thiserror` deps)
- Test: same file (`#[cfg(test)]` module)

- [ ] **Step 1: Add the module declaration**

Edit `crates/rowforge-core/src/lib.rs`, find the existing `pub mod manifest;` line, add below it:

```rust
pub mod build;
```

- [ ] **Step 2: Verify deps**

```bash
grep -E 'which|chrono|thiserror' crates/rowforge-core/Cargo.toml
```

Expect `which`, `chrono`, `thiserror` present. If `which` is missing, add to `[dependencies]`:
```toml
which = "6"
```

- [ ] **Step 3: Write the type skeleton + first failing test**

Create `crates/rowforge-core/src/build.rs`:

```rust
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
    todo!()
}

pub fn run_build(handler_dir: &Path, manifest: &Manifest) -> Result<BuildOutcome, BuildError> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{Entry, HandlerKind, Manifest};
    use tempfile::TempDir;

    fn mk_manifest(cmd: Vec<&str>, build: Option<Vec<&str>>) -> Manifest {
        Manifest {
            name: "t".into(),
            version: None,
            language: None,
            kind: HandlerKind::Row,
            primary_field: "id".into(),
            entry: Entry {
                cmd: cmd.into_iter().map(String::from).collect(),
                build: build.map(|v| v.into_iter().map(String::from).collect()),
                startup_timeout_ms: None,
            },
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
}
```

> **Note:** The exact `Manifest` constructor may differ — check the real shape with `grep -nA 20 'pub struct Manifest' crates/rowforge-core/src/manifest.rs` and adjust the test helper.

- [ ] **Step 4: Run test to verify it fails**

```bash
cargo test -p rowforge-core --lib build::tests
```

Expected: `todo!()` panic OR struct mismatch error. Fix struct mismatch first, then proceed.

- [ ] **Step 5: Implement `needs_build` — early returns**

Replace the `needs_build` body:

```rust
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
```

- [ ] **Step 6: Run the no-build-cmd test — should pass**

```bash
cargo test -p rowforge-core --lib build::tests::needs_build_false_when_no_build_command
```

Expected: PASS.

- [ ] **Step 7: Add the rest of needs_build tests**

Append to `mod tests`:

```rust
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
```

> If `filetime` isn't already in `[dev-dependencies]`, add it: `filetime = "0.2"`.

- [ ] **Step 8: Run all needs_build tests**

```bash
cargo test -p rowforge-core --lib build::tests::needs_build
```

Expected: all 5 PASS.

- [ ] **Step 9: Implement `run_build`**

Replace the `todo!()` body:

```rust
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
```

- [ ] **Step 10: Add run_build tests**

```rust
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
```

- [ ] **Step 11: Run all build module tests**

```bash
cargo test -p rowforge-core --lib build::tests
```

Expected: 9 PASS (5 needs_build + 4 run_build).

- [ ] **Step 12: Commit**

```bash
git add crates/rowforge-core/src/build.rs crates/rowforge-core/src/lib.rs crates/rowforge-core/Cargo.toml
git commit -m "rowforge-core: build module — needs_build + run_build

New rowforge-core::build module shared by CLI and Studio:
- needs_build(dir, manifest) → bool: returns true when entry.build
  exists AND entry.cmd[0] is a relative path AND (binary missing OR
  source mtime > binary mtime). Source files matched by extension
  (.go .rs .py .js .ts .mjs .java .c .cpp .h .hpp), top-level only.
- run_build(dir, manifest) → Result<BuildOutcome, BuildError>:
  sync std::process::Command with cwd=handler_dir, captures full
  stdout/stderr; ENOENT-style spawn failures resolve up-front via
  which::which to surface ToolchainMissing.
- BuildOutcome carries command echo + timestamps + exit + streams.
- BuildError variants: NoBuildCommand, ToolchainMissing, BuildFailed
  (carries the outcome so callers can show the log), Io.

9 unit tests covering the needs_build decision matrix and run_build
success/failure/toolchain-missing paths."
```

---

## Task 2: validate_manifest extension

**Files:**
- Modify: `crates/rowforge-core/src/manifest.rs` (find `validate_manifest` or equivalent; add 2 warnings)
- Test: same file

- [ ] **Step 1: Locate the validator and warning code enum**

```bash
grep -nE 'fn validate_manifest|ManifestWarning|ManifestWarningCode' crates/rowforge-core/src/manifest.rs
```

Read enough surrounding context to understand the warning emission pattern (push to a Vec, build a struct, etc.).

- [ ] **Step 2: Add new warning code variants**

Edit `ManifestWarningCode`:

```rust
pub enum ManifestWarningCode {
    // ...existing variants...
    BuildToolNotInPath,
    CmdTargetMissing,
}
```

- [ ] **Step 3: Write failing tests first**

Append to the test module:

```rust
#[test]
fn validate_warns_when_build_tool_not_in_path() {
    use tempfile::TempDir;
    let tmp = TempDir::new().unwrap();
    let yaml = r#"
name: t
kind: row
primary_field: id
entry:
  cmd: ["./handler"]
  build: ["this-tool-xyz-does-not-exist", "build"]
"#;
    std::fs::write(tmp.path().join("rowforge.yaml"), yaml).unwrap();
    let report = Manifest::load_from_dir(tmp.path()).unwrap();
    assert!(report.warnings.iter().any(|w| matches!(w.code, ManifestWarningCode::BuildToolNotInPath)));
}

#[test]
fn validate_warns_when_relative_cmd_missing_and_no_build() {
    use tempfile::TempDir;
    let tmp = TempDir::new().unwrap();
    let yaml = r#"
name: t
kind: row
primary_field: id
entry:
  cmd: ["./missing-binary"]
"#;
    std::fs::write(tmp.path().join("rowforge.yaml"), yaml).unwrap();
    let report = Manifest::load_from_dir(tmp.path()).unwrap();
    assert!(report.warnings.iter().any(|w| matches!(w.code, ManifestWarningCode::CmdTargetMissing)));
}

#[test]
fn validate_does_not_warn_when_cmd_missing_but_build_present() {
    use tempfile::TempDir;
    let tmp = TempDir::new().unwrap();
    let yaml = r#"
name: t
kind: row
primary_field: id
entry:
  cmd: ["./not-yet-built"]
  build: ["sh", "-c", "echo build"]
"#;
    std::fs::write(tmp.path().join("rowforge.yaml"), yaml).unwrap();
    let report = Manifest::load_from_dir(tmp.path()).unwrap();
    assert!(!report.warnings.iter().any(|w| matches!(w.code, ManifestWarningCode::CmdTargetMissing)));
}
```

> The exact API may be `Manifest::load_from_dir` returning a report-like struct, or `validate_manifest(&manifest, dir)`. Adapt to actual code shape.

- [ ] **Step 4: Run tests — verify they fail**

```bash
cargo test -p rowforge-core --lib manifest::tests::validate
```

Expected: at least 2 FAIL (the warnings don't fire yet — variants exist but logic doesn't).

- [ ] **Step 5: Implement the warning emitters**

Find the function that returns `ManifestWarning`s. Add (pseudocode):

```rust
fn check_path_resolution(manifest: &Manifest, handler_dir: &Path, warnings: &mut Vec<ManifestWarning>) {
    // 1. build tool
    if let Some(build) = &manifest.entry.build {
        if let Some(tool) = build.first() {
            if which::which(tool).is_err() {
                warnings.push(ManifestWarning {
                    code: ManifestWarningCode::BuildToolNotInPath,
                    message: format!("build tool '{}' not found in PATH", tool),
                });
            }
        }
    }
    // 2. cmd target
    if let Some(t) = manifest.entry.cmd.first() {
        let t = t.as_str();
        let exists = if t.starts_with('/') {
            std::path::Path::new(t).exists()
        } else if t.contains('/') || t.starts_with('.') {
            // relative path → resolve against handler_dir
            handler_dir.join(t.trim_start_matches("./")).exists()
        } else {
            // bare name → must be on PATH
            which::which(t).is_ok()
        };
        if !exists {
            // Skip warning when build is present (build is expected to produce it).
            if manifest.entry.build.is_none() {
                warnings.push(ManifestWarning {
                    code: ManifestWarningCode::CmdTargetMissing,
                    message: format!("entry.cmd target '{}' not found", t),
                });
            }
        }
    }
}
```

Wire into the validator's warning-collection path.

- [ ] **Step 6: Run tests — all pass**

```bash
cargo test -p rowforge-core --lib manifest::tests
```

Expected: all manifest tests PASS, with the 3 new ones included.

- [ ] **Step 7: Commit**

```bash
git add crates/rowforge-core/src/manifest.rs
git commit -m "rowforge-core: validate_manifest — 2 new PATH warnings

ManifestWarningCode gains BuildToolNotInPath + CmdTargetMissing.

Logic:
- entry.build[0] not resolvable via which::which → BuildToolNotInPath
- entry.cmd[0] either: (a) absolute path not existing, (b) relative
  path not existing in handler_dir, or (c) bare name not on PATH
  → CmdTargetMissing. Skipped when entry.build is present (build
  is expected to produce the target).

Both are warnings (yellow in Studio UI), not errors — handlers
still load and the user can still run them; runtime spawn will
fail loudly with the real error if needed.

3 unit tests cover the warn / no-warn paths."
```

---

## Task 3: CLI exec run auto-build gate

**Files:**
- Modify: `crates/rowforge-cli/src/exec_cmd.rs` (or wherever the spawn path lives — verify)
- Test: integration test in `crates/rowforge-cli/tests/`

- [ ] **Step 1: Find the spawn site**

```bash
grep -nE 'pool_streaming|spawn_worker|start_run' crates/rowforge-cli/src/*.rs | head -20
```

Identify the function that loads the manifest and dispatches workers. Read enough context to know where to insert the build gate (logically: after manifest load, before the first worker spawn).

- [ ] **Step 2: Add the gate**

Insert at the chosen call site:

```rust
use rowforge_core::build::{needs_build, run_build, BuildError};

// ... after manifest is loaded and handler_dir is known ...

if needs_build(&handler_dir, &manifest) {
    eprintln!("[rowforge] building {} ...", manifest.name);
    let started = std::time::Instant::now();
    match run_build(&handler_dir, &manifest) {
        Ok(outcome) => {
            eprintln!(
                "[rowforge] build ok ({} ms)",
                outcome.finished_at.signed_duration_since(outcome.started_at).num_milliseconds()
            );
            let _ = started; // suppress unused if we don't print this
        }
        Err(BuildError::BuildFailed { exit_code, outcome, .. }) => {
            eprintln!("[rowforge] build failed (exit {}):", exit_code);
            if !outcome.stdout.is_empty() {
                eprint!("{}", outcome.stdout);
            }
            if !outcome.stderr.is_empty() {
                eprint!("{}", outcome.stderr);
            }
            std::process::exit(2);
        }
        Err(BuildError::ToolchainMissing { tool }) => {
            eprintln!("[rowforge] build tool '{}' not found in PATH", tool);
            std::process::exit(2);
        }
        Err(BuildError::NoBuildCommand) => {
            // needs_build returned true but NoBuildCommand fires only when build is None.
            // Should never happen; fall through.
            unreachable!("needs_build/run_build invariant violated");
        }
        Err(BuildError::Io(e)) => {
            eprintln!("[rowforge] build io error: {}", e);
            std::process::exit(2);
        }
    }
}
```

- [ ] **Step 3: Integration test — auto-build path**

Create or extend `crates/rowforge-cli/tests/exec_run_build.rs`:

```rust
use std::path::PathBuf;
use std::process::Command;

#[test]
fn exec_run_auto_builds_stale_handler() {
    // Skip on CI without `go`; check ahead of time.
    if which::which("sh").is_err() {
        eprintln!("skipping: sh not available");
        return;
    }

    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path();
    let handler_dir = workspace.join("handlers/stub");
    std::fs::create_dir_all(&handler_dir).unwrap();

    // Tiny shell-based handler: build step creates an executable that
    // echoes outcomes; cmd runs it.
    std::fs::write(
        handler_dir.join("rowforge.yaml"),
        r#"
name: stub
kind: row
primary_field: id
entry:
  cmd: ["./stub-bin"]
  build: ["sh", "-c", "cat > stub-bin <<'EOF'\n#!/bin/sh\nwhile IFS= read -r line; do echo \"{\\\"row_id\\\":1,\\\"status\\\":\\\"success\\\",\\\"output\\\":{}}\"; done\nEOF\nchmod +x stub-bin"]
"#,
    )
    .unwrap();
    // Source file to influence mtime decisions.
    std::fs::write(handler_dir.join("handler.go"), "// placeholder\n").unwrap();

    // CSV with one row.
    let csv = workspace.join("input.csv");
    std::fs::write(&csv, "id\n1\n").unwrap();

    let exe = env!("CARGO_BIN_EXE_rowforge");
    let exec_start = Command::new(exe)
        .env("ROWFORGE_HOME", workspace)
        .args(["exec", "start", "--csv", csv.to_str().unwrap(), "--name", "smoke"])
        .output()
        .expect("exec start runs");
    assert!(exec_start.status.success(), "exec start failed: {}", String::from_utf8_lossy(&exec_start.stderr));

    let stdout = String::from_utf8_lossy(&exec_start.stdout);
    let exec_id = stdout.lines().find(|l| l.starts_with("e_")).expect("exec id printed");

    let run = Command::new(exe)
        .env("ROWFORGE_HOME", workspace)
        .args([
            "exec", "run",
            "--handler", handler_dir.to_str().unwrap(),
            exec_id.trim(),
            "--workers", "1",
        ])
        .output()
        .expect("exec run runs");

    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(run.status.success(), "exec run failed:\nSTDOUT:\n{}\nSTDERR:\n{}", String::from_utf8_lossy(&run.stdout), stderr);
    assert!(stderr.contains("building stub"), "expected auto-build banner; got: {}", stderr);
    assert!(handler_dir.join("stub-bin").exists(), "binary should exist after auto-build");
}
```

> If `tempfile`, `which` aren't in `[dev-dependencies]` for `rowforge-cli`, add them.

- [ ] **Step 4: Run the integration test**

```bash
cargo test -p rowforge-cli --test exec_run_build
```

Expected: PASS. If the test infrastructure is wrong (env var name, command flags), adjust to match the actual CLI argv shape.

- [ ] **Step 5: Add build-failure CLI test**

Append:

```rust
#[test]
fn exec_run_exits_nonzero_when_build_fails() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path();
    let handler_dir = workspace.join("handlers/broken");
    std::fs::create_dir_all(&handler_dir).unwrap();
    std::fs::write(
        handler_dir.join("rowforge.yaml"),
        r#"
name: broken
kind: row
primary_field: id
entry:
  cmd: ["./broken-bin"]
  build: ["sh", "-c", "echo build-fail >&2; exit 7"]
"#,
    )
    .unwrap();
    std::fs::write(handler_dir.join("handler.go"), "// placeholder\n").unwrap();

    let csv = workspace.join("input.csv");
    std::fs::write(&csv, "id\n1\n").unwrap();

    let exe = env!("CARGO_BIN_EXE_rowforge");
    let exec_start = Command::new(exe)
        .env("ROWFORGE_HOME", workspace)
        .args(["exec", "start", "--csv", csv.to_str().unwrap(), "--name", "smoke"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&exec_start.stdout);
    let exec_id = stdout.lines().find(|l| l.starts_with("e_")).unwrap().trim();

    let run = Command::new(exe)
        .env("ROWFORGE_HOME", workspace)
        .args([
            "exec", "run",
            "--handler", handler_dir.to_str().unwrap(),
            exec_id,
            "--workers", "1",
        ])
        .output()
        .unwrap();
    assert!(!run.status.success(), "expected non-zero exit on build failure");
    assert!(String::from_utf8_lossy(&run.stderr).contains("build failed"));
}
```

- [ ] **Step 6: Run all CLI tests**

```bash
cargo test -p rowforge-cli
```

Expected: all PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/rowforge-cli/src/exec_cmd.rs crates/rowforge-cli/tests/exec_run_build.rs crates/rowforge-cli/Cargo.toml
git commit -m "rowforge-cli: exec run auto-builds stale handlers

Before the first worker spawns, exec run checks rowforge-core::build::
needs_build. When true, runs run_build sync (cwd=handler_dir) and:
- success: prints '[rowforge] build ok (NNN ms)' and continues
- BuildFailed: dumps full stdout+stderr to stderr, exits 2
- ToolchainMissing: prints friendly 'build tool X not found in PATH',
  exits 2
- Io: prints error, exits 2

This closes today's ENOENT pain when running examples/handlers/*
on a fresh checkout — the build step from rowforge.yaml now actually
runs.

2 integration tests cover the auto-build happy path and build-failure
non-zero-exit path using sh-based stub handlers."
```

---

## Task 4: CLI `rowforge handler build [name]` subcommand

**Files:**
- Modify: `crates/rowforge-cli/src/main.rs` (or `cli.rs` — wherever clap definitions live)
- Create: `crates/rowforge-cli/src/handler_build_cmd.rs`
- Test: `crates/rowforge-cli/tests/handler_build_cmd.rs`

- [ ] **Step 1: Verify clap structure**

```bash
grep -nE '#\[derive\(Subcommand\)\]|#\[command' crates/rowforge-cli/src/*.rs | head -20
```

Find the top-level Subcommand enum and any `handler` sub-subcommand grouping. Plan 7 didn't add a `handler` subcommand, so likely this is the first one.

- [ ] **Step 2: Add `handler` subcommand to the CLI tree**

In `main.rs` (or `cli.rs`):

```rust
#[derive(Subcommand)]
enum Cmd {
    // ...existing variants...
    /// Operate on handlers (build, validate, etc.).
    Handler {
        #[command(subcommand)]
        action: HandlerCmd,
    },
}

#[derive(Subcommand)]
enum HandlerCmd {
    /// Build one or all handlers under <workspace>/handlers/.
    Build {
        /// Handler name (omit to build all).
        name: Option<String>,
        /// Force rebuild even when not stale.
        #[arg(long)]
        force: bool,
    },
}
```

Dispatch:

```rust
match cli.cmd {
    // ...
    Cmd::Handler { action } => match action {
        HandlerCmd::Build { name, force } => {
            handler_build_cmd::run(workspace, name, force)
        }
    },
}
```

- [ ] **Step 3: Create the subcommand module**

`crates/rowforge-cli/src/handler_build_cmd.rs`:

```rust
use rowforge_core::build::{needs_build, run_build, BuildError};
use rowforge_core::manifest::Manifest;
use std::path::Path;

pub fn run(workspace: &Path, name: Option<String>, force: bool) -> anyhow::Result<()> {
    let handlers_dir = workspace.join("handlers");
    if !handlers_dir.is_dir() {
        eprintln!("[rowforge] no handlers/ directory in workspace");
        std::process::exit(0);
    }
    let targets: Vec<std::path::PathBuf> = match name {
        Some(n) => {
            let d = handlers_dir.join(&n);
            if !d.is_dir() {
                eprintln!("[rowforge] handler '{}' not found", n);
                std::process::exit(1);
            }
            vec![d]
        }
        None => std::fs::read_dir(&handlers_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| e.path())
            .collect(),
    };

    let mut failed = 0usize;
    for dir in targets {
        let name = dir.file_name().unwrap().to_string_lossy().to_string();
        let manifest = match Manifest::load_from_dir(&dir) {
            Ok(report) => match report.manifest {
                Some(m) => m,
                None => {
                    eprintln!("[{}] skipped (invalid manifest)", name);
                    continue;
                }
            },
            Err(e) => {
                eprintln!("[{}] skipped: {}", name, e);
                continue;
            }
        };
        if manifest.entry.build.is_none() {
            eprintln!("[{}] skipped (no entry.build)", name);
            continue;
        }
        if !force && !needs_build(&dir, &manifest) {
            eprintln!("[{}] up to date", name);
            continue;
        }
        match run_build(&dir, &manifest) {
            Ok(outcome) => {
                let dur = outcome
                    .finished_at
                    .signed_duration_since(outcome.started_at)
                    .num_milliseconds();
                eprintln!("[{}] ok ({} ms)", name, dur);
            }
            Err(BuildError::BuildFailed { exit_code, outcome, .. }) => {
                eprintln!("[{}] failed (exit {})", name, exit_code);
                eprint!("{}", outcome.stdout);
                eprint!("{}", outcome.stderr);
                failed += 1;
            }
            Err(BuildError::ToolchainMissing { tool }) => {
                eprintln!("[{}] toolchain missing: {}", name, tool);
                failed += 1;
            }
            Err(e) => {
                eprintln!("[{}] error: {}", name, e);
                failed += 1;
            }
        }
    }
    let code = failed.min(125) as i32;
    std::process::exit(code);
}
```

> Adapt `Manifest::load_from_dir` shape to match actual API.

Register the module in `main.rs`:

```rust
mod handler_build_cmd;
```

- [ ] **Step 4: Integration test**

`crates/rowforge-cli/tests/handler_build_cmd.rs`:

```rust
use std::process::Command;
use std::path::PathBuf;

fn workspace_with_handlers() -> tempfile::TempDir {
    let tmp = tempfile::TempDir::new().unwrap();
    let h_a = tmp.path().join("handlers/alpha");
    std::fs::create_dir_all(&h_a).unwrap();
    std::fs::write(
        h_a.join("rowforge.yaml"),
        r#"
name: alpha
kind: row
primary_field: id
entry:
  cmd: ["./bin"]
  build: ["sh", "-c", "echo built > bin && chmod +x bin"]
"#,
    )
    .unwrap();
    std::fs::write(h_a.join("handler.go"), "package main\n").unwrap();

    let h_b = tmp.path().join("handlers/no-build");
    std::fs::create_dir_all(&h_b).unwrap();
    std::fs::write(
        h_b.join("rowforge.yaml"),
        r#"
name: no-build
kind: row
primary_field: id
entry:
  cmd: ["python3", "handler.py"]
"#,
    )
    .unwrap();

    tmp
}

#[test]
fn handler_build_builds_all_handlers() {
    let tmp = workspace_with_handlers();
    let exe = env!("CARGO_BIN_EXE_rowforge");
    let out = Command::new(exe)
        .env("ROWFORGE_HOME", tmp.path())
        .args(["handler", "build"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "expected success: {}", stderr);
    assert!(stderr.contains("[alpha] ok"));
    assert!(stderr.contains("[no-build] skipped"));
    assert!(tmp.path().join("handlers/alpha/bin").exists());
}

#[test]
fn handler_build_builds_specific_handler() {
    let tmp = workspace_with_handlers();
    let exe = env!("CARGO_BIN_EXE_rowforge");
    let out = Command::new(exe)
        .env("ROWFORGE_HOME", tmp.path())
        .args(["handler", "build", "alpha"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(tmp.path().join("handlers/alpha/bin").exists());
}

#[test]
fn handler_build_force_rebuilds_even_when_fresh() {
    let tmp = workspace_with_handlers();
    let exe = env!("CARGO_BIN_EXE_rowforge");
    // First build to make alpha fresh.
    Command::new(exe).env("ROWFORGE_HOME", tmp.path()).args(["handler", "build", "alpha"]).output().unwrap();
    // Second build without --force → "up to date".
    let out_a = Command::new(exe).env("ROWFORGE_HOME", tmp.path()).args(["handler", "build", "alpha"]).output().unwrap();
    assert!(String::from_utf8_lossy(&out_a.stderr).contains("up to date"));
    // With --force → builds again.
    let out_b = Command::new(exe).env("ROWFORGE_HOME", tmp.path()).args(["handler", "build", "--force", "alpha"]).output().unwrap();
    assert!(String::from_utf8_lossy(&out_b.stderr).contains("[alpha] ok"));
}
```

- [ ] **Step 5: Run tests**

```bash
cargo test -p rowforge-cli --test handler_build_cmd
```

Expected: 3 PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/rowforge-cli/src/main.rs crates/rowforge-cli/src/handler_build_cmd.rs crates/rowforge-cli/tests/handler_build_cmd.rs
git commit -m "rowforge-cli: handler build [name] [--force] subcommand

New 'rowforge handler' group under clap with 'build' as first action:
- 'rowforge handler build' iterates <workspace>/handlers/* and
  builds each that has entry.build AND is stale (per needs_build).
- 'rowforge handler build <name>' targets one.
- '--force' bypasses the staleness check.

Per-handler outcomes printed to stderr ('[name] ok (NNN ms)' /
'[name] failed (exit N)' / '[name] up to date' / '[name] skipped').
Exit code = number of failures (capped at 125).

3 integration tests cover all-handlers, single-handler, and
--force paths."
```

---

## Task 5: studio-core UiError variants

**Files:**
- Modify: `crates/rowforge-studio-core/src/error.rs`
- Test: existing `crates/rowforge-studio-core/tests/foundation.rs` or `error.rs` unit tests

- [ ] **Step 1: Add the three variants**

Edit the `UiError` enum (preserve existing variants):

```rust
#[derive(Debug, Clone, thiserror::Error, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "message", rename_all = "snake_case")]
#[non_exhaustive]
pub enum UiError {
    // ...existing variants...

    /// Build subprocess exited non-zero.
    #[error("build failed for handler '{name}' (exit {exit_code})")]
    BuildFailed { name: String, exit_code: i32 },

    /// First token of entry.build not resolvable via which::which.
    #[error("build tool '{tool}' for handler '{name}' not found in PATH")]
    ToolchainMissing { name: String, tool: String },

    /// Attempted to build a handler whose manifest has no entry.build.
    #[error("handler '{name}' has no entry.build in its manifest")]
    NoBuildCommand { name: String },
}
```

> The exact serde tag/content pattern follows Plan 7. Confirm by reading the existing `UiError` shape before adding.

- [ ] **Step 2: Test serde shape**

Append a unit test (or extend an existing one):

```rust
#[test]
fn build_failed_serializes_with_named_fields() {
    let e = UiError::BuildFailed { name: "alpha".into(), exit_code: 7 };
    let v = serde_json::to_value(&e).unwrap();
    assert_eq!(v["kind"], "build_failed");
    // Adjacent tagging with content="message" means the data nests under "message".
    // Adjust assertions to whatever the existing UiError serde envelope uses.
}
```

> Plan 7's UiError uses `#[serde(tag = "kind", content = "message")]` adjacent tagging. Confirm and match.

- [ ] **Step 3: Run cargo test, ensure compile + test PASS**

```bash
cargo test -p rowforge-studio-core --lib error
```

- [ ] **Step 4: Commit**

```bash
git add crates/rowforge-studio-core/src/error.rs
git commit -m "studio-core: UiError gains BuildFailed/ToolchainMissing/NoBuildCommand

Three new variants for Plan 8 build path:
- BuildFailed { name, exit_code } — non-zero build exit
- ToolchainMissing { name, tool } — entry.build[0] missing from PATH
- NoBuildCommand { name } — Build attempted on handler with no
  entry.build

Same adjacent-tag serde envelope as the rest of UiError so existing
TS uiErrorMessage switch can extend cleanly."
```

---

## Task 6: studio-core handler::build + HandlerDetail.last_build

**Files:**
- Modify: `crates/rowforge-studio-core/src/handler.rs` (add `build` fn + `HandlerDetail.last_build`)
- Modify: `crates/rowforge-studio-core/src/lib.rs` (add `build_cache` to StudioCore + `handler_build` method)
- Test: `crates/rowforge-studio-core/tests/foundation.rs`

- [ ] **Step 1: Add `last_build` to HandlerDetail**

Find the struct in `handler.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct HandlerDetail {
    // ...existing fields...
    pub last_build: Option<rowforge_core::build::BuildOutcome>,  // NEW
}
```

Find every constructor of `HandlerDetail` (search `HandlerDetail {`) and seed `last_build: None`. Note this is filled in at the `StudioCore::handler_show` level, not inside `handler::show` — because the cache lives on StudioCore.

- [ ] **Step 2: Implement `handler::build_raw`**

Add to `handler.rs`:

```rust
use rowforge_core::build::{run_build, BuildError, BuildOutcome};

/// Low-level: runs the manifest's entry.build sync. Returns the raw
/// `BuildError` so callers can decide cache + UiError mapping. Always
/// forces — does NOT call needs_build (Studio's contract per design §7.3).
pub fn build_raw(workspace_root: &Path, name: &str) -> Result<BuildOutcome, BuildError> {
    if !validate_name(name) {
        return Err(BuildError::Io(format!("invalid handler name: {}", name)));
    }
    let dir = workspace_root.join("handlers").join(name);
    if !dir.is_dir() {
        return Err(BuildError::Io(format!("handler '{}' not found", name)));
    }
    let manifest = crate::manifest_helpers::load_manifest(&dir)
        .map_err(|e| BuildError::Io(format!("manifest load: {}", e)))?;
    run_build(&dir, &manifest)
}
```

> Adjust import paths to existing helpers (the manifest load path was added in Plan 7 T3 — find it).

- [ ] **Step 3: Wire StudioCore**

In `crates/rowforge-studio-core/src/lib.rs`:

```rust
use std::collections::HashMap;
use std::sync::Mutex;
use rowforge_core::build::BuildOutcome;

pub struct StudioCore {
    // ...existing fields...
    build_cache: Mutex<HashMap<String, BuildOutcome>>,
}

impl StudioCore {
    // ...existing methods...

    pub fn handler_build(&self, name: &str) -> Result<BuildOutcome, crate::UiError> {
        // Refuse early if manifest has no entry.build (cleaner UiError than letting
        // BuildError::NoBuildCommand propagate via a string-Io fallback).
        let dir = self.workspace.root.as_path().join("handlers").join(name);
        let manifest_report = rowforge_core::manifest::Manifest::load_from_dir(&dir)
            .map_err(|e| crate::UiError::Io(format!("manifest load: {}", e)))?;
        let manifest = manifest_report.manifest
            .ok_or_else(|| crate::UiError::HandlerNotFound { name: name.to_string() })?;
        if manifest.entry.build.is_none() {
            return Err(crate::UiError::NoBuildCommand { name: name.to_string() });
        }

        match crate::handler::build_raw(self.workspace.root.as_path(), name) {
            Ok(outcome) => {
                self.build_cache.lock().unwrap().insert(name.to_string(), outcome.clone());
                Ok(outcome)
            }
            Err(rowforge_core::build::BuildError::BuildFailed { exit_code, outcome, .. }) => {
                // Cache the failed outcome so the UI can show the log.
                self.build_cache.lock().unwrap().insert(name.to_string(), outcome);
                Err(crate::UiError::BuildFailed { name: name.to_string(), exit_code })
            }
            Err(rowforge_core::build::BuildError::ToolchainMissing { tool }) => {
                Err(crate::UiError::ToolchainMissing { name: name.to_string(), tool })
            }
            Err(rowforge_core::build::BuildError::NoBuildCommand) => {
                // Unreachable after the entry.build check above, but propagate cleanly.
                Err(crate::UiError::NoBuildCommand { name: name.to_string() })
            }
            Err(rowforge_core::build::BuildError::Io(e)) => Err(crate::UiError::Io(e)),
        }
    }
}
```

Initialize `build_cache: Mutex::new(HashMap::new())` in the StudioCore constructor (search for `StudioCore {`).

- [ ] **Step 4: Inject cache into handler_show**

Find `StudioCore::handler_show`:

```rust
pub fn handler_show(&self, name: &str) -> Result<crate::handler::HandlerDetail, crate::UiError> {
    let mut detail = crate::handler::show(self.workspace.root.as_path(), name)?;
    detail.last_build = self.build_cache.lock().unwrap().get(name).cloned();
    Ok(detail)
}
```

- [ ] **Step 5: Tests**

Append to `crates/rowforge-studio-core/tests/foundation.rs`:

```rust
#[test]
fn handler_build_no_command_returns_no_build_command_error() {
    let tmp = TempDir::new().unwrap();
    let core = open_test_workspace(tmp.path());  // existing helper
    let h = tmp.path().join("handlers/nobuild");
    std::fs::create_dir_all(&h).unwrap();
    std::fs::write(
        h.join("rowforge.yaml"),
        "name: nobuild\nkind: row\nprimary_field: id\nentry:\n  cmd: [\"python3\", \"x.py\"]\n",
    ).unwrap();

    let err = core.handler_build("nobuild").unwrap_err();
    assert!(matches!(err, UiError::NoBuildCommand { ref name } if name == "nobuild"));
}

#[test]
fn handler_build_success_populates_last_build_in_show() {
    let tmp = TempDir::new().unwrap();
    let core = open_test_workspace(tmp.path());
    let h = tmp.path().join("handlers/sh");
    std::fs::create_dir_all(&h).unwrap();
    std::fs::write(
        h.join("rowforge.yaml"),
        "name: sh\nkind: row\nprimary_field: id\nentry:\n  cmd: [\"./bin\"]\n  build: [\"sh\", \"-c\", \"echo hi\"]\n",
    ).unwrap();

    let outcome = core.handler_build("sh").expect("build ok");
    assert_eq!(outcome.exit_code, 0);

    let detail = core.handler_show("sh").expect("show ok");
    assert!(detail.last_build.is_some());
    assert_eq!(detail.last_build.unwrap().exit_code, 0);
}

#[test]
fn handler_build_failure_caches_outcome_for_inspection() {
    let tmp = TempDir::new().unwrap();
    let core = open_test_workspace(tmp.path());
    let h = tmp.path().join("handlers/bad");
    std::fs::create_dir_all(&h).unwrap();
    std::fs::write(
        h.join("rowforge.yaml"),
        "name: bad\nkind: row\nprimary_field: id\nentry:\n  cmd: [\"./bin\"]\n  build: [\"sh\", \"-c\", \"echo oops >&2; exit 5\"]\n",
    ).unwrap();

    let err = core.handler_build("bad").unwrap_err();
    assert!(matches!(err, UiError::BuildFailed { exit_code: 5, .. }));

    let detail = core.handler_show("bad").expect("show ok");
    let lb = detail.last_build.expect("failed outcome cached");
    assert_eq!(lb.exit_code, 5);
    assert!(lb.stderr.contains("oops"));
}

#[test]
fn handler_build_toolchain_missing_returns_error_without_cache_write() {
    let tmp = TempDir::new().unwrap();
    let core = open_test_workspace(tmp.path());
    let h = tmp.path().join("handlers/notool");
    std::fs::create_dir_all(&h).unwrap();
    std::fs::write(
        h.join("rowforge.yaml"),
        "name: notool\nkind: row\nprimary_field: id\nentry:\n  cmd: [\"./bin\"]\n  build: [\"this-tool-xyz-does-not-exist\"]\n",
    ).unwrap();

    let err = core.handler_build("notool").unwrap_err();
    assert!(matches!(err, UiError::ToolchainMissing { .. }));

    let detail = core.handler_show("notool").expect("show ok");
    assert!(detail.last_build.is_none(), "no outcome to cache on toolchain-missing");
}
```

- [ ] **Step 6: Run tests**

```bash
cargo test -p rowforge-studio-core
```

Expected: existing tests still pass; 4 new ones PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/rowforge-studio-core/src/handler.rs crates/rowforge-studio-core/src/lib.rs crates/rowforge-studio-core/tests/foundation.rs
git commit -m "studio-core: handler_build + last_build cache

StudioCore gains an in-memory build_cache (HashMap<HandlerName,
BuildOutcome>) lost on restart per spec §8.4.7 / design §7.2.

handler_build(name):
- pre-flight checks entry.build is Some (else NoBuildCommand)
- calls handler::build_raw -> rowforge-core::build::run_build
- success → cache + Ok(outcome)
- BuildFailed → cache the outcome (UI shows the log) + UiError
- ToolchainMissing → UiError, no cache write
- NoBuildCommand / Io → propagate

handler_show now injects last_build from the cache into HandlerDetail
so the React detail page can render LastBuildSection without an
extra round-trip.

4 new integration tests cover success / build-failure / toolchain-
missing / no-build-command paths."
```

---

## Task 7: Tauri shell — handler_build command

**Files:**
- Modify: `apps/rowforge-studio/src-tauri/src/commands.rs`
- Modify: `apps/rowforge-studio/src-tauri/src/lib.rs` (register in `invoke_handler!`)
- Modify: `apps/rowforge-studio/src-tauri/tests/ipc_contract.rs`

- [ ] **Step 1: Add the command**

Append to `commands.rs`:

```rust
use rowforge_core::build::BuildOutcome;

#[tauri::command]
pub async fn handler_build(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    name: String,
) -> Result<BuildOutcome, UiError> {
    let core_arc = state.core.clone();
    let result = tokio::task::spawn_blocking(move || {
        let guard = core_arc.lock().unwrap_or_else(|p| p.into_inner());
        let core = guard.as_ref()
            .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
        core.handler_build(&name)
    })
    .await
    .map_err(|e| UiError::Io(format!("spawn_blocking join: {}", e)))?;

    // Emit list-refresh hint so HandlerSummary.last_modified (which folds
    // over top-level entries, including the new binary) gets picked up.
    use tauri::Emitter;
    let _ = app.emit("handlers:list", ());
    result
}
```

> Verify `state.core` is an `Arc<Mutex<Option<StudioCore>>>` — if it's just `Arc<Mutex<StudioCore>>` adjust the unwrap chain.

- [ ] **Step 2: Register**

In `lib.rs`'s `generate_handler![...]` block, add `commands::handler_build`.

- [ ] **Step 3: Extend ipc_contract test**

In `apps/rowforge-studio/src-tauri/tests/ipc_contract.rs`, follow the existing pattern (compile-time symbol or JSON shape):

```rust
#[test]
fn plan8_handler_build_command_registered() {
    let _ = crate::commands::handler_build;
}

#[test]
fn plan8_build_outcome_json_shape() {
    use chrono::Utc;
    let outcome = rowforge_core::build::BuildOutcome {
        started_at: Utc::now(),
        finished_at: Utc::now(),
        exit_code: 0,
        command: vec!["sh".into(), "-c".into(), "echo".into()],
        stdout: "hi".into(),
        stderr: "".into(),
    };
    let v = serde_json::to_value(&outcome).unwrap();
    assert_eq!(v["exit_code"], 0);
    assert!(v["command"].is_array());
    assert!(v["stdout"].is_string());
}
```

- [ ] **Step 4: Verify**

```bash
cargo test -p rowforge-studio --test ipc_contract
cargo build
```

Expected: builds clean; ipc_contract gains 2.

- [ ] **Step 5: Commit**

```bash
git add apps/rowforge-studio/src-tauri/src/commands.rs apps/rowforge-studio/src-tauri/src/lib.rs apps/rowforge-studio/src-tauri/tests/ipc_contract.rs
git commit -m "studio-shell: handler_build Tauri command

Async command wraps StudioCore::handler_build in spawn_blocking so
the tokio reactor stays free during a multi-second go build / cargo
build. After success, emits handlers:list event so HandlerList's
last_modified picks up the new binary mtime (Plan 7 round-2 added
max-mtime-over-contents).

Registered in invoke_handler!. ipc_contract +2 (symbol + JSON shape)."
```

---

## Task 8: TS mirrors + useHandlerBuild hook

**Files:**
- Modify: `apps/rowforge-studio/src/ipc/types.ts`
- Modify: `apps/rowforge-studio/src/ipc/client.ts`
- Modify: `apps/rowforge-studio/src/ipc/use-handlers.ts`
- Test: `apps/rowforge-studio/src/__tests__/ui-error.test.ts`

- [ ] **Step 1: Add BuildOutcome + HandlerDetail.last_build**

In `types.ts`:

```ts
export interface BuildOutcome {
  started_at: string;        // ISO 8601
  finished_at: string;
  exit_code: number;
  command: string[];
  stdout: string;
  stderr: string;
}

export interface HandlerDetail {
  // ...existing fields...
  last_build: BuildOutcome | null;
}
```

Find every place that constructs / asserts on HandlerDetail (mocks in tests, etc.) and add `last_build: null` where needed.

- [ ] **Step 2: Extend UiError union**

```ts
export type UiError =
  // ...existing arms...
  | { kind: "build_failed";       message: { name: string; exit_code: number } | null }
  | { kind: "toolchain_missing";  message: { name: string; tool: string } | null }
  | { kind: "no_build_command";   message: { name: string } | null };
```

> Match the existing arm shape — Plan 7 used `message` as the content slot. If the existing arms are different, mirror them.

Extend `uiErrorMessage`:

```ts
case "build_failed":
  return `Build failed for "${e.message?.name ?? "handler"}" (exit ${e.message?.exit_code ?? "?"}). See the Last build section for details.`;
case "toolchain_missing":
  return `Build tool "${e.message?.tool ?? "?"}" not found in PATH. Install it or update entry.build in your manifest.`;
case "no_build_command":
  return `Handler "${e.message?.name ?? "?"}" has no entry.build command in rowforge.yaml.`;
```

- [ ] **Step 3: Test UiError mirrors**

In `ui-error.test.ts`, add:

```ts
it("renders build_failed copy", () => {
  expect(uiErrorMessage({ kind: "build_failed", message: { name: "alpha", exit_code: 3 } })).toContain("Build failed");
});

it("renders toolchain_missing copy", () => {
  expect(uiErrorMessage({ kind: "toolchain_missing", message: { name: "alpha", tool: "go" } })).toContain("go");
});

it("renders no_build_command copy", () => {
  expect(uiErrorMessage({ kind: "no_build_command", message: { name: "alpha" } })).toContain("entry.build");
});
```

- [ ] **Step 4: ipc client wrapper**

In `client.ts`:

```ts
handler_build: (args: { name: string }) => invoke<BuildOutcome>("handler_build", args),
```

- [ ] **Step 5: TanStack hook**

In `use-handlers.ts`:

```ts
import type { BuildOutcome } from "./types";

export const useHandlerBuild = () => {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: { name: string }) => ipc.handler_build(args),
    onSuccess: (_data, vars) => {
      qc.invalidateQueries({ queryKey: ["handler_show", vars.name] });
      qc.invalidateQueries({ queryKey: ["handler_list"] });
    },
  });
};
```

- [ ] **Step 6: Verify**

```bash
cd apps/rowforge-studio
pnpm tsc -b
pnpm test
```

Expected: TS clean; 3 new ui-error tests pass.

- [ ] **Step 7: Commit**

```bash
git add apps/rowforge-studio/src/ipc/types.ts apps/rowforge-studio/src/ipc/client.ts apps/rowforge-studio/src/ipc/use-handlers.ts apps/rowforge-studio/src/__tests__/ui-error.test.ts
git commit -m "studio-shell: TS mirrors for BuildOutcome + useHandlerBuild hook

types.ts:
- BuildOutcome { started_at, finished_at, exit_code, command,
  stdout, stderr } — mirror of rowforge-core BuildOutcome
- HandlerDetail.last_build: BuildOutcome | null
- UiError gains 3 arms: build_failed, toolchain_missing,
  no_build_command. uiErrorMessage switch extended with friendly
  copy for each.

client.ts: ipc.handler_build wrapper.

use-handlers.ts: useHandlerBuild mutation hook; invalidates
handler_show + handler_list on success so list timestamps and
detail's last_build refresh.

+3 vitest covering the new uiErrorMessage arms."
```

---

## Task 9: LastBuildSection + HandlerDetailPage integration

**Files:**
- Create: `apps/rowforge-studio/src/components/LastBuildSection.tsx`
- Create: `apps/rowforge-studio/src/components/__tests__/LastBuildSection.test.tsx`
- Modify: `apps/rowforge-studio/src/pages/HandlerDetailPage.tsx`
- Modify: `apps/rowforge-studio/src/pages/__tests__/HandlerDetailPage.test.tsx`

- [ ] **Step 1: Build the component**

`LastBuildSection.tsx`:

```tsx
import { useState } from "react";
import type { BuildOutcome } from "@/ipc/types";

interface Props {
  last_build: BuildOutcome | null;
  pending: boolean;
}

export function LastBuildSection({ last_build, pending }: Props) {
  const [open, setOpen] = useState(false);

  if (pending) {
    return (
      <section className="space-y-2">
        <h2 className="text-sm font-medium uppercase text-muted-foreground">Last build</h2>
        <div className="flex items-center gap-2 text-sm text-muted-foreground">
          <span className="inline-block h-3 w-3 animate-spin rounded-full border-2 border-zinc-400 border-t-transparent" />
          Building…
        </div>
      </section>
    );
  }
  if (!last_build) return null;

  const success = last_build.exit_code === 0;
  const durationMs =
    new Date(last_build.finished_at).getTime() -
    new Date(last_build.started_at).getTime();
  const badgeCls = success
    ? "bg-green-500/15 text-green-300 border-green-500/30"
    : "bg-red-500/15 text-red-300 border-red-500/30";

  return (
    <section className="space-y-2">
      <h2 className="text-sm font-medium uppercase text-muted-foreground">Last build</h2>
      <div className="flex items-center gap-3">
        <span className={`inline-block rounded px-2 py-0.5 text-xs border ${badgeCls}`}>
          {success ? "success" : "failed"}
        </span>
        <span className="text-sm text-muted-foreground">
          exit {last_build.exit_code} · {durationMs} ms · {new Date(last_build.finished_at).toLocaleTimeString()}
        </span>
      </div>
      <button
        onClick={() => setOpen((v) => !v)}
        className="text-xs text-blue-400 hover:underline"
      >
        {open ? "Hide output ▴" : "Show output ▾"}
      </button>
      {open && (
        <pre className="max-h-64 overflow-auto rounded border border-zinc-700 bg-zinc-900 p-2 text-xs font-mono whitespace-pre-wrap">
          {last_build.stdout}
          {last_build.stderr && "\n--- stderr ---\n"}
          {last_build.stderr}
        </pre>
      )}
    </section>
  );
}
```

- [ ] **Step 2: Test LastBuildSection (4 states)**

`LastBuildSection.test.tsx`:

```tsx
import { describe, it, expect } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { LastBuildSection } from "@/components/LastBuildSection";
import type { BuildOutcome } from "@/ipc/types";

const ok: BuildOutcome = {
  started_at: "2026-05-25T10:00:00Z",
  finished_at: "2026-05-25T10:00:01Z",
  exit_code: 0,
  command: ["sh", "-c", "echo hi"],
  stdout: "hi\n",
  stderr: "",
};
const fail: BuildOutcome = { ...ok, exit_code: 7, stderr: "oops\n", stdout: "" };

describe("LastBuildSection", () => {
  it("renders nothing when no build and not pending", () => {
    const { container } = render(<LastBuildSection last_build={null} pending={false} />);
    expect(container.firstChild).toBeNull();
  });

  it("renders 'Building…' when pending", () => {
    render(<LastBuildSection last_build={null} pending={true} />);
    expect(screen.getByText(/Building…/)).toBeInTheDocument();
  });

  it("renders success badge for exit_code 0", () => {
    render(<LastBuildSection last_build={ok} pending={false} />);
    expect(screen.getByText("success")).toBeInTheDocument();
    expect(screen.getByText(/exit 0/)).toBeInTheDocument();
  });

  it("renders failed badge for non-zero exit", () => {
    render(<LastBuildSection last_build={fail} pending={false} />);
    expect(screen.getByText("failed")).toBeInTheDocument();
    expect(screen.getByText(/exit 7/)).toBeInTheDocument();
  });

  it("expands log on Show output click", () => {
    render(<LastBuildSection last_build={fail} pending={false} />);
    expect(screen.queryByText(/oops/)).not.toBeInTheDocument();
    fireEvent.click(screen.getByText(/Show output ▾/));
    expect(screen.getByText((c) => c.includes("oops"))).toBeInTheDocument();
  });
});
```

- [ ] **Step 3: Wire into HandlerDetailPage**

Edit `HandlerDetailPage.tsx`. In the header action buttons section, add Build between Open in editor and Rename:

```tsx
import { useHandlerBuild } from "@/ipc/use-handlers";
import { LastBuildSection } from "@/components/LastBuildSection";

// inside the component:
const build = useHandlerBuild();
const hasBuildCmd = data?.manifest?.entry?.build != null;

// in DetailHeader:
{hasBuildCmd && (
  <Button
    onClick={() => build.mutate({ name })}
    disabled={build.isPending}
    variant="outline"
  >
    {build.isPending ? "Building…" : "Build"}
  </Button>
)}
```

Render `<LastBuildSection>` between Manifest section and Files section:

```tsx
<ManifestSection detail={data} />
<LastBuildSection last_build={data.last_build} pending={build.isPending} />
<SourceFilesSection detail={data} />
```

> The exact `manifest.entry.build` path depends on the Manifest TS shape. If TS doesn't expose `entry.build`, fall back to checking via a cast: `(data?.manifest as any)?.entry?.build != null`. Or thread a hint from the backend (HandlerSummary could carry `has_build_command: bool` — but adding that is more scope).

- [ ] **Step 4: HandlerDetailPage tests**

Add to `HandlerDetailPage.test.tsx`:

```tsx
it("renders Build button when manifest has entry.build", () => {
  const detail = makeValidDetail();
  (detail as any).manifest.entry.build = ["sh", "-c", "echo"];
  // mock ipc.handler_show to return detail
  // ...render, assert getByRole("button", { name: /^Build$/ })
});

it("hides Build button when manifest has no entry.build", () => {
  const detail = makeValidDetail();
  // detail.manifest.entry.build undefined
  // ...render, assert queryByRole("button", { name: /^Build$/ }) is null
});

it("clicking Build invokes handler_build", async () => {
  // mock + render + click + assert invoke called with name
});
```

(Mirror existing test mocking pattern.)

- [ ] **Step 5: Verify**

```bash
cd apps/rowforge-studio
pnpm tsc -b
pnpm test
pnpm build
```

Expected: all green. Vitest ~117.

- [ ] **Step 6: Commit**

```bash
git add apps/rowforge-studio/src/components/LastBuildSection.tsx apps/rowforge-studio/src/components/__tests__/LastBuildSection.test.tsx apps/rowforge-studio/src/pages/HandlerDetailPage.tsx apps/rowforge-studio/src/pages/__tests__/HandlerDetailPage.test.tsx
git commit -m "studio-shell: LastBuildSection + Build button in HandlerDetailPage

New LastBuildSection component renders 4 states:
- null + not pending: returns null (no rendered chrome)
- pending: animated spinner + 'Building…'
- success: green badge + 'exit 0 · NNN ms · HH:MM:SS' + expand-to-log
- failed: red badge + non-zero exit + expand-to-log

Logs render in a max-h-64 scrollable <pre> with stdout then a
'--- stderr ---' separator then stderr (handlers conventionally
write progress to stderr).

HandlerDetailPage:
- Build button in header action row, between Open in editor and
  Rename… Hidden when manifest.entry.build is null/undefined.
  Disabled + 'Building…' label while mutation pending.
- LastBuildSection rendered between Manifest and Files sections,
  driven by data.last_build (cached on StudioCore for the session)
  and build.isPending (from the useHandlerBuild mutation).

+5 LastBuildSection tests + 3 HandlerDetailPage tests covering
button visibility, click → mutation, and the cached last_build
display path."
```

---

## Task 10: Spec docs + HUMAN_SMOKE walkthrough

**Files:**
- Modify: `docs/spec/studio/part-8-handler-authoring.md`
- Modify: `docs/spec/studio/part-5-api.md`
- Modify: `docs/spec/studio/zh-Hant/part-8-handler-authoring.md`
- Modify: `docs/spec/studio/zh-Hant/part-5-api.md`
- Modify: `apps/rowforge-studio/HUMAN_SMOKE.md` (append Plan 08 section)

- [ ] **Step 1: Update part-8 §8.2 (manifest shape)**

Rewrite §8.2 to describe `entry.cmd: Vec<String>` + `entry.build: Option<Vec<String>>` (not top-level `build`/`run`). Drop the migration paragraph; note the shape is preserved as-is from prior plans.

- [ ] **Step 2: Update part-8 §8.3 (model)**

- Rename `BuildRecord` → `BuildOutcome` in the doc to match code
- Add `HandlerDetail.last_build: Option<BuildOutcome>` to the struct
- Drop `last_build: Option<BuildRecord>` field if it was named differently

- [ ] **Step 3: Mark §8.4.3 (smoke) + §8.4.5 (interlock) deferred**

Add a banner at the top of each:

```
> **Deferred from Plan 8** — see Plan 8 design doc §10. Smoke test
> and exec-run interlock will land in a later plan.
```

- [ ] **Step 4: Update part-8 §8.4.2 (build lifecycle) to match sync-minimal**

Replace the cancel + streaming language with:

```
Build is synchronous from the caller's perspective. Studio wraps
the call in tokio::task::spawn_blocking; the CLI runs on its main
thread. No mid-flight cancel in v1. Full stdout + stderr captured
and returned in BuildOutcome.

needs_build (caller-side staleness check, used by CLI):
- Returns false when entry.build is None.
- Returns false when entry.cmd[0] is an absolute path or a PATH-
  resolvable bare name (interpreter case: no binary concept).
- Otherwise treats entry.cmd[0] as a relative binary in handler_dir.
  Returns true when missing OR when max source mtime (.go .rs .py
  .js .ts .mjs .java .c .cpp .h .hpp, top-level) exceeds binary mtime.

CLI exec run honors needs_build before spawning workers. Studio
always forces (no staleness check) on Build button click.
```

- [ ] **Step 5: Update part-8 §8.5.3 (Tauri commands)**

Add `handler_build` row.

- [ ] **Step 6: Update part-8 §8.5.4 (UiError variants)**

Add `BuildFailed`, `ToolchainMissing`, `NoBuildCommand` entries with payload + when emitted + UI copy.

- [ ] **Step 7: Update part-5 §5.3 (UiError catalog) and §5.5 (commands)**

Mirror §8.5.3 + §8.5.4 additions in part-5.

- [ ] **Step 8: Update part-8 §8.4.2 validate_manifest section**

Add the 2 new warnings (`BuildToolNotInPath`, `CmdTargetMissing`) to the warning catalog.

- [ ] **Step 9: Mirror all changes in zh-Hant**

Re-do steps 1-8 in `docs/spec/studio/zh-Hant/part-8-handler-authoring.md` and `zh-Hant/part-5-api.md`.

- [ ] **Step 10: Add HUMAN_SMOKE Plan 08 section**

Append to `apps/rowforge-studio/HUMAN_SMOKE.md` (after the existing Plan 07 section):

```markdown
## Plan 08 additions — Handler build

### CLI auto-build (closes today's ENOENT pain)

1. Fresh checkout / clean workspace. Don't pre-build anything.
2. `rowforge exec start --csv path/to/any.csv --name smoke`
3. `rowforge exec run --handler examples/handlers/golang-stats-refund-records <exec_id> --sample 2 --workers 1`
4. Expected: stderr shows `[rowforge] building golang-stats-refund-records ...`
   then `[rowforge] build ok (NNN ms)` then the run proceeds. The
   binary is now present in the handler dir.
5. Re-run the same command → no build banner (binary is fresh).
6. `touch examples/handlers/golang-stats-refund-records/handler.go`
7. Re-run → build banner appears again (source mtime > binary mtime).

### CLI explicit build subcommand

8. `rowforge handler build` — builds every handler under
   `<workspace>/handlers/*` that has entry.build AND is stale.
   Per-handler outcomes printed to stderr.
9. `rowforge handler build alpha` — single handler.
10. `rowforge handler build --force alpha` — bypasses staleness check.

### CLI build failure

11. Edit a handler's rowforge.yaml to make entry.build a failing command
    (e.g. `["sh", "-c", "exit 3"]`).
12. `rowforge exec run --handler <dir> <exec_id>` → expected: stderr
    shows `[rowforge] build failed (exit 3):` followed by the build
    output, exit code 2 from the CLI.

### Studio Build button (happy path)

13. Open Studio. Navigate to a handler with entry.build (e.g.
    golang-stats-refund-records).
14. Detail page header shows a **Build** button between Open in editor
    and Rename…
15. Click Build → label changes to "Building…", button disabled.
16. After completion (~3-10 s for a Go handler): Last build section
    appears between Manifest and Files. Green "success" badge, exit 0,
    duration, timestamp.
17. Click "Show output ▾" → log expands; shows go build stdout (usually
    empty) and stderr.
18. Re-click "Hide output ▴" → collapses.

### Studio Build button (failure)

19. Edit a handler's rowforge.yaml to make entry.build fail
    (e.g. `["sh", "-c", "echo broken >&2; exit 5"]`).
20. From Studio's detail page, click Build.
21. Sonner toast: "Build failed for 'NAME' (exit 5). See the Last build
    section for details."
22. Last build section: red "failed" badge, exit 5. Expand → stderr
    contains "broken".

### Studio Build button hidden for python/node handlers

23. Navigate to a handler whose entry.cmd is `["python3", ...]` (no
    entry.build). Detail page header does NOT show a Build button.

### Toolchain missing

24. Edit a handler so entry.build is `["this-tool-xyz-does-not-exist"]`.
25. Click Build → toast: "Build tool 'this-tool-xyz-does-not-exist' not
    found in PATH. Install it or update entry.build in your manifest."
26. Last build section is NOT populated (no outcome to cache).

### Manifest validation warnings (Plan 8 additions)

27. Detail page of a handler whose entry.build first token isn't on
    PATH → Manifest section shows a yellow warning chip
    "build tool 'X' not found in PATH".
28. Detail page of a handler whose entry.cmd points to a missing
    relative file AND entry.build is None → yellow warning
    "entry.cmd target './X' not found".
29. Same setup but WITH entry.build present → no warning (build is
    expected to produce the target).

### Known Plan 8 limitations (deferred to later plans)

- No smoke test surface — handler verification still requires running
  an actual exec.
- No build cancel — long builds block the Build button until exit.
- stderr not streamed — log appears all at once at completion.
- No build / exec-run interlock — concurrent build + run on the same
  handler can race; user is expected to wait.
- BuildOutcome cache lost on Studio quit — re-open shows no "Last build"
  until the next build.
```

- [ ] **Step 11: Verify docs render**

```bash
ls docs/spec/studio/{,zh-Hant/}part-{5,8}*.md
git diff --stat docs/spec/studio/ apps/rowforge-studio/HUMAN_SMOKE.md
```

Expected: 5 files modified.

- [ ] **Step 12: Commit**

```bash
git add docs/spec/studio/ apps/rowforge-studio/HUMAN_SMOKE.md
git commit -m "docs: Plan 8 spec sync (en + zh-Hant) + HUMAN_SMOKE Plan 08

part-8:
- §8.2 rewritten to describe entry.cmd/entry.build (not top-level
  build/run) — matches code instead of vice-versa
- §8.3 BuildRecord renamed BuildOutcome; HandlerDetail.last_build
  documented
- §8.4.2 simplified to sync-minimal (no cancel, no streaming);
  needs_build matrix documented
- §8.4.3 (smoke) marked deferred banner
- §8.4.5 (interlock) marked deferred banner
- §8.5.3 / §8.5.4 add handler_build command + 3 UiError variants
- validate_manifest catalog gains BuildToolNotInPath +
  CmdTargetMissing warnings

part-5: mirrors §8.5 additions in §5.3 / §5.5.

zh-Hant: mirrored.

HUMAN_SMOKE: Plan 08 section appended — 29 numbered steps covering
CLI auto-build, explicit subcommand, build failure, Studio Build
button (happy/failure/toolchain-missing/hidden-for-python), manifest
warning rendering, and known Plan 8 limitations."
```

---

## Final verification + PR

```bash
cargo build && cargo test
cd apps/rowforge-studio
pnpm tsc -b && pnpm test && pnpm build
```

Expected counts:
- Cargo: 317 → ~333 (+16: 9 build module + 3 validate + 2 cli exec + 3 handler_build cmd + 4 studio-core)
- Vitest: 110 → ~118 (+8: 3 ui-error + 5 LastBuildSection + (replaced by Step 4 of T9 — adjust upward if HandlerDetailPage gains 3 more))

Manual smoke per HUMAN_SMOKE Plan 08 section, especially the auto-build path on `examples/handlers/golang-stats-refund-records` that motivated this plan.

Open PR:

```bash
git push -u origin studio-plan-08-build
gh pr create --title "studio Plan 8: handler build + validate (minimum scope)" --body-file - <<'EOF'
## Summary

Closes today's ENOENT pain when running examples/handlers/* on a
fresh checkout: rowforge-core now has a real build executor, CLI
auto-builds before exec run when binary is stale, Studio's handler
detail page gets a Build button + collapsible Last build section.

Explicit deferrals (will land in later plans): smoke test, build
cancel, stderr streaming, build / exec-run interlock, persisted
BuildOutcome.

## Test plan

- [x] cargo build && cargo test
- [x] pnpm tsc -b && pnpm test && pnpm build
- [ ] Manual smoke per apps/rowforge-studio/HUMAN_SMOKE.md Plan 08
      (29 numbered steps covering CLI auto-build, explicit subcommand,
      Studio Build button happy/failure/toolchain-missing paths)
EOF
```

---

## Acceptance criteria

(Cross-reference design doc §13.)

1. `cargo build` clean
2. `cargo test` clean (~333 passes)
3. `pnpm tsc -b` + `pnpm test` (~118) + `pnpm build` clean
4. `rowforge exec run` on a fresh `examples/handlers/golang-stats-refund-records` auto-builds and proceeds without ENOENT
5. `rowforge handler build [name] [--force]` works on all-handlers, single-handler, and force paths
6. Studio Build button shows for handlers with `entry.build`; hides otherwise
7. Build success: LastBuildSection green badge + exit 0 + log
8. Build failure: LastBuildSection red badge + non-zero exit + log; toast surfaces UiError friendly copy
9. Build cache survives within a session, dies on quit
10. validate_manifest emits BuildToolNotInPath / CmdTargetMissing warnings appropriately
11. HUMAN_SMOKE Plan 08 walkthrough merged
12. Spec docs (part-5 + part-8 en + zh-Hant) reflect landed behavior

---

## Order dependency

T1 (build module) → T2 (validate, independent) → T3 (CLI exec run, needs T1) → T4 (CLI handler build cmd, needs T1) → T5 (UiError variants) → T6 (studio-core handler_build, needs T1 + T5) → T7 (Tauri, needs T6) → T8 (TS mirrors, needs T7) → T9 (UI, needs T8) → T10 (docs).

T2 + T4 + T5 are parallelizable with the critical path (T1 → T3 / T6 → T7 → T8 → T9). Single-implementer execution: do in numbered order.
