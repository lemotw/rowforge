# Plan 11 — Re-run failed rows from attempt

**Date:** 2026-05-26
**Branch:** `studio-plan-11-rerun-failed`
**Builds on:** Plans 3-10

## 1. Purpose

Workflow plan: after an attempt finishes with N failed/crashed rows, the user should be able to click a button on that attempt's **Failed rows** tab and dispatch a new attempt that targets only those rows. Same execution, same handler (snapshot from source attempt), filtered row dispatch.

Today the only way to "retry just the failures" is to manually export the failed rows to CSV, re-create an execution, and run again. Plan 11 collapses that to one click.

## 2. Scope decisions

| Decision | Choice | Rationale |
|---|---|---|
| Data model | Same exec, new attempt with row_id filter | Exec history stays clean; rollup naturally reflects latest state ("last attempt wins") |
| Failed scope | failed_last + crashed_last | "Ran but didn't succeed". Cancelled (user interrupted) excluded — those go through normal resume. Never_attempted excluded — those are handled by Plan 4's skip_attempted flag. |
| UI entry point | Button on AttemptDetail's Failed rows tab | User is already looking at the rows; "re-run these" is in-context |
| Handler choice | Same handler as source attempt; no picker | Intent is "re-run THIS exact same thing"; if user wants different handler they use the normal RunButton |
| Confirm UX | Simple Yes/No dialog showing row count + source attempt id | Lightweight; matches Plan 4-5 cancel-dialog pattern |
| Active-run gate | Refuse if exec has active run | Matches Plan 10 delete gate; consistent UX |

## 3. Backend changes

### 3.1 `rowforge-core::RunRequest` extension

Add a new field:
```rust
pub struct RunRequest {
    // ...existing fields...
    pub only_row_ids: Option<Vec<u64>>,
}
```

Semantics:
- `None` (default): existing behavior — dispatch all rows from the input file (modulo `skip_attempted`)
- `Some(empty_vec)`: dispatch nothing. Validation: reject empty vec at the caller level — calling backend with `Some(vec![])` returns an error or runs vacuously (it's a noop; we lean toward error)
- `Some([id1, id2, ...])`: dispatch only those row_ids. Skip all others in the input stream.

Interaction with existing `skip_attempted`:
- `only_row_ids` is more specific; when both are set, `only_row_ids` wins and `skip_attempted` is ignored.

### 3.2 `pool_streaming.rs` filter

In the row-iteration loop that reads CSV/JSONL input and decides whether to dispatch each row:

```rust
let only_set: Option<HashSet<u64>> = config.only_row_ids
    .as_ref()
    .map(|v| v.iter().copied().collect());

for row in input_iter {
    if let Some(set) = &only_set {
        if !set.contains(&row.row_id) {
            continue;
        }
    }
    // ...existing skip_attempted check (skip if only_row_ids unset)...
    // ...dispatch row...
}
```

Order: only_row_ids filter applied FIRST. If row_id is in the set, dispatch unconditionally (no skip_attempted override). If not in the set, skip.

### 3.3 `StudioCore::attempt_failed_row_ids`

New API:

```rust
impl StudioCore {
    /// Read the outcomes.jsonl for an attempt and return row_ids whose
    /// status is `failed` or `crashed`. Used by the React UI to populate
    /// the "Re-run N rows" button's count and the actual run dispatch.
    pub fn attempt_failed_row_ids(
        &self,
        exec_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<u64>, UiError>;
}
```

Implementation:
- Validate both IDs via `is_valid_id_component`
- Resolve attempt dir: `<workspace>/executions/<exec_id>/attempts/<attempt_id>/`
- Read `outcomes.jsonl` line-by-line; parse each as `Outcome { row_id, status, ... }`
- Collect row_ids where `status == "failed" || status == "crashed"` (verify exact string values)
- Return Vec<u64>, deduplicated and sorted (deterministic for tests)

Edge cases:
- File missing → `Ok(vec![])` (attempt may not have produced outcomes yet)
- Lines that fail to parse → skip silently with tracing::warn
- Order: outcomes.jsonl is append-only; the same row_id may appear multiple times if the attempt retried internally. Deduplicate.

### 3.4 `StudioCore::start_run` propagation

`start_run` already accepts `RunStartArgs` (or equivalent). Add `only_row_ids: Option<Vec<u64>>` to the args struct, plumb through to `RunRequest.only_row_ids`.

CLI's existing `start_run` callers pass `None` — no behavior change for CLI.

### 3.5 Handler resolution for re-run

The new attempt needs a handler_dir. Resolution order:
1. The source attempt's handler_dir (if attempts table stores it — look at how Plan 4-6 modeled handler_instance per attempt)
2. Fallback to `exec.last_handler_dir` (Plan 6 added this)
3. If neither exists → return `Err(UiError::Io("source attempt has no handler reference"))`

The React UI fetches the source attempt's handler_dir from its detail data and passes it to `start_run` alongside `only_row_ids`. No new backend logic needed beyond passing through.

## 4. Tauri shell

### 4.1 New command

```rust
#[tauri::command]
pub fn attempt_failed_row_ids(
    state: State<'_, AppState>,
    exec_id: String,
    attempt_id: String,
) -> Result<Vec<u64>, UiError>;
```

### 4.2 Existing `run_start` extension

Find the existing `run_start` Tauri command. Add `only_row_ids: Option<Vec<u64>>` to its arg shape and propagate to `StudioCore::start_run`. Backward compat: existing callers (RunButton) pass undefined → None.

## 5. React UI

### 5.1 New hook

```ts
export const useAttemptFailedRowIds = (execId: string, attemptId: string | null) =>
  useQuery({
    queryKey: ["attempt_failed_row_ids", execId, attemptId],
    queryFn: () =>
      ipc.attempt_failed_row_ids({ execId, attemptId: attemptId! }),
    enabled: !!attemptId,
  });
```

### 5.2 AttemptDetail Failed rows tab

Existing Failed rows tab shows a table. Add at the top:

```tsx
<div className="flex items-center justify-between mb-3">
  <span className="text-sm text-muted-foreground">
    {failedRowIds.length} failed row{failedRowIds.length === 1 ? "" : "s"}
  </span>
  <Button
    onClick={() => setRerunConfirmOpen(true)}
    disabled={failedRowIds.length === 0 || hasActiveRun || !handlerDir}
    title={
      failedRowIds.length === 0
        ? "No failed rows to re-run"
        : hasActiveRun
        ? "Cancel active run first"
        : !handlerDir
        ? "Source attempt has no handler reference"
        : undefined
    }
  >
    Re-run {failedRowIds.length} row{failedRowIds.length === 1 ? "" : "s"}
  </Button>
</div>
```

### 5.3 Confirm dialog: `RerunFailedDialog`

```
┌─ Re-run N failed rows? ────────────────────────────┐
│ A new attempt will be created on this execution     │
│ targeting only the N rows that failed or crashed   │
│ in this attempt.                                    │
│                                                     │
│ Handler: <handler_name>                            │
│ Source attempt: r_01ABC...                          │
│                                                     │
│         [Cancel]      [Re-run N rows]              │
└─────────────────────────────────────────────────────┘
```

On confirm → `useRunStart().mutate({ executionId, handlerDir, onlyRowIds: failedRowIds, rowLimit: null, workers: null, dryRun: null, skipAttempted: null })`. onSuccess: toast + navigate `/exec/<execId>/attempt/<new_attempt_id>?run=<handle>` (Plan 5 T15 pattern).

### 5.4 Active-run detection

Use the same approach as ExecList Plan 10: check `exec.last_attempt_state === "running"` from the loaded ExecDetail data. Or call `useActiveRuns()` if it's already wired on the detail page.

## 6. CLI

No CLI surface for re-run in this plan. Could add `rowforge attempt rerun-failed <attempt_id>` later if there's demand. Out of scope for v1.

## 7. Out of scope (explicit)

- ExecDetail header-level "Re-run all currently-failed across exec"
- Handler picker override on the confirm dialog
- Per-row preview / individual row selection (re-run all-or-none of this attempt's failures)
- Cross-attempt re-run (merge failures from multiple attempts)
- Custom sample / workers / dry-run for the re-run (uses defaults; user can use normal RunButton for that)
- CLI `attempt rerun-failed` subcommand

## 8. Testing

| Suite | Adds | Notes |
|---|---|---|
| rowforge-core | ~3 | only_row_ids filters dispatch; empty vec rejected; non-existent row_id skipped silently |
| rowforge-studio-core | ~4 | attempt_failed_row_ids reads outcomes.jsonl; empty file → empty vec; mixed status filtered correctly; ID validation rejects traversal |
| studio-shell ipc_contract | ~2 | new attempt_failed_row_ids command registered; run_start args shape includes only_row_ids |
| vitest | ~5 | useAttemptFailedRowIds hook; Re-run button disabled states; dialog renders; mutation calls run_start with only_row_ids; navigates on success |

Targets:
- cargo: 386 → ~395 (+9)
- vitest: 153 → ~158 (+5)

## 9. Spec doc updates

- `docs/spec/studio/part-3-runtime.md`: pool_streaming gains only_row_ids filter; row-dispatch precedence (only_row_ids > skip_attempted)
- `docs/spec/studio/part-5-api.md`: §5.5 new `attempt_failed_row_ids` command; existing `run_start` args extended with `only_row_ids`
- `docs/spec/studio/part-7-ui.md`: AttemptDetail Failed rows tab gains Re-run button + confirm flow
- Mirror in zh-Hant
- HUMAN_SMOKE Plan 11 walkthrough: ~15 steps covering happy path (fail some rows, re-run, verify only those re-dispatched), 0-failures button disabled state, active-run gate, dialog confirm, post-success navigation

## 10. Acceptance criteria

1. `cargo build && cargo test` clean
2. `pnpm tsc -b && pnpm test && pnpm build` clean
3. Failed rows tab shows "Re-run N rows" button reflecting accurate failed-row count
4. Button disabled (with tooltip) when N=0, when exec has active run, or when source attempt has no handler reference
5. Click → confirm dialog showing N + handler name + source attempt id
6. Confirm → new attempt created on same exec; auto-navigate to Live tab of new attempt
7. New attempt dispatches ONLY the failed row_ids; other rows not re-attempted
8. Rollup updates correctly after re-run completes (resolved rows transition out of failed-last)
9. HUMAN_SMOKE Plan 11 walkthrough added
10. Spec docs (part-3 + part-5 + part-7 en + zh-Hant) updated

## 11. Open questions

None at design time.
