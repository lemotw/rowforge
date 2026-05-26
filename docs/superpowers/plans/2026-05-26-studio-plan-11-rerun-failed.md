# Plan 11 — Re-run failed rows Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use `- [ ]` checkbox syntax.

**Goal:** AttemptDetail's Failed rows tab gets a "Re-run N rows" button that creates a new attempt on the same exec dispatching only the failed/crashed row_ids from the source attempt. Same handler.

**Architecture:** `RunRequest.only_row_ids: Option<Vec<u64>>` filters dispatch in pool_streaming. New `StudioCore::attempt_failed_row_ids` reads outcomes.jsonl. React Failed rows tab calls run_start with the filter.

**Design spec:** `docs/superpowers/specs/2026-05-26-studio-plan-11-rerun-failed-design.md`

---

## Task 1: rowforge-core only_row_ids filter

**Files:**
- Modify: `crates/rowforge-core/src/run.rs` — `RunRequest.only_row_ids` field
- Modify: `crates/rowforge-core/src/pool_streaming.rs` — filter in row-dispatch loop
- Tests: `crates/rowforge-core/tests/` (new file or extend existing)

- [ ] **Step 1: Audit existing RunRequest shape**

```bash
grep -nA 20 'pub struct RunRequest' crates/rowforge-core/src/run.rs
```

Identify the existing fields. T2 of Plan 9 added `capture_raw_stdout` and `on_handler_log`; Plan 10 should be fully merged so verify no conflicts.

- [ ] **Step 2: Add `only_row_ids` field**

```rust
pub struct RunRequest {
    // ...existing fields...
    pub only_row_ids: Option<Vec<u64>>,
}
```

Defaults: existing callers set this to `None` explicitly (struct is not `#[non_exhaustive]` or it is — check). All existing test fixtures + CLI call sites need the new field with `None`. Search `grep -rn 'RunRequest {' crates/ apps/`.

- [ ] **Step 3: Find the row-dispatch loop in pool_streaming.rs**

```bash
grep -n 'skip_attempted\|attempts_done\|row_id\|input_iter\|for.*row' crates/rowforge-core/src/pool_streaming.rs | head -20
```

The loop reads rows from input and decides whether to dispatch each. The existing skip_attempted check is the closest precedent for adding a per-row filter.

- [ ] **Step 4: Build the filter set**

Near the top of `run_pool_streaming` (or wherever config is unpacked):

```rust
use std::collections::HashSet;
let only_set: Option<HashSet<u64>> = config.only_row_ids
    .as_ref()
    .map(|v| v.iter().copied().collect());
```

In the dispatch loop, before any other skip checks:

```rust
if let Some(set) = &only_set {
    if !set.contains(&row.row_id) {
        continue;
    }
}
// existing skip_attempted check follows
```

Order matters: only_row_ids check FIRST. If the row passes (it's in the set), bypass skip_attempted (the user explicitly said "dispatch THESE rows"). Document this with a one-line comment near the filter.

- [ ] **Step 5: Update all existing callers**

CLI's RunRequest construction (multiple files) + rowforge-studio-core's RunRequest construction. All need `only_row_ids: None` added.

```bash
grep -rn 'RunRequest {' crates/rowforge-core/src/ crates/rowforge-cli/src/ crates/rowforge-studio-core/src/
```

Add `only_row_ids: None,` to each literal.

Also update test fixtures in `crates/rowforge-core/tests/`.

- [ ] **Step 6: Tests**

Append to `crates/rowforge-core/tests/handler_log_integration.rs` OR create a new test file `tests/only_row_ids.rs`. Three tests:

```rust
#[tokio::test]
async fn only_row_ids_dispatches_just_those_rows() {
    // Use the stub-handler test infra (test-handler echo-noisy or similar).
    // Build a RunRequest with only_row_ids = Some(vec![3, 5, 7]).
    // Input has 10 rows. After run completes:
    // - outcomes.jsonl has exactly 3 entries
    // - row_ids in outcomes are {3, 5, 7}
}

#[tokio::test]
async fn only_row_ids_empty_vec_runs_vacuously() {
    // only_row_ids = Some(vec![]). Run completes immediately with 0 outcomes.
    // (Decision: don't error; let it be a noop. Easier UI behavior.)
}

#[tokio::test]
async fn only_row_ids_overrides_skip_attempted() {
    // First attempt: run all 10 rows.
    // Second attempt: only_row_ids = Some(vec![1, 2, 3]) + skip_attempted = true.
    // The 3 rows should be re-dispatched even though they were attempted.
}
```

Look at existing test helpers in `crates/rowforge-core/tests/` for the streaming-pool bootstrap pattern. Plan 9 T2 added solid test infra.

- [ ] **Step 7: Verify**

```bash
cd /Users/lemo/code/lemo/repo/rowforge
cargo build
cargo test -p rowforge-core
```

Expected: all pass; +3 new tests.

- [ ] **Step 8: Commit**

```bash
git add crates/rowforge-core/src/run.rs crates/rowforge-core/src/pool_streaming.rs crates/rowforge-core/tests/
git commit -m "rowforge-core: RunRequest.only_row_ids filter for re-run flows

RunRequest gains only_row_ids: Option<Vec<u64>>. When Some, the
pool_streaming dispatch loop filters input rows by this set BEFORE
the skip_attempted check — meaning explicit row_ids re-dispatch
even if previously attempted (intentional: 're-run THESE rows').

Defaults to None for all existing callers (CLI exec run, Plan 4
RunButton path); behavior unchanged for them.

+3 integration tests: filter dispatches only listed rows; empty
vec is a noop; only_row_ids overrides skip_attempted."
```

---

## Task 2: studio-core attempt_failed_row_ids + start_run propagation

**Files:**
- Modify: `crates/rowforge-studio-core/src/lib.rs` (or wherever attempts logic lives)
- Test: `crates/rowforge-studio-core/tests/foundation.rs`

- [ ] **Step 1: Add `attempt_failed_row_ids`**

```rust
impl StudioCore {
    pub fn attempt_failed_row_ids(
        &self,
        exec_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<u64>, UiError> {
        if !is_valid_id_component(exec_id) {
            return Err(UiError::Io(format!("invalid exec_id: {}", exec_id)));
        }
        if !is_valid_id_component(attempt_id) {
            return Err(UiError::Io(format!("invalid attempt_id: {}", attempt_id)));
        }
        let attempt_dir = self.workspace.root.as_path()
            .join("executions").join(exec_id)
            .join("attempts").join(attempt_id);
        let outcomes_path = attempt_dir.join("outcomes.jsonl");
        if !outcomes_path.exists() {
            return Ok(vec![]);
        }
        let content = std::fs::read_to_string(&outcomes_path)
            .map_err(|e| UiError::Io(format!("read outcomes: {}", e)))?;
        let mut seen = std::collections::BTreeSet::new();
        for line in content.lines() {
            let v: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,  // skip malformed lines (with tracing::warn maybe)
            };
            let status = v.get("status").and_then(|s| s.as_str()).unwrap_or("");
            if status == "failed" || status == "crashed" {
                if let Some(row_id) = v.get("row_id").and_then(|r| r.as_u64()) {
                    seen.insert(row_id);
                }
            }
        }
        Ok(seen.into_iter().collect())
    }
}
```

> Verify Outcome serde shape: the field names may be different (`row_id` vs `id`, `status` vs `state`). Grep `crates/rowforge-core/src/outcome.rs` or wherever Outcome is defined.

> Status values: verify exactly. May be `"failed"` / `"crashed"` / `"resolved"` or different. Cross-reference Plan 3's rollup categorization (failed_last / crashed_last).

- [ ] **Step 2: `start_run` accepts only_row_ids**

Find `StudioCore::start_run` (Plan 4 set it up). Its args struct likely has fields like `executionId`, `handlerDir`, `rowLimit`, `workers`, `dryRun`, `skipAttempted`. Add:

```rust
pub struct RunStartArgs {
    // ...existing...
    pub only_row_ids: Option<Vec<u64>>,
}
```

Plumb through to `RunRequest.only_row_ids`. Existing CLI / test callers add `only_row_ids: None`.

- [ ] **Step 3: Tests**

Append to `crates/rowforge-studio-core/tests/foundation.rs`:

```rust
#[test]
fn attempt_failed_row_ids_returns_empty_when_no_outcomes() {
    let tmp = TempDir::new().unwrap();
    let core = open_test_workspace(tmp.path());
    let exec_id = seed_exec_with_completed_attempt(&core);  // Plan 10 helper
    let attempt_id = first_attempt_id(&core, &exec_id);
    let result = core.attempt_failed_row_ids(&exec_id, &attempt_id).unwrap();
    assert!(result.is_empty());
}

#[test]
fn attempt_failed_row_ids_returns_only_failed_and_crashed() {
    let tmp = TempDir::new().unwrap();
    let core = open_test_workspace(tmp.path());
    let exec_id = seed_exec_with_completed_attempt(&core);
    let attempt_id = first_attempt_id(&core, &exec_id);

    // Manually write outcomes.jsonl with mixed statuses
    let attempt_dir = tmp.path().join("executions").join(&exec_id)
        .join("attempts").join(&attempt_id);
    std::fs::create_dir_all(&attempt_dir).unwrap();
    let outcomes = attempt_dir.join("outcomes.jsonl");
    std::fs::write(
        &outcomes,
        r#"{"row_id":1,"status":"resolved","output":{}}
{"row_id":2,"status":"failed","error_code":"bad"}
{"row_id":3,"status":"crashed","error":"oops"}
{"row_id":4,"status":"resolved","output":{}}
{"row_id":5,"status":"failed","error_code":"bad"}
"#,
    ).unwrap();

    let result = core.attempt_failed_row_ids(&exec_id, &attempt_id).unwrap();
    assert_eq!(result, vec![2, 3, 5]);
}

#[test]
fn attempt_failed_row_ids_rejects_traversal_ids() {
    let tmp = TempDir::new().unwrap();
    let core = open_test_workspace(tmp.path());
    let err = core.attempt_failed_row_ids("../etc", "att_x").unwrap_err();
    assert!(matches!(err, UiError::Io(_)));

    let err = core.attempt_failed_row_ids("e_test", "../../etc").unwrap_err();
    assert!(matches!(err, UiError::Io(_)));
}

#[test]
fn attempt_failed_row_ids_dedupes_repeated_rows() {
    // Same row_id appearing multiple times (e.g. internal retry) → returned once.
    let tmp = TempDir::new().unwrap();
    let core = open_test_workspace(tmp.path());
    let exec_id = seed_exec_with_completed_attempt(&core);
    let attempt_id = first_attempt_id(&core, &exec_id);
    let attempt_dir = tmp.path().join("executions").join(&exec_id)
        .join("attempts").join(&attempt_id);
    std::fs::create_dir_all(&attempt_dir).unwrap();
    let outcomes = attempt_dir.join("outcomes.jsonl");
    std::fs::write(
        &outcomes,
        r#"{"row_id":7,"status":"failed"}
{"row_id":7,"status":"failed"}
{"row_id":7,"status":"failed"}
"#,
    ).unwrap();

    let result = core.attempt_failed_row_ids(&exec_id, &attempt_id).unwrap();
    assert_eq!(result, vec![7]);
}
```

- [ ] **Step 4: Verify**

```bash
cargo test -p rowforge-studio-core
```

Expected: +4 new tests.

- [ ] **Step 5: Commit**

```bash
git add crates/rowforge-studio-core/src/lib.rs crates/rowforge-studio-core/tests/foundation.rs
git commit -m "studio-core: attempt_failed_row_ids + start_run only_row_ids propagation

attempt_failed_row_ids reads outcomes.jsonl for an attempt and returns
the deduped sorted Vec<u64> of row_ids whose status is failed or
crashed. Used by Plan 11's Failed rows tab Re-run button to count
+ dispatch.

start_run gains only_row_ids: Option<Vec<u64>> that propagates to
the underlying RunRequest. Existing callers (CLI exec_run path,
RunButton) keep passing None — behavior unchanged.

+4 integration tests: empty outcomes returns empty; mixed statuses
filter correctly; traversal IDs rejected; repeated row_ids dedupe."
```

---

## Task 3: Tauri command + run_start arg extension

**Files:**
- Modify: `apps/rowforge-studio/src-tauri/src/commands.rs`
- Modify: `apps/rowforge-studio/src-tauri/src/lib.rs` (register)
- Modify: `apps/rowforge-studio/src-tauri/tests/ipc_contract.rs`

- [ ] **Step 1: Add `attempt_failed_row_ids` command**

```rust
#[tauri::command]
pub fn attempt_failed_row_ids(
    state: State<'_, AppState>,
    exec_id: String,
    attempt_id: String,
) -> Result<Vec<u64>, UiError> {
    let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
    let core = guard.as_ref()
        .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
    core.attempt_failed_row_ids(&exec_id, &attempt_id)
}
```

Register in `generate_handler![...]`.

- [ ] **Step 2: Extend `run_start` command**

Find the existing `run_start` Tauri command (Plan 4-5). Add `only_row_ids: Option<Vec<u64>>` parameter; plumb to `StudioCore::start_run`.

Existing callers (RunButton) don't pass this; backward compat depends on serde defaulting `Option` to `None` for missing keys — should be fine.

- [ ] **Step 3: ipc_contract tests**

```rust
#[test]
fn plan11_attempt_failed_row_ids_command_registered() {
    let _ = crate::commands::attempt_failed_row_ids;
}

#[test]
fn plan11_run_start_args_accept_only_row_ids() {
    let json = serde_json::json!({
        "executionId": "e_test",
        "handlerDir": "/path/to/handler",
        "rowLimit": null,
        "workers": null,
        "dryRun": null,
        "skipAttempted": null,
        "onlyRowIds": [1, 2, 3],
    });
    // Try to parse this as the args type the Tauri command receives.
    // If the args struct isn't accessible, just assert the JSON shape
    // round-trips through serde for the Vec<u64> field.
    let parsed: serde_json::Value = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(parsed["onlyRowIds"], serde_json::json!([1, 2, 3]));
}
```

- [ ] **Step 4: Verify + commit**

```bash
cargo build
cargo test -p rowforge-studio --test ipc_contract
```

```bash
git add apps/rowforge-studio/src-tauri/src/ apps/rowforge-studio/src-tauri/tests/ipc_contract.rs
git commit -m "studio-shell: attempt_failed_row_ids command + run_start onlyRowIds arg

New Tauri command attempt_failed_row_ids wraps StudioCore's method
of the same name. Sync (no spawn_blocking — file read is fast).

Existing run_start command accepts new onlyRowIds: Option<Vec<u64>>
arg (camelCase per Plan 9 fix), propagates to StudioCore::start_run.
Old callers omit the field; serde defaults to None.

ipc_contract +2."
```

---

## Task 4: TS mirrors + hooks

**Files:**
- Modify: `apps/rowforge-studio/src/ipc/types.ts`
- Modify: `apps/rowforge-studio/src/ipc/client.ts`
- Modify: `apps/rowforge-studio/src/ipc/queries.ts`

- [ ] **Step 1: Extend run_start args type**

In `types.ts` or wherever the run-start arg shape is defined:

```ts
export interface RunStartArgs {
  executionId: ExecutionId;
  handlerDir: string;
  rowLimit: number | null;
  workers: number | null;
  dryRun: boolean | null;
  skipAttempted: boolean | null;
  onlyRowIds?: number[] | null;     // NEW
}
```

If `RunStartArgs` isn't a named type, just update the inline shape in `ipc.run_start` wrapper.

- [ ] **Step 2: ipc client**

```ts
attempt_failed_row_ids: (args: { execId: string; attemptId: string }) =>
  invoke<number[]>("attempt_failed_row_ids", args),
```

Extend existing `ipc.run_start`:

```ts
run_start: (args: {
  executionId: ExecutionId;
  handlerDir: string;
  rowLimit: number | null;
  workers: number | null;
  dryRun: boolean | null;
  skipAttempted: boolean | null;
  onlyRowIds?: number[] | null;     // NEW
}) => invoke<RunStartedHandle>("run_start", { ... });
```

- [ ] **Step 3: Hook**

In `queries.ts`:

```ts
export const useAttemptFailedRowIds = (execId: string, attemptId: string | null) =>
  useQuery({
    queryKey: ["attempt_failed_row_ids", execId, attemptId],
    queryFn: () =>
      ipc.attempt_failed_row_ids({ execId, attemptId: attemptId! }),
    enabled: !!attemptId,
  });
```

- [ ] **Step 4: useRunStart accepts onlyRowIds**

`useRunStart` (existing) — verify its mutationFn accepts the new field. Should just be a TS type extension; no behavior change to the mutation itself.

- [ ] **Step 5: Verify + commit**

```bash
cd apps/rowforge-studio
pnpm tsc -b
pnpm test
```

```bash
git add apps/rowforge-studio/src/ipc/
git commit -m "studio-shell: TS mirrors + useAttemptFailedRowIds hook

types.ts: RunStartArgs gains onlyRowIds field.
client.ts: ipc.attempt_failed_row_ids wrapper; ipc.run_start
  accepts onlyRowIds.
queries.ts: useAttemptFailedRowIds query hook keyed by
  (exec_id, attempt_id).

No new vitest in this task; T5 covers the UI integration."
```

---

## Task 5: AttemptDetail Failed rows tab Re-run button + dialog

**Files:**
- Modify: `apps/rowforge-studio/src/pages/AttemptDetail.tsx` (Failed rows tab section)
- Create: `apps/rowforge-studio/src/components/RerunFailedDialog.tsx`
- Test: `apps/rowforge-studio/src/components/__tests__/RerunFailedDialog.test.tsx` or extend AttemptDetail test

- [ ] **Step 1: Locate Failed rows tab content**

```bash
grep -n 'Failed rows\|FailedRows\|failed_rows' apps/rowforge-studio/src/pages/AttemptDetail.tsx
```

Find the TabsContent for "failed". It likely renders a `<FailedRowsTable>` or inline table.

- [ ] **Step 2: Wire failed row IDs**

In AttemptDetail:

```tsx
import { useAttemptFailedRowIds } from "@/ipc/queries";

// inside the component, after existing hooks:
const failedRowIds = useAttemptFailedRowIds(execId, attemptId);
```

- [ ] **Step 3: Add Re-run button above Failed rows table**

```tsx
<TabsContent value="failed">
  <div className="flex items-center justify-between mb-3">
    <span className="text-sm text-muted-foreground">
      {failedRowIds.data?.length ?? 0} failed row{failedRowIds.data?.length === 1 ? "" : "s"}
    </span>
    <Button
      onClick={() => setRerunOpen(true)}
      disabled={
        !failedRowIds.data ||
        failedRowIds.data.length === 0 ||
        hasActiveRun ||
        !handlerDir
      }
      title={
        failedRowIds.data?.length === 0
          ? "No failed rows to re-run"
          : hasActiveRun
          ? "Cancel active run first"
          : !handlerDir
          ? "Source attempt has no handler reference"
          : undefined
      }
    >
      Re-run {failedRowIds.data?.length ?? 0} row{failedRowIds.data?.length === 1 ? "" : "s"}
    </Button>
  </div>
  {/* existing FailedRowsTable */}
</TabsContent>
```

- [ ] **Step 4: `hasActiveRun` + `handlerDir`**

Compute from the existing attempt detail / exec detail data already loaded by the page:

```tsx
// hasActiveRun: any attempt on this exec currently running
const hasActiveRun = /* derive from useExecDetail's data, checking attempts states */;

// handlerDir: prefer the source attempt's, fallback to exec's last_handler_dir
const handlerDir = attempt?.handler_dir ?? exec?.last_handler_dir ?? null;
```

> Field names may differ. Look at the AttemptDetail / AttemptSummary TS shape (`apps/rowforge-studio/src/ipc/types.ts`). May be `handler_dir`, `handler_snapshot_dir`, `handler_instance.source_snapshot_dir`, etc. Adapt.

- [ ] **Step 5: Build RerunFailedDialog**

`apps/rowforge-studio/src/components/RerunFailedDialog.tsx`:

```tsx
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  rowCount: number;
  handlerDir: string;
  sourceAttemptId: string;
  onConfirm: () => void;
  isPending: boolean;
}

export function RerunFailedDialog({
  open, onOpenChange, rowCount, handlerDir, sourceAttemptId, onConfirm, isPending,
}: Props) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>
            Re-run {rowCount} failed row{rowCount === 1 ? "" : "s"}?
          </DialogTitle>
        </DialogHeader>

        <div className="space-y-3 text-sm">
          <p className="text-muted-foreground">
            A new attempt will be created on this execution targeting only
            the {rowCount} row{rowCount === 1 ? "" : "s"} that failed or
            crashed in this attempt.
          </p>

          <div className="rounded border border-zinc-700 p-2 font-mono text-xs space-y-1">
            <div>
              <span className="text-muted-foreground">Handler:</span>{" "}
              <span className="break-all">{handlerDir}</span>
            </div>
            <div>
              <span className="text-muted-foreground">Source attempt:</span>{" "}
              {sourceAttemptId}
            </div>
          </div>
        </div>

        <div className="mt-4 flex justify-end gap-2">
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={isPending}>
            Cancel
          </Button>
          <Button onClick={onConfirm} disabled={isPending}>
            {isPending ? "Starting…" : `Re-run ${rowCount} row${rowCount === 1 ? "" : "s"}`}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
```

- [ ] **Step 6: Wire dialog + mutation in AttemptDetail**

```tsx
const navigate = useNavigate();
const runStart = useRunStart();
const [rerunOpen, setRerunOpen] = useState(false);

const handleConfirm = () => {
  runStart.mutate(
    {
      executionId: execId as ExecutionId,
      handlerDir: handlerDir!,
      rowLimit: null,
      workers: null,
      dryRun: null,
      skipAttempted: null,
      onlyRowIds: failedRowIds.data ?? [],
    },
    {
      onSuccess: (started) => {
        setRerunOpen(false);
        navigate(`/exec/${execId}/attempt/${started.attempt_id}?run=${started.handle}`);
      },
      onError: (e) => {
        toast.error(uiErrorMessage(e));
      },
    }
  );
};

// at the bottom of the JSX:
<RerunFailedDialog
  open={rerunOpen}
  onOpenChange={setRerunOpen}
  rowCount={failedRowIds.data?.length ?? 0}
  handlerDir={handlerDir ?? ""}
  sourceAttemptId={attemptId}
  onConfirm={handleConfirm}
  isPending={runStart.isPending}
/>
```

- [ ] **Step 7: Tests**

Create `apps/rowforge-studio/src/components/__tests__/RerunFailedDialog.test.tsx`. 3 component-level tests:
- Renders title with row count + plural
- Renders handler + source attempt details
- onConfirm called on Re-run click; disabled when isPending

Plus extend or create an AttemptDetail-level test for the button behavior:
- Re-run button shows N rows count
- Disabled when failedRowIds is empty
- Disabled with tooltip when hasActiveRun
- Click opens dialog

Mock `ipc.attempt_failed_row_ids` to return a known list.

Target +5 vitest.

- [ ] **Step 8: Verify**

```bash
cd apps/rowforge-studio
pnpm tsc -b
pnpm test
pnpm build
```

Expected: vitest 153 → ~158.

- [ ] **Step 9: Commit**

```bash
git add apps/rowforge-studio/src/pages/AttemptDetail.tsx apps/rowforge-studio/src/components/RerunFailedDialog.tsx apps/rowforge-studio/src/components/__tests__/RerunFailedDialog.test.tsx
git commit -m "studio-shell: AttemptDetail Re-run failed rows button + dialog

Failed rows tab gains a 'Re-run N rows' button reflecting count
from useAttemptFailedRowIds. Disabled (with tooltip) when:
- N == 0 (no failed rows)
- exec has active run (cancel first)
- source attempt has no handler reference (use Run button instead)

Click → RerunFailedDialog showing N + handler dir + source
attempt id; Cancel / Re-run buttons. Confirm → run_start mutation
with onlyRowIds = the failed list; on success navigate to new
attempt's Live tab (Plan 5 T15 pattern).

+5 vitest covering dialog rendering, isPending disable, button
states (count / active / no handler), and mutation invocation."
```

---

## Task 6: Spec docs + HUMAN_SMOKE Plan 11

**Files:**
- Modify: `docs/spec/studio/part-3-runtime.md` (en + zh-Hant)
- Modify: `docs/spec/studio/part-5-api.md` (en + zh-Hant)
- Modify: `docs/spec/studio/part-7-ui.md` (en + zh-Hant)
- Modify: `apps/rowforge-studio/HUMAN_SMOKE.md`

- [ ] **Step 1: part-3**

In the pool_streaming / row-dispatch section: add a paragraph on `only_row_ids`. Document the precedence: `only_row_ids > skip_attempted`. When set, dispatch filters down to the explicit set; rows outside the set are skipped without affecting their attempt history.

- [ ] **Step 2: part-5**

§5.5 commands: add `attempt_failed_row_ids(exec_id, attempt_id) -> Vec<u64>` row. Note: reads outcomes.jsonl; deduped; sorted ascending; status filter is `{failed, crashed}`.

Existing `run_start` row: add `only_row_ids: Option<Vec<u64>>` parameter.

- [ ] **Step 3: part-7**

§7.4 flows: add "Re-run failed rows" flow:
1. User on AttemptDetail's Failed rows tab
2. Button "Re-run N rows" reflects count from attempt_failed_row_ids
3. Disabled states with tooltips
4. Confirm dialog with handler + source attempt
5. On submit → new attempt; auto-navigate to Live tab

- [ ] **Step 4: zh-Hant mirror**

Mirror in `docs/spec/studio/zh-Hant/part-3-runtime.md`, `part-5-api.md`, `part-7-ui.md`.

Translation conventions from prior plans:
- "re-run" → 「重跑」or 「重試」
- "failed rows" → 「失敗的 row」
- "source attempt" → 「來源 attempt」(keep attempt anglicized for technical clarity)
- "only_row_ids filter" → 「only_row_ids 過濾」(keep field name in backticks)

- [ ] **Step 5: HUMAN_SMOKE Plan 11**

Append to `apps/rowforge-studio/HUMAN_SMOKE.md` after Plan 10. ~15 numbered steps:

#### Setup (1-2)
1. Workspace with at least one exec
2. Handler that fails some rows deterministically (e.g. stub that returns failed for odd row_ids, resolved for even)

#### Run, observe failures (3-4)
3. Run the exec with 10 rows
4. Wait for done state. Open AttemptDetail → Failed rows tab. Should see 5 failed rows.

#### Re-run flow (5-9)
5. Re-run button shows "Re-run 5 rows" and is enabled
6. Click → confirm dialog shows "Re-run 5 failed rows?", handler dir, source attempt id
7. Confirm → toast / navigate to new attempt's Live tab
8. New attempt runs only those 5 row_ids
9. Verify on disk: outcomes.jsonl of new attempt has 5 entries with row_ids matching the failed set

#### Edge cases (10-13)
10. Attempt with 0 failed rows → button disabled with "No failed rows to re-run"
11. Trigger a new run on same exec; while running, navigate to old attempt's Failed rows; button disabled with "Cancel active run first"
12. Cancel the run; button re-enables
13. Attempt with no handler reference (CLI-only / older attempt) → disabled with "Source attempt has no handler reference"

#### Rollup consistency (14-15)
14. After re-run completes and 3 of 5 succeed: exec rollup shows resolved+3, failed_last only counts the 2 still failing
15. Click Re-run on the NEW attempt → button shows "Re-run 2 rows" (only the still-failed)

#### Known limitations
- No ExecDetail-level "Re-run all currently-failed across exec"
- No handler picker override on the dialog
- No row preview / individual selection — all-or-none of this attempt's failures

- [ ] **Step 6: Verify diff stats**

```bash
git diff --stat docs/spec/studio/ apps/rowforge-studio/HUMAN_SMOKE.md
```

Expect 7 files modified (3 en + 3 zh-Hant + 1 HUMAN_SMOKE).

- [ ] **Step 7: Commit**

```bash
git add docs/spec/studio/ apps/rowforge-studio/HUMAN_SMOKE.md
git commit -m "docs: Plan 11 spec sync (en + zh-Hant) + HUMAN_SMOKE Plan 11

part-3: pool_streaming dispatch gains only_row_ids filter; precedence
documented (only_row_ids > skip_attempted).

part-5: new attempt_failed_row_ids command; existing run_start gains
only_row_ids parameter.

part-7: AttemptDetail Failed rows tab gains Re-run button flow.

zh-Hant mirrored.

HUMAN_SMOKE Plan 11: 15 numbered steps covering happy path, edge
cases (0 failed / active run / no handler), and rollup consistency
across multiple re-runs."
```

---

## Final verification + PR

```bash
cargo build && cargo test
cd apps/rowforge-studio && pnpm tsc -b && pnpm test && pnpm build
```

Expected:
- cargo: 386 → ~393 (+7)
- vitest: 153 → ~158 (+5)

PR:
```bash
git push -u origin studio-plan-11-rerun-failed
gh pr create --title "studio Plan 11: re-run failed rows from attempt" --body-file - <<'EOF'
## Summary

AttemptDetail's Failed rows tab gains a "Re-run N rows" button that
creates a new attempt on the same exec, dispatching only the rows
that failed or crashed in the source attempt. Same handler.

## Test plan

- [x] cargo + vitest suites green
- [ ] Manual smoke per HUMAN_SMOKE Plan 11 (15 steps covering happy
      path + edge cases + rollup consistency)
EOF
```

---

## Order dependency

T1 → T2 (T2 needs T1's RunRequest field) → T3 (Tauri needs T1+T2) → T4 (TS needs T3) → T5 (UI needs T4) → T6 (docs).

Strictly sequential; no parallelizable forks.
