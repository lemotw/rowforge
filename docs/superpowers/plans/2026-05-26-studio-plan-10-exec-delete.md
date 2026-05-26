# Plan 10 — Execution deletion + ExecList size column Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use `- [ ]` checkbox syntax.

**Goal:** ExecList page gains (1) Select mode for bulk delete that also rm-rf's the on-disk attempt data, (2) a Size column, (3) hover-tooltip for the full exec_id on Name. CLI gains `rowforge exec delete`.

**Architecture:** `StudioCore::execution_delete` validates name, refuses if there's an active run, manually cascades child sqlite rows in transaction order, then `fs::remove_dir_all` on the attempt dir. Bulk wraps single; serial; accumulates per-item failures. `exec_list` walks each exec dir to compute `size_bytes` lazily.

**Tech stack:** Rust (rowforge-core / rowforge-studio-core / rowforge-cli / Tauri 2), React 19 + TanStack Query v5 + Tailwind + shadcn.

**Design spec:** `docs/superpowers/specs/2026-05-26-studio-plan-10-exec-delete-design.md`

---

## Task 1: studio-core `execution_delete` single

**Files:**
- Modify: `crates/rowforge-studio-core/src/lib.rs` — `execution_delete` method
- Modify: `crates/rowforge-studio-core/src/error.rs` — `UiError::ExecutionInUse` variant
- Test: `crates/rowforge-studio-core/tests/foundation.rs`

- [ ] **Step 1: Add UiError variant**

In `crates/rowforge-studio-core/src/error.rs`:

```rust
pub enum UiError {
    // ...existing...
    #[error("execution '{exec_id}' has an active run; cancel it first")]
    ExecutionInUse { exec_id: String },
}
```

Match the existing adjacent-tag serde envelope (Plan 7 round-2 confirmed `#[serde(tag = "kind", content = "message", rename_all = "snake_case")]`).

- [ ] **Step 2: Discover sqlite schema for cascade**

```bash
cd /Users/lemo/code/lemo/repo/rowforge
grep -rnE 'CREATE TABLE.*(attempts|run_rollup|failed_rows|executions)' crates/rowforge-studio-core/src/ crates/rowforge-core/src/
```

List every child table that holds a `exec_id` column and verify FK constraints. The exact set may include: `attempts`, `run_rollup`, `failed_rows`, possibly others. Note the column name in each (`exec_id` vs `execution_id`).

- [ ] **Step 3: Write failing test**

Append to `crates/rowforge-studio-core/tests/foundation.rs`:

```rust
#[test]
fn execution_delete_removes_row_attempts_and_dir() {
    let tmp = TempDir::new().unwrap();
    let core = open_test_workspace(tmp.path());  // existing helper
    // Seed an exec with at least one attempt + on-disk dir.
    // Look at how existing tests seed an exec — Plan 5 added create_execution
    // and Plan 6 added last_handler_dir. Use those.
    let exec_id = seed_exec_with_attempt(&core);
    let exec_dir = tmp.path().join("executions").join(&exec_id);
    assert!(exec_dir.exists());

    core.execution_delete(&exec_id).expect("delete ok");

    // Sqlite row gone
    assert!(core.exec_show(&exec_id).is_err());
    // Dir gone
    assert!(!exec_dir.exists());
}
```

> `seed_exec_with_attempt` doesn't exist yet — write it as a helper in the test file, OR inline a `ExecutionStore::create_execution` call + `start_run` mock OR direct sqlite inserts. Look at Plan 5 `create_execution` happy-path tests for the cheapest seeding pattern.

- [ ] **Step 4: Run test, verify it fails**

```bash
cargo test -p rowforge-studio-core --test foundation execution_delete_removes
```

Expected: compile error (method doesn't exist) or panic.

- [ ] **Step 5: Implement `execution_delete`**

In `crates/rowforge-studio-core/src/lib.rs`:

```rust
impl StudioCore {
    pub fn execution_delete(&self, exec_id: &str) -> Result<(), crate::UiError> {
        // Defense: reject malformed IDs early (Plan 9 round-2 helper).
        if !is_valid_id_component(exec_id) {
            return Err(crate::UiError::Io(format!("invalid exec_id: {}", exec_id)));
        }

        // Active-run gate via SessionRegistry.
        if self.session_registry.has_active_run_for_exec(exec_id) {
            return Err(crate::UiError::ExecutionInUse { exec_id: exec_id.to_string() });
        }

        // Sqlite cascade (transaction).
        let store = self.store.clone();
        let conn_result: Result<(), rusqlite::Error> = (|| {
            let mut conn = store.conn()?;
            let tx = conn.transaction()?;
            // Order matters when not using ON DELETE CASCADE — delete leaves first.
            tx.execute("DELETE FROM failed_rows WHERE exec_id = ?", [exec_id])?;
            tx.execute("DELETE FROM run_rollup WHERE exec_id = ?", [exec_id])?;
            tx.execute("DELETE FROM attempts WHERE exec_id = ?", [exec_id])?;
            let rows = tx.execute("DELETE FROM executions WHERE id = ?", [exec_id])?;
            if rows == 0 {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
            tx.commit()?;
            Ok(())
        })();
        match conn_result {
            Ok(()) => {}
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                return Err(crate::UiError::NotFound(format!("execution '{}' not found", exec_id)));
            }
            Err(e) => return Err(crate::UiError::Io(format!("sqlite cascade: {}", e))),
        }

        // Best-effort dir rm. Don't roll back sqlite on failure — row's already
        // gone; orphan dir is acceptable. Log via tracing for diagnostics.
        let dir = self.workspace.root.as_path().join("executions").join(exec_id);
        if dir.exists() {
            if let Err(e) = std::fs::remove_dir_all(&dir) {
                tracing::warn!(exec_id = %exec_id, error = %e, "execution_delete: rm_dir_all failed");
                // Note: NOT returning an error here; sqlite is authoritative
            }
        }
        Ok(())
    }
}
```

> Adapt:
> - The exact `SessionRegistry` method name for "is exec in use" — may be `has_active_attempt_for_exec` or you may need to add one. If it doesn't exist, add a small helper: iterate `active_runs`, return true if any has matching `exec_id`.
> - The `store.conn()` API — Plan 1-3 set this up; check the exact call pattern. May be `self.store.with_conn(|conn| ...)`.
> - Child table names — verify in Step 2. If a table doesn't exist, drop that DELETE line.
> - `UiError::NotFound` — verify variant exists; might be named differently. If not, add it or use `Io` with a friendly message.

- [ ] **Step 6: Run test — should pass**

- [ ] **Step 7: Add edge-case tests**

```rust
#[test]
fn execution_delete_refuses_when_active_run() {
    let tmp = TempDir::new().unwrap();
    let core = open_test_workspace(tmp.path());
    let exec_id = seed_exec_with_attempt(&core);
    // Force the SessionRegistry to register an active run for this exec.
    // If there's no public API to do this in tests, you may need to expose
    // a test-only helper or use the broadcast-channel insert path.
    seed_active_run_in_registry(&core, &exec_id);

    let err = core.execution_delete(&exec_id).unwrap_err();
    assert!(matches!(err, UiError::ExecutionInUse { ref exec_id: _ }));
}

#[test]
fn execution_delete_idempotent_returns_not_found() {
    let tmp = TempDir::new().unwrap();
    let core = open_test_workspace(tmp.path());
    let err = core.execution_delete("e_nonexistent").unwrap_err();
    assert!(matches!(err, UiError::NotFound(_)) || matches!(err, UiError::Io(_)));
}

#[test]
fn execution_delete_rejects_traversal_id() {
    let tmp = TempDir::new().unwrap();
    let core = open_test_workspace(tmp.path());
    let err = core.execution_delete("../etc").unwrap_err();
    assert!(matches!(err, UiError::Io(_)));
}

#[test]
fn execution_delete_succeeds_when_dir_already_missing() {
    let tmp = TempDir::new().unwrap();
    let core = open_test_workspace(tmp.path());
    let exec_id = seed_exec_with_attempt(&core);
    // Externally rm the dir first.
    let dir = tmp.path().join("executions").join(&exec_id);
    std::fs::remove_dir_all(&dir).unwrap();
    // Delete should still succeed (sqlite path works; dir-missing is OK).
    core.execution_delete(&exec_id).expect("delete should succeed");
}
```

- [ ] **Step 8: Run all new tests + verify no regressions**

```bash
cargo test -p rowforge-studio-core
```

Expected: all pass; new tests included; existing tests unchanged.

- [ ] **Step 9: Commit**

```bash
git add crates/rowforge-studio-core/src/lib.rs crates/rowforge-studio-core/src/error.rs crates/rowforge-studio-core/tests/foundation.rs
git commit -m "studio-core: execution_delete (single, with active-run gate + cascade)

Hard-delete an exec:
- Validate exec_id (is_valid_id_component, Plan 9 helper)
- Refuse if SessionRegistry has an active run for this exec
  (UiError::ExecutionInUse)
- Manual sqlite cascade in transaction: failed_rows, run_rollup,
  attempts, then executions
- Best-effort fs::remove_dir_all of <workspace>/executions/<id>/.
  If the dir is missing or rm fails, sqlite is still authoritative
  — log warning but return Ok.

+4 integration tests: happy path, active-run refusal, idempotent
NotFound on second delete, traversal rejection, dir-already-missing
path."
```

---

## Task 2: studio-core `execution_delete_bulk`

**Files:**
- Modify: `crates/rowforge-studio-core/src/lib.rs` — `execution_delete_bulk` method + result types
- Test: `crates/rowforge-studio-core/tests/foundation.rs`

- [ ] **Step 1: Add result types**

In `lib.rs` (near the other API result types):

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ExecDeleteBulkResult {
    pub deleted: Vec<String>,
    pub failed: Vec<ExecDeleteFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ExecDeleteFailure {
    pub exec_id: String,
    pub reason: String,
}
```

- [ ] **Step 2: Implement bulk**

```rust
impl StudioCore {
    pub fn execution_delete_bulk(&self, exec_ids: &[String]) -> ExecDeleteBulkResult {
        let mut deleted = Vec::new();
        let mut failed = Vec::new();
        for id in exec_ids {
            match self.execution_delete(id) {
                Ok(()) => deleted.push(id.clone()),
                Err(e) => failed.push(ExecDeleteFailure {
                    exec_id: id.clone(),
                    reason: format!("{}", e),
                }),
            }
        }
        ExecDeleteBulkResult { deleted, failed }
    }
}
```

Serial. Never aborts. Returns a complete report.

- [ ] **Step 3: Tests**

```rust
#[test]
fn execution_delete_bulk_all_succeed() {
    let tmp = TempDir::new().unwrap();
    let core = open_test_workspace(tmp.path());
    let id_a = seed_exec_with_attempt(&core);
    let id_b = seed_exec_with_attempt(&core);
    let result = core.execution_delete_bulk(&[id_a.clone(), id_b.clone()]);
    assert_eq!(result.deleted.len(), 2);
    assert!(result.failed.is_empty());
}

#[test]
fn execution_delete_bulk_partial_failure() {
    let tmp = TempDir::new().unwrap();
    let core = open_test_workspace(tmp.path());
    let id_a = seed_exec_with_attempt(&core);
    let id_b = seed_exec_with_attempt(&core);
    seed_active_run_in_registry(&core, &id_a);  // a is busy
    let result = core.execution_delete_bulk(&[id_a.clone(), id_b.clone()]);
    assert_eq!(result.deleted, vec![id_b]);
    assert_eq!(result.failed.len(), 1);
    assert_eq!(result.failed[0].exec_id, id_a);
    assert!(result.failed[0].reason.contains("active run"));
}

#[test]
fn execution_delete_bulk_empty_input_returns_empty_result() {
    let tmp = TempDir::new().unwrap();
    let core = open_test_workspace(tmp.path());
    let result = core.execution_delete_bulk(&[]);
    assert!(result.deleted.is_empty());
    assert!(result.failed.is_empty());
}
```

- [ ] **Step 4: Verify + commit**

```bash
cargo test -p rowforge-studio-core
```

```bash
git add crates/rowforge-studio-core/src/lib.rs crates/rowforge-studio-core/tests/foundation.rs
git commit -m "studio-core: execution_delete_bulk

Serial wrapper over execution_delete that accumulates per-item
outcomes:
- ExecDeleteBulkResult { deleted: Vec<String>, failed: Vec<ExecDeleteFailure> }
- ExecDeleteFailure { exec_id, reason }

Never aborts on item failure; UI renders partial outcomes.

+3 integration tests: all-success, partial-fail (one with active
run), empty input."
```

---

## Task 3: `ExecSummary.size_bytes` + walkdir helper

**Files:**
- Modify: `crates/rowforge-studio-core/src/exec.rs` (or wherever ExecSummary is defined; verify)
- Modify: `crates/rowforge-studio-core/src/lib.rs` if exec_list lives there
- Modify: `crates/rowforge-studio-core/Cargo.toml` — add `walkdir`
- Modify: workspace `Cargo.toml` — `walkdir = "2"` in `[workspace.dependencies]`
- Test: foundation.rs

- [ ] **Step 1: Add walkdir dep**

Workspace `Cargo.toml`:
```toml
walkdir = "2"
```

`crates/rowforge-studio-core/Cargo.toml`:
```toml
walkdir.workspace = true
```

- [ ] **Step 2: Find ExecSummary**

```bash
grep -rnE 'pub struct ExecSummary' crates/rowforge-studio-core/src/
```

Add field:
```rust
pub struct ExecSummary {
    // ...existing...
    pub size_bytes: Option<u64>,
}
```

Update every constructor (search `ExecSummary {` in src/ and tests/) to set `size_bytes: None` by default. The real value is populated in `exec_list`.

- [ ] **Step 3: walkdir helper**

In a sensible place (probably the same module as `exec_list` or a small `util` module):

```rust
fn dir_size_bytes(dir: &Path) -> Option<u64> {
    if !dir.exists() { return None; }
    let mut total: u64 = 0;
    for entry in walkdir::WalkDir::new(dir).follow_links(false).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            if let Ok(meta) = entry.metadata() {
                total = total.saturating_add(meta.len());
            }
        }
    }
    Some(total)
}
```

- [ ] **Step 4: Wire into exec_list**

Find the `exec_list` function. For each summary it builds, compute `size_bytes` by passing the exec_id and workspace root:

```rust
let dir = workspace_root.join("executions").join(&summary.id);
summary.size_bytes = dir_size_bytes(&dir);
```

- [ ] **Step 5: Tests**

```rust
#[test]
fn exec_list_includes_size_bytes() {
    let tmp = TempDir::new().unwrap();
    let core = open_test_workspace(tmp.path());
    let exec_id = seed_exec_with_attempt(&core);
    // Write a known-size file inside the exec dir to make the assertion stable.
    let exec_dir = tmp.path().join("executions").join(&exec_id);
    std::fs::write(exec_dir.join("dummy.bin"), vec![0u8; 1024]).unwrap();

    let list = core.exec_list().unwrap();
    let entry = list.iter().find(|e| e.id == exec_id).unwrap();
    assert!(entry.size_bytes.is_some());
    assert!(entry.size_bytes.unwrap() >= 1024);
}

#[test]
fn exec_list_size_bytes_none_when_dir_missing() {
    let tmp = TempDir::new().unwrap();
    let core = open_test_workspace(tmp.path());
    let exec_id = seed_exec_with_attempt(&core);
    let exec_dir = tmp.path().join("executions").join(&exec_id);
    std::fs::remove_dir_all(&exec_dir).unwrap();  // externally rm

    let list = core.exec_list().unwrap();
    let entry = list.iter().find(|e| e.id == exec_id).unwrap();
    assert!(entry.size_bytes.is_none());
}
```

- [ ] **Step 6: Verify + commit**

```bash
cargo test -p rowforge-studio-core
```

```bash
git add Cargo.toml Cargo.lock crates/rowforge-studio-core/Cargo.toml crates/rowforge-studio-core/src/ crates/rowforge-studio-core/tests/foundation.rs
git commit -m "studio-core: ExecSummary.size_bytes (lazy walkdir per list query)

ExecSummary gains size_bytes: Option<u64>. exec_list now walks
each exec dir and sums file sizes via walkdir (follow_links=false,
filter_map to skip permission errors).

Returns None for missing dirs (e.g. externally rm'd). Saturating
add prevents overflow on pathological sizes.

Perf: O(n_files); ~200ms for 50 execs x 30 files. Document the
trade-off in design doc §3.6.

+2 integration tests."
```

---

## Task 4: Tauri commands + ipc_contract

**Files:**
- Modify: `apps/rowforge-studio/src-tauri/src/commands.rs`
- Modify: `apps/rowforge-studio/src-tauri/src/lib.rs` (register)
- Modify: `apps/rowforge-studio/src-tauri/tests/ipc_contract.rs`

- [ ] **Step 1: Two commands**

```rust
use rowforge_studio_core::ExecDeleteBulkResult;

#[tauri::command]
pub fn execution_delete(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    exec_id: String,
) -> Result<(), UiError> {
    let result = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        let core = guard.as_ref()
            .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
        core.execution_delete(&exec_id)
    };
    if result.is_ok() {
        use tauri::Emitter;
        let _ = app.emit("exec_list:refresh", ());
    }
    result
}

#[tauri::command]
pub fn execution_delete_bulk(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    exec_ids: Vec<String>,
) -> Result<ExecDeleteBulkResult, UiError> {
    let result = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        let core = guard.as_ref()
            .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
        core.execution_delete_bulk(&exec_ids)
    };
    if !result.deleted.is_empty() {
        use tauri::Emitter;
        let _ = app.emit("exec_list:refresh", ());
    }
    Ok(result)
}
```

- [ ] **Step 2: Register**

`lib.rs` `generate_handler![]`: add `commands::execution_delete, commands::execution_delete_bulk`.

- [ ] **Step 3: ipc_contract tests**

```rust
#[test]
fn plan10_execution_delete_commands_registered() {
    let _ = crate::commands::execution_delete;
    let _ = crate::commands::execution_delete_bulk;
}

#[test]
fn plan10_exec_delete_bulk_result_json_shape() {
    let result = rowforge_studio_core::ExecDeleteBulkResult {
        deleted: vec!["e_1".into(), "e_2".into()],
        failed: vec![rowforge_studio_core::ExecDeleteFailure {
            exec_id: "e_3".into(),
            reason: "active run".into(),
        }],
    };
    let v = serde_json::to_value(&result).unwrap();
    assert!(v["deleted"].is_array());
    assert!(v["failed"].is_array());
    assert_eq!(v["failed"][0]["exec_id"], "e_3");
}
```

> If `ExecDeleteBulkResult` is `#[non_exhaustive]` and can't be constructed cross-crate, use `serde_json::from_value` round-trip (mirror Plan 8 T7 pattern).

- [ ] **Step 4: Verify + commit**

```bash
cargo build
cargo test -p rowforge-studio --test ipc_contract
```

```bash
git add apps/rowforge-studio/src-tauri/src/ apps/rowforge-studio/src-tauri/tests/ipc_contract.rs
git commit -m "studio-shell: execution_delete + execution_delete_bulk Tauri commands

Both sync commands. delete_bulk returns the per-item ExecDeleteBulkResult
even on partial success (Tauri Result<T,E> only signals catastrophic
errors; partial = data, not error). Both emit 'exec_list:refresh'
event after successful delete so other ExecList views invalidate.

ipc_contract +2."
```

---

## Task 5: TS mirrors + ipc hooks

**Files:**
- Modify: `apps/rowforge-studio/src/ipc/types.ts`
- Modify: `apps/rowforge-studio/src/ipc/client.ts`
- Modify: `apps/rowforge-studio/src/ipc/queries.ts` (or wherever Plan 3 hooks live)

- [ ] **Step 1: Types**

```ts
export interface ExecSummary {
  // ...existing...
  size_bytes: number | null;
}

export interface ExecDeleteFailure {
  exec_id: string;
  reason: string;
}

export interface ExecDeleteBulkResult {
  deleted: string[];
  failed: ExecDeleteFailure[];
}

// UiError union:
export type UiError =
  // ...existing...
  | { kind: "execution_in_use"; message: { exec_id: string } | null };

// uiErrorMessage switch:
case "execution_in_use":
  return `Execution "${e.message?.exec_id ?? "?"}" has an active run. Cancel the run first.`;
```

Find every existing ExecSummary mock in tests; add `size_bytes: null` (or a number).

- [ ] **Step 2: ipc client**

```ts
execution_delete: (args: { execId: string }) => invoke<void>("execution_delete", args),
execution_delete_bulk: (args: { execIds: string[] }) =>
  invoke<ExecDeleteBulkResult>("execution_delete_bulk", args),
```

> camelCase per Plan 9 fix (Tauri auto-converts).

- [ ] **Step 3: Hooks**

In `queries.ts`:

```ts
export const useExecutionDelete = () => {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: { execId: string }) => ipc.execution_delete(args),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["exec_list"] });
    },
  });
};

export const useExecutionDeleteBulk = () => {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: { execIds: string[] }) => ipc.execution_delete_bulk(args),
    onSuccess: (result) => {
      // Even partial success invalidates the list.
      if (result.deleted.length > 0) {
        qc.invalidateQueries({ queryKey: ["exec_list"] });
      }
    },
  });
};
```

- [ ] **Step 4: ui-error.test.ts +1**

```ts
it("renders execution_in_use copy", () => {
  expect(uiErrorMessage({ kind: "execution_in_use", message: { exec_id: "e_test" } })).toContain("active run");
});
```

- [ ] **Step 5: Verify + commit**

```bash
cd apps/rowforge-studio
pnpm tsc -b
pnpm test
```

```bash
git add apps/rowforge-studio/src/ipc/
git commit -m "studio-shell: TS mirrors + execution delete hooks

types.ts: ExecSummary.size_bytes; ExecDeleteFailure +
ExecDeleteBulkResult; UiError.execution_in_use + uiErrorMessage arm.

client.ts: ipc.execution_delete + ipc.execution_delete_bulk
(camelCase args per Plan 9 convention).

queries.ts: useExecutionDelete + useExecutionDeleteBulk; both
invalidate exec_list on success (bulk also on partial success).

+1 ui-error vitest."
```

---

## Task 6: `DeleteExecutionsDialog` component

**Files:**
- Create: `apps/rowforge-studio/src/components/DeleteExecutionsDialog.tsx`
- Create: `apps/rowforge-studio/src/components/__tests__/DeleteExecutionsDialog.test.tsx`

- [ ] **Step 1: Component**

```tsx
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import type { ExecSummary } from "@/ipc/types";

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  selected: ExecSummary[];
  onConfirm: () => void;
  isPending: boolean;
}

const MAX_LISTED = 10;

export function DeleteExecutionsDialog({ open, onOpenChange, selected, onConfirm, isPending }: Props) {
  const total = selected.reduce((sum, e) => sum + (e.size_bytes ?? 0), 0);
  const listed = selected.slice(0, MAX_LISTED);
  const remaining = selected.length - listed.length;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Delete {selected.length} execution{selected.length === 1 ? "" : "s"}?</DialogTitle>
        </DialogHeader>

        <div className="space-y-3 text-sm">
          <p className="text-muted-foreground">
            This permanently deletes the selected executions and all their
            attempt data (outcomes, handler logs, exports, etc.). Total:{" "}
            <span className="font-mono">{formatBytes(total)}</span>. This cannot be undone.
          </p>

          <ul className="max-h-64 overflow-auto rounded border border-zinc-700 p-2 font-mono text-xs">
            {listed.map((e) => (
              <li key={e.id} className="flex justify-between gap-2 py-0.5">
                <span className="truncate">{e.name ?? "—"}</span>
                <span className="text-muted-foreground shrink-0">{formatBytes(e.size_bytes)}</span>
              </li>
            ))}
            {remaining > 0 && (
              <li className="text-muted-foreground py-0.5">… and {remaining} more</li>
            )}
          </ul>
        </div>

        <div className="mt-4 flex justify-end gap-2">
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={isPending}>
            Cancel
          </Button>
          <Button
            variant="outline"
            onClick={onConfirm}
            disabled={isPending}
            className="bg-red-500/10 text-red-200 border-red-500/40 hover:bg-red-500/20"
          >
            {isPending ? "Deleting…" : `Delete ${selected.length} execution${selected.length === 1 ? "" : "s"}`}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function formatBytes(n: number | null | undefined): string {
  if (n == null) return "—";
  if (n < 1024) return `${n} B`;
  if (n < 1024 ** 2) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 ** 3) return `${(n / 1024 ** 2).toFixed(1)} MB`;
  return `${(n / 1024 ** 3).toFixed(2)} GB`;
}
```

> Extract `formatBytes` to a shared util if used in 3+ places after T7 (ExecList Size column uses the same helper).

- [ ] **Step 2: Tests**

```tsx
describe("DeleteExecutionsDialog", () => {
  const mkExec = (id: string, name: string, bytes: number): ExecSummary => ({
    id, name, created_at: "2026-05-26T00:00:00Z",
    input_rows: 100, attempts_count: 1, size_bytes: bytes,
  });

  it("renders title with selection count", () => {
    const selected = [mkExec("e_1", "alpha", 1024)];
    render(<DeleteExecutionsDialog open={true} onOpenChange={() => {}} selected={selected} onConfirm={() => {}} isPending={false} />);
    expect(screen.getByText(/Delete 1 execution\?/)).toBeInTheDocument();
  });

  it("renders total size", () => {
    const selected = [mkExec("e_1", "alpha", 1024), mkExec("e_2", "beta", 2048)];
    render(<DeleteExecutionsDialog open={true} onOpenChange={() => {}} selected={selected} onConfirm={() => {}} isPending={false} />);
    expect(screen.getByText(/3\.0 KB/)).toBeInTheDocument();
  });

  it("truncates list with '… and N more'", () => {
    const selected = Array.from({ length: 15 }, (_, i) => mkExec(`e_${i}`, `n${i}`, 100));
    render(<DeleteExecutionsDialog open={true} onOpenChange={() => {}} selected={selected} onConfirm={() => {}} isPending={false} />);
    expect(screen.getByText(/… and 5 more/)).toBeInTheDocument();
  });

  it("disables Delete button when pending", () => {
    const selected = [mkExec("e_1", "alpha", 1024)];
    render(<DeleteExecutionsDialog open={true} onOpenChange={() => {}} selected={selected} onConfirm={() => {}} isPending={true} />);
    const btn = screen.getByRole("button", { name: /Deleting…/ });
    expect(btn).toBeDisabled();
  });

  it("calls onConfirm when Delete clicked", () => {
    const onConfirm = vi.fn();
    const selected = [mkExec("e_1", "alpha", 1024)];
    render(<DeleteExecutionsDialog open={true} onOpenChange={() => {}} selected={selected} onConfirm={onConfirm} isPending={false} />);
    fireEvent.click(screen.getByRole("button", { name: /^Delete 1 execution$/ }));
    expect(onConfirm).toHaveBeenCalled();
  });
});
```

- [ ] **Step 3: Verify + commit**

```bash
cd apps/rowforge-studio
pnpm test
```

```bash
git add apps/rowforge-studio/src/components/DeleteExecutionsDialog.tsx apps/rowforge-studio/src/components/__tests__/DeleteExecutionsDialog.test.tsx
git commit -m "studio-shell: DeleteExecutionsDialog

Lists up to 10 selected execs by name + size; '… and N more' for
overflow. Shows total size of selection. Destructive Delete button
(red Tailwind on outline variant since shadcn Button has no
destructive variant in this repo). Pending state disables both
buttons + shows 'Deleting…' label.

+5 vitest covering title / total / overflow / pending / onConfirm."
```

---

## Task 7: ExecList Select mode + Size column + hover tooltip

**Files:**
- Modify: `apps/rowforge-studio/src/pages/ExecList.tsx`
- Modify: existing ExecList test (if any) or create

- [ ] **Step 1: State**

```tsx
const [selectMode, setSelectMode] = useState(false);
const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
const [confirmOpen, setConfirmOpen] = useState(false);
const [bulkAlert, setBulkAlert] = useState<ExecDeleteFailure[] | null>(null);

const bulkMutation = useExecutionDeleteBulk();
const activeRuns = useActiveRuns();  // existing query from Plan 4
const activeExecIds = useMemo(
  () => new Set(activeRuns.data?.map(r => r.exec_id) ?? []),
  [activeRuns.data]
);
```

- [ ] **Step 2: Header buttons**

Top of the page, near "New execution":

```tsx
<div className="flex gap-2">
  {!selectMode ? (
    <>
      <Button onClick={() => setSelectMode(true)} variant="outline" size="sm">
        Select
      </Button>
      <Button onClick={() => navigate("/new")} size="sm">New execution</Button>
    </>
  ) : (
    <>
      <Button
        variant="outline"
        size="sm"
        disabled={selectedIds.size === 0 || bulkMutation.isPending}
        onClick={() => setConfirmOpen(true)}
        className="bg-red-500/10 text-red-200 border-red-500/40 hover:bg-red-500/20"
      >
        Delete {selectedIds.size} execution{selectedIds.size === 1 ? "" : "s"}
      </Button>
      <Button
        variant="ghost"
        size="sm"
        onClick={() => {
          setSelectMode(false);
          setSelectedIds(new Set());
          setBulkAlert(null);
        }}
      >
        Cancel
      </Button>
    </>
  )}
</div>
```

- [ ] **Step 3: Table columns reorder + Size column + hover**

New `<Thead>`:
```tsx
<Tr>
  {selectMode && <Th className="w-8"></Th>}
  <Th>Name</Th>
  <Th className="text-right">Rows</Th>
  <Th className="text-right">Attempts</Th>
  <Th className="text-right">Size</Th>
  <Th>Created</Th>
</Tr>
```

Body row:
```tsx
<Tr
  key={e.id}
  className={selectMode ? "" : "cursor-pointer"}
  onClick={() => {
    if (selectMode) {
      if (activeExecIds.has(e.id)) return;  // disabled rows ignore clicks
      setSelectedIds(prev => {
        const next = new Set(prev);
        if (next.has(e.id)) next.delete(e.id);
        else next.add(e.id);
        return next;
      });
    } else {
      navigate(`/exec/${e.id}`);
    }
  }}
>
  {selectMode && (
    <Td>
      <input
        type="checkbox"
        checked={selectedIds.has(e.id)}
        disabled={activeExecIds.has(e.id)}
        title={activeExecIds.has(e.id) ? "Cancel active run first" : undefined}
        onChange={() => {}}  // change handled by Tr onClick to keep row-wide click area
        onClick={(ev) => ev.stopPropagation()}  // prevent double-toggle from row click
      />
    </Td>
  )}
  <Td className="font-mono" title={e.id}>{e.name || "—"}</Td>
  <Td className="text-right">{e.input_rows ?? "—"}</Td>
  <Td className="text-right">{e.attempts_count}</Td>
  <Td className="text-right">{formatBytes(e.size_bytes)}</Td>
  <Td className="font-mono">
    {new Date(e.created_at).toISOString().replace("T", " ").slice(0, 16)}
  </Td>
</Tr>
```

> Move `formatBytes` to a shared util (e.g. `apps/rowforge-studio/src/lib/format.ts`) since T6 already duplicated it.

- [ ] **Step 4: Bulk-fail alert**

Above the table, conditional:

```tsx
{bulkAlert && bulkAlert.length > 0 && (
  <div className="rounded border border-yellow-500/30 bg-yellow-500/10 p-3 text-sm text-yellow-200 flex items-start gap-3">
    <div className="flex-1">
      <div className="font-medium mb-1">
        ⚠ {bulkAlert.length} deletion{bulkAlert.length === 1 ? "" : "s"} failed:
      </div>
      <ul className="text-xs space-y-0.5">
        {bulkAlert.map((f) => (
          <li key={f.exec_id} className="font-mono">
            • {f.exec_id.slice(0, 12)}…: {f.reason}
          </li>
        ))}
      </ul>
    </div>
    <Button variant="ghost" size="sm" onClick={() => setBulkAlert(null)}>
      Dismiss
    </Button>
  </div>
)}
```

- [ ] **Step 5: Wire dialog + mutation**

```tsx
const selectedExecs = useMemo(
  () => (list.data ?? []).filter(e => selectedIds.has(e.id)),
  [list.data, selectedIds]
);

const handleConfirm = () => {
  bulkMutation.mutate(
    { execIds: Array.from(selectedIds) },
    {
      onSuccess: (result) => {
        setConfirmOpen(false);
        setSelectMode(false);
        setSelectedIds(new Set());
        if (result.failed.length === 0) {
          toast.success(`${result.deleted.length} execution${result.deleted.length === 1 ? "" : "s"} deleted`);
        } else {
          toast.warning(`${result.deleted.length} deleted, ${result.failed.length} failed`);
          setBulkAlert(result.failed);
        }
      },
      onError: (err) => {
        toast.error(uiErrorMessage(err));
      },
    }
  );
};

// at the bottom of the JSX tree:
<DeleteExecutionsDialog
  open={confirmOpen}
  onOpenChange={setConfirmOpen}
  selected={selectedExecs}
  onConfirm={handleConfirm}
  isPending={bulkMutation.isPending}
/>
```

- [ ] **Step 6: ExecDetail 404 fallback (bonus)**

Check `apps/rowforge-studio/src/pages/ExecDetail.tsx`. If it doesn't already handle "this exec doesn't exist anymore" gracefully, add:

```tsx
if (detail.isError) {
  return (
    <div className="p-6 space-y-3">
      <div className="text-red-300">This execution has been deleted or is unavailable.</div>
      <Link to="/" className="text-blue-400 hover:underline">← Back to executions</Link>
    </div>
  );
}
```

(Plan 7 HandlerDetailPage has the same pattern — mirror.)

- [ ] **Step 7: Tests**

Create `apps/rowforge-studio/src/pages/__tests__/exec-list-select.test.tsx`:

```tsx
describe("ExecList Select mode", () => {
  it("Select toggle reveals checkboxes", () => {
    // mock invoke for exec_list returning 2 execs
    // render, click Select, expect 2 checkboxes
  });

  it("active-run row checkbox is disabled with tooltip", () => {
    // mock active_runs to include exec_id of one row
    // assert that row's checkbox disabled + title text
  });

  it("Delete N button disabled when nothing selected", () => {
    // render in select mode with 0 selected; expect button disabled
  });

  it("clicking Delete N opens dialog with correct items", () => {
    // select 2; click Delete N; expect dialog title 'Delete 2 executions?'
  });

  it("Size column renders formatted bytes", () => {
    // mock exec_list with size_bytes: 5_242_880; expect '5.0 MB' in DOM
  });

  it("Name has hover tooltip with full id", () => {
    // expect the <td> with name to have title={exec_id}
  });
});
```

- [ ] **Step 8: Verify + commit**

```bash
cd apps/rowforge-studio
pnpm tsc -b
pnpm test
pnpm build
```

```bash
git add apps/rowforge-studio/src/pages/ExecList.tsx apps/rowforge-studio/src/pages/__tests__/exec-list-select.test.tsx apps/rowforge-studio/src/lib/format.ts
git commit -m "studio-shell: ExecList Select mode + Size column + hover tooltip

Select toggle in header exposes checkbox column; active-run rows
have disabled checkbox with 'Cancel active run first' tooltip.
Delete N button (red destructive styling) opens
DeleteExecutionsDialog with selection. Partial-fail alert renders
above the table; Dismiss clears it.

Column order: [checkbox] | Name | Rows | Attempts | Size | Created.
Name has title={exec_id} for hover-to-see-id. Size formatted via
shared formatBytes helper (extracted to lib/format.ts).

ExecDetail 404 fallback added in same commit since it's a small
mirror of Plan 7 HandlerDetailPage's not-found state.

+N vitest covering Select toggle, disabled-checkbox, button state,
dialog open, Size cell, hover tooltip."
```

---

## Task 8: CLI `rowforge exec delete`

**Files:**
- Modify: `crates/rowforge-cli/src/main.rs` (clap)
- Create: `crates/rowforge-cli/src/exec_delete_cmd.rs`
- Test: `crates/rowforge-cli/tests/exec_delete_cmd.rs`

- [ ] **Step 1: Clap**

Find the `Exec` subcommand variant (Plan 5 added `exec start` / `exec run` / `exec export`). Add `Delete`:

```rust
enum ExecCmd {
    // ...existing...
    /// Delete an execution and its attempt data.
    Delete {
        /// Execution id (mutually exclusive with --all-completed).
        exec_id: Option<String>,
        /// Delete every execution that has no active run.
        #[arg(long, conflicts_with = "exec_id")]
        all_completed: bool,
    },
}
```

- [ ] **Step 2: Subcommand module**

`crates/rowforge-cli/src/exec_delete_cmd.rs`:

```rust
use std::path::Path;
use anyhow::Context;

pub fn run(workspace: &Path, exec_id: Option<String>, all_completed: bool) -> anyhow::Result<()> {
    // Build a StudioCore on this workspace (or use rowforge-core directly if
    // CLI typically doesn't go through studio-core — check Plan 5/8 CLI cmds
    // for the convention).
    // ...

    let targets: Vec<String> = if all_completed {
        // List all execs via core.exec_list(); for each, check active-run
        // membership (if SessionRegistry is accessible here, use it; otherwise
        // attempt delete and skip on ExecutionInUse).
        let list = core.exec_list()?;
        list.into_iter().map(|s| s.id).collect()
    } else {
        vec![exec_id.context("exec_id or --all-completed required")?]
    };

    let result = core.execution_delete_bulk(&targets);
    for id in &result.deleted {
        eprintln!("[{}] deleted", id);
    }
    for f in &result.failed {
        eprintln!("[{}] skipped: {}", f.exec_id, f.reason);
    }
    let code = result.failed.len().min(125) as i32;
    std::process::exit(code);
}
```

> CLI may not have a StudioCore — Plan 8 T4 used `rowforge_core::manifest::Manifest::load_from_dir` directly. For delete the cascade logic lives in studio-core. EITHER:
> 1. CLI imports studio-core and uses StudioCore::execution_delete, OR
> 2. CLI duplicates the logic against rowforge-core::storage primitives.
>
> Option 1 is cleaner. Verify whether rowforge-cli already depends on rowforge-studio-core (Plan 1-9 may not have wired this). If not, add it.

- [ ] **Step 3: Tests**

```rust
#[test]
fn exec_delete_single_succeeds() {
    let tmp = workspace_with_exec();
    let exe = env!("CARGO_BIN_EXE_rowforge");
    let out = Command::new(exe)
        .env("ROWFORGE_HOME", tmp.path())
        .args(["exec", "delete", "e_test_id"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("deleted"));
}

#[test]
fn exec_delete_missing_id_or_flag_fails() {
    let tmp = TempDir::new().unwrap();
    let exe = env!("CARGO_BIN_EXE_rowforge");
    let out = Command::new(exe)
        .env("ROWFORGE_HOME", tmp.path())
        .args(["exec", "delete"])
        .output()
        .unwrap();
    assert!(!out.status.success());
}

#[test]
fn exec_delete_all_completed_skips_active() {
    // Seed 2 execs; mark 1 active (somehow — may need SQL fixture);
    // run delete --all-completed; assert active was skipped.
    // If seed-active is too hard from CLI alone, skip this test and
    // document.
    todo!()
}
```

- [ ] **Step 4: Verify + commit**

---

## Task 9: Spec docs + HUMAN_SMOKE Plan 10

Per design §10. 5 files modified + HUMAN_SMOKE. Roughly:

- **part-2-model.md**: `ExecSummary.size_bytes`, `ExecDeleteBulkResult` + `ExecDeleteFailure` shapes
- **part-3-runtime.md**: exec lifecycle — deletion section (cascade order + rm policy + idempotent semantics + dir-missing tolerance)
- **part-5-api.md**: `execution_delete` + `execution_delete_bulk` commands; `UiError::ExecutionInUse`; `exec_list:refresh` event
- **part-7-ui.md**: ExecList Select mode flow + new column order; ExecDetail not-found state
- Mirror in zh-Hant for all 4 files

HUMAN_SMOKE Plan 10 (~20 steps): Setup, Select toggle, checkbox interactions, active-run disabled tooltip, Delete dialog opens, bulk happy path, bulk partial fail (set up one exec with active run), single delete via Select+1, hover tooltip on Name, Size column display, ExecDetail navigation to deleted exec, CLI single delete, CLI --all-completed, known limitations.

- [ ] **Step 1-N**: dispatch as a single subagent task in the same pattern as Plan 8/9 T10.

---

## Final verification + PR

```bash
cargo build && cargo test
cd apps/rowforge-studio && pnpm tsc -b && pnpm test && pnpm build
```

Expected counts:
- Cargo: 369 → ~383 (+14: 9 studio-core + 3 CLI + 2 ipc_contract)
- Vitest: 139 → ~147 (+8: 5 dialog + 6 ExecList Select... budget tight)

PR:
```bash
git push -u origin studio-plan-10-exec-delete
gh pr create --title "studio Plan 10: exec delete (single+bulk) + Size column" --body-file - <<'EOF'
## Summary

ExecList page gains: Select mode toggle for bulk delete (hard rm
of sqlite rows + on-disk attempt dirs), Size column showing disk
usage, hover-tooltip for full exec_id on Name. CLI gets rowforge
exec delete. Active-run gate refuses outright.

## Test plan

- [x] cargo build + test
- [x] pnpm tsc/test/build
- [ ] Manual smoke per HUMAN_SMOKE Plan 10 (~20 steps)
EOF
```

---

## Order dependency

T1 → T2 (bulk wraps single) → T3 (size, independent) → T4 (Tauri, needs T1+T2) → T5 (TS, needs T4) → T6 (dialog, needs T5 types) → T7 (ExecList, needs T6) → T8 (CLI, needs T1+T2) → T9 (docs).

T3 / T8 / T9 parallelizable. Single-implementer execution: numbered order.
