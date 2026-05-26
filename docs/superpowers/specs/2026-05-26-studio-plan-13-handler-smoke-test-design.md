# Plan 13 — Handler smoke test

**Date:** 2026-05-26
**Branch:** `studio-plan-13-handler-smoke-test`
**Builds on:** Plans 7-12

## 1. Purpose

After Plan 7 (scaffold), Plan 8 (build), and Plan 12 (import + fork), the user can create a handler quickly. Verifying it works still requires the full exec ritual: create execution, supply a CSV, dispatch a run, inspect outcomes. That round-trip is heavy when the question is just *"does this handler run? what does it output for one row?"*

Plan 13 adds a **Smoke Test** tab on HandlerDetailPage that dispatches N rows directly to the handler binary and shows outcomes inline. No execution is created, no exec history is touched, no input file is required. Sources for rows: pasted JSON, fixtures dir on disk, or one synthetic row.

This closes the Plan 8 spec §8.4.3 "smoke test" deferred surface.

## 2. Scope decisions

| Decision | Choice | Rationale |
|---|---|---|
| Storage model | Ephemeral — outcomes returned by IPC call, never persisted | Smoke runs aren't auditable artifacts; persistence pollutes exec history |
| Build gate | Reuse Plan 8 `needs_build` + `run_build` | Same handler binary path as normal exec; consistent UX |
| Row sources | (a) Pasted JSON lines (b) Fixtures dir (c) Single synthetic row | Covers manual exploration, regression fixtures, and zero-config first run |
| Default row count | 5 | Enough to see batch behavior; small enough to be instant |
| Hard limit | 100 rows | Smoke is not an exec; if you need more, run an exec |
| Active-run gate | Refuse if any exec attempt is running using this handler | Same single-binary contention argument as Plan 8 |
| Concurrency | workers=1, batch_size=1 (force row mode) | Smoke is for "does this work?", not throughput. Removes batch protocol from the surface area. |
| UI location | New "Smoke test" tab on HandlerDetailPage | Discoverable next to Files / Last build / Settings |
| Cancellation | Reuse soft cancel + grace (no hard cancel in this plan; that's Plan 14) | Same as exec runs |
| Output rendering | Inline table with seq / status / message / dur_ms / data preview | Mirrors AttemptDetail Failed-rows table column set |

## 3. Backend changes

### 3.1 New API: `StudioCore::handler_smoke_run`

```rust
#[derive(Debug, Clone)]
pub struct SmokeRunRequest {
    pub handler_name: String,
    pub rows: Vec<serde_json::Value>,   // each row is the row's JSON object
}

#[derive(Debug, Clone, serde::Serialize)]
#[non_exhaustive]
pub struct SmokeOutcome {
    pub seq: u64,
    pub status: String,                  // "success" | "error" | "crash"
    pub code: Option<String>,
    pub message: Option<String>,
    pub dur_ms: u64,
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[non_exhaustive]
pub struct SmokeRunResult {
    pub outcomes: Vec<SmokeOutcome>,
    pub stderr_tail: String,             // last ~4 KiB
    pub exit_code: Option<i32>,
    pub elapsed_ms: u64,
}

impl StudioCore {
    pub async fn handler_smoke_run(
        &self,
        req: SmokeRunRequest,
    ) -> Result<SmokeRunResult, UiError>;
}
```

Algorithm:
1. Validate `handler_name` via `is_valid_id_component`.
2. Resolve `handler_dir = <workspace>/handlers/<handler_name>/`. Must exist.
3. Reject if `rows.len() == 0` → `UiError::InvalidArg("smoke needs at least 1 row")`.
4. Reject if `rows.len() > 100` → `UiError::InvalidArg("smoke limit is 100 rows")`.
5. Cross-process active-run gate: if any attempt for ANY exec in this workspace uses this handler and is `running`, refuse → `UiError::HandlerBusy { name }`. (sqlite query: `SELECT 1 FROM attempts WHERE handler_dir = ? AND state = 'running'`).
6. Build gate: call `rowforge_core::build::needs_build(&handler_dir)`. If true, call `run_build`. On failure → `UiError::BuildFailed { stderr_tail }`. On `NoBuildCommand` from a non-prebuilt handler, propagate.
7. Spawn one worker via existing `WorkerHandle::spawn(&handler_dir, /* worker_id */ 0)`.
8. Force row mode: for each row, send `Outbound::Row { seq, data: row }`, await one inbound `Inbound::RowResult`, map to `SmokeOutcome`. Cap dispatch with a per-row timeout of `smoke_timeout_per_row` from Settings (default 30s; new setting added in §3.3).
9. After last row, call `worker.shutdown(Duration::from_secs(2))`, capture exit code.
10. Drain stderr into a ring buffer; keep last 4 KiB; return as `stderr_tail`.

Concurrency: this whole function holds an internal mutex per `handler_dir` to prevent two parallel smoke runs from racing on the same binary (Settings could expose multi-handler smoke later).

### 3.2 New API: `StudioCore::handler_smoke_load_fixtures`

```rust
impl StudioCore {
    /// Read up to `limit` rows from a fixtures path. Supports:
    /// - `.jsonl` / `.ndjson`: one JSON object per line
    /// - `.json`: top-level array of objects
    /// - `.csv`: header row → object per data row (strings only)
    /// - directory: pick the first matching file (above order)
    pub fn handler_smoke_load_fixtures(
        &self,
        path: &Path,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, UiError>;
}
```

Algorithm:
1. Path must exist; if dir, enumerate non-hidden entries top-level and pick the first matching extension by the precedence above.
2. Stop reading after `limit` rows.
3. Parse errors on individual lines → continue with `tracing::warn`; collect up to `limit` parseable rows.
4. Empty result → `UiError::InvalidArg("no rows found in fixtures path")`.

### 3.3 Settings additions

```rust
pub struct Settings {
    // ...existing...
    pub smoke_default_rows: usize,        // default 5, range 1..=100
    pub smoke_timeout_per_row_secs: u64,  // default 30
}
```

Both surfaced via existing `workspace_open` / `settings_save` plumbing (the Plan 9 raw_stdout pattern).

## 4. Tauri shell

Two new commands:

```rust
#[tauri::command]
pub async fn handler_smoke_run(
    state: State<'_, AppState>,
    request: SmokeRunRequest,
) -> Result<SmokeRunResult, UiError>;

#[tauri::command]
pub fn handler_smoke_load_fixtures(
    state: State<'_, AppState>,
    path: String,
    limit: usize,
) -> Result<Vec<serde_json::Value>, UiError>;
```

`handler_smoke_run` is async because it awaits worker IO. Args use camelCase from JS (Tauri auto-converts to snake_case).

No new events — smoke is one-shot request/response. Live tail is out of scope for v1 (the run is bounded to N≤100 rows; latency is fine).

## 5. React UI

### 5.1 New tab on HandlerDetailPage

Header tabs become: `Overview | Files | Smoke test | Last build | Settings`.

### 5.2 SmokeTab component

`apps/rowforge-studio/src/components/SmokeTab.tsx`:

```
┌─ Smoke test ────────────────────────────────────────────────┐
│ Source: (•) Paste JSON  ( ) Fixtures…  ( ) One synthetic row│
│                                                             │
│ ┌───────────────────────────────────────────────────────┐  │
│ │ {"id":"1","email":"a@example.com"}                    │  │
│ │ {"id":"2","email":"b@example.com"}                    │  │
│ │ {"id":"3","email":"c@example.com"}                    │  │
│ └───────────────────────────────────────────────────────┘  │
│ 3 rows parsed                                              │
│                                                             │
│ Rows to run: [ 5 ]  (max 100)         [ Run smoke test ]   │
│                                                             │
│ ─── Outcomes (3) ────────────── ✓ 2 success • ✗ 1 error ──  │
│  seq │ status  │ message      │ dur_ms │ data              │
│  ────┼─────────┼──────────────┼────────┼────────────────── │
│   1  │ success │ —            │   12   │ {"sent":true}     │
│   2  │ success │ —            │   14   │ {"sent":true}     │
│   3  │ error   │ smtp timeout │  3001  │ —                 │
│                                                             │
│ ▸ stderr tail (4.0 KiB)                                    │
└─────────────────────────────────────────────────────────────┘
```

State shape:
```tsx
const [source, setSource] = useState<"paste" | "fixtures" | "synthetic">("paste");
const [pasted, setPasted] = useState("");
const [fixturePath, setFixturePath] = useState<string | null>(null);
const [loadedRows, setLoadedRows] = useState<unknown[] | null>(null);
const [rowCount, setRowCount] = useState(settings.smoke_default_rows);
const [result, setResult] = useState<SmokeRunResult | null>(null);
```

Source modes:
- **Paste JSON**: textarea. Parse each non-empty line as JSON; show "N rows parsed" or "line K: <err>".
- **Fixtures**: `[Pick file or folder…]` via `@tauri-apps/plugin-dialog`. On pick → `useHandlerSmokeLoadFixtures.mutate({ path, limit: 100 })` → store rows. Show first 3 row keys as a sanity preview: `keys: id, email, amount (3 columns)`.
- **One synthetic row**: synthesizes `[{ "row": 1 }]`. Useful for "does the binary start at all?".

Run gating:
- Disable Run when: parse error, no rows, `loadedRows == null` on fixtures, mutation pending.
- Tooltip on disabled state explains why.

Outcomes table reuses `apps/rowforge-studio/src/pages/AttemptDetail.tsx`'s row-table styling (extract to shared `<OutcomeTable />` if convenient; otherwise inline). `data` column: render as `<code>` with `JSON.stringify(value).slice(0, 60)` and tooltip-full on hover.

stderr tail: collapsible details block, monospace, `whitespace-pre-wrap`.

### 5.3 Hooks

```ts
export const useHandlerSmokeRun = () =>
  useMutation({
    mutationFn: (request: SmokeRunRequest) => ipc.handler_smoke_run({ request }),
  });

export const useHandlerSmokeLoadFixtures = () =>
  useMutation({
    mutationFn: (args: { path: string; limit: number }) =>
      ipc.handler_smoke_load_fixtures(args),
  });
```

No query invalidation needed; smoke is ephemeral.

### 5.4 Error rendering

- `HandlerBusy` → "Handler has an active run on exec <id>. Cancel it before smoke testing."
- `BuildFailed` → render with the Plan 8 LastBuildSection error panel pattern (collapsible stderr).
- `InvalidArg("smoke limit is 100 rows")` → friendly inline error above the run button.
- All other UiError → generic toast + inline message.

## 6. CLI

Out of scope for v1. The natural shape would be `rowforge handler smoke <name> [--rows N] [--fixtures path]`. Defer until we hear a CLI need.

## 7. Out of scope (explicit)

- Persistence of smoke runs across sessions / window reloads
- Multiple parallel smoke runs against different handlers (allowed by design but no UI for it)
- Batch mode smoke (forced row mode for simplicity; batch handlers still work — we send rows one at a time to a batch-capable handler, which the protocol supports via batch_size: 1 dispatch)
- Live tail (smoke is bounded; not worth the broadcast plumbing)
- Hard cancel (Plan 14)
- Editing pasted rows in a structured form (paste raw JSON only)
- Exporting smoke outcomes to CSV (one-shot; copy from table if needed)
- Workspace-level fixtures library

## 8. Testing

| Suite | Adds | Notes |
|---|---|---|
| rowforge-studio-core | ~8 | smoke_run happy path; row limit rejected; empty rows rejected; handler-not-found; handler-busy (sqlite gate); build-failure propagation; load_fixtures (jsonl + json + csv + dir + empty) |
| studio-shell ipc_contract | ~2 | handler_smoke_run and handler_smoke_load_fixtures registered + arg shapes |
| vitest | ~7 | SmokeTab paste-mode parses lines + shows count; fixtures-mode disabled until picker resolves; synthetic mode auto-fills; run button disabled states; outcomes table renders success/error/crash variants; stderr tail collapsible; HandlerBusy error rendering |

Targets:
- cargo: 408 → ~418 (+10)
- vitest: 166 → ~173 (+7)

## 9. Spec doc updates

- `docs/spec/studio/part-5-api.md`: §5.X (next free) two new commands; SmokeRunRequest/SmokeRunResult/SmokeOutcome shapes
- `docs/spec/studio/part-7-ui.md`: HandlerDetailPage tabs gain "Smoke test"
- `docs/spec/studio/part-8-handler-authoring.md`: §8.4.3 "Smoke test" section replaces the deferred-surface placeholder from Plan 8
- Mirror in zh-Hant
- HUMAN_SMOKE Plan 13: ~18 steps covering paste happy path, fixtures (jsonl + csv + dir), synthetic mode, build-failure surface, handler-busy gate, row-limit cap, stderr tail visibility

## 10. Acceptance criteria

1. `cargo build && cargo test` clean
2. `pnpm tsc -b && pnpm test && pnpm build` clean
3. HandlerDetailPage shows "Smoke test" tab
4. Paste 3 JSON lines, click Run → outcomes table renders 3 rows with correct status mapping
5. Fixtures picker accepts `.jsonl`, `.json` (array), `.csv`, and directories
6. Picking a file with no rows shows "no rows found in fixtures path"
7. Row count input clamped to 1..=100
8. Active-run gate: if an exec attempt is running using this handler, Run is rejected with HandlerBusy
9. Build failure surfaces the build stderr (same panel style as Plan 8 LastBuildSection)
10. stderr tail collapsible block shows handler's stderr output (last 4 KiB)
11. HUMAN_SMOKE Plan 13 walkthrough added
12. Spec docs (part-5 + part-7 + part-8 en + zh-Hant) updated

## 11. Open questions

None at design time.
