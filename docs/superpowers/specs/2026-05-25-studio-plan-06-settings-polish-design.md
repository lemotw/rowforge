# Studio Plan 06 — Settings + Workspace + carry-forwards (design)

> **Status:** brainstorm output for Plan 6. Builds on Plans 1-5 (foundation, Tauri shell, exec history, live runs, exec lifecycle). This is the *design* document; the implementation plan lives at `docs/superpowers/plans/2026-05-25-studio-plan-06-settings-polish.md`.

## 1. Goal

Close every Plan 5 deferred item that isn't its own plan. After Plan 6, all common workspace / per-exec settings live behind a proper UI, the lingering `0.0` / `null` placeholders in `RunRollupTick` carry real numbers, and the last-handler-dir hack moves from `localStorage` to sqlite.

No new user workflow, no rowforge-core API additions beyond a sqlite migration. **Pure polish.**

## 2. Scope

### In scope
- **Settings page** at `/settings` (4 form fields: workspace_root display + switch, default_workers, max_concurrent_runs, telemetry_opt_in)
- **Workspace switching** — directory picker → save Settings → reopen workspace. Blocked when active_runs > 0
- **`max_concurrent_runs` → SessionRegistry** wire-up at `workspace_open` time. Settings page shows a "Will apply on next workspace open" banner when dirty.
- **`last_handler_dir` per-exec** (sqlite schema v3 migration; `executions.last_handler_dir TEXT NULL`). Written on every successful `run_start`. Read by `RunButton` as the default. Drops the `localStorage.studio.lastHandlerDir` hack from Plan 5.
- **`RunRollupTick.total_rate` / `slowest_run` real computation** — sum per-session `rate_10s`; pick `slowest_run` as the active session with min `rate_10s`. Requires extending `ProgressSnapshot` with rate fields and `SessionRegistry::rollup_tick()` to use them.

### Out of scope (Plan 7+ candidates)
- Handler Authoring (Part 8 entirely) → Plan 7
- Hard cancel actually killing workers (needs rowforge-core API addition)
- Export streaming progress + cancel
- Multi-workspace recents list / "recently opened" picker
- Settings hot-reload (Studio currently can't apply `max_concurrent_runs` without re-opening workspace; we acknowledge this and surface a banner rather than building hot reload)

## 3. Decisions locked during brainstorm

| Question | Choice | Rationale |
|---|---|---|
| Scope size | **Polish 純收尾** | One coherent sprint, no rowforge-core API additions |
| Workspace switch with active runs | **Block** — disable Switch button + tooltip "Cancel N active runs first" | Avoids state leak; user must explicitly cancel before switching |
| `max_concurrent_runs` reload | **Next `workspace_open`** | SessionRegistry is constructed once in `StudioCore::open`; saving Settings shows "Will apply on next workspace open" banner. Cheaper than adding `SessionRegistry::set_limits`. |
| `last_handler_dir` storage | **Per-exec in sqlite (schema v3)** | Different execs naturally use different handlers; per-exec is the right granularity. Settings.json global is wrong for this. |

## 4. Backend design

### 4.1 sqlite schema v3 migration

Add a nullable `last_handler_dir TEXT NULL` column to the `executions` table.

```sql
-- migrations/0003_executions_last_handler_dir.sql
ALTER TABLE executions ADD COLUMN last_handler_dir TEXT;
```

Migration mechanism follows rowforge-core's existing pattern (`migrations/` dir + `MIGRATIONS: &[(u8, &str)]` table). v2 → v3 bump.

### 4.2 `Execution` struct + accessor

```rust
// rowforge-core/src/execution_store.rs
pub struct Execution {
    // existing fields…
    pub last_handler_dir: Option<PathBuf>,
}

impl ExecutionStore {
    pub fn set_last_handler_dir(
        &mut self,
        id: &str,
        dir: &Path,
    ) -> Result<()>;
}
```

Idempotent write. Called from `StudioCore::start_run` after the handler dir is canonicalized and the attempt is created.

### 4.3 Propagate to studio-core projections

- `ExecSummary` gains `last_handler_dir: Option<PathBuf>`
- `ExecDetail.summary` already reuses `ExecSummary`; no separate work
- TS mirror updated

### 4.4 Drop `localStorage.studio.lastHandlerDir` (Plan 5 hack)

`RunButton` reads `lastHandlerDir` from the prop (sourced from `ExecDetail.summary.last_handler_dir`). The `LS_HANDLER_DIR` constant and the `useEffect` mirror are deleted.

### 4.5 `ProgressSnapshot.rate_10s` + `total_rate` / `slowest_run`

```rust
// studio-core/src/aggregator.rs
#[non_exhaustive]
#[derive(Debug, Clone, Default, Serialize)]
pub struct ProgressSnapshot {
    // existing fields…
    pub rate_10s: f32,
}

impl ProgressAggregator {
    pub fn snapshot(&self) -> ProgressSnapshot {
        // existing snapshot logic + compute rate_10s from rate_10s_buf
    }
}
```

`SessionRegistry::rollup_tick()`:

```rust
pub fn rollup_tick(&self) -> RunRollupTick {
    let snaps = self.snapshots();
    let active = snaps.len() as u32;
    let total_processed: u64 = snaps.iter().map(|(_, s)| s.processed).sum();
    let total_failed: u64 = snaps.iter().map(|(_, s)| s.failed + s.crashed).sum();
    let total_rate: f32 = snaps.iter().map(|(_, s)| s.rate_10s).sum();
    let slowest_run = snaps
        .iter()
        .filter(|(_, s)| s.rate_10s > 0.0)  // ignore Running-but-no-throughput
        .min_by(|(_, a), (_, b)| a.rate_10s.partial_cmp(&b.rate_10s).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(h, _)| h.clone());
    RunRollupTick {
        active_runs: active,
        total_processed,
        total_failed,
        total_rate,
        slowest_run,
    }
}
```

Edge case: a freshly-started run has `rate_10s = 0` for the first ~10s (the sliding window hasn't filled). `slowest_run` filters those out so we don't pick a "still warming up" run as the slowest.

### 4.6 SessionRegistry max_concurrent_runs wire

```rust
// studio-core/src/lib.rs
impl StudioCore {
    pub fn open(opts: OpenOpts) -> Result<Self, UiError> {
        // existing logic…
        let settings = Settings::load_or_default(&workspace);
        let workspace_limit = settings.max_concurrent_runs.unwrap_or(3);
        let sessions = Arc::new(SessionRegistry::new(workspace_limit, 1));
        // …
    }
}
```

If `Settings.max_concurrent_runs` is `None` (default), use spec value `3`. Settings page lets the user override per-workspace.

## 5. Frontend design

### 5.1 Settings page (`/settings`)

Mounted via React Router. Sidebar link already present (existing AppShell).

```
┌─ Settings ──────────────────────────────────────────┐
│ Workspace                                           │
│   /Users/lemo/.rowforge                             │
│   [Switch workspace…]                               │
│   ⚠ 2 active runs — cancel them to switch          │
│                                                     │
│ Concurrency                                         │
│   Default workers       [ 2 ]                       │
│   Max concurrent runs   [ 3 ]                       │
│   ℹ Changes apply on next workspace open            │
│                                                     │
│ Telemetry                                           │
│   ☐ Opt in to anonymous usage metrics               │
│                                                     │
│                            [Cancel]  [Save]         │
└─────────────────────────────────────────────────────┘
```

#### Save behavior

`Save` button calls `workspace_settings_save(settings)`. If the new value of `max_concurrent_runs` differs from the value that was loaded, a banner appears noting that the change applies on next workspace open. **Save** does not trigger `workspace_open`.

#### Switch workspace behavior

- Click `Switch workspace…` opens the OS directory picker.
- Before opening picker, check `run_active()`; if `len() > 0`, button is disabled with a tooltip "Cancel N active runs first" and the picker doesn't even open.
- On directory picked: call `workspace_settings_save({ ...settings, workspace_root: <picked> })`, then `workspace_open({ path: <picked> })`. The existing AppState swaps the core; the existing `forward_active_runs` task is auto-aborted (Plan 4 fix) and a new one starts.
- Settings page navigates to `/` after successful switch so user lands on the new workspace's exec list.

### 5.2 `SettingsForm` component

Controlled form bound to `Settings`. Dirty state tracked per-field for the "Will apply on next workspace open" banner. Cancel restores from server state.

### 5.3 `WorkspaceSwitchButton` component

Self-contained: queries `run_active`, manages picker open state, calls the save+open chain on commit.

### 5.4 `RunButton`: drop localStorage

Replace:

```tsx
const LS_HANDLER_DIR = "studio.lastHandlerDir";

const [handlerDir, setHandlerDir] = useState<string | null>(() => {
  if (lastHandlerDir) return lastHandlerDir;
  try { return localStorage.getItem(LS_HANDLER_DIR); } catch { return null; }
});

useEffect(() => {
  try { if (handlerDir) localStorage.setItem(LS_HANDLER_DIR, handlerDir); } catch {}
}, [handlerDir]);
```

With:

```tsx
const [handlerDir, setHandlerDir] = useState<string | null>(lastHandlerDir ?? null);
```

`ExecDetail` passes `exec.summary.last_handler_dir` as the `lastHandlerDir` prop (was hardcoded `null`). Migration cleanup: on mount, if `localStorage.studio.lastHandlerDir` exists, remove it.

### 5.5 ActiveRunsPill / RunRollupTick consumers

The TS mirror for `RunRollupTick.total_rate` already accepts a number; the placeholder rendering "—" remains until backend supplies a non-zero value. No frontend change needed — the data just stops being placeholder.

## 6. Risks / open questions

1. **Sqlite migration verification.** Need a test that loads a v2 workspace fixture and verifies the migration runs without data loss. rowforge-core's migration runner already supports incremental version bumps; verify by adding an explicit fixture if not already covered.

2. **`rate_10s` accuracy for short runs.** Sliding window has 40 × 250ms = 10s warm-up. Sub-10s runs report rate < expected. Cosmetic only; UI renders 0 → "—" via existing formatter.

3. **Workspace switch race.** Even with the active-runs block, a run could start between the check and the swap. Mitigation: check + switch under one mutex window OR re-check after the new core is open and refuse if dirty. Decision: re-check in `workspace_open` body; if active runs from the OLD core somehow exist, log a warning and proceed (the AppState swap drops the reference; runs become orphans → orphan recovery on next start handles them).

4. **localStorage cleanup.** Old `studio.lastHandlerDir` key should be removed on first mount of `RunButton` post-upgrade so it doesn't linger. One-time `try { localStorage.removeItem(...) } catch {}`.

5. **`slowest_run` heuristic.** Picking by min `rate_10s > 0` is a placeholder. Better heuristics (e.g., highest ETA, longest stall) deferred. Document the choice in the field's doc comment.

6. **Settings schema v2.** `Settings` currently is `schema_version: 1`. No fields added in this plan, but if we ever need to bump it (e.g. Plan 7 adds `preferred_editor`), the tolerant-reader (`#[serde(default)]`) handles forward compat. No work needed now.

## 7. Acceptance criteria

1. `cargo build` clean on workspace MSRV (1.88)
2. `cargo test` workspace passes; new migration test for v2 → v3 schema bump
3. `cargo test -p rowforge-studio-core` passes (77 baseline + new tests for `set_last_handler_dir`, rate_10s snapshot, rollup_tick computation)
4. `pnpm tsc -b` + `pnpm test` + `pnpm build` clean
5. New workspace shows schema_version = 3 in sqlite
6. Existing v2 workspace auto-migrates without data loss (test fixture)
7. Settings page renders all 4 form sections; Save persists; Cancel restores
8. Switch workspace button: disabled with tooltip when `run_active().len() > 0`; enabled otherwise
9. RunButton default handler dir comes from `exec.summary.last_handler_dir` after a successful run, NOT from localStorage
10. `RunRollupTick.total_rate` is non-zero during a steady-state run; `slowest_run` is `Some(handle)` when ≥ 2 runs are active with throughput
11. Spec docs (en + zh-Hant) updated: part-2 §2.x for `last_handler_dir` on ExecSummary; part-5 §5.6 for `max_concurrent_runs` wire-up; part-6 §6.6 for `slowest_run` heuristic; part-7 §7.x Settings page wireframe
12. **(human)** HUMAN_SMOKE.md Plan 6 walkthrough: edit Settings → save → switch workspace → restart Studio → verify last_handler_dir survives

## 8. Out-of-scope captured for future plans

| Item | Target plan |
|---|---|
| Handler Authoring panel (Part 8 entirely) | Plan 7 |
| Hard cancel actually killing workers | Future (needs rowforge-core API) |
| Export streaming progress + cancel | Future |
| Multi-workspace recents / "recently opened" picker | Plan 8 candidate |
| Settings hot-reload of `max_concurrent_runs` | Future (low ROI) |
| Better `slowest_run` heuristic (ETA / stall-aware) | Future |
| `Settings.preferred_editor` | Plan 7 (handler authoring depends on this) |
