# Studio Plan 05 — Exec Lifecycle 收尾 (design)

> **Status:** brainstorm output for Plan 5. Builds on Plans 1-4 (foundation, Tauri shell, exec history, live runs). This is the *design* document; the implementation plan lives at `docs/superpowers/plans/2026-05-24-studio-plan-05-exec-lifecycle.md` (to be written next).

## 1. Goal

Close the **create → run → export** user journey end-to-end in the Studio GUI. After Plan 5, a user opening Studio for the first time should be able to:

1. Pick a workspace
2. Create an execution via the **New Execution Wizard** (no CLI needed)
3. Run it (existing Plan 4 Run button, now with auto-navigate to Live tab)
4. **Export** results to disk via a modal — without touching `rowforge exec export`

The plan also retires Plan 4's three highest-value carry-forward items so the run lifecycle stops accreting debt: struct `UiError::{RunAborted, RunBusy}`, removal of `RunStatus::Pending`, and `run_start` returning the new `AttemptId`.

## 2. Scope

### In scope
- **StudioCore APIs**: `start_exec`, `export`, `validate_manifest`
- **Tauri commands**: `exec_start`, `exec_export`, `manifest_validate`; `run_start` signature updated
- **React UI**: New Execution Wizard (`/new`), Export dialog, Run-button auto-navigate, Workspace Home empty-state CTA
- **Carry-forward refactors**: struct `UiError::RunAborted/RunBusy`, remove `RunStatus::Pending`, `run_start` returns `(RunHandle, AttemptId)`
- **rowforge-core refactor**: extract CLI export writers into a shared `rowforge-core::export` module

### Out of scope (deferred)
- Settings page UI → Plan 6 candidate
- `Settings.max_concurrent_runs` wire-up to `SessionRegistry` → Plan 6
- Workspace Picker boot UI improvements (current auto-redirect stays)
- Handler authoring panel (Part 8 entirely) → its own plan
- Export streaming progress / cancel
- `total_rate` / `slowest_run` in `RunRollupTick` (still `0.0` / `None`)

## 3. Backend design

### 3.1 `rowforge-core::export` extraction

The CLI's `exec_cmd.rs` currently owns ~400 LOC of export logic (`write_success_csv`, `write_failed_csv`, `write_success_jsonl`, `write_failed_jsonl`, `write_resolution_json_with_completeness`, `discover_success_keys`, `discover_failure_data_keys`, `collect_aborted_attempts`, `emit_export_warnings`). Move these into a new module `rowforge-core/src/export.rs` with a single public entry:

```rust
pub fn export_execution(
    store: &ExecutionStore,
    id: &ExecutionId,
    opts: &ExportOpts,
) -> Result<ExportReport, ExportError>;
```

- CLI's `exec_cmd::run_export` becomes a thin wrapper that builds `ExportOpts` from `ExportArgs` and calls `export_execution`.
- Studio's `StudioCore::export` calls `export_execution` directly.
- CLI tests must remain green. Move tests too; refactor doesn't widen behavior.

`ExportReport`:
```rust
#[non_exhaustive]
struct ExportReport {
    output_dir: PathBuf,
    written_files: Vec<PathBuf>,     // absolute paths, in write order
    success_count: u64,
    failed_count: u64,
    warnings: Vec<ExportWarning>,    // mirrored from current CLI behavior
}
```

### 3.2 `StudioCore::start_exec`

```rust
pub fn start_exec(&self, args: StartExecArgs) -> Result<ExecutionId, UiError>;
```

`StartExecArgs` is already defined in spec §5.2:
```rust
struct StartExecArgs {
    input_path: PathBuf,
    name: String,
    csv_id: Option<String>,
    pinned_handler_instance: Option<HandlerInstanceId>,
}
```

Behavior:
1. Validate `input_path` exists, readable, format detectable (csv/jsonl/ndjson by extension; sniff first line if extension absent).
2. `ExecutionStore::create_execution(NewExecution { ... })` — already exists in rowforge-core.
3. Return the new `ExecutionId`.

Errors (two new `UiError` variants needed):
- `UiError::InvalidInput { reason: String }` (path missing, unreadable, format undetectable)
- `UiError::DuplicateExecName { name: String }` — `executions.name` is unique per workspace (existing constraint)
- `UiError::Internal` for sqlite failures

Does **not** start a run. Wizard's "Start a run immediately" checkbox is a chained client-side call to `run_start`, not a server-side coupling.

### 3.3 `StudioCore::export`

```rust
pub fn export(&self, e: &ExecutionId, opts: ExportOpts)
    -> Result<ExportReport, UiError>;
```

Thin wrapper around `rowforge_core::export::export_execution`. Maps `ExportError` → `UiError`:
- `ExportError::Incomplete { missing_count }` → `UiError::ExportIncomplete { missing_count }` (new variant)
- `ExportError::Io(_)` → `UiError::Internal`
- `ExportError::NotFound` → `UiError::NotFound`

`ExportOpts` per spec §5.2:
```rust
struct ExportOpts {
    output_dir: Option<PathBuf>,     // defaults to <workspace>/exports/<exec_name>_<ulid>/
    format: ExportFormat,            // Csv | Jsonl | Both
    require_complete: bool,
}
```

`require_complete` is re-checked at export time inside `export_execution` (don't trust the UI). Returns `Incomplete` before any file is written.

### 3.4 `StudioCore::validate_manifest`

```rust
pub fn validate_manifest(&self, source: ManifestSource)
    -> Result<ManifestReport, UiError>;
```

In v1, `ManifestSource` is just `Path(PathBuf)`. Validation per spec §8.2:

1. Read `<dir>/manifest.toml`; missing file → `ManifestError::ManifestMissing`
2. Parse TOML → `Manifest` struct; parse failure → `ManifestError::ParseFailed { message }`
3. Verify required fields: `run` is non-empty `String`; missing → `ManifestError::MissingRequired { field: "run" }`
4. Verify shell-words parse for both `build` (if present) and `run`; failure → `ManifestError::ShellParseFailed { field, message }`
5. PATH-probe the first token of `run`; failure → `ManifestWarning::PathLookupFailed { field: "run", token }` (warning, not error — `PATH` differs across machines)
6. PATH-probe the first token of `build` if present → warning on miss

Use the `which` crate for cross-platform PATH lookup (handles Windows PATHEXT correctly).

```rust
struct ManifestReport {
    manifest: Option<Manifest>,         // Some if parse succeeded
    errors: Vec<ManifestError>,
    warnings: Vec<ManifestWarning>,
}
```

Errors block submission; warnings are informational. Pure function — no side effects, no caching needed (warm class).

### 3.5 Carry-forward refactors

**`UiError::RunAborted` → struct variant**

Currently:
```rust
RunAborted(String),
```

New (per spec §5.3):
```rust
RunAborted { reason: AbortReason },
```

Serde: `#[serde(tag = "kind", content = "message")]` adjacent tagging stays — the content position holds the `AbortReason` discriminated union JSON.

**`UiError::RunBusy` → struct variant**

Currently:
```rust
RunBusy(String),
```

New:
```rust
RunBusy {
    execution_id: ExecutionId,
    limit: u32,
    scope: BusyScope,     // PerExec | PerWorkspace
},
```

`BusyScope` is a new public type:
```rust
#[non_exhaustive]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BusyScope { PerExec, PerWorkspace }
```

`SessionRegistry::can_start` already returns a structured `BusyReason`. Plumb the fields through.

**Remove `RunStatus::Pending`**

Plan 4 finding: sessions transition `Starting → Running → terminal`. `Pending` was reserved for "queued but not yet starting" — that never materialized. Remove the variant from:
- `crates/rowforge-studio-core/src/run_handle.rs` (RunStatus enum)
- spec part 3 §3.3 + spec part 7 §7.5 color mapping table
- TS mirror `apps/rowforge-studio/src/ipc/types.ts`
- Any test fixtures

This is a breaking serde change. Plan 5 ships pre-1.0; no backwards compat shim needed.

**`run_start` returns `(RunHandle, AttemptId)`**

Currently:
```rust
pub fn start_run(&self, exec: &ExecutionId, opts: RunOpts) -> Result<RunHandle, UiError>;
```

New:
```rust
pub fn start_run(&self, exec: &ExecutionId, opts: RunOpts)
    -> Result<RunStartedHandle, UiError>;

#[non_exhaustive]
struct RunStartedHandle {
    handle: RunHandle,
    attempt_id: AttemptId,
}
```

Tauri command + TS mirror updated. `useRunStart` mutation returns the struct; consumers navigate to `/exec/:id/attempt/:aid?run=<handle>`.

## 4. Tauri layer

Three new sync commands:
- `exec_start(args: StartExecArgs) -> ExecutionId`
- `exec_export(id, opts: ExportOpts) -> ExportReport`
- `manifest_validate(source: ManifestSource) -> ManifestReport`

`exec_export` is potentially long (seconds to minutes). Use `tauri::async_runtime::spawn_blocking` or run on Tauri's tokio runtime via `async fn` command — same pattern as Plan 4 `run_start` (block guard scoped before `.await`).

`run_start` signature change: command return type updated to `RunStartedHandle`.

No new event channels. Export is fire-and-await, no streaming.

## 5. React UI

### 5.1 New Execution Wizard

Modal-as-route at `/new` (HashRouter). Closes via "Cancel" or after successful submit. Two steps:

**Step 1 — Identity + input**
- Name (text input, required, regex `[a-z0-9_-]+`, max 64)
- Input path (Tauri file dialog, csv/jsonl/ndjson filter)
- Auto-detect format from extension; show inferred format as read-only chip
- "Next" enabled when both fields valid + path exists

**Step 2 — Handler + validate**
- Handler dir picker (Tauri dialog, directory mode)
- "Validate" button → calls `manifest_validate({ Path: dir })`
- Inline render of `ManifestReport`:
  - Errors: red banner with list (blocks Submit)
  - Warnings: amber banner with list (does not block)
  - Success state: green check + `manifest.version` + `manifest.language` chips
- Optional checkbox: **"Start a run immediately after creation"**
- "Submit" enabled when validate succeeded with no errors

**Submit flow:**
1. Call `exec_start({ name, input_path, csv_id: null, pinned_handler_instance: null })` → get `ExecutionId`
2. If "Start a run immediately":
   - Call `run_start(execution_id, handler_dir)` (Tauri command signature; backend constructs `RunOpts::new(handler_dir)` with all other fields at their defaults: `limit=None`, `dry_run=false`, `workers=None`, `force=false`, `retry_failed=false`, `config_overrides={}`, `mapping=None`, `sync_data=false`) → get `RunStartedHandle`
   - Navigate to `/exec/:id/attempt/:aid?run=<handle>` (live mode)
3. Else: navigate to `/exec/:id`

Files: `apps/rowforge-studio/src/pages/NewExecutionWizard.tsx` (or `routes/new.tsx`), `src/components/ManifestReportView.tsx`.

### 5.2 Export dialog

Modal on ExecDetail header (right of "Run" button). Triggered by Export button.

Fields:
- Output dir (Tauri directory picker; default value: `<workspace>/exports/<exec_name>_<timestamp>/`)
- Format: segmented control `Csv | Jsonl | Both`
- `require_complete` checkbox
  - Disabled with tooltip "All rows resolved" if rollup shows no `never_attempted`
  - Otherwise enabled, unchecked by default

**Submit:**
1. Show indeterminate progress toast via `sonner` (loading state)
2. Await `exec_export(id, opts)` (blocks; can take minutes)
3. On success: dismiss loading toast, show success toast with:
   - "Exported N rows to `<output_dir>`"
   - Action button: **"Reveal output dir"** → Tauri `shell::open(output_dir)`
4. On `UiError::ExportIncomplete`: error toast "N rows still unresolved — uncheck 'Require complete' or finish the run first"
5. On other error: error toast with `UiError.kind` message

Files: `apps/rowforge-studio/src/components/ExportDialog.tsx`, `src/components/ExportButton.tsx`, `src/ipc/use-export.ts`.

### 5.3 Run button auto-navigate

`RunButton.tsx` currently shows "✓ Started" + navigates to `/exec/:id`. Update:
- `useRunStart` mutation returns `RunStartedHandle`
- On success: `navigate(\`/exec/${execId}/attempt/${attempt_id}?run=${handle}\`)`
- Plan 4's HUMAN_SMOKE limitation note "Auto-navigate to ?run=... after Run button" can be removed.

### 5.4 Workspace Home empty-state CTA

`ExecList.tsx` currently shows "No executions yet" when list is empty. Add primary "New execution" button → `navigate("/new")`. Also add a secondary "New execution" button to the non-empty list header for convenience.

## 6. Risks / open questions

1. **Export refactor blast radius.** Moving ~400 LOC of CLI export logic into rowforge-core touches existing tests. Mitigation: refactor in one commit, keep behavior identical, move tests along with code.

2. **Modal-as-route vs in-page modal.** Spec §7.3 says modal-as-route. Deep-link works (good); browser back closes the wizard (acceptable). Decision: follow spec, use route.

3. **`which` crate adoption.** Adding a new core dep. Alternative: roll PATH probe by hand (~20 LOC). `which` (v6+) is mature, ~50 KB, handles Windows correctly. Decision: use `which`.

4. **`require_complete` enforcement.** Backend re-validates inside `export_execution` even if UI also gates. Returns `Incomplete { missing_count }` before any file is written.

5. **`UiError::RunAborted` carries `AbortReason` (which contains nested data).** Serde shape: `{ kind: "run_aborted", message: { kind: "user_cancelled" } }` — the `content` slot holds the discriminated union. Verify TS mirror handles nested kind correctly.

6. **`RunStatus::Pending` removal is a breaking change in stored data?** No — `RunStatus` is in-memory only (sessions live in `SessionRegistry`). No sqlite migration needed. `AttemptState` (the persisted enum) is unaffected.

7. **Export filename collisions.** Default output dir is `<workspace>/exports/<exec_name>_<timestamp>/`. Timestamp guarantees uniqueness. If user picks an existing dir manually, `export_execution` writes files inside; collisions overwrite (existing CLI behavior).

8. **Wizard "Start run immediately" failure modes.** If `exec_start` succeeds but `run_start` fails (e.g., concurrency limit), what happens? Decision: navigate to `/exec/:id` anyway, show a toast with the run error. Exec is created; user can manually retry the run.

## 7. Acceptance criteria

1. `cargo test -p rowforge-core` passes (including moved export tests)
2. `cargo test -p rowforge-studio-core` passes; net test delta tracked
3. `cargo test -p rowforge-studio --test ipc_contract` passes
4. `pnpm tsc -b` clean
5. `pnpm test` clean (new tests for ManifestReportView, ExportDialog, NewExecutionWizard)
6. `pnpm build` clean
7. `manifest_validate` reports `ManifestError::MissingRequired` when `run` absent; `ManifestWarning::PathLookupFailed` when `run` points to missing binary
8. `exec_start` rejects duplicate name with `UiError::DuplicateExecName`
9. `exec_export` with `require_complete=true` returns `UiError::ExportIncomplete` when applicable; writes Csv/Jsonl/Both correctly otherwise
10. Wizard end-to-end (empty workspace → exec on Detail page) walks Flow A from spec §7.4
11. Wizard "Start a run immediately" chain navigates to Live tab
12. Run button auto-navigates to new attempt's Live tab
13. `UiError::RunAborted` JSON shape: `{ kind: "run_aborted", message: { kind: <AbortReason variant>, ... } }`
14. `UiError::RunBusy` JSON includes `execution_id`, `limit`, `scope`
15. `RunStatus` has 6 variants (Pending removed); TS mirror in lockstep
16. **(human)** HUMAN_SMOKE.md walkthrough: Flow A (Wizard) + Flow D (Export)

## 8. Spec impact

This plan implements existing spec surface; no spec changes required other than confirming behavior at:
- §5.2 `StartExecArgs`, `ExportOpts`, `ExportReport`
- §5.3 `UiError` variants gain struct payloads
- §5.5 `exec_start`, `exec_export`, `manifest_validate` commands
- §7.4 Flow A + Flow D
- §8.2 `validate_manifest` extended for `build` + `run` PATH probing

After Plan 5 lands, spec §3.3 `RunStatus` table must drop the `Pending` row.

## 9. Out-of-scope items captured for future plans

| Item | Target plan |
|---|---|
| Settings page UI (workspace_root, preferred_editor, max_concurrent_runs, lastHandlerDir) | Plan 6 |
| `Settings.max_concurrent_runs` → `SessionRegistry` wire-up + live-reload | Plan 6 |
| Workspace Picker boot improvements (proper empty-state UI, switch workspace) | Plan 6 |
| Handler authoring panel (Part 8 entirely) | Separate plan |
| Export streaming progress + cancel | Future |
| `total_rate` / `slowest_run` in `RunRollupTick` | Future |
| Hard cancel actually kills workers (needs rowforge-core API addition) | Future |
| In-Studio code editor | Explicitly non-goal |
