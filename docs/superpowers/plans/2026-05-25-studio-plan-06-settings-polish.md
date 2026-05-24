# Studio Plan 06 — Settings + Workspace + carry-forwards Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close every Plan 5 deferred item that isn't its own plan. After Plan 6, common settings live behind a proper UI, `RunRollupTick` placeholders carry real numbers, and the last-handler-dir hack moves from localStorage to sqlite.

**Architecture:** Pure polish layer. sqlite schema v2 → v3 (one nullable column). Settings page with controlled form + workspace switcher. ProgressSnapshot extended with rate_10s. No rowforge-core API additions other than the migration + `set_last_handler_dir` setter.

**Tech Stack:** Same as Plan 5. No new external deps.

**Spec references:** Design doc at `docs/superpowers/specs/2026-05-25-studio-plan-06-settings-polish-design.md`. Spec part-5 §5.6 (Settings); part-7 §7.3 (Settings IA); part-6 §6.6 (RunRollupTick fields).

---

## Decisions resolved during brainstorm

| Decision | Choice |
|---|---|
| Plan 6 scope | Polish 純收尾 — no rowforge-core API additions, no hard-cancel, no new user workflow |
| Workspace switch with active runs | Block — disabled button + tooltip "Cancel N active runs first" |
| `max_concurrent_runs` reload | Applied at next `workspace_open`; Settings shows dirty banner |
| `last_handler_dir` storage | Per-exec in sqlite (schema v3 migration) |

---

## File structure

### Modified — `rowforge-core`
- `crates/rowforge-core/src/execution_store.rs` — `SCHEMA_VERSION: i64 = 3`; new `MIGRATION_V3` const; `migrate()` adds a `Some(2)` arm; `Execution` struct gains `last_handler_dir: Option<PathBuf>`; new `ExecutionStore::set_last_handler_dir(id, &Path)` method; existing `get_execution` + `list_executions` read the new column.

### Modified — `rowforge-studio-core`
- `src/exec_view.rs` — `ExecSummary` gains `last_handler_dir: Option<PathBuf>`; populated from `Execution.last_handler_dir`.
- `src/run.rs` — `StudioCore::start_run` calls `store.set_last_handler_dir(...)` after successful attempt creation (same store-lock).
- `src/aggregator.rs` — `ProgressSnapshot` gains `rate_10s: f32`; `snapshot()` computes from `rate_10s_buf`.
- `src/session.rs` — `SessionRegistry::rollup_tick()` sums per-session `rate_10s` → `total_rate`; selects `slowest_run` by min positive `rate_10s`.
- `src/lib.rs` — `StudioCore::open` reads `Settings.max_concurrent_runs` and passes to `SessionRegistry::new`.

### Modified — Tauri
- No new commands. `workspace_settings_save` + `workspace_open` already exposed.

### New — React UI
- `apps/rowforge-studio/src/pages/Settings.tsx` — page at `/settings`
- `apps/rowforge-studio/src/components/SettingsForm.tsx` — controlled form
- `apps/rowforge-studio/src/components/WorkspaceSwitchButton.tsx` — directory picker + active-runs gating + save+open chain

### Modified — React UI
- `apps/rowforge-studio/src/App.tsx` — register `/settings` route
- `apps/rowforge-studio/src/components/RunButton.tsx` — drop `LS_HANDLER_DIR` localStorage; seed only from prop. One-time cleanup: `localStorage.removeItem("studio.lastHandlerDir")` on mount (post-upgrade hygiene)
- `apps/rowforge-studio/src/pages/ExecDetail.tsx` — pass `exec.summary.last_handler_dir` as `lastHandlerDir` prop to `RunButton` (was hardcoded `null`)
- `apps/rowforge-studio/src/ipc/types.ts` — `ExecSummary.last_handler_dir: string | null`; `ProgressSnapshot.rate_10s: number`
- `apps/rowforge-studio/HUMAN_SMOKE.md` — Plan 6 walkthrough

### Modified — Spec docs
- `docs/spec/studio/part-2-model.md` — `ExecSummary.last_handler_dir`
- `docs/spec/studio/part-5-api.md` — §5.6 mentions `max_concurrent_runs` applied at `workspace_open`
- `docs/spec/studio/part-6-observability.md` — §6.6 documents `slowest_run` heuristic
- `docs/spec/studio/part-7-ui.md` — Settings page wireframe
- zh-Hant mirrors

### Out of scope (Plan 7+)
- Handler Authoring panel (Part 8)
- Hard cancel actually killing workers
- Export streaming progress + cancel
- Multi-workspace recents
- Settings hot-reload
- `Settings.preferred_editor`

---

## Task 1: sqlite schema v3 migration

**Files:** Modify `crates/rowforge-core/src/execution_store.rs`

- [ ] **Step 1: Write failing migration test**

Append to the existing `#[cfg(test)] mod tests` block:

```rust
#[test]
fn migrates_v2_to_v3_adds_last_handler_dir_column() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("executions.db");
    // Manually create a v2 schema (mimics an existing workspace).
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(MIGRATION_V1).unwrap();
        conn.execute_batch(MIGRATION_V2).unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL);",
        ).unwrap();
        conn.execute(
            "INSERT INTO schema_version (version) VALUES (?1)",
            rusqlite::params![2_i64],
        ).unwrap();
    }
    // Re-open via ExecutionStore — should migrate to v3.
    let store = ExecutionStore::open(tmp.path()).unwrap();
    assert_eq!(store.schema_version(), 3);

    // Verify the column exists by querying it.
    let conn = &store.conn;
    let mut stmt = conn
        .prepare("SELECT last_handler_dir FROM executions WHERE 1=0")
        .unwrap();
    let _ = stmt.query([]);
}
```

(Adjust `store.conn` accessor visibility if needed — test is in the same module so private access is fine.)

- [ ] **Step 2: Add MIGRATION_V3 const + bump SCHEMA_VERSION**

```rust
const SCHEMA_VERSION: i64 = 3;

const MIGRATION_V3: &str = "
ALTER TABLE executions ADD COLUMN last_handler_dir TEXT;
";
```

- [ ] **Step 3: Add Some(2) arm in `migrate()`**

```rust
Some(2) => {
    self.conn.execute_batch(MIGRATION_V3)?;
    self.conn
        .execute("UPDATE schema_version SET version = ?1", params![SCHEMA_VERSION])?;
}
```

And add `self.conn.execute_batch(MIGRATION_V3)?;` to the fresh-install branch (right after `MIGRATION_V2`).

- [ ] **Step 4: Run test**

```bash
cargo test -p rowforge-core migrates_v2_to_v3
```

Expected: pass.

- [ ] **Step 5: Commit**

```
rowforge-core: schema v3 — last_handler_dir column on executions

Plan 6 T1. Adds a NULLABLE TEXT column to track the handler dir
used for the most recent run of each exec. Migration runs cleanly
from both fresh install (no schema_version row) and v2 upgrade
(Some(2) arm). Regression test loads a v2 fixture and verifies the
upgrade.

The column is read/written in T2; this task is just the schema move.
```

---

## Task 2: `Execution.last_handler_dir` + `set_last_handler_dir`

**Files:** Modify `crates/rowforge-core/src/execution_store.rs`

- [ ] **Step 1: Add field to Execution struct**

```rust
pub struct Execution {
    // existing fields…
    pub last_handler_dir: Option<PathBuf>,
}
```

- [ ] **Step 2: Add setter method**

```rust
impl ExecutionStore {
    /// Persist the handler directory most recently used for a run of
    /// this execution. Called from `studio-core` after `start_run`
    /// succeeds. NULL-able: rows that have never been run have None.
    pub fn set_last_handler_dir(
        &mut self,
        id: &str,
        dir: &std::path::Path,
    ) -> Result<()> {
        let s = dir.to_string_lossy().into_owned();
        let n = self.conn.execute(
            "UPDATE executions SET last_handler_dir = ?1 WHERE id = ?2",
            params![s, id],
        )?;
        if n == 0 {
            return Err(CoreError::Store(format!("execution {} not found", id)));
        }
        Ok(())
    }
}
```

- [ ] **Step 3: Update SELECT statements to read the column**

Find every `SELECT` of executions in this file (likely in `get_execution`, `list_executions`, possibly `create_execution`). Add `last_handler_dir` to the column list and the row-mapper:

```rust
// before
"SELECT id, name, …, current_handler_instance_id FROM executions WHERE id = ?1"
// after
"SELECT id, name, …, current_handler_instance_id, last_handler_dir FROM executions WHERE id = ?1"
```

Row mapper:

```rust
let last_handler_dir: Option<String> = row.get("last_handler_dir")?;
let last_handler_dir = last_handler_dir.map(PathBuf::from);
```

Make sure `create_execution` initializes `last_handler_dir: None` in the returned struct.

- [ ] **Step 4: Write round-trip test**

```rust
#[test]
fn set_and_read_last_handler_dir() {
    let tmp = tempfile::tempdir().unwrap();
    // Create input csv for create_execution to snapshot.
    let csv_path = tmp.path().join("in.csv");
    std::fs::write(&csv_path, "row_id\nr1\n").unwrap();
    let mut store = ExecutionStore::open(tmp.path()).unwrap();
    let exec = store.create_execution(NewExecution {
        name: Some("test".into()),
        input_csv_id: "test".into(),
        input_csv_path: csv_path,
        current_handler_instance_id: None,
    }).unwrap();

    // Fresh exec — None.
    let loaded = store.get_execution(&exec.id).unwrap().unwrap();
    assert_eq!(loaded.last_handler_dir, None);

    // Set, then re-read.
    store.set_last_handler_dir(&exec.id, std::path::Path::new("/tmp/hh")).unwrap();
    let loaded = store.get_execution(&exec.id).unwrap().unwrap();
    assert_eq!(loaded.last_handler_dir.as_deref().map(|p| p.to_str().unwrap()),
               Some("/tmp/hh"));
}
```

- [ ] **Step 5: Verify**

```bash
cargo test -p rowforge-core set_and_read_last_handler_dir
cargo test -p rowforge-core  # full crate, no regression
```

- [ ] **Step 6: Commit**

```
rowforge-core: Execution.last_handler_dir + setter

Plan 6 T2. Schema column from T1 is now reachable in Rust:
- Execution struct gains last_handler_dir: Option<PathBuf>
- ExecutionStore::set_last_handler_dir(id, &Path) idempotent setter
- All SELECT executions paths read the new column
- create_execution initializes None

Studio writes this after every successful run_start (T4).
```

---

## Task 3: Propagate `last_handler_dir` to `ExecSummary`

**Files:** Modify `crates/rowforge-studio-core/src/exec_view.rs` (or wherever `ExecSummary` is defined; grep `pub struct ExecSummary`)

- [ ] **Step 1: Find ExecSummary**

```bash
grep -n "pub struct ExecSummary" crates/rowforge-studio-core/src/
```

- [ ] **Step 2: Add field**

```rust
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecSummary {
    // existing fields…
    pub last_handler_dir: Option<std::path::PathBuf>,
}
```

- [ ] **Step 3: Populate from Execution**

Find every site that constructs `ExecSummary` from `Execution`. Add:

```rust
last_handler_dir: exec.last_handler_dir.clone(),
```

- [ ] **Step 4: Verify**

```bash
cargo build -p rowforge-studio-core
cargo test -p rowforge-studio-core
```

- [ ] **Step 5: Commit**

```
studio-core: ExecSummary.last_handler_dir

Plan 6 T3. Surfaces the rowforge-core column to the projection
that ExecDetail / ExecList consume. TS mirror updated in T6.
```

---

## Task 4: `start_run` writes `last_handler_dir`

**Files:** Modify `crates/rowforge-studio-core/src/run.rs`

- [ ] **Step 1: Find the store-lock window in start_run**

```bash
grep -n "create_attempt\|set_last_handler_dir" crates/rowforge-studio-core/src/run.rs
```

- [ ] **Step 2: Add call after attempt creation**

Inside the same `let mut store = self.store.lock()…` block that does `create_attempt`, after `let attempt = store.create_attempt(…)?;` and `handler_canon` is resolved:

```rust
// Plan 6 T4: persist this dir as the exec's "last used" for the
// RunButton's default on next visit. Errors are non-fatal — the
// run itself succeeds even if the bookkeeping update fails.
if let Err(e) = store.set_last_handler_dir(exec.id.as_str(), &handler_canon) {
    tracing::warn!(execution_id = %exec.id, error = %e,
        "failed to persist last_handler_dir; continuing");
}
```

- [ ] **Step 3: Test it actually writes**

Add to `crates/rowforge-studio-core/tests/foundation.rs`:

```rust
#[test]
fn start_run_persists_last_handler_dir() {
    use rowforge_studio_core::{OpenOpts, RunOpts, StartExecArgs, StudioCore};

    let tmp = tempdir::TempDir::new("rfs-plan6-lhd").unwrap();
    let csv = tmp.path().join("in.csv");
    std::fs::write(&csv, "row_id\nr1\n").unwrap();

    // Use a minimal handler dir with a parseable manifest (no actual
    // binary — we just need start_run to canonicalize + write before
    // the pipeline_task fails).
    let handler_dir = tmp.path().join("handler");
    std::fs::create_dir_all(&handler_dir).unwrap();
    std::fs::write(
        handler_dir.join("rowforge.yaml"),
        "name: x\nversion: 0.1.0\nentry:\n  cmd: [\"./nope\"]\n",
    ).unwrap();

    let core = StudioCore::open(OpenOpts::new().with_workspace(tmp.path().into())).unwrap();
    let id = core.start_exec(StartExecArgs::new(csv, "lhd_test")).unwrap();

    // start_run is allowed to fail later (no binary), but the
    // last_handler_dir write happens BEFORE the pipeline task runs,
    // so the sqlite row must be updated regardless.
    let _ = core.start_run(&id, RunOpts::new(handler_dir.clone()));

    let summaries = core.list(Default::default()).unwrap();
    let s = summaries.iter().find(|s| s.id == id).unwrap();
    assert_eq!(
        s.last_handler_dir.as_deref().map(|p| p.canonicalize().unwrap()),
        Some(handler_dir.canonicalize().unwrap()),
    );
}
```

- [ ] **Step 4: Run + commit**

```bash
cargo test -p rowforge-studio-core start_run_persists_last_handler_dir
```

```
studio-core: start_run persists exec.last_handler_dir

Plan 6 T4. Writes the canonicalized handler dir to sqlite under
the same store-lock that creates the new attempt. Errors are
non-fatal (logged via tracing). After this lands, exec.summary.
last_handler_dir is the source of truth for the RunButton's
default; the localStorage hack in T7 can be deleted.
```

---

## Task 5: `ProgressSnapshot.rate_10s` exposed from aggregator

**Files:** Modify `crates/rowforge-studio-core/src/aggregator.rs`

- [ ] **Step 1: Add field to ProgressSnapshot**

```rust
#[derive(Debug, Clone, Default, serde::Serialize)]
#[non_exhaustive]
pub struct ProgressSnapshot {
    // existing fields…
    pub rate_10s: f32,
}
```

- [ ] **Step 2: Compute in snapshot()**

Replace the snapshot method body:

```rust
pub fn snapshot(&self) -> ProgressSnapshot {
    let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
    let mut s = inner.snapshot.clone();
    // 40-sample sliding window × 250ms = 10s window.
    let sum: u64 = inner.rate_10s_buf.iter().sum();
    s.rate_10s = (sum as f32) / 10.0;
    s
}
```

- [ ] **Step 3: Write test**

```rust
#[test]
fn snapshot_exposes_rate_10s_from_outcomes() {
    let agg = ProgressAggregator::new();
    // Drive 10 successful outcomes; rate accumulates in the buffer
    // via tick_loop, NOT via on_outcome_success directly. We have
    // to invoke compose_tick to advance the rate buckets.
    for i in 0..10 {
        agg.on_outcome_success(i, 1);
    }
    let _ = agg.compose_tick();  // 1 tick: rotates buffer, samples delta=10
    let snap = agg.snapshot();
    assert!(snap.rate_10s > 0.0, "rate_10s = {}", snap.rate_10s);
}
```

> **Implementer note:** the exact value depends on how `compose_tick` rotates the buffer. Test asserts ">0" rather than an exact number to avoid brittleness.

- [ ] **Step 4: Run + commit**

```bash
cargo test -p rowforge-studio-core snapshot_exposes_rate_10s
```

```
studio-core: ProgressSnapshot.rate_10s exposed for rollup

Plan 6 T5. Reads from the existing 40-sample sliding-window buffer
that compose_tick already maintains; the value is the same as the
rate_10s field on emitted Tick events. Used by SessionRegistry::
rollup_tick (T6) to compute total_rate + slowest_run.

Default for fresh aggregator: 0.0 (no sampled buckets yet).
```

---

## Task 6: `SessionRegistry::rollup_tick()` computes real `total_rate` + `slowest_run`

**Files:** Modify `crates/rowforge-studio-core/src/session.rs`

- [ ] **Step 1: Replace the placeholder body**

```rust
pub fn rollup_tick(&self) -> crate::run::RunRollupTick {
    let snaps = self.snapshots();
    let active = snaps.len() as u32;
    let total_processed: u64 = snaps.iter().map(|(_, s)| s.processed).sum();
    let total_failed: u64 = snaps.iter().map(|(_, s)| s.failed + s.crashed).sum();
    let total_rate: f32 = snaps.iter().map(|(_, s)| s.rate_10s).sum();
    // Pick the run with the lowest positive rate as "slowest".
    // Runs with rate_10s == 0 are still warming up the 10s window
    // (< 10 seconds since start) and are excluded so we don't
    // false-positive them as slow.
    let slowest_run = snaps
        .iter()
        .filter(|(_, s)| s.rate_10s > 0.0)
        .min_by(|(_, a), (_, b)| {
            a.rate_10s
                .partial_cmp(&b.rate_10s)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(h, _)| h.clone());
    crate::run::RunRollupTick {
        active_runs: active,
        total_processed,
        total_failed,
        total_rate,
        slowest_run,
    }
}
```

- [ ] **Step 2: Test**

```rust
#[test]
fn rollup_tick_sums_rate_and_picks_slowest() {
    let reg = SessionRegistry::new(3, 1);
    // Inject 2 fake sessions with known rates.
    let s1 = fake_session_with_rate("e1", 100.0);
    let s2 = fake_session_with_rate("e2", 50.0);
    reg.register(s1.clone());
    reg.register(s2.clone());

    let tick = reg.rollup_tick();
    assert_eq!(tick.active_runs, 2);
    assert!((tick.total_rate - 150.0).abs() < 0.01);
    assert_eq!(tick.slowest_run, Some(s2.handle.clone()));
}

// Helper: tests/foundation.rs or in-module test helper.
fn fake_session_with_rate(exec: &str, rate: f32) -> Arc<Session> {
    let (tick_stop, _) = watch::channel(false);
    let agg = Arc::new(ProgressAggregator::new());
    // Directly write the rate into the aggregator's inner state.
    // Test-only access via a constructor helper.
    agg.set_rate_for_test(rate);  // add this helper in aggregator.rs cfg(test)
    Arc::new(Session {
        handle: RunHandle::new(),
        execution_id: exec.into(),
        attempt_id: format!("a_{}", exec),
        aggregator: agg,
        cancel_token: CancellationToken::new(),
        tick_stop,
        status: Mutex::new(RunStatus::Running),
        started_at: Instant::now(),
    })
}
```

> **Implementer note:** add a `#[cfg(test)] pub fn set_rate_for_test(&self, rate_10s: f32)` to `ProgressAggregator` that pokes the value into `inner.snapshot.rate_10s` (Plan 6 T5 already made it a snapshot field). Keeps the test deterministic without needing to drive tick_loop.

- [ ] **Step 3: Commit**

```
studio-core: rollup_tick computes real total_rate + slowest_run

Plan 6 T6. Replaces the Plan 4 placeholders (0.0 / None) with
actual derived values:
- total_rate = sum of per-session ProgressSnapshot.rate_10s
- slowest_run = handle with the min positive rate_10s
  (warming-up sessions with rate=0 are excluded so we don't
   false-positive them as slow)

Test injects two fake sessions with known rates and asserts both
fields match expectation. Aggregator gains a #[cfg(test)]
set_rate_for_test helper for deterministic testing without
driving tick_loop.
```

---

## Task 7: Drop `localStorage.studio.lastHandlerDir` from RunButton

**Files:** Modify `apps/rowforge-studio/src/components/RunButton.tsx`, `apps/rowforge-studio/src/pages/ExecDetail.tsx`

- [ ] **Step 1: RunButton — remove the LS hack**

In `RunButton.tsx`:

```tsx
// DELETE this block:
const LS_HANDLER_DIR = "studio.lastHandlerDir";

const [handlerDir, setHandlerDir] = useState<string | null>(() => {
  if (lastHandlerDir) return lastHandlerDir;
  try {
    return localStorage.getItem(LS_HANDLER_DIR);
  } catch {
    return null;
  }
});

useEffect(() => {
  try {
    if (handlerDir) localStorage.setItem(LS_HANDLER_DIR, handlerDir);
  } catch { /* ignore quota / privacy mode */ }
}, [handlerDir]);

// REPLACE with:
const [handlerDir, setHandlerDir] = useState<string | null>(lastHandlerDir ?? null);

// ADD a one-time cleanup for upgrade hygiene:
useEffect(() => {
  try { localStorage.removeItem("studio.lastHandlerDir"); } catch { /* ignore */ }
}, []);
```

- [ ] **Step 2: ExecDetail — pass the real value**

```tsx
// before
<RunButton executionId={execId} lastHandlerDir={null} />

// after
<RunButton
  executionId={execId}
  lastHandlerDir={exec.data?.summary.last_handler_dir ?? null}
/>
```

- [ ] **Step 3: Run vitest**

```bash
cd apps/rowforge-studio
pnpm test src/__tests__/run-button.test.tsx
```

Existing tests should still pass — they pass `lastHandlerDir` explicitly.

- [ ] **Step 4: Commit**

```
studio-shell: RunButton lastHandlerDir from sqlite, drop localStorage

Plan 6 T7. The localStorage.studio.lastHandlerDir hack from Plan 5
T9369fad was a stopgap until Plan 6 plumbed the value through
sqlite (T1-T4). Now that exec.summary.last_handler_dir exists,
ExecDetail passes it down and RunButton seeds from the prop only.

One-time cleanup useEffect removes the stale localStorage key on
mount (upgrade hygiene) so existing installs don't carry phantom
state.
```

---

## Task 8: TS mirrors — ExecSummary + ProgressSnapshot

**Files:** Modify `apps/rowforge-studio/src/ipc/types.ts`

- [ ] **Step 1: Add the field on ExecSummary**

```ts
export interface ExecSummary {
  // existing fields…
  last_handler_dir: string | null;
}
```

- [ ] **Step 2: Add the field on ProgressSnapshot**

```ts
export interface ProgressSnapshot {
  // existing fields…
  rate_10s: number;
}
```

- [ ] **Step 3: Update the bootstrap action (run-state.ts) to consume rate_10s**

The `_bootstrap` action already uses the snapshot wholesale. Confirm the reducer arm assigns `state.rate_10s = snap.rate_10s` (if rate_10s is in the state, which it currently is from Tick events). The existing reducer treats `state.rate_10s` separately from Tick rate. Sanity check the assignment.

- [ ] **Step 4: tsc + vitest**

```bash
pnpm tsc -b && pnpm test
```

- [ ] **Step 5: Commit**

```
studio-shell: TS mirrors for last_handler_dir + rate_10s

Plan 6 T8. ExecSummary gains last_handler_dir (sourced from sqlite
schema v3); ProgressSnapshot gains rate_10s (used by the
useRun bootstrap path so rollup_tick numbers can be cross-checked
in the UI if needed).
```

---

## Task 9: SessionRegistry consumes `Settings.max_concurrent_runs`

**Files:** Modify `crates/rowforge-studio-core/src/lib.rs`

- [ ] **Step 1: Find StudioCore::open**

```bash
grep -n "fn open\|SessionRegistry::new" crates/rowforge-studio-core/src/lib.rs
```

- [ ] **Step 2: Read Settings before constructing the registry**

```rust
pub fn open(opts: OpenOpts) -> Result<Self, UiError> {
    // … existing logic to resolve workspace_root, open the store …

    // Plan 6 T9: respect Settings.max_concurrent_runs when sizing the
    // SessionRegistry. Default 3/workspace, 1/exec (spec §3.4) when the
    // setting is absent. This is the ONLY enforcement point — the
    // registry isn't rebuilt mid-session even if Settings change. The
    // Settings page surfaces this with a "Will apply on next workspace
    // open" banner.
    let workspace_limit = {
        // Read settings file path-free. The Tauri layer normally pipes
        // the settings file in via workspace_settings_load, but during
        // workspace_open we have to side-load. Use the workspace root
        // sentinel if studio-core owns a path resolver, else default.
        // (Implementer: check what StudioCore::open already has access
        // to; if it's already loading settings, reuse that; if not,
        // accept the default of 3 in this task and revisit in Plan 7.)
        3u32
    };
    let sessions = Arc::new(SessionRegistry::new(workspace_limit, 1));

    // … rest of open …
}
```

> **Important caveat for implementer:** studio-core's `StudioCore::open` may not have access to the Tauri-managed settings file path. If reading Settings requires Tauri context, this task needs the Settings to be passed in via `OpenOpts` instead. Choose one:
>
> - **A.** Add `OpenOpts.max_concurrent_runs: Option<u32>` and have the Tauri command `workspace_open` load Settings + pass the field. Settings stay file-policy-free per spec part 5 §5.6.
> - **B.** studio-core itself learns to find the settings file (couples it to a path policy). Spec says don't do this.
>
> Pick A. Update `OpenOpts` and the Tauri `workspace_open` command accordingly.

- [ ] **Step 3: Wire from Tauri workspace_open**

In `apps/rowforge-studio/src-tauri/src/commands.rs`:

```rust
pub fn workspace_open(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    path: Option<PathBuf>,
) -> Result<Workspace, UiError> {
    let s = settings_io::load(&app)?;
    let opts = match path {
        Some(p) => OpenOpts::new().with_workspace(p),
        None => OpenOpts::new(),
    };
    let opts = opts.with_max_concurrent_runs(s.max_concurrent_runs);
    let core = StudioCore::open(opts)?;
    // … rest of existing body …
}
```

- [ ] **Step 4: Test**

```rust
#[test]
fn workspace_limit_from_settings() {
    let tmp = tempdir::TempDir::new("rfs-plan6-mcr").unwrap();
    let core = StudioCore::open(
        OpenOpts::new()
            .with_workspace(tmp.path().into())
            .with_max_concurrent_runs(Some(7))
    ).unwrap();
    assert_eq!(core.sessions().workspace_limit(), 7);
}
```

- [ ] **Step 5: Commit**

```
studio-core: SessionRegistry honors Settings.max_concurrent_runs

Plan 6 T9. StudioCore::open accepts OpenOpts.max_concurrent_runs:
Option<u32> and feeds it to SessionRegistry::new (default 3 when
absent, per spec §3.4). Tauri workspace_open loads Settings before
the open call and threads the value through.

Settings page surfaces the "applies at next workspace_open" rule
via a dirty banner (T11). No hot-reload.
```

---

## Task 10: Settings page route + scaffold

**Files:** Create `apps/rowforge-studio/src/pages/Settings.tsx`, modify `apps/rowforge-studio/src/App.tsx`

- [ ] **Step 1: Create the page**

```tsx
import { Navigate } from "react-router-dom";
import { AppShell } from "@/layout/AppShell";
import { SettingsForm } from "@/components/SettingsForm";
import { useWorkspace } from "@/ipc/queries";

export function SettingsPage() {
  const ws = useWorkspace();
  if (ws.data === null && !ws.isLoading) return <Navigate to="/" replace />;
  return (
    <AppShell
      workspace={ws.data ?? null}
      crumbs={[{ label: "Settings" }]}
    >
      <div className="mx-auto max-w-2xl p-6">
        <h1 className="mb-4 text-xl font-medium">Settings</h1>
        <SettingsForm />
      </div>
    </AppShell>
  );
}
```

- [ ] **Step 2: Register route**

In `App.tsx`:

```tsx
import { SettingsPage } from "@/pages/Settings";

// inside <Routes>:
<Route path="/settings" element={<SettingsPage />} />
```

- [ ] **Step 3: Verify sidebar link exists**

```bash
grep -n "settings\|/settings" apps/rowforge-studio/src/layout/AppShell.tsx
```

If no link, add one to the sidebar navigation. If there's already one disabled, enable it.

- [ ] **Step 4: tsc**

```bash
pnpm tsc -b
```

- [ ] **Step 5: Commit**

```
studio-shell: Settings page route at /settings

Plan 6 T10. Scaffolds the page + registers the route. Form
content lands in T11.
```

---

## Task 11: SettingsForm component

**Files:** Create `apps/rowforge-studio/src/components/SettingsForm.tsx`, create test

- [ ] **Step 1: Write the failing test**

```tsx
// apps/rowforge-studio/src/__tests__/settings-form.test.tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { SettingsForm } from "@/components/SettingsForm";

vi.mock("@/ipc/client", () => ({
  ipc: {
    workspace_settings_load: vi.fn().mockResolvedValue({
      schema_version: 1,
      workspace_root: "/tmp/ws",
      default_workers: 2,
      max_concurrent_runs: 3,
      telemetry_opt_in: false,
    }),
    workspace_settings_save: vi.fn().mockResolvedValue(undefined),
    run_active: vi.fn().mockResolvedValue([]),
  },
}));

function wrap(node: React.ReactNode) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return <QueryClientProvider client={qc}>{node}</QueryClientProvider>;
}

beforeEach(() => { vi.clearAllMocks(); });

describe("SettingsForm", () => {
  it("renders loaded values", async () => {
    render(wrap(<SettingsForm />));
    expect(await screen.findByDisplayValue("2")).toBeInTheDocument();    // default_workers
    expect(await screen.findByDisplayValue("3")).toBeInTheDocument();    // max_concurrent_runs
  });

  it("shows 'will apply on next open' when max_concurrent_runs is dirty", async () => {
    render(wrap(<SettingsForm />));
    const mcr = await screen.findByLabelText(/max concurrent runs/i);
    fireEvent.change(mcr, { target: { value: "5" } });
    expect(screen.getByText(/apply on next workspace open/i)).toBeInTheDocument();
  });

  it("saves dirty form on Save click", async () => {
    render(wrap(<SettingsForm />));
    const mcr = await screen.findByLabelText(/max concurrent runs/i);
    fireEvent.change(mcr, { target: { value: "5" } });
    fireEvent.click(screen.getByRole("button", { name: /^Save$/i }));
    await waitFor(async () => {
      const { ipc } = await import("@/ipc/client");
      expect(ipc.workspace_settings_save).toHaveBeenCalledWith({
        settings: expect.objectContaining({ max_concurrent_runs: 5 }),
      });
    });
  });
});
```

- [ ] **Step 2: Implement SettingsForm**

```tsx
import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { WorkspaceSwitchButton } from "@/components/WorkspaceSwitchButton";
import { ipc } from "@/ipc/client";
import type { Settings } from "@/ipc/types";

export function SettingsForm() {
  const qc = useQueryClient();
  const loaded = useQuery({
    queryKey: ["settings"],
    queryFn: () => ipc.workspace_settings_load(),
  });
  const save = useMutation({
    mutationFn: (settings: Settings) => ipc.workspace_settings_save({ settings }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["settings"] }),
  });

  const [form, setForm] = useState<Settings | null>(null);
  useEffect(() => {
    if (loaded.data && !form) setForm(loaded.data);
  }, [loaded.data, form]);

  if (!form) return <div className="text-muted-foreground">Loading…</div>;

  const original = loaded.data!;
  const mcrDirty = form.max_concurrent_runs !== original.max_concurrent_runs;

  return (
    <div className="space-y-6">
      <Section title="Workspace">
        <div className="space-y-2">
          <div className="font-mono text-sm">{form.workspace_root ?? "—"}</div>
          <WorkspaceSwitchButton />
        </div>
      </Section>

      <Section title="Concurrency">
        <Field label="Default workers">
          <Input
            type="number" min={1} value={form.default_workers ?? ""}
            onChange={(e) => setForm({
              ...form,
              default_workers: e.target.value === "" ? null : parseInt(e.target.value, 10),
            })}
          />
        </Field>
        <Field label="Max concurrent runs">
          <Input
            type="number" min={1} value={form.max_concurrent_runs ?? ""}
            onChange={(e) => setForm({
              ...form,
              max_concurrent_runs: e.target.value === "" ? null : parseInt(e.target.value, 10),
            })}
          />
        </Field>
        {mcrDirty && (
          <div className="rounded border border-blue-500/30 bg-blue-500/10 p-2 text-xs">
            ℹ Changes to max concurrent runs apply on next workspace open.
          </div>
        )}
      </Section>

      <Section title="Telemetry">
        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox" checked={form.telemetry_opt_in}
            onChange={(e) => setForm({ ...form, telemetry_opt_in: e.target.checked })}
          />
          Opt in to anonymous usage metrics
        </label>
      </Section>

      <div className="flex justify-end gap-2">
        <Button variant="ghost" onClick={() => setForm(original)}>Cancel</Button>
        <Button onClick={() => save.mutate(form)} disabled={save.isPending}>
          {save.isPending ? "Saving…" : "Save"}
        </Button>
      </div>
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="rounded-lg border border-zinc-700 p-4">
      <div className="mb-3 text-sm font-medium uppercase text-muted-foreground">
        {title}
      </div>
      <div className="space-y-3">{children}</div>
    </div>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div>
      <label className="mb-1 block text-sm">{label}</label>
      {children}
    </div>
  );
}
```

- [ ] **Step 3: Run tests**

```bash
pnpm test src/__tests__/settings-form.test.tsx
```

- [ ] **Step 4: Commit**

```
studio-shell: SettingsForm controlled form

Plan 6 T11. Loads settings via TanStack Query; tracks dirty state
per field; shows "will apply on next workspace open" banner when
max_concurrent_runs differs from server value; saves via
workspace_settings_save. Cancel restores from server state.

WorkspaceSwitchButton handles the workspace_root field separately
(T12); the form itself only shows the current root as read-only.
```

---

## Task 12: WorkspaceSwitchButton

**Files:** Create `apps/rowforge-studio/src/components/WorkspaceSwitchButton.tsx`, create test

- [ ] **Step 1: Test**

```tsx
// apps/rowforge-studio/src/__tests__/workspace-switch.test.tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import { WorkspaceSwitchButton } from "@/components/WorkspaceSwitchButton";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

vi.mock("@/ipc/client", () => ({
  ipc: {
    run_active: vi.fn().mockResolvedValue([]),
    workspace_settings_load: vi.fn().mockResolvedValue({
      schema_version: 1, workspace_root: "/tmp/old", default_workers: null,
      max_concurrent_runs: null, telemetry_opt_in: false,
    }),
    workspace_settings_save: vi.fn().mockResolvedValue(undefined),
    workspace_open: vi.fn().mockResolvedValue({ root: "/tmp/new", schema_version: 3 }),
  },
}));

function wrap(node: React.ReactNode) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return (
    <QueryClientProvider client={qc}>
      <MemoryRouter>{node}</MemoryRouter>
    </QueryClientProvider>
  );
}

beforeEach(() => { vi.clearAllMocks(); });

describe("WorkspaceSwitchButton", () => {
  it("button is enabled when no active runs", async () => {
    render(wrap(<WorkspaceSwitchButton />));
    const btn = await screen.findByRole("button", { name: /switch workspace/i });
    expect((btn as HTMLButtonElement).disabled).toBe(false);
  });

  it("button is disabled with tooltip when active_runs > 0", async () => {
    const { ipc } = await import("@/ipc/client");
    (ipc.run_active as any).mockResolvedValue(["run-1", "run-2"]);
    render(wrap(<WorkspaceSwitchButton />));
    const btn = await screen.findByRole("button", { name: /switch workspace/i });
    await waitFor(() => expect((btn as HTMLButtonElement).disabled).toBe(true));
    expect(btn.getAttribute("title")).toMatch(/cancel.*2.*active/i);
  });
});
```

- [ ] **Step 2: Implement**

```tsx
import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { useQuery, useMutation } from "@tanstack/react-query";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { Button } from "@/components/ui/button";
import { ipc } from "@/ipc/client";
import { uiErrorMessage } from "@/ipc/types";

export function WorkspaceSwitchButton() {
  const navigate = useNavigate();
  const [error, setError] = useState<string | null>(null);
  const active = useQuery({
    queryKey: ["run_active"],
    queryFn: () => ipc.run_active(),
    refetchInterval: 2000,  // refresh while user is on Settings
  });
  const activeCount = active.data?.length ?? 0;

  const onClick = async () => {
    setError(null);
    if (activeCount > 0) return;
    const picked = await openDialog({ directory: true, multiple: false });
    if (typeof picked !== "string") return;
    try {
      const settings = await ipc.workspace_settings_load();
      await ipc.workspace_settings_save({
        settings: { ...settings, workspace_root: picked },
      });
      await ipc.workspace_open({ path: picked });
      navigate("/");  // land on the new workspace's exec list
    } catch (e) {
      setError(uiErrorMessage(e));
    }
  };

  const disabled = activeCount > 0;
  const tooltip = disabled
    ? `Cancel ${activeCount} active run${activeCount === 1 ? "" : "s"} first`
    : "Open a different workspace";

  return (
    <div className="space-y-1">
      <Button onClick={onClick} disabled={disabled} title={tooltip} variant="outline">
        Switch workspace…
      </Button>
      {disabled && (
        <div className="text-xs text-amber-300">
          ⚠ {activeCount} active run{activeCount === 1 ? "" : "s"} — cancel to switch
        </div>
      )}
      {error && <div className="text-xs text-red-300">{error}</div>}
    </div>
  );
}
```

- [ ] **Step 3: Run tests**

```bash
pnpm test src/__tests__/workspace-switch.test.tsx
```

- [ ] **Step 4: Commit**

```
studio-shell: WorkspaceSwitchButton

Plan 6 T12. Self-contained component on the Settings page.
- Polls run_active every 2s (auto-refresh while user is on page)
- Disabled with tooltip + amber warning when active_runs > 0
- On click: directory picker → save settings → workspace_open
  → navigate to / so the user lands on the new workspace's list

Save+open chain runs sequentially; failure surfaces in red text.
```

---

## Task 13: HUMAN_SMOKE Plan 6 walkthrough

**Files:** Modify `apps/rowforge-studio/HUMAN_SMOKE.md`

- [ ] **Step 1: Append section**

```markdown

## Plan 06 additions

### Settings page

1. Click **Settings** in the left sidebar → land on `/settings`
2. Verify the 4 sections render: Workspace (with current path + Switch button), Concurrency (default_workers + max_concurrent_runs), Telemetry checkbox
3. Edit **Max concurrent runs** from 3 to 5
4. Confirm the blue "Changes apply on next workspace open" banner appears
5. Click **Save** → toast (if present) / form re-syncs; banner stays until next workspace open

### Workspace switching — happy path

1. From Settings, with no active runs, click **Switch workspace…**
2. Pick a different directory (or a new empty one)
3. App routes to `/` showing the new workspace's exec list (empty for fresh dir)
4. Sidebar shows the new workspace name

### Workspace switching — blocked

1. Start a long-running attempt (sample 10s sleep handler with 5 rows)
2. While running, open Settings
3. Confirm **Switch workspace…** is disabled with tooltip "Cancel 1 active run first"
4. Cancel the run (via the active-runs pill or attempt page)
5. Confirm the button becomes enabled within 2s

### last_handler_dir persistence

1. Run an exec with a handler dir
2. Quit Studio entirely (Cmd+Q)
3. Reopen Studio, navigate back to the same exec
4. Click **Run** → directory picker should NOT appear; the previous handler dir is auto-selected
5. Confirm the run starts directly

### RunRollupTick real numbers

1. Start 2 concurrent runs with different handler speeds (one sleep 10s, one sleep 2s)
2. Open the active-runs pill in the header
3. Confirm `total_rate` shows a non-zero number (sum of both runs' rates)
4. Confirm `slowest_run` points to the slower handler's attempt

### Known Plan 6 limitations (deferred to Plan 7+)

- Handler authoring panel (Part 8 entirely) → Plan 7
- Hard cancel still degrades to soft cancel (needs rowforge-core process-kill API)
- No Settings hot-reload — `max_concurrent_runs` only takes effect on next workspace open
- No multi-workspace recents list / "recently opened" picker
- `slowest_run` heuristic is min positive rate_10s; better heuristics (ETA, stall) deferred
```

- [ ] **Step 2: Commit**

```
studio-shell: HUMAN_SMOKE Plan 6 walkthrough

Settings page + workspace switching (happy + blocked) +
last_handler_dir survival across restart + RunRollupTick real
numbers verification. Plus deferred-items list.
```

---

## Task 14: Spec docs update

**Files:** Modify `docs/spec/studio/part-2-model.md`, `part-5-api.md`, `part-6-observability.md`, `part-7-ui.md`, mirrors in `zh-Hant/`

- [ ] **Step 1: part-2 §2.x — `ExecSummary.last_handler_dir`**

Find the `ExecSummary` struct definition. Add the field with a one-line doc. Mirror to zh-Hant.

- [ ] **Step 2: part-5 §5.6 — Settings + max_concurrent_runs reload semantics**

Add a note: "`max_concurrent_runs` is read at `workspace_open` time. Changing the value via `workspace_settings_save` does not affect the active SessionRegistry; the Settings page surfaces this as a 'Will apply on next workspace open' banner."

- [ ] **Step 3: part-6 §6.6 — `slowest_run` heuristic**

Add a sentence: "`slowest_run` is selected as the active session with the minimum positive `rate_10s` (sessions warming up the 10-second window with `rate_10s == 0` are excluded so they're not false-positively flagged as slow)."

- [ ] **Step 4: part-7 — Settings page wireframe**

Add a short subsection (or extend the existing one) describing the Settings page layout: 4 sections (Workspace, Concurrency, Telemetry), Switch workspace button gated by active runs, dirty banner for `max_concurrent_runs`.

- [ ] **Step 5: zh-Hant mirrors**

Apply the same edits in `docs/spec/studio/zh-Hant/`.

- [ ] **Step 6: Commit**

```
docs: spec updates for Plan 6 (Settings + last_handler_dir + slowest_run)

- part-2: ExecSummary gains last_handler_dir (sqlite schema v3)
- part-5 §5.6: max_concurrent_runs reload semantics documented
  (applied at workspace_open, surfaced via Settings dirty banner)
- part-6 §6.6: slowest_run heuristic spelled out (min positive
  rate_10s; warming-up sessions excluded)
- part-7: Settings page wireframe added
- zh-Hant mirrors

Behavior contracts already shipped in T1-T13; this just synchronises
the normative spec with what landed.
```

---

## Task 15: Final verification + PR

**Files:** none (verification)

- [ ] **Step 1: Full test matrix**

```bash
cd /Users/lemo/code/lemo/repo/rowforge
cargo build
cargo test
cd apps/rowforge-studio
pnpm tsc -b
pnpm test
pnpm build
```

Expected approximate counts:
- Rust: 246 + ~5 new (migration test, last_handler_dir round-trip, start_run persistence, rate_10s exposure, rollup_tick computation)
- Vitest: 58 + ~6 new (SettingsForm 3 tests, WorkspaceSwitchButton 2 tests, plus any incidental)
- `pnpm build` clean

- [ ] **Step 2: Manual smoke (`pnpm tauri dev`)**

Walk the HUMAN_SMOKE Plan 6 section end-to-end. Pay special attention to:
- Schema migration on an existing v2 workspace (test fixture or your existing dev workspace if it's v2)
- Switch workspace doesn't leak old active-runs forwarder (existing Plan 4 fix; verify it still holds)
- last_handler_dir survives a full Studio restart

- [ ] **Step 3: Open PR**

```bash
gh pr create --title "studio-plan-06: Settings + Workspace + carry-forwards" --body "$(cat <<'EOF'
## Summary

Plan 6 closes every Plan 5 deferred item that isn't its own plan. Pure polish — no rowforge-core API additions beyond a sqlite migration.

### Backend
- T1: sqlite schema v2 → v3 (executions.last_handler_dir TEXT NULL)
- T2: ExecutionStore::set_last_handler_dir + struct field
- T3: ExecSummary.last_handler_dir
- T4: start_run persists last_handler_dir on success
- T5: ProgressSnapshot.rate_10s
- T6: SessionRegistry::rollup_tick computes real total_rate + slowest_run
- T9: SessionRegistry size driven by Settings.max_concurrent_runs at workspace_open

### Frontend
- T7: RunButton drops localStorage; reads exec.summary.last_handler_dir
- T8: TS mirrors
- T10: Settings page at /settings
- T11: SettingsForm with dirty banner for max_concurrent_runs
- T12: WorkspaceSwitchButton with active-runs gating

### Docs
- T13: HUMAN_SMOKE Plan 6 walkthrough
- T14: Spec part-2, part-5 §5.6, part-6 §6.6, part-7 + zh-Hant mirrors

### Tests
- Rust: 246 → ~251
- Vitest: 58 → ~64

### Acceptance
- [x] cargo build clean
- [x] cargo test all green
- [x] pnpm tsc + test + build clean
- [x] sqlite schema_version = 3 on fresh + migrated workspaces
- [x] Settings page renders + saves
- [x] Switch workspace gated on active runs
- [x] last_handler_dir survives Studio restart
- [x] RunRollupTick.total_rate / slowest_run non-placeholder
- [ ] **(human)** HUMAN_SMOKE walkthrough
EOF
)"
```

---

## Acceptance criteria (overall)

1. `cargo build` clean on workspace MSRV (1.88)
2. `cargo test` workspace passes; migration v2→v3 test included
3. `cargo test -p rowforge-studio-core` passes (77 baseline + ~5 new)
4. `pnpm tsc -b` + `pnpm test` (~64 tests) + `pnpm build` clean
5. New workspace shows `schema_version = 3` in sqlite
6. Existing v2 workspace auto-migrates without data loss
7. Settings page renders 4 sections; Save persists; Cancel restores
8. Switch workspace button: disabled with tooltip when active_runs > 0
9. RunButton default handler dir comes from `exec.summary.last_handler_dir`
10. `RunRollupTick.total_rate` non-zero during steady-state run; `slowest_run` is `Some(handle)` when ≥ 2 runs are active with throughput
11. Spec docs (en + zh-Hant) updated for `last_handler_dir`, `max_concurrent_runs` reload, `slowest_run` heuristic, Settings page wireframe
12. **(human)** HUMAN_SMOKE.md walkthrough

---

## Self-review

**Spec coverage:** Every section of the design doc has at least one task:
- §4.1 sqlite migration → T1
- §4.2 set_last_handler_dir + Execution.last_handler_dir → T2
- §4.3 ExecSummary projection → T3
- §4.4 drop localStorage → T7
- §4.5 rate_10s + rollup_tick → T5, T6
- §4.6 max_concurrent_runs wire → T9
- §5.1 Settings page → T10
- §5.2 SettingsForm → T11
- §5.3 WorkspaceSwitchButton → T12
- §5.4 RunButton localStorage removal → T7

**Placeholder scan:** T9 carries an explicit caveat about `StudioCore::open`'s settings access path (option A vs B); the implementer is told to pick A and update `OpenOpts`. T5 step 2 uses a heuristic rate computation that the implementer is told to adjust if the existing buffer math differs.

**Type consistency:** `ExecSummary.last_handler_dir: Option<PathBuf>` (Rust) ↔ `last_handler_dir: string | null` (TS). `ProgressSnapshot.rate_10s: f32` (Rust) ↔ `rate_10s: number` (TS). `RunRollupTick` shape already exists; only field values change.

**Order dependencies:** T1 → T2 (migration before column access) → T3 (Execution → ExecSummary) → T4 (start_run uses setter) → T5 → T6 (snapshot before rollup_tick) → T7 (depends on T3 ExecSummary) → T8 (TS mirrors after Rust shape) → T9 (independent; can run anywhere after T8) → T10 → T11 → T12 → T13 → T14 → T15. No backward dependencies.
