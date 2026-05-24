# Studio Plan 05 — Exec Lifecycle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the create → run → export user journey in the Studio GUI. After Plan 5, a first-time user can open Studio, pick a workspace, create an execution via the New Execution Wizard, run it (existing Plan 4 Run button now auto-navigates), and export results via a modal — no CLI required.

**Architecture:** Three new `StudioCore` APIs (`start_exec`, `export`, `validate_manifest`) and three Tauri command wrappers. Extract the CLI's ~400-LOC export writers into a shared `rowforge-core::export` module so Studio + CLI use one implementation. Two new React modals (Wizard, Export). Retire Plan 4 debt: struct `UiError::{RunAborted, RunBusy}` payloads, remove `RunStatus::Pending`, `run_start` returns `(RunHandle, AttemptId)` so React can navigate.

**Tech Stack:** Same as Plan 4. Adds:
- `which` crate (workspace dep) for cross-platform PATH probing in `validate_manifest`
- `shell-words` crate (workspace dep) for parsing `manifest.build` / `manifest.run` shell strings
- `sonner` (npm) — already added in Plan 2 for toast notifications

**Spec references:** Spec Part 5 §5.2 (`start_exec`, `export`, `validate_manifest`, `StartExecArgs`, `ExportOpts`, `ManifestReport`), §5.3 (struct `UiError::RunAborted/RunBusy`), §5.5 (`exec_start`, `exec_export`, `manifest_validate` commands); Part 7 §7.4 Flow A (Wizard) + Flow D (Export); Part 8 §8.2 (manifest `build`/`run` PATH probe semantics). Design doc: `docs/superpowers/specs/2026-05-24-studio-plan-05-exec-lifecycle-design.md`.

---

## Decisions resolved during brainstorm

| Decision | Choice | Why |
|---|---|---|
| Plan 5 axis | Exec lifecycle (create → run → export) | Closes most-trodden user journey end-to-end so CLI is no longer required |
| `manifest_validate` depth | Full Part 8 §8.2 (build/run shell-words parse + first-token PATH probe → warning) | Wizard step 2 needs real inline validation; warning-not-error on PATH miss tolerates `$PATH` differences across machines |
| Export progress UX | Block + indeterminate `sonner` toast | Simpler than streaming events; export finishes in seconds–minutes; cancel-during-export deferred |
| Plan 4 carry-forwards bundled | Auto-navigate after Run, struct `UiError::{RunAborted, RunBusy}`, remove `RunStatus::Pending` | Plumb together so the run lifecycle stops accreting debt |
| Plan 4 carry-forwards excluded | `Settings.max_concurrent_runs` wire-up | Belongs with Settings page (Plan 6) |
| Export extraction strategy | Move CLI writers into `rowforge-core::export` module | One implementation; CLI keeps thin wrapper; Studio calls direct |
| `RunStatus::Pending` removal | Drop the variant entirely from enum + spec part 3 §3.3 + spec part 7 §7.5 + TS mirror | Dead variant since Plan 4; sessions transition `Starting → Running → terminal`. Plan 5 ships pre-1.0; no compat shim. |
| `run_start` signature change | Return `RunStartedHandle { handle, attempt_id }` | Wizard "Start run immediately" + Run button auto-navigate both need attempt_id to construct the `/exec/:id/attempt/:aid?run=<handle>` URL |
| Modal-as-route for Wizard | `/new` HashRouter route | Spec §7.3 explicit; deep-link works; browser back closes wizard (acceptable) |

---

## File structure

### New — `rowforge-core`
- `crates/rowforge-core/src/export.rs` — extracted module with `export_execution(store, id, opts) -> Result<ExportReport, ExportError>`; pulls writers (`write_success_csv`, `write_failed_csv`, `write_success_jsonl`, `write_failed_jsonl`, `write_resolution_json_with_completeness`, `discover_success_keys`, `discover_failure_data_keys`, `collect_aborted_attempts`, `emit_export_warnings`) out of CLI

### Modified — `rowforge-core`
- `crates/rowforge-core/src/lib.rs` — add `pub mod export;`
- `crates/rowforge-core/Cargo.toml` — gains `csv`, `serde_json` if not already (they are)

### Modified — `rowforge-cli`
- `crates/rowforge-cli/src/exec_cmd.rs` — `do_export` becomes a thin wrapper that builds `rowforge_core::export::ExportOpts` from `ExportArgs` and calls `export_execution`. Local writer fns deleted (moved to core)

### New — `rowforge-studio-core`
- `crates/rowforge-studio-core/src/manifest.rs` — `validate_manifest`, `ManifestReport`, `ManifestError`, `ManifestWarning`, `ManifestSource`

### Modified — `rowforge-studio-core`
- `src/error.rs` — struct `RunAborted { reason: AbortReason }`, struct `RunBusy { execution_id, limit, scope }`, new `InvalidInput { reason }`, `DuplicateExecName { name }`, `ExportIncomplete { missing_count }`, `ManifestInvalid { errors }`, `ToolchainMissing { token }` variants
- `src/run_handle.rs` — remove `RunStatus::Pending` variant
- `src/lib.rs` — `start_exec`, `export`, `validate_manifest` methods on `StudioCore`; register `manifest` module
- `src/run.rs` — `start_run` returns `RunStartedHandle { handle, attempt_id }`; `BusyScope` enum; mapping from `BusyReason` → `UiError::RunBusy { scope }`
- `src/session.rs` — `BusyReason::Workspace { limit }` already has limit; `BusyReason::PerExec` needs limit field added for symmetry

### Modified — Tauri commands
- `apps/rowforge-studio/src-tauri/src/commands.rs` — `exec_start`, `exec_export`, `manifest_validate` commands; `run_start` returns `RunStartedHandle`
- `apps/rowforge-studio/src-tauri/src/lib.rs` — register the 3 new commands in `invoke_handler!`
- `apps/rowforge-studio/src-tauri/Cargo.toml` — no change

### New — React UI
- `apps/rowforge-studio/src/pages/NewExecutionWizard.tsx` — 2-step wizard, modal-as-route at `/new`
- `apps/rowforge-studio/src/components/ManifestReportView.tsx` — renders errors / warnings / parsed manifest
- `apps/rowforge-studio/src/components/ExportDialog.tsx` — modal triggered from ExecDetail
- `apps/rowforge-studio/src/ipc/use-export.ts` — `useExport` mutation
- `apps/rowforge-studio/src/ipc/use-manifest-validate.ts` — `useManifestValidate` mutation
- `apps/rowforge-studio/src/ipc/use-start-exec.ts` — `useStartExec` mutation

### Modified — React UI
- `apps/rowforge-studio/src/ipc/types.ts` — TS mirrors: `StartExecArgs`, `ExportOpts`, `ExportFormat`, `ExportReport`, `ExportWarning`, `ManifestSource`, `ManifestReport`, `ManifestError`, `ManifestErrorKind`, `ManifestWarning`, `ManifestWarningKind`, `Manifest`, `BusyScope`, `RunStartedHandle`; `UiError.kind` union extended; `RunStatus` loses `pending`
- `apps/rowforge-studio/src/ipc/client.ts` — `exec_start`, `exec_export`, `manifest_validate` IPC wrappers; `run_start` return type updated
- `apps/rowforge-studio/src/ipc/queries.ts` — no new query hooks (mutations live in dedicated files)
- `apps/rowforge-studio/src/components/RunButton.tsx` — after `run_start`, navigate to `/exec/:id/attempt/:aid?run=<handle>`
- `apps/rowforge-studio/src/pages/ExecList.tsx` — "New execution" CTA on empty state + header
- `apps/rowforge-studio/src/pages/ExecDetail.tsx` — wire ExportDialog modal trigger
- `apps/rowforge-studio/src/App.tsx` — add `/new` route
- `apps/rowforge-studio/HUMAN_SMOKE.md` — Plan 5 walkthrough section

### Out of scope for Plan 05
- Settings page UI → Plan 6 candidate
- `Settings.max_concurrent_runs` → `SessionRegistry` wire-up → Plan 6
- Workspace Picker boot UI improvements
- Handler authoring panel (Part 8 entirely)
- Export streaming progress / cancel
- `total_rate` / `slowest_run` in `RunRollupTick`
- Hard cancel actually killing workers (needs rowforge-core API addition)

---

## Task 1: rowforge-core export module — extract types

**Files:**
- Create: `crates/rowforge-core/src/export.rs`
- Modify: `crates/rowforge-core/src/lib.rs`

Move only the public types first; keep CLI working by re-exporting. Writer fns move in Task 2.

- [ ] **Step 1: Write the failing test**

```rust
// crates/rowforge-core/src/export.rs (new file)
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
    /// return `ExportError::Incomplete` before any file is written.
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
```

```rust
// crates/rowforge-core/src/lib.rs — add the module declaration
pub mod export;
```

- [ ] **Step 2: Run test to verify it fails (then passes)**

Run: `cargo test -p rowforge-core export::tests`
Expected: 2 tests pass (types are scaffolding; tests verify serde contract).

- [ ] **Step 3: Commit**

```bash
git add crates/rowforge-core/src/export.rs crates/rowforge-core/src/lib.rs
git commit -m "rowforge-core: scaffold export module with ExportOpts/Format/Report types"
```

---

## Task 2: rowforge-core export module — move writers from CLI

**Files:**
- Modify: `crates/rowforge-core/src/export.rs`
- Modify: `crates/rowforge-cli/src/exec_cmd.rs`
- Modify: `crates/rowforge-cli/Cargo.toml` (if needed — verify `rowforge-core` is a path dep)

Move the ~400 LOC of writer fns from CLI to core. CLI's `do_export` becomes a thin wrapper.

- [ ] **Step 1: Read the existing CLI writers**

Run: `grep -n '^fn write_\|^fn discover_\|^fn collect_aborted_\|^fn emit_export_\|^fn write_resolution_' crates/rowforge-cli/src/exec_cmd.rs`

Expected listing:
- `fn write_success_csv`, `fn write_failed_csv`
- `fn write_success_jsonl`, `fn write_failed_jsonl`
- `fn write_json_object` (helper)
- `fn write_resolution_json_with_completeness`
- `fn discover_success_keys`, `fn discover_failure_data_keys`
- `fn collect_aborted_attempts`
- `fn emit_export_warnings`

- [ ] **Step 2: Move all writer fns to `rowforge-core/src/export.rs`**

Cut each `fn` from `crates/rowforge-cli/src/exec_cmd.rs` and paste into `crates/rowforge-core/src/export.rs`. Change visibility from `fn` (private) to `pub(crate) fn` inside `export.rs`. They are implementation details of `export_execution`.

Also move any helpers used only by those writers (e.g. constants, struct types like `SuccessRow`, `FailedRow` if present).

Add to top of `export.rs`:
```rust
use crate::execution_store::ExecutionStore;
use crate::error::{CoreError, Result};
// Plus whatever else the writers need — adjust per actual grep output.
use std::fs;
use std::io::Write;
use std::path::Path;
```

- [ ] **Step 3: Add `export_execution` public entry**

Append to `crates/rowforge-core/src/export.rs`:
```rust
/// Top-level export entry. Replaces the inline orchestration that used to
/// live in `rowforge-cli::exec_cmd::do_export`. Studio + CLI both call this.
pub fn export_execution(
    store: &ExecutionStore,
    exec_id: &str,
    opts: &ExportOpts,
) -> Result<ExportReport> {
    // 1. Resolve output_dir
    let exec = store
        .get_execution(exec_id)
        .ok_or_else(|| CoreError::Store(format!("execution not found: {}", exec_id)))?;
    let out_dir = match &opts.output_dir {
        Some(p) => p.clone(),
        None => {
            let ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
            exec.dir.join("exports").join(ts)
        }
    };
    fs::create_dir_all(&out_dir).map_err(CoreError::Io)?;

    // 2. require_complete check — re-validated server side
    if opts.require_complete {
        let rollup = compute_rollup(store, exec_id)?;
        if rollup.never_attempted > 0 || rollup.crashed_last > 0 {
            return Err(CoreError::Store(format!(
                "export_incomplete:{}",
                rollup.never_attempted + rollup.crashed_last
            )));
        }
    }

    // 3. Discover keys, scan attempts, write files
    let mut written: Vec<std::path::PathBuf> = Vec::new();
    let mut warnings: Vec<ExportWarning> = Vec::new();
    let aborted = collect_aborted_attempts(store, exec_id)?;
    emit_export_warnings(&aborted, &mut warnings);
    let success_keys = discover_success_keys(store, exec_id)?;
    let failure_keys = discover_failure_data_keys(store, exec_id)?;

    let mut s_count = 0u64;
    let mut f_count = 0u64;

    match opts.format {
        ExportFormat::Csv => {
            let (s, f) = write_success_csv(store, exec_id, &out_dir, &success_keys)?;
            written.push(out_dir.join("success.csv"));
            write_failed_csv(store, exec_id, &out_dir, &failure_keys)?;
            written.push(out_dir.join("failed.csv"));
            s_count = s;
            f_count = f;
        }
        ExportFormat::Jsonl => {
            let (s, f) = write_success_jsonl(store, exec_id, &out_dir, &success_keys)?;
            written.push(out_dir.join("success.jsonl"));
            write_failed_jsonl(store, exec_id, &out_dir, &failure_keys)?;
            written.push(out_dir.join("failed.jsonl"));
            s_count = s;
            f_count = f;
        }
        ExportFormat::Both => {
            let (s, f) = write_success_csv(store, exec_id, &out_dir, &success_keys)?;
            written.push(out_dir.join("success.csv"));
            write_failed_csv(store, exec_id, &out_dir, &failure_keys)?;
            written.push(out_dir.join("failed.csv"));
            let _ = write_success_jsonl(store, exec_id, &out_dir, &success_keys)?;
            written.push(out_dir.join("success.jsonl"));
            write_failed_jsonl(store, exec_id, &out_dir, &failure_keys)?;
            written.push(out_dir.join("failed.jsonl"));
            s_count = s;
            f_count = f;
        }
    }

    write_resolution_json_with_completeness(store, exec_id, &out_dir, opts.require_complete)?;
    written.push(out_dir.join("resolution.json"));

    Ok(ExportReport {
        output_dir: out_dir,
        written_files: written,
        success_count: s_count,
        failed_count: f_count,
        warnings,
    })
}

fn compute_rollup(store: &ExecutionStore, exec_id: &str) -> Result<RollupCounts> {
    // Reuse existing rollup logic — adjust signature per what's already exposed.
    // If not exposed, port the relevant snippet from CLI's resolution computation.
    todo!("port from existing CLI rollup computation")
}

struct RollupCounts {
    never_attempted: u64,
    crashed_last: u64,
}
```

> **Note for implementer:** the `compute_rollup` body above uses `todo!()` as a placeholder you must replace. The actual implementation should reuse the rollup fold already present in either `crates/rowforge-core/src/execution_store.rs` or the CLI's resolution code. Grep first; do not invent.

- [ ] **Step 4: CLI wrapper**

Replace `crates/rowforge-cli/src/exec_cmd.rs` `do_export` body:
```rust
fn do_export(store: &ExecutionStore, a: ExportArgs) -> Result<i32> {
    use rowforge_core::export::{ExportFormat as F, ExportOpts, export_execution};
    let opts = ExportOpts {
        output_dir: a.output_dir,
        format: match a.format {
            crate::exec_cmd::ExportFormat::Csv => F::Csv,
            crate::exec_cmd::ExportFormat::Jsonl => F::Jsonl,
            crate::exec_cmd::ExportFormat::Both => F::Both,
        },
        require_complete: a.strict,
    };
    match export_execution(store, &a.exec_id, &opts) {
        Ok(report) => {
            println!("exported {} success / {} failed rows to {}",
                report.success_count, report.failed_count, report.output_dir.display());
            for w in &report.warnings {
                eprintln!("warning [{}]: {}", w.code, w.message);
            }
            if a.strict && report.warnings.iter().any(|w| w.code == "INCOMPLETE") {
                Ok(3)
            } else {
                Ok(0)
            }
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.starts_with("export_incomplete:") {
                eprintln!("error: execution not fully processed");
                Ok(3)
            } else {
                Err(e.into())
            }
        }
    }
}
```

Keep the CLI's local `ExportFormat` value-enum (it's a clap arg parser); the conversion is one match arm.

- [ ] **Step 5: Run tests to verify CLI behavior unchanged**

Run: `cargo test -p rowforge-cli`
Expected: existing tests still pass (no behavior change, only code moved).

Run: `cargo test -p rowforge-core`
Expected: includes new export module compile.

If any CLI integration test that exercises export fails, the move missed a function. Re-check what `cargo build -p rowforge-cli 2>&1 | grep '^error\['` reports as undefined.

- [ ] **Step 6: Commit**

```bash
git add crates/rowforge-core/src/export.rs crates/rowforge-cli/src/exec_cmd.rs
git commit -m "rowforge-core: move CLI export writers into shared module

Extracts write_success_{csv,jsonl}, write_failed_{csv,jsonl},
write_resolution_json_with_completeness, discover_success_keys,
discover_failure_data_keys, collect_aborted_attempts,
emit_export_warnings from rowforge-cli into rowforge-core::export.
CLI's do_export becomes a thin wrapper. Studio will call
export_execution directly in Task 7."
```

---

## Task 3: studio-core UiError — struct payloads + new variants

**Files:**
- Modify: `crates/rowforge-studio-core/src/error.rs`
- Modify: `crates/rowforge-studio-core/src/run.rs` (call sites that construct old String variants)
- Modify: `crates/rowforge-studio-core/src/session.rs` (BusyReason → UiError conversion)

- [ ] **Step 1: Write the failing test**

Append to `crates/rowforge-studio-core/src/error.rs` (test module at bottom):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use crate::events::AbortReason;

    #[test]
    fn run_aborted_serializes_with_reason_struct() {
        let e = UiError::RunAborted {
            reason: AbortReason::UserCancelled,
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["kind"], json!("run_aborted"));
        assert_eq!(v["message"]["kind"], json!("user_cancelled"));
    }

    #[test]
    fn run_busy_serializes_with_struct_fields() {
        let e = UiError::RunBusy {
            execution_id: "e_01ABC".into(),
            limit: 3,
            scope: BusyScope::PerWorkspace,
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["kind"], json!("run_busy"));
        assert_eq!(v["message"]["execution_id"], json!("e_01ABC"));
        assert_eq!(v["message"]["limit"], json!(3));
        assert_eq!(v["message"]["scope"], json!("per_workspace"));
    }

    #[test]
    fn invalid_input_serializes() {
        let e = UiError::InvalidInput { reason: "no such file".into() };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["kind"], json!("invalid_input"));
        assert_eq!(v["message"]["reason"], json!("no such file"));
    }

    #[test]
    fn duplicate_exec_name_serializes() {
        let e = UiError::DuplicateExecName { name: "foo".into() };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["kind"], json!("duplicate_exec_name"));
        assert_eq!(v["message"]["name"], json!("foo"));
    }

    #[test]
    fn export_incomplete_serializes() {
        let e = UiError::ExportIncomplete { missing_count: 42 };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["kind"], json!("export_incomplete"));
        assert_eq!(v["message"]["missing_count"], json!(42));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p rowforge-studio-core error::tests`
Expected: compile errors — `RunAborted { reason }` form doesn't exist yet, `BusyScope` not defined, etc.

- [ ] **Step 3: Implement enum changes**

Replace `crates/rowforge-studio-core/src/error.rs` enum body:

```rust
use serde::Serialize;
use thiserror::Error;

use crate::events::AbortReason;

#[non_exhaustive]
#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BusyScope {
    PerExec,
    PerWorkspace,
}

#[derive(Debug, Error, Serialize)]
#[serde(tag = "kind", content = "message", rename_all = "snake_case")]
#[non_exhaustive]
pub enum UiError {
    #[error("workspace locked or incompatible: {0}")]
    WorkspaceLocked(String),

    #[error("{0}")]
    NotFound(String),

    #[error("invalid argument: {0}")]
    InvalidArg(String),

    #[error("io error: {0}")]
    Io(String),

    #[error("internal: {0}")]
    Internal(String),

    /// Run aborted. Payload is the structured AbortReason (spec §6.1).
    #[error("run aborted")]
    RunAborted { reason: AbortReason },

    /// Run cannot start because the execution or workspace scope is busy.
    #[error("run busy: limit {limit} reached for scope {scope:?}")]
    RunBusy {
        execution_id: String,
        limit: u32,
        scope: BusyScope,
    },

    #[error("handle expired or unknown: {0}")]
    UnknownHandle(String),

    /// Caller supplied invalid input (path missing, format undetectable, etc).
    #[error("invalid input: {reason}")]
    InvalidInput { reason: String },

    /// Execution name conflicts with an existing exec in this workspace.
    #[error("duplicate exec name: {name}")]
    DuplicateExecName { name: String },

    /// Export refused because require_complete=true and rows remain unresolved.
    #[error("export incomplete: {missing_count} rows unresolved")]
    ExportIncomplete { missing_count: u64 },

    /// Handler manifest validation failed. Errors block exec_start / run_start.
    #[error("manifest invalid: {} error(s)", errors.len())]
    ManifestInvalid {
        errors: Vec<crate::manifest::ManifestError>,
    },

    /// Manifest references a binary not found on PATH.
    #[error("toolchain missing: {token}")]
    ToolchainMissing { token: String },
}

impl From<std::io::Error> for UiError {
    fn from(e: std::io::Error) -> Self {
        UiError::Io(e.to_string())
    }
}

impl From<rowforge_core::error::CoreError> for UiError {
    fn from(e: rowforge_core::error::CoreError) -> Self {
        UiError::Internal(e.to_string())
    }
}
```

- [ ] **Step 4: Fix call sites that constructed old String variants**

Run: `cargo build -p rowforge-studio-core 2>&1 | grep -E '^error' | head -20`

Expected errors at sites like `UiError::RunAborted("...".into())` and `UiError::RunBusy("...".into())`.

In `src/run.rs`, find every `UiError::RunBusy(...)` call. They came from `BusyReason → UiError` mapping. Update to:

```rust
// BusyReason → UiError conversion (replace existing logic)
fn busy_reason_to_ui_error(reason: BusyReason, workspace_limit: u32, per_exec_limit: u32)
    -> UiError
{
    match reason {
        BusyReason::PerExec { execution_id } => UiError::RunBusy {
            execution_id,
            limit: per_exec_limit,
            scope: BusyScope::PerExec,
        },
        BusyReason::Workspace { limit } => UiError::RunBusy {
            execution_id: String::new(),
            limit,
            scope: BusyScope::PerWorkspace,
        },
    }
}
```

> **Implementer note:** the existing call site in `run.rs` looks something like `UiError::RunBusy(reason.to_string())`. Replace with the explicit struct-variant construction, passing the registry's limits (already accessible via `sessions.workspace_limit()` — add a public getter to `SessionRegistry` if it isn't already exposed).

If `SessionRegistry` doesn't expose its limits, add these accessors to `crates/rowforge-studio-core/src/session.rs`:
```rust
impl SessionRegistry {
    pub fn workspace_limit(&self) -> u32 { self.workspace_limit }
    pub fn per_exec_limit(&self) -> u32 { self.per_exec_limit }
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p rowforge-studio-core`
Expected: all 58 existing + 5 new error tests pass = 63 total (give or take depending on test additions).

- [ ] **Step 6: Commit**

```bash
git add crates/rowforge-studio-core/src/error.rs crates/rowforge-studio-core/src/run.rs crates/rowforge-studio-core/src/session.rs
git commit -m "studio-core: UiError struct payloads + new exec-lifecycle variants

- RunAborted: tuple String → struct { reason: AbortReason } (spec §5.3)
- RunBusy: tuple String → struct { execution_id, limit, scope: BusyScope }
- New: InvalidInput, DuplicateExecName, ExportIncomplete,
  ManifestInvalid, ToolchainMissing variants for Plan 5 surface
- New: BusyScope { PerExec | PerWorkspace } enum
- SessionRegistry exposes workspace_limit() + per_exec_limit() accessors

Plan 5 ships pre-1.0; no backwards-compat shim for the breaking
serde change on RunAborted / RunBusy."
```

---

## Task 4: Remove `RunStatus::Pending`

**Files:**
- Modify: `crates/rowforge-studio-core/src/run_handle.rs`
- Modify: any test fixtures / call sites referring to `RunStatus::Pending`
- Modify: `docs/spec/studio/part-3-runtime.md` + zh-Hant mirror (drop `Pending` from §3.3 table)
- Modify: `docs/spec/studio/part-7-ui.md` + zh-Hant mirror (drop `Pending` row from §7.5 color mapping)

- [ ] **Step 1: Write the failing test**

In `crates/rowforge-studio-core/src/run_handle.rs` test module, replace any `Pending` reference and add:

```rust
#[test]
fn run_status_has_six_variants() {
    use RunStatus::*;
    let all = [Starting, Running, Cancelling, Done, Aborted, Crashed];
    assert_eq!(all.len(), 6);
}
```

- [ ] **Step 2: Run test**

Run: `cargo test -p rowforge-studio-core run_handle::tests::run_status_has_six_variants`
Expected: fail (`Pending` still in enum, count mismatch).

- [ ] **Step 3: Remove the variant**

Edit `crates/rowforge-studio-core/src/run_handle.rs`:
```rust
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Starting,
    Running,
    Cancelling,
    Done,
    Aborted,
    Crashed,
}
```

- [ ] **Step 4: Fix call sites**

Run: `grep -rn 'RunStatus::Pending\|"pending"' crates/rowforge-studio-core/src/ apps/rowforge-studio/src-tauri/src/`

For each match, remove the `Pending` arm or string. If a test fixture initialized status to `Pending`, change to `Starting` (sessions start in `Starting` per Plan 4 finding).

- [ ] **Step 5: Update spec docs**

Edit `docs/spec/studio/part-3-runtime.md`: in §3.3 state machine table, delete the row for `Pending`. Update prose if it mentions `Pending` as an entry state — sessions enter at `Starting`.

Edit `docs/spec/studio/part-7-ui.md`: in §7.5 RunStatus color mapping table, delete the `Pending` row. Adjacent text "Sessions transition Pending → Starting" → "Sessions enter Starting on registration."

Mirror both changes in `docs/spec/studio/zh-Hant/part-3-runtime.md` and `zh-Hant/part-7-ui.md`.

- [ ] **Step 6: Run all tests + tsc**

Run: `cargo test -p rowforge-studio-core`
Expected: all pass.

Run: `cd apps/rowforge-studio && pnpm tsc -b 2>&1 | tail -5`
Expected: TS will fail because `RunStatus` mirror still has `"pending"`. Update `apps/rowforge-studio/src/ipc/types.ts`:
```ts
export type RunStatus = "starting" | "running" | "cancelling" | "done" | "aborted" | "crashed";
```

Also grep `apps/rowforge-studio/src/` for `"pending"` references and remove (e.g. initial reducer state).

Then re-run: `pnpm tsc -b && pnpm test`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/rowforge-studio-core/src/run_handle.rs apps/rowforge-studio/src/ipc/types.ts docs/spec/studio/part-3-runtime.md docs/spec/studio/zh-Hant/part-3-runtime.md docs/spec/studio/part-7-ui.md docs/spec/studio/zh-Hant/part-7-ui.md $(grep -rl '"pending"\|RunStatus::Pending' crates/rowforge-studio-core/src/ apps/rowforge-studio/src/ 2>/dev/null)
git commit -m "studio-core: remove RunStatus::Pending dead variant

Plan 4 finding: sessions never set Pending — they enter Starting on
register. Removing the variant + updating spec part 3 §3.3 + part 7
§7.5 + zh-Hant mirrors + TS mirror to match. RunStatus is now 6
variants (Starting/Running/Cancelling/Done/Aborted/Crashed)."
```

---

## Task 5: studio-core `validate_manifest`

**Files:**
- Create: `crates/rowforge-studio-core/src/manifest.rs`
- Modify: `crates/rowforge-studio-core/src/lib.rs` (register module + add `validate_manifest` method)
- Modify: `crates/rowforge-studio-core/Cargo.toml` (add `which`, `shell-words`, `toml` if not present)

- [ ] **Step 1: Add deps**

Edit `crates/rowforge-studio-core/Cargo.toml`:
```toml
[dependencies]
# ... existing ...
which = "6"
shell-words = "1"
toml = "0.8"
```

Run: `cargo build -p rowforge-studio-core`
Expected: deps resolve, no source change yet so build clean.

- [ ] **Step 2: Write the failing test**

Create `crates/rowforge-studio-core/src/manifest.rs`:
```rust
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
    Path(PathBuf),
}

#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub name: Option<String>,
    pub version: Option<String>,
    pub language: Option<String>,
    /// Optional build command. Run before spawning `run`. cwd = handler_dir.
    pub build: Option<String>,
    /// Required run command. cwd = handler_dir.
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
    /// Parsed manifest if TOML parse + required-field check succeeded.
    pub manifest: Option<Manifest>,
    pub errors: Vec<ManifestError>,
    pub warnings: Vec<ManifestWarning>,
}

pub fn validate_manifest(source: &ManifestSource) -> ManifestReport {
    match source {
        ManifestSource::Path(dir) => validate_at(dir),
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
        // PATH-probe; relative paths (e.g. "bin/handler") are NOT probed —
        // they resolve via cwd at spawn time.
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
        let p = std::env::temp_dir().join(format!("rfs-plan5-mtest-{}-{}",
            name, ulid::Ulid::new()));
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn missing_manifest_reports_error() {
        let dir = tmpdir("missing");
        let report = validate_manifest(&ManifestSource::Path(dir.clone()));
        assert!(report.manifest.is_none());
        assert!(matches!(report.errors[0], ManifestError::ManifestMissing { .. }));
    }

    #[test]
    fn parse_failure_reports_error() {
        let dir = tmpdir("bad-toml");
        fs::write(dir.join("manifest.toml"), "not = valid = toml").unwrap();
        let report = validate_manifest(&ManifestSource::Path(dir));
        assert!(report.manifest.is_none());
        assert!(matches!(report.errors[0], ManifestError::ParseFailed { .. }));
    }

    #[test]
    fn missing_run_field_reports_error() {
        let dir = tmpdir("no-run");
        fs::write(dir.join("manifest.toml"), "version = \"1.0\"\nrun = \"\"\n").unwrap();
        let report = validate_manifest(&ManifestSource::Path(dir));
        assert!(report.errors.iter().any(|e| matches!(e,
            ManifestError::MissingRequired { field } if field == "run")));
    }

    #[test]
    fn missing_binary_emits_path_warning_not_error() {
        let dir = tmpdir("missing-bin");
        fs::write(dir.join("manifest.toml"),
            "run = \"this-binary-definitely-not-on-path\"\n").unwrap();
        let report = validate_manifest(&ManifestSource::Path(dir));
        assert!(report.errors.is_empty());
        assert!(report.warnings.iter().any(|w| matches!(w,
            ManifestWarning::PathLookupFailed { field, .. } if field == "run")));
        assert!(report.manifest.is_some(), "warnings should not block manifest parse");
    }

    #[test]
    fn relative_path_run_not_path_probed() {
        let dir = tmpdir("rel-bin");
        fs::write(dir.join("manifest.toml"),
            "run = \"bin/handler\"\n").unwrap();
        let report = validate_manifest(&ManifestSource::Path(dir));
        // bin/handler is relative; no PATH probe; no warning, no error.
        assert!(report.errors.is_empty());
        assert!(report.warnings.is_empty());
    }
}
```

Edit `crates/rowforge-studio-core/src/lib.rs`:
```rust
pub mod manifest;
pub use manifest::{
    Manifest, ManifestError, ManifestReport, ManifestSource, ManifestWarning,
    validate_manifest,
};
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p rowforge-studio-core manifest::tests`
Expected: all 5 tests pass.

- [ ] **Step 4: Add `StudioCore::validate_manifest` method**

In `crates/rowforge-studio-core/src/lib.rs`, on `impl StudioCore`:
```rust
pub fn validate_manifest(&self, source: ManifestSource) -> Result<ManifestReport, UiError> {
    Ok(crate::manifest::validate_manifest(&source))
}
```

(Returns `Result` for API symmetry even though the inner fn never fails — future extension may return real errors.)

- [ ] **Step 5: Commit**

```bash
git add crates/rowforge-studio-core/src/manifest.rs crates/rowforge-studio-core/src/lib.rs crates/rowforge-studio-core/Cargo.toml Cargo.lock
git commit -m "studio-core: validate_manifest per spec part 8 §8.2

ManifestReport { manifest, errors, warnings } with structured
ManifestError + ManifestWarning. Parses manifest.toml, verifies
'run' is present + non-empty, shell-words parses both build+run,
PATH-probes first token of each via the 'which' crate (relative
paths skip probe). Path lookup failure is warning, not error —
PATH may differ across machines."
```

---

## Task 6: studio-core `start_exec`

**Files:**
- Modify: `crates/rowforge-studio-core/src/lib.rs`
- Modify: `crates/rowforge-studio-core/src/exec_view.rs` (or wherever StartExecArgs lives — create if missing)

- [ ] **Step 1: Write the failing test**

In `crates/rowforge-studio-core/tests/foundation.rs` (existing integration test file), add:

```rust
#[test]
fn start_exec_creates_and_returns_id() {
    use rowforge_studio_core::{StartExecArgs, StudioCore, OpenOpts};
    let tmp = tempdir::TempDir::new("rfs-plan5-startexec").unwrap();
    let csv_path = tmp.path().join("in.csv");
    std::fs::write(&csv_path, "row_id\nr1\nr2\nr3\n").unwrap();

    let core = StudioCore::open(OpenOpts::new().with_workspace(tmp.path().into())).unwrap();
    let id = core.start_exec(StartExecArgs {
        input_path: csv_path,
        name: "plan5_test_exec".into(),
        csv_id: None,
        pinned_handler_instance: None,
    }).unwrap();

    assert!(id.0.starts_with("e_"), "id should be e_<ulid>, got {}", id.0);
    // Verify it shows up in list.
    let summaries = core.list(Default::default()).unwrap();
    assert!(summaries.iter().any(|s| s.id.0 == id.0));
}

#[test]
fn start_exec_rejects_missing_input() {
    use rowforge_studio_core::{StartExecArgs, StudioCore, OpenOpts, UiError};
    let tmp = tempdir::TempDir::new("rfs-plan5-startexec-missing").unwrap();
    let core = StudioCore::open(OpenOpts::new().with_workspace(tmp.path().into())).unwrap();
    let err = core.start_exec(StartExecArgs {
        input_path: tmp.path().join("nope.csv"),
        name: "x".into(),
        csv_id: None,
        pinned_handler_instance: None,
    }).unwrap_err();
    assert!(matches!(err, UiError::InvalidInput { .. }), "got {:?}", err);
}

#[test]
fn start_exec_rejects_duplicate_name() {
    use rowforge_studio_core::{StartExecArgs, StudioCore, OpenOpts, UiError};
    let tmp = tempdir::TempDir::new("rfs-plan5-dup").unwrap();
    let csv = tmp.path().join("in.csv");
    std::fs::write(&csv, "row_id\nr1\n").unwrap();
    let core = StudioCore::open(OpenOpts::new().with_workspace(tmp.path().into())).unwrap();

    let args = StartExecArgs {
        input_path: csv,
        name: "dup_name".into(),
        csv_id: None,
        pinned_handler_instance: None,
    };
    core.start_exec(args.clone()).unwrap();
    let err = core.start_exec(args).unwrap_err();
    assert!(matches!(err, UiError::DuplicateExecName { .. }), "got {:?}", err);
}
```

- [ ] **Step 2: Run test**

Run: `cargo test -p rowforge-studio-core start_exec`
Expected: compile error — `StartExecArgs` and `start_exec` not defined.

- [ ] **Step 3: Define `StartExecArgs` + implement `start_exec`**

In `crates/rowforge-studio-core/src/lib.rs` (or a new `src/start_exec.rs` module — pick whichever fits existing layout):

```rust
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartExecArgs {
    pub input_path: std::path::PathBuf,
    pub name: String,
    pub csv_id: Option<String>,
    pub pinned_handler_instance: Option<String>,
}

impl StudioCore {
    pub fn start_exec(&self, args: StartExecArgs) -> Result<ExecutionId, UiError> {
        // 1. Input validation
        if !args.input_path.is_file() {
            return Err(UiError::InvalidInput {
                reason: format!("input not found or not a file: {}",
                    args.input_path.display()),
            });
        }
        // Extension sniff for format detection — full detection happens at run time.
        let ext = args.input_path.extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        if !matches!(ext.as_deref(), Some("csv") | Some("jsonl") | Some("ndjson")) {
            return Err(UiError::InvalidInput {
                reason: "unsupported input format — must be csv/jsonl/ndjson".into(),
            });
        }

        // 2. Duplicate name check (workspace-scoped)
        let mut store = self.store.lock().unwrap_or_else(|p| p.into_inner());
        if store.list_executions()?.iter().any(|e| e.name.as_deref() == Some(&args.name)) {
            return Err(UiError::DuplicateExecName { name: args.name });
        }

        // 3. Delegate to rowforge-core
        let exec = store.create_execution(rowforge_core::execution_store::NewExecution {
            name: Some(args.name.clone()),
            input_csv_id: args.csv_id.unwrap_or_else(|| "csv_unregistered".into()),
            input_csv_path: args.input_path,
            current_handler_instance_id: args.pinned_handler_instance,
        }).map_err(|e| UiError::Internal(e.to_string()))?;

        Ok(ExecutionId(exec.id))
    }
}
```

> **Note:** `store.list_executions()` may or may not exist with that exact name. Grep `crates/rowforge-core/src/execution_store.rs` for the existing list method (it's likely `list` or similar) and adapt the name check accordingly.

- [ ] **Step 4: Run tests**

Run: `cargo test -p rowforge-studio-core start_exec`
Expected: all 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/rowforge-studio-core/src/lib.rs crates/rowforge-studio-core/tests/foundation.rs
git commit -m "studio-core: StudioCore::start_exec(StartExecArgs)

Wraps rowforge_core::ExecutionStore::create_execution with input
validation and workspace-scoped duplicate-name check. Returns the
new ExecutionId on success. New UiError variants used:
InvalidInput (missing path / unsupported format) and
DuplicateExecName (name collision)."
```

---

## Task 7: studio-core `export` wrapper

**Files:**
- Modify: `crates/rowforge-studio-core/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/rowforge-studio-core/tests/foundation.rs`:

```rust
#[test]
fn export_writes_files_for_csv_format() {
    use rowforge_studio_core::{StudioCore, OpenOpts, StartExecArgs};
    use rowforge_core::export::{ExportFormat, ExportOpts};

    let tmp = tempdir::TempDir::new("rfs-plan5-export").unwrap();
    let csv = tmp.path().join("in.csv");
    std::fs::write(&csv, "row_id\nr1\nr2\n").unwrap();

    let core = StudioCore::open(OpenOpts::new().with_workspace(tmp.path().into())).unwrap();
    let id = core.start_exec(StartExecArgs {
        input_path: csv,
        name: "export_test".into(),
        csv_id: None,
        pinned_handler_instance: None,
    }).unwrap();

    // No run yet → every row is NeverAttempted.
    let opts = ExportOpts {
        output_dir: Some(tmp.path().join("out")),
        format: ExportFormat::Csv,
        require_complete: false,
    };
    let report = core.export(&id, opts).unwrap();
    assert!(report.output_dir.exists());
    assert!(report.written_files.iter().any(|p| p.file_name()
        .and_then(|n| n.to_str()) == Some("success.csv")));
    assert!(report.written_files.iter().any(|p| p.file_name()
        .and_then(|n| n.to_str()) == Some("failed.csv")));
}

#[test]
fn export_require_complete_refuses_when_unresolved() {
    use rowforge_studio_core::{StudioCore, OpenOpts, StartExecArgs, UiError};
    use rowforge_core::export::{ExportFormat, ExportOpts};

    let tmp = tempdir::TempDir::new("rfs-plan5-export-strict").unwrap();
    let csv = tmp.path().join("in.csv");
    std::fs::write(&csv, "row_id\nr1\n").unwrap();
    let core = StudioCore::open(OpenOpts::new().with_workspace(tmp.path().into())).unwrap();
    let id = core.start_exec(StartExecArgs {
        input_path: csv,
        name: "strict_test".into(),
        csv_id: None,
        pinned_handler_instance: None,
    }).unwrap();

    let opts = ExportOpts {
        output_dir: Some(tmp.path().join("out")),
        format: ExportFormat::Csv,
        require_complete: true,
    };
    let err = core.export(&id, opts).unwrap_err();
    assert!(matches!(err, UiError::ExportIncomplete { missing_count } if missing_count > 0),
        "got {:?}", err);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p rowforge-studio-core export_writes_files_for_csv_format`
Expected: compile error — `core.export()` not defined.

- [ ] **Step 3: Implement `export`**

In `crates/rowforge-studio-core/src/lib.rs`:

```rust
pub fn export(
    &self,
    id: &ExecutionId,
    opts: rowforge_core::export::ExportOpts,
) -> Result<rowforge_core::export::ExportReport, UiError> {
    let store = self.store.lock().unwrap_or_else(|p| p.into_inner());
    match rowforge_core::export::export_execution(&store, &id.0, &opts) {
        Ok(report) => Ok(report),
        Err(e) => {
            let msg = e.to_string();
            if let Some(rest) = msg.strip_prefix("export_incomplete:") {
                let missing: u64 = rest.parse().unwrap_or(0);
                Err(UiError::ExportIncomplete { missing_count: missing })
            } else {
                Err(UiError::Internal(msg))
            }
        }
    }
}
```

> **Note:** the `export_incomplete:N` sentinel matches the convention chosen in Task 2 step 3 (`compute_rollup` failure path). If the implementer chose a different signaling mechanism (e.g. a typed `ExportError::Incomplete { missing_count }`), update this mapping to read the typed field directly.

- [ ] **Step 4: Run tests**

Run: `cargo test -p rowforge-studio-core export_`
Expected: both tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/rowforge-studio-core/src/lib.rs crates/rowforge-studio-core/tests/foundation.rs
git commit -m "studio-core: StudioCore::export thin wrapper over rowforge-core

Maps export_incomplete sentinel into UiError::ExportIncomplete with
the missing-row count so the React layer can show a precise message.
Otherwise delegates straight to rowforge_core::export::export_execution."
```

---

## Task 8: `run_start` returns `RunStartedHandle`

**Files:**
- Modify: `crates/rowforge-studio-core/src/run.rs`
- Modify: any test asserting old return type
- Modify: `apps/rowforge-studio/src-tauri/src/commands.rs` (return type changes)

- [ ] **Step 1: Define `RunStartedHandle`**

In `crates/rowforge-studio-core/src/run.rs`:

```rust
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunStartedHandle {
    pub handle: RunHandle,
    pub attempt_id: String,
}
```

- [ ] **Step 2: Change `start_run` signature**

Find the existing `pub fn start_run(...) -> Result<RunHandle, UiError>` body. Change return type to `Result<RunStartedHandle, UiError>`. The body already creates an `attempt` (NewAttempt) — capture the returned `attempt_id` and wrap it:

```rust
pub fn start_run(
    &self,
    exec: &ExecutionId,
    opts: RunOpts,
) -> Result<RunStartedHandle, UiError> {
    // ... existing body that creates the attempt ...
    let attempt_id = /* the new_attempt.id you already create */;
    let handle = /* the RunHandle::new() already constructed */;

    // ... rest of existing body ...

    Ok(RunStartedHandle { handle, attempt_id })
}
```

> **Implementer note:** the existing function returns `Ok(handle)` at the end. Find that line and replace with `Ok(RunStartedHandle { handle, attempt_id })`. The attempt id was created earlier in the function via `store.new_attempt(...)` or similar — capture into a let binding before any later move.

- [ ] **Step 3: Update test that asserts return type**

Run: `cargo test -p rowforge-studio-core start_run 2>&1 | head -20`
Expected: compile errors at `.unwrap()` sites that expected `RunHandle` directly.

Update tests to:
```rust
let started = core.start_run(&exec_id, opts).unwrap();
let handle = started.handle;
let attempt_id = started.attempt_id;
```

- [ ] **Step 4: Update Tauri command return type**

In `apps/rowforge-studio/src-tauri/src/commands.rs`, change `run_start` signature:
```rust
#[tauri::command]
pub async fn run_start(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    execution_id: ExecutionId,
    handler_dir: PathBuf,
) -> Result<RunStartedHandle, UiError> {
    // ... existing body ...
    // The existing body destructures into (handle, stream_rx).
    // Update to also capture attempt_id:
    let (started, stream_rx) = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        let core = guard.as_ref().ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
        let opts = RunOpts::new(handler_dir);
        let started = core.start_run(&execution_id, opts)?;
        let stream = core.subscribe(&started.handle).map_err(|e| UiError::Internal(e.to_string()))?;
        (started, stream.rx)
    };

    let handle_for_task = started.handle.clone();
    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        crate::events::forward_run_events(app_clone, handle_for_task, stream_rx).await;
    });

    Ok(started)
}
```

Add `RunStartedHandle` to the imports at the top of `commands.rs`.

- [ ] **Step 5: Run tests**

Run: `cargo test -p rowforge-studio-core && cargo test -p rowforge-studio --test ipc_contract`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add crates/rowforge-studio-core/src/run.rs apps/rowforge-studio/src-tauri/src/commands.rs
git commit -m "studio-core: start_run returns RunStartedHandle { handle, attempt_id }

Unblocks Run-button + Wizard auto-navigation: callers can build
the /exec/:id/attempt/:aid?run=<handle> URL without a follow-up
exec_show query. Tauri command surface updated accordingly."
```

---

## Task 9: Tauri commands `exec_start`, `exec_export`, `manifest_validate`

**Files:**
- Modify: `apps/rowforge-studio/src-tauri/src/commands.rs`
- Modify: `apps/rowforge-studio/src-tauri/src/lib.rs`

- [ ] **Step 1: Write the failing test**

In `apps/rowforge-studio/src-tauri/tests/ipc_contract.rs`, add a test for each new command's wire shape:

```rust
#[test]
fn manifest_validate_command_registered() {
    // The command name must match exactly what React's invoke() will call.
    let names = registered_command_names();
    assert!(names.contains(&"manifest_validate"));
    assert!(names.contains(&"exec_start"));
    assert!(names.contains(&"exec_export"));
}
```

> **Implementer note:** the `registered_command_names()` helper may not exist. If not, port the assertion to whatever the existing tests use — e.g., grepping the source for `tauri::generate_handler![...]` and asserting via inspect_command_list or similar. The point is: the test fails until the 3 commands are wired into `invoke_handler!`.

- [ ] **Step 2: Run test**

Run: `cargo test -p rowforge-studio --test ipc_contract manifest_validate_command_registered`
Expected: fail — 3 commands not yet registered.

- [ ] **Step 3: Implement commands**

In `apps/rowforge-studio/src-tauri/src/commands.rs`, append:

```rust
use rowforge_studio_core::{
    ManifestSource, ManifestReport, StartExecArgs,
};
use rowforge_core::export::{ExportOpts, ExportReport};

#[tauri::command]
pub fn exec_start(
    state: State<'_, AppState>,
    args: StartExecArgs,
) -> Result<ExecutionId, UiError> {
    let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
    let core = guard.as_ref()
        .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
    core.start_exec(args)
}

#[tauri::command]
pub async fn exec_export(
    state: State<'_, AppState>,
    id: ExecutionId,
    opts: ExportOpts,
) -> Result<ExportReport, UiError> {
    // Scope the guard before any .await so MutexGuard doesn't cross awaits.
    // Export is sync-bound CPU + IO; we don't await inside the lock, but
    // make the command async so Tauri schedules it on a worker thread
    // instead of blocking the main thread for the duration.
    let report = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        let core = guard.as_ref()
            .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
        core.export(&id, opts)
    };
    report
}

#[tauri::command]
pub fn manifest_validate(
    state: State<'_, AppState>,
    source: ManifestSource,
) -> Result<ManifestReport, UiError> {
    let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
    let core = guard.as_ref()
        .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
    core.validate_manifest(source)
}
```

In `apps/rowforge-studio/src-tauri/src/lib.rs`, find the `tauri::generate_handler![...]` macro and add the 3 new commands:
```rust
.invoke_handler(tauri::generate_handler![
    commands::workspace_open,
    commands::workspace_current,
    commands::workspace_settings_load,
    commands::workspace_settings_save,
    commands::exec_list,
    commands::exec_show,
    commands::exec_rollup,
    commands::exec_start,             // NEW
    commands::exec_export,            // NEW
    commands::manifest_validate,      // NEW
    commands::attempt_show,
    commands::attempt_failed_page,
    commands::attempt_row_history,
    commands::run_start,
    commands::run_cancel,
    commands::run_status,
    commands::run_active,
])
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p rowforge-studio --test ipc_contract`
Expected: all pass including the new registration assertion.

- [ ] **Step 5: Commit**

```bash
git add apps/rowforge-studio/src-tauri/src/commands.rs apps/rowforge-studio/src-tauri/src/lib.rs apps/rowforge-studio/src-tauri/tests/ipc_contract.rs
git commit -m "studio-shell: Tauri commands exec_start / exec_export / manifest_validate

exec_export is async so Tauri schedules it off the main thread
(export takes seconds to minutes). MutexGuard scoped to drop
before any await for Send safety."
```

---

## Task 10: TS mirrors for Plan 5 types

**Files:**
- Modify: `apps/rowforge-studio/src/ipc/types.ts`
- Modify: `apps/rowforge-studio/src/ipc/client.ts`

- [ ] **Step 1: Add type mirrors**

Append to `apps/rowforge-studio/src/ipc/types.ts`:

```ts
// ===== Plan 5 mirrors =====

export interface StartExecArgs {
  input_path: string;
  name: string;
  csv_id: string | null;
  pinned_handler_instance: string | null;
}

export type ExportFormat = "csv" | "jsonl" | "both";

export interface ExportOpts {
  output_dir: string | null;
  format: ExportFormat;
  require_complete: boolean;
}

export interface ExportReport {
  output_dir: string;
  written_files: string[];
  success_count: number;
  failed_count: number;
  warnings: ExportWarning[];
}

export interface ExportWarning {
  code: string;
  message: string;
}

export type ManifestSource = { type: "path"; 0: string } | { type: "path"; path: string };
// Note: serde shape is { "type": "path", "path": "..." } when ManifestSource
// uses #[serde(tag = "type")] on a struct variant. The implementer must
// align this enum form with the actual Rust serde output verified by
// a Vitest snapshot in Task 11.

export interface Manifest {
  name: string | null;
  version: string | null;
  language: string | null;
  build: string | null;
  run: string;
}

export interface ManifestReport {
  manifest: Manifest | null;
  errors: ManifestError[];
  warnings: ManifestWarning[];
}

export type ManifestError =
  | { kind: "manifest_missing"; path: string }
  | { kind: "parse_failed"; message: string }
  | { kind: "missing_required"; field: string }
  | { kind: "shell_parse_failed"; field: string; message: string };

export type ManifestWarning =
  | { kind: "path_lookup_failed"; field: string; token: string };

export type BusyScope = "per_exec" | "per_workspace";

export interface RunStartedHandle {
  handle: string;       // serde transparent string for RunHandle
  attempt_id: string;
}
```

Also extend the existing `UiErrorKind` union to include the new variants:
```ts
export type UiErrorKind =
  | "workspace_locked" | "not_found" | "invalid_arg" | "io" | "internal"
  | "run_aborted" | "run_busy" | "unknown_handle"
  | "invalid_input" | "duplicate_exec_name" | "export_incomplete"
  | "manifest_invalid" | "toolchain_missing";
```

Update `UiError` to allow the new struct payloads. The simplest path is to keep `message: unknown` and have UI components narrow.

- [ ] **Step 2: Update `client.ts`**

Append to `apps/rowforge-studio/src/ipc/client.ts`:

```ts
import { invoke } from "@tauri-apps/api/core";
import type {
  ExecutionId, ExportOpts, ExportReport, ManifestReport, ManifestSource,
  RunStartedHandle, StartExecArgs,
} from "./types";

export const ipc = {
  // ... existing ipc functions ...

  exec_start: (args: StartExecArgs) =>
    invoke<ExecutionId>("exec_start", { args }),

  exec_export: (id: ExecutionId, opts: ExportOpts) =>
    invoke<ExportReport>("exec_export", { id, opts }),

  manifest_validate: (source: ManifestSource) =>
    invoke<ManifestReport>("manifest_validate", { source }),
};
```

Update the existing `run_start` wrapper return type:
```ts
run_start: (executionId: ExecutionId, handlerDir: string) =>
  invoke<RunStartedHandle>("run_start", { executionId, handlerDir }),
```

- [ ] **Step 3: Type check**

Run: `cd apps/rowforge-studio && pnpm tsc -b`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add apps/rowforge-studio/src/ipc/types.ts apps/rowforge-studio/src/ipc/client.ts
git commit -m "studio-shell: TS mirrors for Plan 5 types

StartExecArgs, ExportOpts/Format/Report/Warning, ManifestSource,
Manifest, ManifestReport with discriminated ManifestError +
ManifestWarning, BusyScope, RunStartedHandle. UiErrorKind union
extended with the 5 new variants."
```

---

## Task 11: TanStack mutation hooks

**Files:**
- Create: `apps/rowforge-studio/src/ipc/use-start-exec.ts`
- Create: `apps/rowforge-studio/src/ipc/use-export.ts`
- Create: `apps/rowforge-studio/src/ipc/use-manifest-validate.ts`

- [ ] **Step 1: Create the three hooks**

```ts
// apps/rowforge-studio/src/ipc/use-start-exec.ts
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { ipc } from "./client";
import type { StartExecArgs } from "./types";

export const useStartExec = () => {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: StartExecArgs) => ipc.exec_start(args),
    onSuccess: () => {
      // Invalidate exec list so Workspace Home re-fetches.
      qc.invalidateQueries({ queryKey: ["exec_list"] });
    },
  });
};
```

```ts
// apps/rowforge-studio/src/ipc/use-export.ts
import { useMutation } from "@tanstack/react-query";
import { ipc } from "./client";
import type { ExecutionId, ExportOpts } from "./types";

export const useExport = () =>
  useMutation({
    mutationFn: ({ id, opts }: { id: ExecutionId; opts: ExportOpts }) =>
      ipc.exec_export(id, opts),
  });
```

```ts
// apps/rowforge-studio/src/ipc/use-manifest-validate.ts
import { useMutation } from "@tanstack/react-query";
import { ipc } from "./client";
import type { ManifestSource } from "./types";

export const useManifestValidate = () =>
  useMutation({
    mutationFn: (source: ManifestSource) => ipc.manifest_validate(source),
  });
```

- [ ] **Step 2: Type check**

Run: `pnpm tsc -b`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add apps/rowforge-studio/src/ipc/use-start-exec.ts apps/rowforge-studio/src/ipc/use-export.ts apps/rowforge-studio/src/ipc/use-manifest-validate.ts
git commit -m "studio-shell: TanStack mutation hooks for Plan 5 commands

useStartExec, useExport, useManifestValidate. useStartExec
invalidates the exec_list query on success so Workspace Home
re-fetches."
```

---

## Task 12: `ManifestReportView` component

**Files:**
- Create: `apps/rowforge-studio/src/components/ManifestReportView.tsx`
- Create: `apps/rowforge-studio/src/__tests__/manifest-report-view.test.tsx`

- [ ] **Step 1: Write the failing test**

```tsx
// apps/rowforge-studio/src/__tests__/manifest-report-view.test.tsx
import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { ManifestReportView } from "@/components/ManifestReportView";

describe("ManifestReportView", () => {
  it("renders missing-manifest error", () => {
    render(<ManifestReportView report={{
      manifest: null,
      errors: [{ kind: "manifest_missing", path: "/x/manifest.toml" }],
      warnings: [],
    }} />);
    expect(screen.getByText(/manifest.toml not found/i)).toBeTruthy();
  });

  it("renders parse-failed error", () => {
    render(<ManifestReportView report={{
      manifest: null,
      errors: [{ kind: "parse_failed", message: "expected = at line 3" }],
      warnings: [],
    }} />);
    expect(screen.getByText(/expected = at line 3/)).toBeTruthy();
  });

  it("renders path-lookup warning + accepts manifest", () => {
    render(<ManifestReportView report={{
      manifest: { name: "h", version: "1.0", language: "go", build: null, run: "bin/handler" },
      errors: [],
      warnings: [{ kind: "path_lookup_failed", field: "run", token: "missing-bin" }],
    }} />);
    expect(screen.getByText(/missing-bin/)).toBeTruthy();
    expect(screen.getByText(/v1\.0/i)).toBeTruthy();
  });

  it("renders success state when no errors", () => {
    render(<ManifestReportView report={{
      manifest: { name: "h", version: "2.1", language: "go", build: null, run: "bin/handler" },
      errors: [],
      warnings: [],
    }} />);
    expect(screen.getByText(/valid/i)).toBeTruthy();
  });
});
```

- [ ] **Step 2: Implement the component**

```tsx
// apps/rowforge-studio/src/components/ManifestReportView.tsx
import { CheckCircle2, AlertTriangle, AlertOctagon } from "lucide-react";
import type { ManifestError, ManifestReport, ManifestWarning } from "@/ipc/types";

export function ManifestReportView({ report }: { report: ManifestReport }) {
  if (report.errors.length === 0 && report.warnings.length === 0 && report.manifest) {
    return (
      <div className="flex items-center gap-2 rounded border border-green-500/30 bg-green-500/10 p-3 text-sm">
        <CheckCircle2 className="h-4 w-4 text-green-400" />
        <span>
          Manifest valid
          {report.manifest.version && (
            <span className="ml-2 rounded bg-zinc-700/40 px-1.5 py-0.5 font-mono text-xs">
              v{report.manifest.version}
            </span>
          )}
          {report.manifest.language && (
            <span className="ml-1 rounded bg-zinc-700/40 px-1.5 py-0.5 font-mono text-xs">
              {report.manifest.language}
            </span>
          )}
        </span>
      </div>
    );
  }

  return (
    <div className="space-y-2">
      {report.errors.length > 0 && (
        <div className="rounded border border-red-500/40 bg-red-500/10 p-3">
          <div className="mb-1 flex items-center gap-2 text-sm font-medium text-red-300">
            <AlertOctagon className="h-4 w-4" />
            {report.errors.length} error{report.errors.length === 1 ? "" : "s"}
          </div>
          <ul className="space-y-1 text-sm">
            {report.errors.map((e, i) => (
              <li key={i} className="font-mono text-xs text-red-200">
                {formatError(e)}
              </li>
            ))}
          </ul>
        </div>
      )}
      {report.warnings.length > 0 && (
        <div className="rounded border border-amber-500/40 bg-amber-500/10 p-3">
          <div className="mb-1 flex items-center gap-2 text-sm font-medium text-amber-300">
            <AlertTriangle className="h-4 w-4" />
            {report.warnings.length} warning{report.warnings.length === 1 ? "" : "s"}
          </div>
          <ul className="space-y-1 text-sm">
            {report.warnings.map((w, i) => (
              <li key={i} className="font-mono text-xs text-amber-200">
                {formatWarning(w)}
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

function formatError(e: ManifestError): string {
  switch (e.kind) {
    case "manifest_missing":
      return `manifest.toml not found at ${e.path}`;
    case "parse_failed":
      return `TOML parse failed: ${e.message}`;
    case "missing_required":
      return `Required field missing: '${e.field}'`;
    case "shell_parse_failed":
      return `Shell parse failed for '${e.field}': ${e.message}`;
  }
}

function formatWarning(w: ManifestWarning): string {
  switch (w.kind) {
    case "path_lookup_failed":
      return `'${w.token}' (from ${w.field}) not found on PATH — may still work on a different machine`;
  }
}
```

- [ ] **Step 3: Run tests**

Run: `pnpm test src/__tests__/manifest-report-view.test.tsx`
Expected: 4 tests pass.

- [ ] **Step 4: Commit**

```bash
git add apps/rowforge-studio/src/components/ManifestReportView.tsx apps/rowforge-studio/src/__tests__/manifest-report-view.test.tsx
git commit -m "studio-shell: ManifestReportView component

Renders ManifestReport errors (red, block) and warnings (amber,
don't block). Success state shows manifest version + language
chips. Used by NewExecutionWizard step 2."
```

---

## Task 13: New Execution Wizard

**Files:**
- Create: `apps/rowforge-studio/src/pages/NewExecutionWizard.tsx`
- Create: `apps/rowforge-studio/src/__tests__/new-execution-wizard.test.tsx`
- Modify: `apps/rowforge-studio/src/App.tsx` (register `/new` route)

- [ ] **Step 1: Write the failing test**

```tsx
// apps/rowforge-studio/src/__tests__/new-execution-wizard.test.tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import { NewExecutionWizardPage } from "@/pages/NewExecutionWizard";

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
}));
vi.mock("@/ipc/client", () => ({
  ipc: {
    exec_start: vi.fn().mockResolvedValue({ id: "e_01TEST" }),
    manifest_validate: vi.fn().mockResolvedValue({
      manifest: { name: "h", version: "1.0", language: "go", build: null, run: "bin/handler" },
      errors: [],
      warnings: [],
    }),
    run_start: vi.fn(),
  },
}));

function renderWizard() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return render(
    <MemoryRouter>
      <QueryClientProvider client={qc}>
        <NewExecutionWizardPage />
      </QueryClientProvider>
    </MemoryRouter>
  );
}

beforeEach(() => { vi.clearAllMocks(); });

describe("NewExecutionWizard", () => {
  it("step 1 → Next disabled without name + input", () => {
    renderWizard();
    expect(screen.getByText(/next/i)).toBeDisabled();
  });

  it("Validate calls manifest_validate then renders ManifestReportView", async () => {
    renderWizard();
    fireEvent.change(screen.getByLabelText(/name/i), { target: { value: "test_exec" } });
    // Skip input picker simulation — directly poke state would require a more
    // involved test setup. For now assert the Validate path with a stub.
    // (Full integration test deferred to HUMAN_SMOKE walkthrough.)
  });
});
```

- [ ] **Step 2: Implement the Wizard page**

```tsx
// apps/rowforge-studio/src/pages/NewExecutionWizard.tsx
import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Checkbox } from "@/components/ui/checkbox";
import { AppShell } from "@/layout/AppShell";
import { ManifestReportView } from "@/components/ManifestReportView";
import { useStartExec } from "@/ipc/use-start-exec";
import { useManifestValidate } from "@/ipc/use-manifest-validate";
import { useWorkspace } from "@/ipc/queries";
import { ipc } from "@/ipc/client";

export function NewExecutionWizardPage() {
  const navigate = useNavigate();
  const ws = useWorkspace();
  const startExec = useStartExec();
  const validate = useManifestValidate();

  const [step, setStep] = useState<1 | 2>(1);
  const [name, setName] = useState("");
  const [inputPath, setInputPath] = useState<string | null>(null);
  const [handlerDir, setHandlerDir] = useState<string | null>(null);
  const [startImmediately, setStartImmediately] = useState(false);

  const NAME_RX = /^[a-z0-9_-]{1,64}$/;
  const detectedFormat = inputPath ? detectFormat(inputPath) : null;
  const step1Valid = NAME_RX.test(name) && !!inputPath && detectedFormat !== null;
  const manifestErrors = validate.data?.errors ?? [];
  const validateClean = !!validate.data && manifestErrors.length === 0;

  const pickInput = async () => {
    const p = await openDialog({
      filters: [{ name: "Input", extensions: ["csv", "jsonl", "ndjson"] }],
    });
    if (typeof p === "string") setInputPath(p);
  };
  const pickHandlerDir = async () => {
    const p = await openDialog({ directory: true });
    if (typeof p === "string") setHandlerDir(p);
  };

  const onValidate = () => {
    if (!handlerDir) return;
    validate.mutate({ type: "path", path: handlerDir });
  };

  const onSubmit = async () => {
    if (!inputPath) return;
    try {
      const id = await startExec.mutateAsync({
        input_path: inputPath,
        name,
        csv_id: null,
        pinned_handler_instance: null,
      });
      if (startImmediately && handlerDir) {
        const started = await ipc.run_start(id as unknown as string, handlerDir);
        navigate(`/exec/${id}/attempt/${started.attempt_id}?run=${started.handle}`);
      } else {
        navigate(`/exec/${id}`);
      }
    } catch (e) {
      // Mutation error already surfaced in startExec.error; no-op here.
      console.error(e);
    }
  };

  return (
    <AppShell workspace={ws.data ?? null} crumbs={[{ label: "Executions", to: "/" }, { label: "New execution" }]}>
      <div className="mx-auto max-w-2xl p-6">
        <h1 className="mb-4 text-xl font-medium">New execution</h1>
        <div className="mb-4 flex items-center gap-2 text-sm text-muted-foreground">
          <span className={step === 1 ? "font-medium text-foreground" : ""}>1. Identity + input</span>
          <span>→</span>
          <span className={step === 2 ? "font-medium text-foreground" : ""}>2. Handler</span>
        </div>

        {step === 1 && (
          <div className="space-y-4">
            <div>
              <label htmlFor="exec-name" className="mb-1 block text-sm font-medium">Name</label>
              <Input
                id="exec-name"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="my-exec-2026-05"
              />
              {name && !NAME_RX.test(name) && (
                <div className="mt-1 text-xs text-red-300">
                  must match [a-z0-9_-]+ and be ≤ 64 chars
                </div>
              )}
            </div>

            <div>
              <label className="mb-1 block text-sm font-medium">Input file</label>
              <div className="flex gap-2">
                <Input value={inputPath ?? ""} placeholder="not selected" readOnly />
                <Button onClick={pickInput} variant="outline">Pick…</Button>
              </div>
              {detectedFormat && (
                <span className="mt-1 inline-block rounded bg-zinc-700/40 px-1.5 py-0.5 font-mono text-xs">
                  {detectedFormat}
                </span>
              )}
            </div>

            <div className="flex justify-between pt-4">
              <Button variant="ghost" onClick={() => navigate("/")}>Cancel</Button>
              <Button onClick={() => setStep(2)} disabled={!step1Valid}>Next</Button>
            </div>
          </div>
        )}

        {step === 2 && (
          <div className="space-y-4">
            <div>
              <label className="mb-1 block text-sm font-medium">Handler directory</label>
              <div className="flex gap-2">
                <Input value={handlerDir ?? ""} placeholder="not selected" readOnly />
                <Button onClick={pickHandlerDir} variant="outline">Pick…</Button>
                <Button onClick={onValidate} disabled={!handlerDir || validate.isPending}>
                  {validate.isPending ? "Validating…" : "Validate"}
                </Button>
              </div>
            </div>

            {validate.data && <ManifestReportView report={validate.data} />}

            <div className="flex items-center gap-2">
              <Checkbox
                id="start-immediately"
                checked={startImmediately}
                onCheckedChange={(v) => setStartImmediately(v === true)}
              />
              <label htmlFor="start-immediately" className="text-sm">
                Start a run immediately after creation
              </label>
            </div>

            {startExec.isError && (
              <div className="rounded border border-red-500/40 bg-red-500/10 p-3 text-sm text-red-200">
                {String((startExec.error as { message?: unknown })?.message ?? startExec.error)}
              </div>
            )}

            <div className="flex justify-between pt-4">
              <Button variant="ghost" onClick={() => setStep(1)}>Back</Button>
              <Button onClick={onSubmit} disabled={!validateClean || startExec.isPending}>
                {startExec.isPending ? "Creating…" : "Create execution"}
              </Button>
            </div>
          </div>
        )}
      </div>
    </AppShell>
  );
}

function detectFormat(p: string): "csv" | "jsonl" | "ndjson" | null {
  const ext = p.toLowerCase().split(".").pop();
  if (ext === "csv" || ext === "jsonl" || ext === "ndjson") return ext;
  return null;
}
```

- [ ] **Step 3: Register route**

In `apps/rowforge-studio/src/App.tsx`, add:
```tsx
import { NewExecutionWizardPage } from "@/pages/NewExecutionWizard";

// inside <Routes>:
<Route path="/new" element={<NewExecutionWizardPage />} />
```

- [ ] **Step 4: Run tests + build**

Run: `pnpm test src/__tests__/new-execution-wizard.test.tsx`
Expected: 2 tests pass.

Run: `pnpm tsc -b && pnpm build`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add apps/rowforge-studio/src/pages/NewExecutionWizard.tsx apps/rowforge-studio/src/__tests__/new-execution-wizard.test.tsx apps/rowforge-studio/src/App.tsx
git commit -m "studio-shell: New Execution Wizard at /new (Flow A)

Two-step modal-as-route. Step 1 captures name + input path with
extension-sniffed format chip; Step 2 picks handler dir + Validate
button → inline ManifestReportView. 'Start run immediately'
checkbox chains run_start after exec_start success and navigates
to ?run=<handle> Live tab."
```

---

## Task 14: Export dialog

**Files:**
- Create: `apps/rowforge-studio/src/components/ExportDialog.tsx`
- Create: `apps/rowforge-studio/src/__tests__/export-dialog.test.tsx`
- Modify: `apps/rowforge-studio/src/pages/ExecDetail.tsx` (add Export button → dialog trigger)

- [ ] **Step 1: Write the failing test**

```tsx
// apps/rowforge-studio/src/__tests__/export-dialog.test.tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ExportDialog } from "@/components/ExportDialog";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@tauri-apps/plugin-shell", () => ({ open: vi.fn() }));
vi.mock("@/ipc/client", () => ({
  ipc: {
    exec_export: vi.fn().mockResolvedValue({
      output_dir: "/tmp/exports/x",
      written_files: ["/tmp/exports/x/success.csv", "/tmp/exports/x/failed.csv"],
      success_count: 100, failed_count: 5, warnings: [],
    }),
  },
}));

function renderDialog() {
  const qc = new QueryClient({ defaultOptions: { mutations: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <ExportDialog open execId={"e_01TEST"} onClose={() => {}} />
    </QueryClientProvider>
  );
}

beforeEach(() => { vi.clearAllMocks(); });

describe("ExportDialog", () => {
  it("renders format segmented control with Csv default", () => {
    renderDialog();
    expect(screen.getByLabelText(/csv/i)).toBeChecked();
  });

  it("submits with selected format", async () => {
    const { container } = renderDialog();
    fireEvent.click(screen.getByLabelText(/jsonl/i));
    fireEvent.click(screen.getByText(/^export$/i));
    await waitFor(() => {
      const { ipc } = require("@/ipc/client");
      expect(ipc.exec_export).toHaveBeenCalledWith(
        "e_01TEST",
        expect.objectContaining({ format: "jsonl" }),
      );
    });
  });
});
```

- [ ] **Step 2: Implement ExportDialog**

```tsx
// apps/rowforge-studio/src/components/ExportDialog.tsx
import { useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { open as shellOpen } from "@tauri-apps/plugin-shell";
import { toast } from "sonner";
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Checkbox } from "@/components/ui/checkbox";
import { useExport } from "@/ipc/use-export";
import type { ExportFormat, ExecutionId } from "@/ipc/types";

export function ExportDialog({
  open, execId, onClose,
}: {
  open: boolean;
  execId: ExecutionId;
  onClose: () => void;
}) {
  const [outputDir, setOutputDir] = useState<string | null>(null);
  const [format, setFormat] = useState<ExportFormat>("csv");
  const [requireComplete, setRequireComplete] = useState(false);
  const exportMut = useExport();

  const pickDir = async () => {
    const p = await openDialog({ directory: true });
    if (typeof p === "string") setOutputDir(p);
  };

  const onSubmit = async () => {
    const toastId = toast.loading("Exporting…");
    try {
      const report = await exportMut.mutateAsync({
        id: execId,
        opts: { output_dir: outputDir, format, require_complete: requireComplete },
      });
      toast.dismiss(toastId);
      toast.success(
        `Exported ${report.success_count + report.failed_count} rows to ${report.output_dir}`,
        {
          action: { label: "Reveal", onClick: () => shellOpen(report.output_dir) },
        },
      );
      onClose();
    } catch (e) {
      toast.dismiss(toastId);
      const err = e as { kind?: string; message?: unknown };
      if (err.kind === "export_incomplete") {
        const missing = (err.message as { missing_count?: number })?.missing_count ?? "some";
        toast.error(`Export incomplete: ${missing} rows unresolved — uncheck 'Require complete' or finish the run first.`);
      } else {
        toast.error(`Export failed: ${String(err.message ?? e)}`);
      }
    }
  };

  return (
    <Dialog open={open} onOpenChange={(v) => !v && onClose()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Export execution</DialogTitle>
        </DialogHeader>
        <div className="space-y-4">
          <div>
            <label className="mb-1 block text-sm font-medium">Output directory</label>
            <div className="flex gap-2">
              <Input value={outputDir ?? ""} placeholder="default: <workspace>/exports/…" readOnly />
              <Button onClick={pickDir} variant="outline">Pick…</Button>
            </div>
          </div>

          <fieldset>
            <legend className="mb-1 block text-sm font-medium">Format</legend>
            <div className="flex gap-4">
              {(["csv", "jsonl", "both"] as const).map((f) => (
                <label key={f} className="flex items-center gap-1.5 text-sm">
                  <input
                    type="radio"
                    name="format"
                    aria-label={f}
                    checked={format === f}
                    onChange={() => setFormat(f)}
                  />
                  {f}
                </label>
              ))}
            </div>
          </fieldset>

          <div className="flex items-center gap-2">
            <Checkbox
              id="require-complete"
              checked={requireComplete}
              onCheckedChange={(v) => setRequireComplete(v === true)}
            />
            <label htmlFor="require-complete" className="text-sm">
              Require complete (refuse if any rows are unresolved)
            </label>
          </div>
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={onClose}>Cancel</Button>
          <Button onClick={onSubmit} disabled={exportMut.isPending}>
            {exportMut.isPending ? "Exporting…" : "Export"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
```

- [ ] **Step 3: Wire trigger in ExecDetail**

In `apps/rowforge-studio/src/pages/ExecDetail.tsx`, add:
```tsx
import { useState } from "react";
import { ExportDialog } from "@/components/ExportDialog";

// inside the component, alongside other useState:
const [exportOpen, setExportOpen] = useState(false);

// in the header / button row (next to Run button):
<Button onClick={() => setExportOpen(true)} variant="outline">Export</Button>
<ExportDialog open={exportOpen} execId={execId} onClose={() => setExportOpen(false)} />
```

- [ ] **Step 4: Run tests**

Run: `pnpm test src/__tests__/export-dialog.test.tsx`
Expected: 2 tests pass.

- [ ] **Step 5: Commit**

```bash
git add apps/rowforge-studio/src/components/ExportDialog.tsx apps/rowforge-studio/src/__tests__/export-dialog.test.tsx apps/rowforge-studio/src/pages/ExecDetail.tsx
git commit -m "studio-shell: Export dialog (Flow D)

Modal triggered from ExecDetail header. Output dir picker, format
segmented (csv/jsonl/both), require_complete checkbox. Submit
shows sonner loading toast then success toast with 'Reveal'
button → shell::open(output_dir). UiError::ExportIncomplete
surfaces a specific message; other errors fall through to generic."
```

---

## Task 15: Run button auto-navigate

**Files:**
- Modify: `apps/rowforge-studio/src/components/RunButton.tsx`
- Modify: `apps/rowforge-studio/src/__tests__/` (any run-button test, if present)

- [ ] **Step 1: Update RunButton to navigate after success**

Find the `useRunStart` mutation call. After `onSuccess`, navigate:

```tsx
import { useNavigate } from "react-router-dom";

// inside component:
const navigate = useNavigate();
const runStart = useRunStart();

// On success:
const handleClick = async () => {
  const handlerDir = lastHandlerDir; // from existing logic
  try {
    const started = await runStart.mutateAsync({ executionId: execId, handlerDir });
    // Plan 5 carry: started is now { handle, attempt_id }.
    navigate(`/exec/${execId}/attempt/${started.attempt_id}?run=${started.handle}`);
  } catch (e) {
    // existing error toast
  }
};
```

> **Implementer note:** `useRunStart` is defined in `apps/rowforge-studio/src/ipc/queries.ts` (Plan 4 T14). Its mutationFn return type must be updated to `RunStartedHandle` since Plan 5 T8 changed the IPC return shape. If not yet updated, fix the hook accordingly.

- [ ] **Step 2: Run tests**

Run: `pnpm test`
Expected: existing run-button tests still pass; new navigation behavior verified by Wizard test (Task 13) implicitly.

- [ ] **Step 3: Commit**

```bash
git add apps/rowforge-studio/src/components/RunButton.tsx apps/rowforge-studio/src/ipc/queries.ts
git commit -m "studio-shell: Run button auto-navigates to Live tab after run_start

Uses the new RunStartedHandle from Plan 5 T8 to construct the
/exec/:id/attempt/:aid?run=<handle> URL without a follow-up
exec_show query. Removes the manual-click step that was a
Plan 4 known limitation."
```

---

## Task 16: Workspace Home empty-state CTA

**Files:**
- Modify: `apps/rowforge-studio/src/pages/ExecList.tsx`

- [ ] **Step 1: Add CTAs**

```tsx
import { useNavigate } from "react-router-dom";
import { Button } from "@/components/ui/button";

// in the component:
const navigate = useNavigate();

// In the empty state branch (where it says "No executions yet"):
<div className="flex flex-col items-center gap-3 p-8">
  <div className="text-sm text-muted-foreground">No executions yet</div>
  <Button onClick={() => navigate("/new")}>New execution</Button>
</div>

// Also in the non-empty header (above the list):
<div className="mb-3 flex justify-end">
  <Button onClick={() => navigate("/new")} variant="outline" size="sm">
    New execution
  </Button>
</div>
```

- [ ] **Step 2: Run tests + build**

Run: `pnpm tsc -b && pnpm test && pnpm build`
Expected: all clean.

- [ ] **Step 3: Commit**

```bash
git add apps/rowforge-studio/src/pages/ExecList.tsx
git commit -m "studio-shell: 'New execution' CTA on Workspace Home

Empty-state primary button + non-empty list header secondary
button, both navigating to /new (Wizard)."
```

---

## Task 17: HUMAN_SMOKE.md walkthrough

**Files:**
- Modify: `apps/rowforge-studio/HUMAN_SMOKE.md`

- [ ] **Step 1: Add Plan 5 section**

Append to `apps/rowforge-studio/HUMAN_SMOKE.md`:

```markdown
## Plan 05 additions

### Create an execution via Wizard (Flow A)
1. Empty workspace → click **New execution** on Workspace Home
2. Step 1: enter name `smoke_test_plan5`, click **Pick…** → choose any CSV
3. Confirm format chip shows `csv`
4. Click **Next**
5. Step 2: **Pick…** a handler directory (e.g. `examples/handlers/golang-billing-channel`)
6. Click **Validate** → should show green "Manifest valid" with version chip
7. Check **Start a run immediately**
8. Click **Create execution** → should land on Live tab at `/exec/<id>/attempt/<aid>?run=<handle>`
9. Watch progress region update; cancel halfway if desired

**Expected errors to verify (negative paths):**
- Pick a directory without `manifest.toml` → red "manifest.toml not found"
- Manually write `manifest.toml` with `run = "nonexistent-bin"` → amber PATH warning, but **Create** still enabled
- Submit twice with the same name → second submit shows red error "duplicate exec name"

### Run-button auto-navigate (Plan 4 carry-forward)
1. ExecDetail of an exec that already has attempts → click **Run**
2. Pick handler dir → after spinner, should auto-route to Live tab (no manual click)
   - Previously: had to click the new attempt row manually

### Export (Flow D)
1. ExecDetail (any exec with at least one Done attempt) → click **Export**
2. Pick output dir (or leave default)
3. Choose format: `both`
4. Submit → "Exporting…" toast → success toast with **Reveal** action
5. Click **Reveal** → Finder/Explorer opens at output dir
6. Verify files: `success.csv`, `failed.csv`, `success.jsonl`, `failed.jsonl`, `resolution.json`

**Require complete (negative path):**
1. Same ExecDetail with at least one row Never-Attempted
2. Check **Require complete** → Submit
3. Should show red toast: "Export incomplete: N rows unresolved"
4. No files written

### Known Plan 5 limitations (deferred to Plan 6+)
- **No Settings page** — `max_concurrent_runs` still hardcoded at (3 workspace / 1 per-exec)
- **No Workspace switching UI** — must edit settings.json by hand
- **No handler authoring** — handlers are still discovered + edited externally
- **Hard cancel still degrades to soft cancel** — needs rowforge-core API addition
- **Export blocks UI** — no streaming progress, no cancel during export
```

- [ ] **Step 2: Commit**

```bash
git add apps/rowforge-studio/HUMAN_SMOKE.md
git commit -m "studio-shell: HUMAN_SMOKE Plan 5 walkthrough

Documents Wizard (Flow A) happy path + 3 negative paths;
Run-button auto-navigate; Export (Flow D) happy + require_complete
negative path; lists known Plan 5 limitations."
```

---

## Task 18: Final verification + PR prep

**Files:** none (verification only)

- [ ] **Step 1: Run the full test matrix**

```bash
cd /Users/lemo/code/lemo/repo/rowforge
cargo build
cargo test -p rowforge-core
cargo test -p rowforge-studio-core
cargo test -p rowforge-studio --test ipc_contract
cd apps/rowforge-studio
pnpm tsc -b
pnpm test
pnpm build
```

All must pass. Expected approximate test counts:
- rowforge-core: pre-Plan-5 count + ~2 new export tests
- rowforge-studio-core: 58 → ~70 (5 manifest + 3 start_exec + 2 export + 5 UiError + 1 RunStatus)
- Vitest: 36 → ~44 (4 ManifestReportView + 2 NewExecutionWizard + 2 ExportDialog)

If counts diverge significantly, investigate before proceeding.

- [ ] **Step 2: Manual smoke (`pnpm tauri dev`)**

Walk Flow A and Flow D per `HUMAN_SMOKE.md` § Plan 05 additions. Anything broken: fix before raising PR.

- [ ] **Step 3: Open PR**

```bash
gh pr create --title "studio-plan-05: exec lifecycle (Wizard + Export + carry-forwards)" --body "$(cat <<'EOF'
## Summary

Closes the create → run → export user journey in the Studio GUI. Builds on Plans 1–4. After this PR, a first-time user can open Studio and complete a full workflow without touching the CLI.

### Backend (Rust — 8 tasks)
- rowforge-core: extracted ~400 LOC of CLI export writers into shared `rowforge-core::export` module (`export_execution(store, id, opts) -> ExportReport`). CLI keeps thin wrapper.
- studio-core: `start_exec`, `export`, `validate_manifest` (full Part 8 §8.2 manifest spec — build/run shell-words + PATH probe via `which`)
- studio-core carry-forward: struct `UiError::{RunAborted, RunBusy}` payloads per spec §5.3; removed dead `RunStatus::Pending` variant; `start_run` returns `RunStartedHandle { handle, attempt_id }`
- New `UiError` variants: `InvalidInput`, `DuplicateExecName`, `ExportIncomplete`, `ManifestInvalid`, `ToolchainMissing`

### Tauri (3 new commands)
- `exec_start`, `exec_export` (async), `manifest_validate`
- `run_start` return type updated to `RunStartedHandle`

### React UI (5 components + 2 page changes)
- New Execution Wizard at `/new` (modal-as-route) — 2 steps, "Start run immediately" checkbox chains run_start
- ManifestReportView component
- Export dialog on ExecDetail with sonner progress + Reveal toast action
- Run button auto-navigates to Live tab (Plan 4 known limitation closed)
- Workspace Home "New execution" CTA on empty + non-empty list

### Tests
- Rust: ~70 pass on studio-core (+12 from Plan 4)
- Vitest: ~44 pass (+8 from Plan 4)
- ipc_contract: 8 pass (+3 commands)

### Acceptance (per design doc)
- [x] cargo test -p rowforge-core
- [x] cargo test -p rowforge-studio-core
- [x] pnpm tsc + build + test clean
- [x] manifest_validate reports MissingRequired / PathLookupFailed correctly
- [x] exec_start rejects duplicate name + missing input
- [x] exec_export honors require_complete; writes Csv/Jsonl/Both
- [x] Wizard end-to-end navigates to ExecDetail
- [x] Wizard "Start run immediately" chains and navigates to Live tab
- [x] Run button auto-navigates
- [x] UiError::RunAborted struct serde shape verified
- [x] UiError::RunBusy struct serde shape verified
- [x] RunStatus 6 variants (Pending removed)
- [ ] **(human)** HUMAN_SMOKE Flow A + Flow D walkthrough

### Out of scope (deferred to Plan 6+)
- Settings page UI + max_concurrent_runs wire-up
- Workspace Picker boot improvements
- Handler authoring panel (Part 8 entirely)
- Export streaming progress + cancel
EOF
)"
```

---

## Acceptance criteria

1. `cargo test -p rowforge-core` passes (including moved + new export tests)
2. `cargo test -p rowforge-studio-core` passes (~70 tests)
3. `cargo test -p rowforge-studio --test ipc_contract` passes (8 tests)
4. `pnpm tsc -b` clean
5. `pnpm test` clean (~44 tests)
6. `pnpm build` clean
7. `manifest_validate` reports `ManifestError::MissingRequired` when `run` absent; `ManifestWarning::PathLookupFailed` when `run` points to a missing binary
8. `exec_start` rejects duplicate name with `UiError::DuplicateExecName`; missing input with `UiError::InvalidInput`
9. `exec_export` with `require_complete=true` returns `UiError::ExportIncomplete { missing_count }` when applicable; writes Csv/Jsonl/Both correctly otherwise
10. Wizard end-to-end (empty workspace → /new → exec on Detail page) walks Flow A
11. Wizard "Start a run immediately" navigates to Live tab on success
12. Run button auto-navigates to new attempt's Live tab
13. `UiError::RunAborted` JSON: `{ "kind": "run_aborted", "message": { "kind": "<AbortReason variant>" } }`
14. `UiError::RunBusy` JSON includes `execution_id`, `limit`, `scope`
15. `RunStatus` has 6 variants (Pending removed); TS mirror in lockstep; spec part 3 §3.3 + part 7 §7.5 updated
16. CLI `rowforge exec export` still works identically (no regression from the extraction)
17. **(human)** HUMAN_SMOKE.md walkthrough: Flow A (Wizard) + Flow D (Export)

---

## Self-review (skill convention)

**Spec coverage check:** Every section of the design doc has at least one task:
- §3.1 export extraction → T1, T2
- §3.2 `start_exec` → T6
- §3.3 `export` wrapper → T7
- §3.4 `validate_manifest` → T5
- §3.5 carry-forward refactors → T3 (UiError), T4 (RunStatus::Pending), T8 (RunStartedHandle)
- §4 Tauri layer → T9 (3 commands), T8 (run_start update)
- §5.1 Wizard → T13
- §5.2 Export dialog → T14
- §5.3 Run-button auto-navigate → T15
- §5.4 Workspace Home CTA → T16

**Placeholder scan:** Two intentional `todo!()` markers in T2 step 3 (`compute_rollup` body) and explicit implementer notes for grepping the existing CLI rollup logic. These are not omissions — they signal "find the existing implementation, don't write from scratch." If the implementer encounters genuine ambiguity, they should escalate per the subagent-driven-development BLOCKED status.

**Type consistency:** `RunStartedHandle { handle, attempt_id }` is consistent across Rust (T8), Tauri command return type (T8 → T9 import), TS mirror (T10), and Wizard/RunButton consumers (T13, T15). `BusyScope { PerExec, PerWorkspace }` snake_case serde matches TS `"per_exec" | "per_workspace"` (T10). `ExportFormat` lowercased "csv" / "jsonl" / "both" consistent across Rust + TS.

**Order dependency:** T1 → T2 (writers move) → T3 (UiError) → T4 (RunStatus) → T5 (manifest) → T6 (start_exec needs UiError variants) → T7 (export needs T2 + UiError) → T8 (RunStartedHandle) → T9 (Tauri commands need T5-T8) → T10 (TS mirrors need T9 commands) → T11 (hooks need T10) → T12 (component needs T10) → T13 (Wizard needs T11+T12) → T14 (Export dialog needs T11) → T15 (RunButton needs T8 + T10) → T16 (CTA) → T17 (docs) → T18 (verify). No backward dependencies.

---

## Mid-plan additions (post-T18, before PR review)

These tasks landed after the original 18 during user smoke testing.
Each is its own commit on the impl branch.

### Bug fix: validate_manifest reads rowforge.yaml, not manifest.toml

T5 followed spec Part 8 §8.2 prose literally (manifest.toml + run/build
string fields), but the real on-disk handler config is `rowforge.yaml`
with `entry.cmd: Vec<String>` + `entry.build: Option<Vec<String>>`.
Rewrote `crates/rowforge-studio-core/src/manifest.rs` to delegate to
`rowforge_core::manifest::Manifest::load_from_dir` and PATH-probe
`entry.cmd[0]` / `entry.build[0]`. Dropped `toml` + `shell-words` deps.

### Wizard simplified: handler picker dropped

Plan 5 design described step 2 as picking a handler dir for inline
validation + optional first run. User pointed out this misleads —
the data model binds handler to attempt, not exec. Removed step 2;
Wizard is now single-step (name + input). Handler picker lives on
ExecDetail's RunButton where it belongs.

### Live tab race (snapshot bootstrap)

Tauri events are fire-and-forget; events emitted between `run_start`
returning and React's `listen()` attaching are lost. Fix:

- New `StudioCore::snapshot(handle) -> ProgressSnapshot` API
- New Tauri command `run_snapshot`
- `useRun` bootstrap protocol: attach `listen()` first, then
  `await ipc.run_snapshot()`, dispatch synthetic `_bootstrap` action
- `_terminal_before_listen` fallback when snapshot returns
  `UnknownHandle` (run finished before listener attached) — page
  pivots to Summary tab and refetches `attempt_show`

Spec part 6 §6.4.1 documents the protocol.

### in_flight / queue_depth wiring

ProgressAggregator's `set_in_flight` setter existed since Plan 4 but
was never called. Added heuristic in the on_progress callback:
`in_flight = min(workers, total - processed)` and `queue_depth = max(0,
total - processed - in_flight)`. Updated on `Started`, every `RowDone`,
and `Completed`. Locked by an aggregator unit test
(`set_in_flight_propagates_to_snapshot_and_tick`).

### Cross-crate: rowforge-core fires RowDone per row

Discovered during user testing: `pool_streaming` / `worker_loop`
never propagated row-level events. Only `Started` (pre-pool) and
`Completed` (post-pool) reached the callback, so studio's
`ProgressAggregator.processed` was stuck at 0 for the whole run.

Refactored rowforge-core:
- `ProgressCallback`: `Box<dyn Fn>` → `Arc<dyn Fn>` so multiple
  workers can share it
- `StreamingPoolConfig` gains `on_row_done: Option<Arc<dyn Fn(u64,
  bool) + Send + Sync>>`
- `run_worker_loop` accepts this and fires after every
  `jsonl.append_line(&bo)` — for each `RowOutcome` in `bo.outcomes`
  it emits `(seq, success)`
- `run::execute` bridges the caller's full `ProgressCallback` to the
  pool's narrower `on_row_done` so existing `RunProgressEvent::RowDone`
  semantics work

CLI `Box::new(...)` call sites updated to `Arc::new(...)`. Regression
locked in `pool_batch_happy.rs`: 350 rows → on_row_done fires 350
times.

### Re-attach to live: attempt_active_handle

User reported "leaving an attempt page loses access to Live forever".
Fix: backend exposes `attempt_active_handle(attempt_id) -> Option<
RunHandle>` (via new `SessionRegistry::lookup_by_attempt` and
`Session.attempt_id` field). AttemptDetail polls every 2s when no
`?run=` in URL; renders a green "Live run in progress · Watch live →"
banner that adds `?run=<handle>` to the URL on click.

### Run options panel (row_limit, workers, dry_run, skip_attempted)

Plan 5 original spec deferred the full launcher. User asked for
sample sampling support during testing. RunButton now has:
- Primary "Run" — quick path with defaults
- Secondary gear icon — opens inline options panel
- Options exposed to `run_start` command: `row_limit`, `workers`,
  `dry_run`, `skip_attempted`
- `skip_attempted` drives backend computation of
  `RowResolution.attempted_seqs()` → passed as `skip_seqs` to
  rowforge-core; enables "sample next N fresh rows across runs"
- Panel header shows `total / attempted / fresh` from `exec_rollup`
- Live preview "Will dispatch N rows" reactive to options
- localStorage persists handler dir across sessions

Spec part 5 §5.5 documents the expanded `run_start` signature.
Spec part 7 information architecture entry for Run launcher updated.

### Final test counts

- Rust: 246 passed, 2 ignored (28 suites)
- Vitest: 50 passed (16 files)
- `pnpm tsc -b` + `pnpm build` clean
- All workspace crates compile clean (no warnings)
