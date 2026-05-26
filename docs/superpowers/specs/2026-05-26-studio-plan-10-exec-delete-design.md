# Plan 10 — Execution deletion (single + bulk) + ExecList column refresh

**Date:** 2026-05-26
**Branch:** `studio-plan-10-exec-delete`
**Builds on:** Plans 3-9

## 1. Purpose

Two related changes on the ExecutionsList surface:

1. **Hard delete** of executions (single and bulk), including the on-disk attempt data (`rm -rf <workspace>/executions/<id>/`). Refuses execs with active runs. UI uses a "Select mode" pattern.
2. **Column refresh**: surface disk size per exec; reorder columns to `Name (hover→id) / Rows / Attempts / Size / Created`.

## 2. Scope decisions

| Decision | Choice | Rationale |
|---|---|---|
| Delete entry point | ExecList "Select mode" toggle → checkbox column + Delete N button | One place to manage many; consistent with bulk-action conventions |
| Confirm UX | Simple Yes/No dialog listing items + warning | User-stated; matches Plan 4-5 cancel dialog pattern |
| Active-run gate | Refuse outright; row checkbox disabled with tooltip | Matches Plan 6 workspace-switch gate; avoids "magical" auto-cancel |
| Trash / undo | None — hard delete | User-stated `rm` semantics; simpler |
| Bulk failure mode | Serial per-item delete; accumulate failures; return `{ deleted, failed }` | Partial success keeps user moving |
| Size compute | Lazy walkdir on every `exec_list` query | Simplest; no cache coherence; document perf trade-off |
| Detail page when exec deleted elsewhere | 404 message + back link | Already a pattern from Plan 7 handler_not_found |

## 3. Backend changes

### 3.1 New API: `StudioCore::execution_delete`

```rust
impl StudioCore {
    /// Hard-delete an execution: sqlite cascade + rm -rf attempt dir.
    /// Refuses if SessionRegistry has any active run for this exec.
    pub fn execution_delete(&self, exec_id: &str) -> Result<(), UiError>;

    /// Bulk version. Serial; never aborts; returns per-exec outcome.
    pub fn execution_delete_bulk(
        &self,
        exec_ids: &[String],
    ) -> ExecDeleteBulkResult;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ExecDeleteBulkResult {
    pub deleted: Vec<String>,            // exec_ids that succeeded
    pub failed: Vec<ExecDeleteFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ExecDeleteFailure {
    pub exec_id: String,
    pub reason: String,                   // uiErrorMessage-compatible
}
```

### 3.2 Algorithm (single delete)

1. Validate `exec_id` via `is_valid_id_component` (defense — Plan 9 round-2 added the helper)
2. Check `SessionRegistry` for active runs on this exec → `Err(UiError::ExecutionInUse { exec_id })`
3. Begin sqlite transaction
4. Verify exec exists → `Err(UiError::NotFound)` if not (already deleted, idempotent? See §3.4)
5. `DELETE FROM executions WHERE id = ?` — relies on `ON DELETE CASCADE` to clean attempts / run_rollup / failed_rows
6. Commit transaction
7. `fs::remove_dir_all(<workspace>/executions/<id>/)` — best-effort; if it fails (permission, race), log warning but DON'T roll back sqlite (the row is already gone; orphan dir is acceptable; rerun can clean it)

### 3.3 Sqlite cascade audit

Plan 1-9 added several tables. Verify `ON DELETE CASCADE` is set on the FK to `executions.id`:
- `attempts` → exec_id
- `run_rollup` (if it has an exec_id column)
- `failed_rows` (if cached)
- any others

If cascade isn't enabled in existing migrations, add a schema migration in this plan that:
- For each child table, drop+recreate FK with `ON DELETE CASCADE`, OR
- Just manually `DELETE` from child tables before the executions row delete (simpler, no schema change)

The plan defaults to the simpler manual-cascade approach to avoid sqlite migration friction. T2 will verify what's needed.

### 3.4 Idempotency

If exec was already deleted (e.g. UI raced with another delete), return `Err(UiError::NotFound)` — caller decides whether to treat as success. Bulk delete swallows this as a "failure" with reason "execution not found"; UI can choose to render or suppress.

### 3.5 New error variants

```rust
pub enum UiError {
    // ...existing...
    ExecutionInUse { exec_id: String },
    // NotFound already exists; reuse for "exec was already gone"
}
```

### 3.6 `ExecSummary.size_bytes`

Extend the existing `ExecSummary` projection used by `exec_list`:

```rust
#[derive(...)]
#[non_exhaustive]
pub struct ExecSummary {
    pub id: String,
    pub name: Option<String>,
    pub created_at: DateTime<Utc>,
    pub input_rows: Option<u64>,
    pub attempts_count: u32,
    pub size_bytes: Option<u64>,         // NEW; None if dir missing or walkdir failed
}
```

`exec_list` walks each exec's dir and sums file sizes via `walkdir`:

```rust
fn dir_size_bytes(dir: &Path) -> Option<u64> {
    use walkdir::WalkDir;
    let mut total = 0u64;
    for entry in WalkDir::new(dir).follow_links(false).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            if let Ok(meta) = entry.metadata() {
                total = total.saturating_add(meta.len());
            }
        }
    }
    Some(total)
}
```

Returns `None` for missing dirs (e.g. user externally `rm`'d them).

**Performance**: walkdir is `O(n_files)`. For a workspace with 50 execs × 30 files each = 1500 stats, < 200ms on SSD. For 1000 execs (atypical), 1-2s — acceptable for a list page that already loads sqlite rows. Document the trade-off; lazy caching can come later if needed.

Add `walkdir = "2"` to workspace deps if not present.

## 4. Tauri shell

Three new commands:

```rust
#[tauri::command]
pub fn execution_delete(state: State<...>, exec_id: String) -> Result<(), UiError>;

#[tauri::command]
pub fn execution_delete_bulk(
    state: State<...>,
    app: tauri::AppHandle,
    exec_ids: Vec<String>,
) -> Result<ExecDeleteBulkResult, UiError>;
```

After successful delete (single or bulk with any successes), emit `exec_list:refresh` event so any other open ExecList views invalidate.

## 5. React UI

### 5.1 ExecList page

#### Columns (new order)

```
[ Select column (mode only) | Name | Rows | Attempts | Size | Created ]
```

- **Name**: monospace; hover shows full exec_id via `title={e.id}` attribute (lightweight; no popover/tooltip lib needed)
- **Rows**: right-aligned, `e.input_rows ?? "—"`
- **Attempts**: right-aligned, `e.attempts_count`
- **Size**: right-aligned, `formatBytes(e.size_bytes)` → "5.2 MB" / "1.3 GB" / "—"
- **Created**: ISO short form (existing)

`formatBytes` helper (TS):
```ts
function formatBytes(n: number | null | undefined): string {
  if (n == null) return "—";
  if (n < 1024) return `${n} B`;
  if (n < 1024 ** 2) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 ** 3) return `${(n / 1024 ** 2).toFixed(1)} MB`;
  return `${(n / 1024 ** 3).toFixed(2)} GB`;
}
```

#### Select mode

Header gets a **Select** toggle button. Default off; click to enter select mode:

- Each row sprouts a left-side `<input type="checkbox">`
- Checkbox is **disabled with tooltip "Cancel active run first"** when the exec has an active run
- Top toolbar shows `Delete N executions…` (red destructive variant; disabled when N = 0) and `Cancel selection`
- Clicking a row in select mode toggles its checkbox instead of navigating
- Cancel exits select mode and clears selection

Active-run detection: use the existing `active_runs` stream / query (Plan 4) and check `exec_id` membership.

#### Delete dialog

`DeleteExecutionsDialog` shows:
- Title: "Delete N executions?"
- List of selected execs by name + size (max ~10 rows; "... and X more" if longer)
- Warning copy:
  > This permanently deletes the executions and all their attempt data
  > (outcomes, handler logs, exports, etc.). Total: ~X GB. This cannot
  > be undone.
- Buttons: `Cancel` / `Delete N executions` (red)

On confirm → `useExecutionDeleteBulk` mutation:
- Success (`failed.length === 0`): toast `"N executions deleted"`, invalidate `exec_list` query, exit select mode
- Partial: toast `"M deleted, K failed"`, render the failed list in an inline alert below the toolbar with reasons
- Outright error (Tauri throws): toast with uiErrorMessage

### 5.2 ExecDetail page

When the user is on `/exec/<id>` and the exec gets deleted (from this or another window):

- TanStack query re-fetches via cache invalidation → returns NotFound
- Page renders: "This execution has been deleted." + `← Back to executions` link
- Reuse the Plan 7 pattern (HandlerDetailPage's not-found branch)

### 5.3 Bulk failure rendering

If bulk delete partial-fails, render below the toolbar (above the table):

```
⚠ 2 of 5 deletions failed:
  • e_01ABC...: execution_in_use (cancel run first)
  • e_01XYZ...: io error: permission denied (rm)
[Dismiss]
```

Dismiss button hides the alert; selection stays cleared.

## 6. CLI

Add `rowforge exec delete <id>` and `rowforge exec delete --all-completed`:

```
USAGE:
  rowforge exec delete <exec_id>          # single
  rowforge exec delete --all-completed    # all execs with no active run
  rowforge exec delete --force <exec_id>  # bypass active-run check; soft-cancel
                                          # in-flight run first (best-effort)
```

`--force` is documented but defers to the existing soft-cancel pathway since hard cancel doesn't exist yet (Plan 11 candidate).

Per-deletion outcome printed to stderr:
```
[e_01ABC...] deleted (245 MB freed)
[e_01XYZ...] skipped: has active run
```

Exit code = failure count.

## 7. Settings

No additions.

## 8. Out of scope (explicit)

- Trash / undo / time-windowed recovery
- Partial-attempt deletion (only at exec layer for v1; attempts always go with their parent)
- Deletion progress UI for very large execs (single rm_dir_all is fast enough at our typical sizes)
- Size cache in sqlite (lazy walkdir for v1)
- Hard cancel of active runs to enable delete (deferred to Plan 11 — same long-standing limitation)
- Filtering ExecList by date / size / name (no search; same as today)

## 9. Testing

| Suite | Adds | Notes |
|---|---|---|
| rowforge-studio-core | ~9 | single delete happy, single delete refuses active, single delete idempotent (already-gone), single delete cascade verify (attempts gone too), single delete removes dir, bulk delete all-succeed, bulk delete partial-fail, size_bytes computation, size_bytes returns None for missing dir |
| rowforge-cli | ~3 | `exec delete <id>` happy, `--all-completed` skips active, exit code = fail count |
| studio-shell ipc_contract | ~2 | command registration + JSON shape (ExecDeleteBulkResult) |
| vitest | ~8 | Select mode toggle, checkbox disabled for active-run row, Delete N button enable/disable, dialog renders + total size, mutation invalidates list, partial-fail alert renders, formatBytes unit tests, hover tooltip on name |

Targets:
- cargo: 369 → ~383 (+14)
- vitest: 139 → ~147 (+8)

## 10. Spec doc updates

- `docs/spec/studio/part-2-model.md`: `ExecSummary.size_bytes: Option<u64>`
- `docs/spec/studio/part-3-runtime.md`: exec lifecycle — deletion (cascade + rm)
- `docs/spec/studio/part-5-api.md`: 2 new commands, 1 new UiError variant
- `docs/spec/studio/part-7-ui.md`: ExecList Select mode flow, column order change
- All mirrored in zh-Hant
- `apps/rowforge-studio/HUMAN_SMOKE.md`: Plan 10 section with ~20 steps covering Select mode, bulk happy, partial fail, active-run gate, ExecDetail 404 fallback, CLI subcommands, Size column display

## 11. Acceptance criteria

1. `cargo build && cargo test` clean
2. `pnpm tsc -b && pnpm test && pnpm build` clean
3. ExecList shows new column order with Size cell populated
4. Hover on Name shows the full exec_id
5. Select toggle exposes checkboxes; active-run rows have disabled checkbox + tooltip
6. Delete N opens confirm dialog listing items; confirm fires bulk mutation
7. Bulk all-success: toast + list refresh
8. Bulk partial-fail: toast + inline alert with reasons
9. Active-run refusal surfaces via `ExecutionInUse` UiError, rendered friendly
10. ExecDetail of a deleted exec shows the "deleted" empty state with back link
11. CLI `rowforge exec delete <id>` works; non-zero exit on failure
12. HUMAN_SMOKE Plan 10 walkthrough added
13. Spec docs (part-2/3/5/7 en + zh-Hant) updated

## 12. Open questions

None at design time.
