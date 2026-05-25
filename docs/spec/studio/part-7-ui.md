# Part 7 — UI

Defines the UI layer of `rowforge-studio`: technical stack, design language,
information architecture, primary flows, state-to-visual mapping, interaction
patterns, empty/boundary states, and loading policy.

This part is **prescriptive about contracts** (color tokens for state, what
the UI must not do, what data is push vs pull) and **suggestive about
visuals** (component library, density, exact pixel values). The visual
suggestions exist so the v1 build does not have to re-decide; reasoned
deviations are fine.

References to other parts are inline. A consolidated cross-reference
table is at §7.12.

## 7.1 Stack & component library

- **Shell:** Tauri (per Part 1 §1.3); webview-hosted React.
- **Component library:** **shadcn/ui + Radix Primitives + Tailwind CSS**.
  - Copy-paste components ⇒ no out-of-tree breaking changes (aligns with
    Part 5 §5.7 internal-crate, in-tree lockstep policy).
  - Radix gives full keyboard + accessibility primitives across macOS /
    Linux / Windows webviews.
  - Tailwind utility-first matches a high-density inspector UI; works
    cleanly with `font-variant-numeric: tabular-nums` and monospace
    stacks.
- **Virtualization:** `@tanstack/react-virtual` for event tails and
  failed-row pages (§7.6).
- **Icons:** `lucide-react`. No illustration assets in v1 (§7.7).

This is the recommended stack; the contract is only that the chosen stack
expose stable Radix-equivalent keyboard primitives. A future switch to a
different lib is not a breaking spec change.

## 7.2 Design principles

1. **Information density first, animation last.** This is a tool, not a
   consumer app. Animation is reserved for state-transition cues
   (e.g. `Cancelling → Aborted` per Part 3 §3.3), not decoration.
2. **Identity fields are monospace, always.** `ExecutionId`, `AttemptId`,
   `HandlerInstanceId`, `worker_id`, `seq`, `row_index`, byte offsets,
   filesystem paths, error codes, SHA digests, `handler_version`.
3. **Semantic color tokens are global.** A given `RowOutcomeKind` uses
   the same token in the progress region, event tail, failed-row table,
   `ExecRollup`, and `RowHistory`. The user never has to re-learn the
   palette.
4. **Live numbers use `tabular-nums` + ≤ 150 ms transitions.** Tick-driven
   counters (`processed`, `rate_1s`, `rate_10s`, `in_flight`,
   `queue_depth`, `eta_ms`; see Part 6 §6.1) must not jitter column
   width. Updates ease, do not snap.
5. **Dark is the default, light is an equal peer.** Long-running batch
   tooling lives on a dark surface. Every semantic token publishes
   dark + light values.
6. **Destructive actions need explicit friction.** Force kill (Part 3
   §3.5), workspace replacement, and orphan mark-aborted go through an
   `AlertDialog` with an explicit confirmation token (e.g. typing the
   exec name prefix), not a single click.

## 7.3 Information architecture

### Page tree (v1)

- **Workspace Picker / Boot** — shown when `Settings.workspace_root` is
  `None` or unreadable. Entity: `Workspace`. Calls: `workspace_open`,
  `workspace_settings_load`.
- **Workspace Home (Exec list)** — default landing. Entity:
  `Vec<ExecSummary>`. Call: `exec_list`. Includes "New execution" CTA and
  a **Select** mode toggle in the header (Plan 10). Column order:
  [checkbox (Select mode only)] | Name | Rows | Attempts | Size | Created.
  Name cells show a `title={exec_id}` hover tooltip. Active-run rows
  show a disabled checkbox with tooltip "Cancel active run first" (detected
  via `last_attempt_state === "running"`). Selecting ≥ 1 row enables a
  "Delete N execution(s)" button that opens `DeleteExecutionsDialog`. A
  yellow alert above the table displays per-item failures from a partial
  bulk delete; it is dismissed by clicking "Dismiss".
- **New Execution Wizard** (modal-as-route `/new`) — entity:
  `StartExecArgs`. Calls: `manifest_validate`, `exec_start`,
  optional `run_start`.
- **Execution Detail** `/exec/:id` — entity: `ExecDetail`. Call:
  `exec_show`. Tabs:
  - **Attempts** (default) — renders `ExecDetail.attempts`.
  - **Rollup** — entity: `ExecRollup`. Call: `exec_rollup`. Cold-loaded
    (Part 2 §2.2.5, Part 4 §4.3).
  - **Bindings** — read-only view of `handler_binding`, `field_mapping`,
    `config_overrides`.
  - **404 fallback (Plan 10):** when `exec_show` returns `NotFound` (e.g.
    the execution was deleted from another window or by the CLI), the page
    renders "This execution has been deleted or is unavailable." with a
    ← Back link to `/` instead of the normal detail view. This extends the
    existing `isError` branch — no new route is required.
- **Attempt Detail** `/exec/:id/attempt/:aid` — entity: `AttemptDetail`.
  Call: `attempt_show`. Sub-tabs:
  - **Live / Summary** — counters + Phase chip bar; subscribes to
    `run:<handle>` when this attempt is an active run.
  - **Failed rows** — entity: `FailedRowPage`. Call:
    `attempt_failed_page`. Cursor-style pagination (Part 2 §2.2.6;
    `limit ≤ 500`).
  - **Errors by code** — renders `AttemptDetail.by_error_code` (with
    `OTHER` overflow at 32).
  - **Logs** — handler stderr/stdout log tail. Bootstrap via
    `handler_log_tail`; live via `handler_log_subscribe` (Plan 9 §7.4
    Flow K).
  - **Artifacts** — `AttemptDetail.paths` with "Reveal in Finder" per
    Part 2 §2.3.
- **Row History drawer** — opened from Failed rows. Entity: `RowHistory`.
  Call: `attempt_row_history`. Strictly on-demand (Part 2 §2.2.7).
- **Run launcher** (Execution Detail header) — primary "Run" button
  for quick-start with defaults, plus a settings-icon button that
  opens an inline options panel for `RunOpts`. The panel exposes:
  handler directory (persisted across sessions via localStorage),
  "Sample first N rows" (`row_limit`), "Workers" override,
  "Skip rows already attempted" checkbox (drives `skip_attempted`,
  uses `RowResolution.attempted_seqs` to deduplicate sampling
  across runs), and "Dry run". The panel header also shows
  `total / attempted / fresh` row counts from `exec_rollup`, plus
  a live preview "Will dispatch N rows" computed from the current
  state. Calls `run_start`.
- **Export dialog** — constructs `ExportOpts`, calls `exec_export`.
- **Settings** `/settings` — entity: `Settings`. Calls:
  `workspace_settings_load`, `workspace_settings_save`, `run_active`
  (via the workspace switch button), `workspace_open` (when switching).

  Layout: four sections in a single-column form.
  1. **Workspace** — current `workspace_root` shown as read-only mono
     text; **Switch workspace…** button opens a directory picker.
     Button is disabled with an amber warning when `run_active().len()
     > 0` (refresh interval 2s while the page is mounted) — switching
     would orphan in-flight runs.
  2. **Concurrency** — `max_concurrent_runs` number input. A blue
     "Will apply on next workspace open" banner appears below it when
     its value differs from the loaded server value, since the field
     is only consumed at `workspace_open` time (Part 5 §5.6).
     (Per-run worker count is set in the RunButton options panel,
     not here — `Settings.default_workers` was removed as dead code:
     nothing in studio-core's `start_run` ever read it.)
  3. **Telemetry** — `telemetry_opt_in` checkbox.
  4. **Logs** (Plan 9) — `handler_log_capture_raw_stdout` toggle
     ("Capture raw stdout"). Label subtext: "When enabled, handler
     stdout lines that contain valid outcome JSON are also written to
     handler_log.log. Increases file size." Default off. Takes effect
     on the next run (not retroactive).

  Save / Cancel buttons at the bottom. Save persists via
  `workspace_settings_save` and invalidates the cached query; Cancel
  restores from the loaded value.

### Anchored, not built in v1

- **Sidebar "Authoring" group** — *was* a disabled "Coming soon" badge
  in this part. **Part 8 supersedes this**: in v1 the Authoring group
  is active and contains a Handlers route. The remaining anchored
  items below (Manifest editor, Pack) still apply.
- **`/handlers` and `/handlers/:name`** — **Plan 7: both routes are active
  in v1.** The sidebar Handlers item is enabled; the list and detail pages
  are shipped. See Part 8 §8.6.1 for full IA details and §7.4 Flows E-H
  below for the associated user flows.
- **`ListFilter` filter bar** — reserved area above the exec list;
  hidden in v1 (Part 5 §5.2 `ListFilter`).
- **`HandlerSource` picker** — `Dir` only in v1; the picker is a single
  field. v2 upgrades to a segmented control (`Dir` / `Sandbox`) without
  changing layout (Part 5 §5.4).

### Global navigation

- **Left sidebar (persistent):** Workspace group (Executions, Settings)
  + Authoring group (Handlers active — Plan 7).
- **Top header (persistent):** workspace name + path tooltip; breadcrumb
  (`Executions / <exec> / Attempt #N / Failed rows`); **Active runs pill**
  on the right.
- **Active runs pill:** consumes `active_runs_stream()` (Part 5 §5.2) and
  the `runs:active` Tauri event (Part 5 §5.5, Part 6 §6.6). Shows
  `n running`; hover expands to per-run mini-progress; click jumps to
  that attempt's Live tab. Hidden when `n = 0`.
- **No floating window, no dock badge in v1.** Multi-run UX is the
  header pill plus per-tab spinner dots; this is sufficient for the
  ≤ 3-runs concurrent default (Part 3 §3.4).

## 7.4 Primary user flows

Each step lists the Tauri command (Part 5 §5.5) it invokes.

### Flow A — New execution, first run (empty workspace)

| # | Step | Command |
|---|---|---|
| 1 | Boot → Workspace Picker (empty-state detected) | `workspace_settings_load` |
| 2 | Pick workspace folder → save | `workspace_settings_save`, `workspace_open` |
| 3 | Workspace Home empty state → "New execution" | — |
| 4 | Wizard step 1: name + input path | — |
| 5 | Wizard step 2: handler dir + "Validate" | `manifest_validate` |
| 6 | Submit → routed to Execution Detail | `exec_start` |
| 7 | Click "Run" → configure `RunOpts`, submit | `run_start` |
| 8 | Auto-route to Attempt Detail (Live); subscribe | event `run:<handle>` |

Step 7 corresponds to the `Starting → Running` transition
(Part 3 §3.3). Sessions register directly into `Starting`.

### Flow B — Observe and cancel a live run

| # | Step | Command |
|---|---|---|
| 1 | Click an active run in the header pill | `run_active` |
| 2 | Land on Attempt Detail (Live); progress + tail | event `run:<handle>` |
| 3 | Click "Cancel" (destructive style) | — |
| 4 | Confirm dialog → soft cancel | `run_cancel(handle, Soft)` |
| 5 | UI shows `Cancelling` + "n rows in flight" countdown | event Tick |
| 6 | After 10 s, "Force kill" button fades in | — |
| 7 | High-friction confirm (typed token) → hard kill | `run_cancel(handle, Hard)` |
| 8 | `Aborted { reason: UserCancelled }` → final summary | event |

Strict trace of Part 3 §3.5 soft/hard semantics.

### Flow C — Inspect failures, retry failed only

| # | Step | Command |
|---|---|---|
| 1 | Execution Detail → Attempts → click a `Done` attempt | `exec_show` |
| 2 | Failed rows tab → load first page | `attempt_failed_page({offset:0, limit:200})` |
| 3 | Top summary from `by_error_code` (cached) | (already in `attempt_show`) |
| 4 | Scroll → "Load more" with `next_offset` | `attempt_failed_page` |
| 5 | Click a row → Row History drawer | `attempt_row_history` |
| 6 | Click "Retry failed only" | — |
| 7 | Run launcher with `retry_failed=true` pre-checked, confirm | `run_start` |
| 8 | Auto-route to new attempt's Live tab | event |

`FailedPageQuery` semantics per Part 2 §2.2.6.

### Flow D — Cross-attempt rollup and export

| # | Step | Command |
|---|---|---|
| 1 | Execution Detail → Rollup tab | `exec_show` (cached) |
| 2 | Cold loading skeleton (Part 2 §2.2.5) | `exec_rollup` |
| 3 | Render `resolved / failed_last / crashed_last / too_large / never_attempted` + `by_error_code` | — |
| 4 | "Export" → dialog | — |
| 5 | Pick `format = Both`; check `require_complete` | — |
| 6 | Confirm → progress toast | `exec_export` |
| 7 | On done, toast offers "Reveal output dir" via `ExportReport.output_dir` | — |

### Flow H — Scaffold a new handler (Plan 7)

| # | Step | Command |
|---|---|---|
| 1 | Sidebar Handlers → `/handlers` | `handler_list` |
| 2 | Click "New Handler" → `ScaffoldDialog` opens | — |
| 3 | Enter name (regex `/^[a-z0-9][a-z0-9-]*$/`, validated client-side), choose template (`GoStdio` / `GoBatch` / `Empty`), enter `primary_field` | — |
| 4 | Submit → `handler_scaffold` mutation | `handler_scaffold` |
| 5 | On success: toast + close dialog + navigate to `/handlers/<name>` | `handler_show` |

Negative paths: `HandlerExists` / `InvalidHandlerName` errors render inline in the dialog; dialog stays open until the user corrects the name or cancels.

### Flow I — Rename a handler (lazy, Plan 7)

| # | Step | Command |
|---|---|---|
| 1 | `/handlers/:name` → "Rename…" → `RenameHandlerDialog` (pre-fills current name) | — |
| 2 | Edit name (must differ from current and pass regex) → click "Rename" | — |
| 3 | `handler_rename` mutation | `handler_rename` |
| 4 | On success: toast + close dialog + navigate to `/handlers/<new-name>` | — |

Lazy semantics: SQLite is untouched; past `ExecSummary.last_handler_dir` rows still reference the old name (informational, not load-bearing — see Part 2 §2.2.2 lazy rename note).

### Flow J — Delete a handler (typed-token confirm, Plan 7)

| # | Step | Command |
|---|---|---|
| 1 | `/handlers/:name` → "Delete…" → `DeleteHandlerDialog` | — |
| 2 | User types the exact handler name (case-sensitive) to enable the Delete button | — |
| 3 | Click "Delete" → `handler_delete` mutation | `handler_delete` |
| 4 | On success: toast + close dialog + navigate to `/handlers` | `handler_list` |

Lazy semantics: past `last_handler_dir` references in `executions` rows survive the delete.
Symlink defense: three layers — (1) regex validate name, (2) canonicalize resolved path, (3) assert `starts_with` workspace `handlers/` parent.

### Flow K — View handler log (Logs tab, Plan 9)

| # | Step | Command |
|---|---|---|
| 1 | Navigate to Attempt Detail; click **Logs** tab | — |
| 2 | Tab mounts → bootstrap load | `handler_log_tail(exec_id, attempt_id, 5000)` |
| 3 | If `handler_log.log` absent → show "No log file. This attempt predates Plan 9 log capture." | — |
| 4 | If file exists but empty → show "Handler has not produced any output yet." | — |
| 5 | If attempt is still running (`isLive`): subscribe for live lines | `handler_log_subscribe(exec_id, attempt_id)` |
| 6 | Live lines arrive via event `handler_log:<attempt_id>` within ~100 ms | event |
| 7 | Worker chip multi-select filter → list narrows to selected workers | — |
| 8 | Stream filter (stdout / stderr / both) → further narrows | — |
| 9 | Text search (substring) → further narrows | — |
| 10 | Filter matches nothing → "No lines match the current filters." | — |
| 11 | Auto-scroll on: viewport stays at bottom as new live lines arrive | — |
| 12 | User scrolls up manually → auto-scroll disengages | — |
| 13 | Click **Pause**: live lines buffer internally, visible list frozen | — |
| 14 | Click **Resume**: buffered lines flush into the visible list, auto-scroll re-engages | — |
| 15 | `dropped > 0` in a batch payload → amber banner "⚠ N log lines dropped — open the log file for full content" | — |
| 16 | Click **Reveal log file** → OS file manager opens at `<attempt_dir>/handler_log.log` | `shell::open` |
| 17 | Tab unmounts or attempt finishes → | `handler_log_unsubscribe(attempt_id)` |

**Component breakdown:**
- `LogsToolbar` — worker chips, stream toggle, search input, Pause/Resume button, Reveal button.
- `LogsVirtualList` — `@tanstack/react-virtual` list; each row 28 px; colored stream chip (yellow stderr / blue stdout); monospace content.
- `AttemptLogsTab` — orchestrates bootstrap, live subscription, filter composition, dropped banner.

### Flow L — Select mode + bulk delete (Plan 10)

| # | Step | Command |
|---|---|---|
| 1 | Exec list header → click **Select** | — |
| 2 | Checkbox column appears left of Name; each row has a checkbox; Cancel button replaces Select in header | — |
| 3 | Active-run rows have a **disabled** checkbox; hover shows "Cancel active run first" (detected via `last_attempt_state === "running"`) | — |
| 4 | Click a non-disabled row → toggles selection; row click no longer navigates in Select mode | — |
| 5 | Click **Cancel** → exits Select mode and clears all selections | — |
| 6 | Select ≥ 1 row → **Delete N execution(s)** button appears in header | — |
| 7 | Click Delete N → `DeleteExecutionsDialog` opens: title "Delete N execution(s)?"; lists up to 10 selected names + "… and M more"; shows total size; destructive **Delete** button | — |
| 8 | Confirm → mutation | `execution_delete_bulk(exec_ids)` |
| 9 | On success: Sonner toast "N execution(s) deleted"; `exec_list` query invalidated; dialog closes; Select mode exits | event `exec_list:refresh` |
| 10 | Partial failure: yellow alert above table shows which exec_ids failed and why; Dismiss button clears alert | — |
| 11 | ExecDetail page for a deleted-elsewhere exec: `exec_show` returns `NotFound`; page renders "This execution has been deleted or is unavailable." + ← Back link | `exec_show` |

**Component breakdown:**
- `DeleteExecutionsDialog` — shadcn `AlertDialog`; item list (max 10 + overflow count); total size via `formatBytes`; destructive confirm button.
- `useExecutionDelete` — single-delete mutation hook; invalidates `exec_list` on success.
- `useExecutionDeleteBulk` — bulk-delete mutation hook; invalidates `exec_list` on any successful delete; exposes `bulkFailures` state.
- `formatBytes` helper — lives in `apps/rowforge-studio/src/lib/format.ts`; shared by dialog and ExecList Size column.

## 7.5 Color & state mapping

The mapping table below is normative for v1.

### `RunStatus` (Part 3 §3.3)

| RunStatus | Token | Hex (dark) | Visual | Icon (lucide) |
|---|---|---|---|---|
| Starting | `info-500` | `#3B82F6` | blue dot + spinner | Loader2 |
| Running | `success-500` | `#10B981` | green dot + heartbeat | Play |
| Cancelling | `warning-500` | `#F59E0B` | amber dot + spinner | Loader2 + Slash |
| Done | `success-600` | `#059669` | solid green dot | CheckCircle2 |
| Aborted | `neutral-400` | `#9CA3AF` | gray dot + strike | XCircle |
| Crashed | `error-500` | `#EF4444` | red dot + jagged border | AlertOctagon |

### `RowOutcomeKind` (Part 2 §2.2.6)

| Kind | Token | Hex | Use |
|---|---|---|---|
| Success | `success-500` | `#10B981` | green left border 2 px |
| Error | `error-500` | `#EF4444` | red left border 2 px + error-code chip |
| Crash | `error-700` | `#B91C1C` | deeper red + AlertOctagon + `WORKER_CRASH` chip |
| TooLarge | `warning-600` | `#D97706` | amber + FileWarning icon |

### `Phase` (Part 6 §6.1)

A horizontal **chip bar** in the Attempt Detail header. Current phase
highlighted; completed phases checkmarked + dimmed; future phases muted.
Phases: `Initializing → Snapshotting → Starting → Running → Cancelling
(conditional) → Persisting`.

| Phase | Chip | Icon |
|---|---|---|
| Initializing | neutral spinner | Settings2 |
| Snapshotting | info spinner | Camera |
| Starting | info spinner | Power |
| Running | success outline (active) | Activity |
| Cancelling | warning solid | StopCircle |
| Persisting | info spinner | Save |

## 7.6 Key interaction patterns

### 7.6.1 Progress region (Part 6 §6.7)

Three-column grid, driven by 4 Hz `Tick` (Part 6 §6.2). Updates use 150 ms
ease; column widths frozen by `tabular-nums`.

- **Left:** progress bar (`h-3`, `rounded-full`, `success-500` fill on
  `neutral-800` track) + `processed / total (xx.x%)` below. If
  `total = None` (input not snapshotted; Part 6 §6.1), hide the
  percent and render `processed —`.
- **Center:** two large numbers `rate_1s` / `rate_10s` (`text-2xl
  tabular-nums`) with `rows/s` subtext; `ETA` large countdown. While
  the 10 s buffer is filling, show `—`.
- **Right:** stacked `in_flight` (Activity icon) and `queue_depth`
  (Layers icon).
- **Heartbeat:** on each Tick a 1 px white highlight flashes on the
  progress bar trailing edge (100 ms). Conveys "events flowing" even
  when counters do not move.

### 7.6.2 Event tail (Part 6 §6.2)

A 200-entry virtualized list. Each row 28 px tall, monospace fields,
left-edge 3 px color band keyed to `RowOutcomeKind` (§7.5).

Columns: `[seq#]` · `row_index` · error-code chip · message (truncate) ·
`dur_ms` (right-aligned, `tabular-nums`).

Filter chips top-right: `All / Errors only / Crashes only`. **Default
is "Errors only"** because 90 % of the `OutcomeSample` token budget is
errors/crashes (Part 6 §6.2).

New entries insert at the top; the tail fades out at the bottom.

### 7.6.3 Cancel two-phase (Part 3 §3.5)

- **Confirm soft cancel:** `AlertDialog` text "Soft cancel? In-flight
  rows will finish."
- **`Cancelling` state:** amber sticky banner "Cancelling — `n` rows in
  flight"; `n` updated from `Tick.in_flight`. A 10 s circular countdown
  next to `in_flight`.
- **After 10 s:** "Force kill" red outline button fades in (Part 3 §3.5
  recommended threshold).
- **Hard kill confirm:** destructive `AlertDialog` "Partial outcomes may
  be lost. This cannot be undone." User must type the first 4 chars
  of the exec name. High friction is the point.

### 7.6.4 Lifecycle banners (`WorkerCrashed`, `StallWarning`, `PipelineWarning`)

All three render **inline in the event tail** at full row width (48 px,
not the standard 28 px) to break the visual rhythm. A simultaneous
side toast (bottom-right, 5 s auto-dismiss) ensures the user sees it
when not on the Live tab.

- `WorkerCrashed`: red background + AlertOctagon + `worker_id` +
  first 3 lines of `stderr_tail` collapsed; click expands a right-side
  Sheet with the full ≤ 64 KiB tail (Part 6 §6.1).
- `StallWarning`: amber + Hourglass + `silent_secs`.
- `PipelineWarning`: blue + Info + `code` + `message`.

### 7.6.5 `EVENT_LAG` sticky banner

Sub-case of `PipelineWarning` (Part 6 §6.2). A persistent banner at the
top of the event tail:

> Display lagging — `n` events dropped. Counters are still accurate.
> [Open `outcomes.jsonl`]

The "Open" link uses `AttemptDetail::paths.outcomes_jsonl` (Part 2 §2.3)
via Tauri `shell::open`. Auto-dismisses after 30 s with no further lag.

The "Counters are still accurate" line is contractual; it tells the user
which surfaces to trust (the durable counts in `Tick`, not the sampled
tail).

### 7.6.6 Failed-row table

- Columns: `seq` · `row_index` · `kind` (chip) · `error_code` (mono chip)
  · `message` (truncate; hover full) · `dur_ms` (right `tabular-nums`).
- Click a row → in-place accordion expands, rendering `raw_record` as a
  collapsible JSON tree (monospace, syntax-highlighted).
- Pagination: **cursor-style only** ("Load more" with `next_offset`).
  v1 does **not** render `n / m` page numbers because
  `FailedRowPage::total_known` is typically `None` without the v2 index
  (Part 4 §4.4).
- "Reveal in Finder" top-right, opens `paths.outcomes_jsonl`.

## 7.7 Empty / boundary states

| # | State | Trigger | What to show | Allowed actions |
|---|---|---|---|---|
| 1 | Empty workspace | `exec_list` → `[]` | Icon + "No executions yet" + primary CTA | New execution; switch workspace |
| 2 | Exec never run | `ExecDetail.attempts == []` | "This execution has never been run" + Run CTA; Rollup tab disabled; Failed rows hidden | Run; view bindings |
| 3 | Attempt all-success | `failed + crashed + too_large == 0` | Success icon + "All rows resolved in this attempt"; Errors-by-code hidden | Back; Rollup; Export |
| 4 | Schema mismatch | `workspace_open → WorkspaceLocked` (Part 5 §5.3) | Full-page blocking modal: `Workspace.schema_version` vs Studio version + "Open different workspace" + "Copy details" | Switch workspace; quit |
| 5a | `RunBusy` (PerExec) | `run_start → RunBusy { scope: PerExec }` | Inline error in Run launcher + link to active attempt | Jump to active; cancel then retry |
| 5b | `RunBusy` (Workspace) | `run_start → RunBusy { scope: Workspace }` | Toast: "Workspace concurrent-run limit reached (3)" + links to Active runs / Settings | Open Active runs; raise limit |
| 6a | Orphan, idle > 5 min | `open` auto-marked aborted (Part 3 §3.7) | Banner at Home top: "N attempt(s) were marked aborted on launch (orphaned)" + Review link | Dismiss; review; retry-failed |
| 6b | Orphan, idle ≤ 5 min | Ambiguous; CLI may be running | Amber banner on Attempt: "This attempt may still be running externally" + Mark-aborted + Refresh | Mark; refresh; wait |
| 7 | Manifest invalid | `manifest_validate → ManifestReport.errors` | Inline `ManifestError` list under handler picker; submit disabled | Fix file; re-validate |
| 8 | Cancel stuck > 10 s | `RunStatus::Cancelling` over threshold | Red sticky bar + Force kill button + high-friction confirm | Wait; force kill |

Cases 4, 5a, 5b, 6a, 6b, 7 are direct reflections of contracts in
Part 3 / Part 5; the UI is the only surface that makes them legible.

## 7.8 Loading policy and time budgets

Backend cost classes (Part 2 §2.1, Part 4 §4.3) translate to UI patterns:

| Surface | Cost | Budget | UI pattern |
|---|---|---|---|
| `workspace_open`, header workspace name | hot | < 10 ms | render direct |
| Exec list switch / filter | warm (mtime hit) | < 100 ms | render direct, no skeleton |
| Attempt Detail (terminal) | warm | < 100 ms | render direct |
| Attempt Detail (running) | hot (aggregator snapshot) | < 50 ms | render + subscribe |
| `ExecRollup` | cold (linear scan all attempts) | 1–10 s | indeterminate progress + "Streaming N attempts..." |
| `FailedRowPage` page N | cold, linear in offset | 100 ms (early) → seconds (late) | cursor "Load more", never page numbers |
| `RowHistory` (one row) | cold, linear in attempt count | < 1 s typical | spinner in drawer |
| `manifest_validate` | warm | < 500 ms | inline live-validation |

**Loading widgets:**
- **Spinner** (Loader2 rotate) — non-blocking < 500 ms operations.
- **Skeleton** (`bg-neutral-800 animate-pulse`) — structured loads:
  ExecSummary table rows, ExecDetail header, AttemptDetail stats grid.
- **Determinate progress bar** — only for `Tick.processed / total`
  (Part 6 §6.1) inside the Live tab.
- **Indeterminate linear bar** — `ExecRollup` (no total known mid-stream)
  and `exec_export` long writes.

**Illustrations:** none in v1. Empty states use a single lucide icon
(neutral-600) + heading + subhead + CTA. Reasons: bundle weight, tonal
consistency for a tool, and sprint cost.

## 7.9 `UiError` presentation table (Part 5 §5.3)

| Variant | Surface | Notes |
|---|---|---|
| `NotFound { kind, id }` | Inline empty state | Not a toast; the page itself is empty |
| `InvalidArg(String)` | Inline form-field error | Live; before submit when possible |
| `HandlerBuildFailed { stderr }` | Modal / right Sheet | Scrollable stderr + copy button |
| `RunAborted { reason }` | Banner on Attempt Detail | Branch by `AbortReason` (see §7.6.4 + §7.6.3) |
| `UnknownHandle(String)` | Toast (info) + auto-refresh `run_active` | Handle expired; recover quietly |
| `WorkspaceLocked { by }` | Full-page blocking modal | App-level; nothing else is usable |
| `ManifestInvalid { errors }` | Side panel list + per-error inline | v2 manifest editor |
| `RunBusy { execution_id, scope }` | Inline disabled button + tooltip (PerExec); toast (Workspace) | No retry-loop; user must resolve |
| `Io(String)` | Toast (error) + copy details | Usually transient |
| `Internal(String)` | Toast (error) + copy details + "Report issue" | Backend bug; UI does not explain |
| `EditorNotFound` | Toast (error) + link to Settings → Editor | Plan 7; `handler_open_editor` only |
| `HandlerNotFound { name }` | `/handlers/:name` inline empty state with back link | Plan 7; stale bookmark or concurrent delete |
| `HandlerExists { name }` | Inline banner in ScaffoldDialog / RenameHandlerDialog | Plan 7; name already taken |
| `InvalidHandlerName { name }` | Inline field error in ScaffoldDialog / RenameHandlerDialog | Plan 7; fails `/^[a-z0-9][a-z0-9-]*$/` |
| `ExecutionInUse { exec_id }` | Checkbox disabled in ExecList select mode + tooltip "Cancel active run first"; yellow alert for bulk partial failure | Plan 10; active-run guard in `execution_delete` |

`AbortReason` (Part 6 §6.5) is a discriminated union of at least 9
variants; the Aborted banner branches into reason-specific detail
panels (e.g. `AllWorkersCrashed` opens a list of `WorkerCrashRecord`
entries; `SnapshotHashMismatch` shows `expected` vs `actual` digest;
`MissingRequiredInput` lists columns).

## 7.10 Things the UI must NOT do

These are spec-contract violations that the UI must refuse to render,
no matter how reasonable they sound to a designer.

1. **No real-time per-row outcome stream.** `OutcomeSample` is sampled
   (20 / s, 90 % errors; Part 6 §6.2). For every row, read
   `outcomes.jsonl` post hoc.
2. **No `ExecRollup` on an in-progress attempt.** Cold-only; `meta.json`
   for the in-progress attempt does not yet exist (Part 2 §2.2.5,
   Part 4 §4.3).
3. **No "resume orphan" action.** Studio can only mark aborted; reruns
   go through `--retry-failed` on a fresh attempt (Part 3 §3.7).
4. **No second concurrent run on the same execution.** The UI must
   block the Run button when the per-exec limit is reached, not let
   the user click and receive `RunBusy` (Part 3 §3.4).
5. **No per-row × per-attempt matrix.** Use `RowHistory` on demand
   (Part 2 §2.3).
6. **No cross-run merged timelines or comparison charts.** Out of scope
   (Part 6 §6.6).
7. **No "page N of M" pagination on failed rows.** `total_known` is
   typically `None` in v1; cursor-style only (Part 4 §4.4).
8. **No "100.0 %" before the total is known.** `Tick.total` is
   `Option<u64>` (Part 6 §6.1); render `processed —` instead.
9. **No direct read of `outcomes.jsonl` from UI code.** All reads go
   through projections in `studio-core` (Part 2 §2.3, Part 5 §5.2).
   `AttemptDetail::paths` is solely for "Reveal in Finder."
10. **No `subscribe_all_runs` multiplex.** Use `active_runs_stream()`,
    a counters-only roll-up (Part 5 §5.2, Part 6 §6.6).

## 7.11 Settings surface

The Settings page exposes `Settings` (Part 2 §2.2.9) one field per row.

- `workspace_root` — read-only display; "Switch workspace" opens picker.
- `max_concurrent_runs` — number input, default 3 (Part 3 §3.4).
  Lowering below current active count shows a confirmation warning.
- `telemetry_opt_in` — switch, default off; tooltip notes telemetry is
  not collected in v1.
- **`preferred_editor`** (Plan 7) — text input, placeholder `"code"`.
  Optional; when empty the resolver falls through to `$VISUAL` / `$EDITOR`
  / probes (Part 8 §8.4.1). Displayed in a fourth "Editor" section of
  the Settings form. Saves via `workspace_settings_save` and takes effect
  immediately on the next `handler_open_editor` call (no restart required).

Note: `default_workers` is **not** a Settings field. Per-run worker
count is configured in the RunButton options panel; nothing in
studio-core's `start_run` ever consumed a workspace-global default,
so the field was removed.

No advanced JSON editor in v1. Path resolution is in the Tauri layer
(Part 5 §5.6).

## 7.12 Cross-references summary

| §7.x | Depends on |
|---|---|
| 7.1 stack | Part 1 §1.3 architecture; Part 5 §5.7 stability policy |
| 7.2 principles | Part 1 §1.2 principles; Part 6 §6.1 event taxonomy |
| 7.3 IA | Part 1 §1.4 scope; Part 2 §2.1 entity inventory; Part 5 §5.5 commands |
| 7.4 flows | Part 3 §3.3 state machine; Part 3 §3.5 cancel; Part 5 §5.2 API |
| 7.5 color | Part 3 §3.3 `RunStatus`; Part 2 §2.2.6 `RowOutcomeKind`; Part 6 §6.1 `Phase` |
| 7.6.1 progress | Part 6 §6.1 `Tick`; §6.2 4 Hz budget; §6.7 metrics |
| 7.6.2 event tail | Part 6 §6.2 token-bucket sampling |
| 7.6.3 cancel | Part 3 §3.5 soft/hard, 10 s threshold |
| 7.6.4 banners | Part 6 §6.1 lifecycle events; §6.5 `WorkerCrashRecord` |
| 7.6.5 `EVENT_LAG` | Part 6 §6.2 `PipelineWarning { code: "EVENT_LAG" }` |
| 7.6.6 failed table | Part 2 §2.2.6 `FailedRow`, `FailedPageQuery`; Part 4 §4.4 v2 index |
| 7.7 boundary states | Part 1 §1.5; Part 3 §3.4 / §3.7; Part 5 §5.3 |
| 7.8 loading | Part 2 §2.1 cost classes; Part 4 §4.3 caching tiers |
| 7.9 errors | Part 5 §5.3 `UiError`; Part 6 §6.5 `AbortReason` |
| 7.10 must-not | Part 2 §2.3; Part 3 §3.4 / §3.7; Part 4 §4.3; Part 6 §6.2 / §6.6 |
| 7.11 settings | Part 2 §2.2.9 `Settings`; Part 5 §5.6 |

## 7.13 Wireframes (illustrative)

ASCII; rough proportions only. Dimensions ~ 96 chars wide. Real layouts
go through Figma later; these exist so reviewers can argue about
information density and grouping before pixels are touched.

### W-1 Workspace Home (Exec list)

```
┌──────────────────────────────────────────────────────────────────────────────────────────────┐
│  ◇  billing-workspace ▾    Executions                              ◯ 2 running ▾    + New    │
├──────────────┬───────────────────────────────────────────────────────────────────────────────┤
│ WORKSPACE    │  Executions                                                                   │
│ ● Executions │  ┌─────────────────────────────────────────────────────────────────────────┐  │
│   Settings   │  │ Name          Created       Rows     Last attempt  Attempts             │  │
│              │  ├─────────────────────────────────────────────────────────────────────────┤  │
│ AUTHORING    │  │ refund-bf-3   2026-05-22    12,043   ● Running     3            ⏵ open  │  │
│ ░Handlers░   │  │ refund-bf-2   2026-05-21    12,043   ✓ Done        5            ⏵ open  │  │
│  Coming soon │  │ refund-bf-1   2026-05-20    12,043   ✓ Done        4            ⏵ open  │  │
│              │  │ apple-rfd     2026-05-19       487   ✗ Aborted     2            ⏵ open  │  │
│              │  │ billing-test  2026-05-18         3   ⊘ Crashed     1            ⏵ open  │  │
│              │  │ smoke-tiny    2026-05-18         3   — never run   0            ⏵ open  │  │
│              │  └─────────────────────────────────────────────────────────────────────────┘  │
│              │  Showing 6 of 6 · sorted by created desc                                      │
└──────────────┴───────────────────────────────────────────────────────────────────────────────┘
```

States: ● Running, ✓ Done, ✗ Aborted, ⊘ Crashed, — never run.
Active runs pill expands on hover (W-2 inset).

### W-2 Active runs pill (hover popover)

```
                                           ┌─────────────────────────────────┐
                                ◯ 2 running│ Active runs                     │
                                           │ ─────────────────────────────── │
                                           │ refund-bf-3  ▓▓▓▓▓░░░  62%  ⏵   │
                                           │   rate 980/s · ETA 1m 04s       │
                                           │ apple-rfd-2  ▓▓░░░░░░  18%  ⏵   │
                                           │   rate  84/s · ETA 4m 22s       │
                                           └─────────────────────────────────┘
```

### W-3 Execution Detail — Attempts tab

```
┌──────────────────────────────────────────────────────────────────────────────────────────────┐
│  Executions / refund-bf-3                                          ◯ 2 running ▾   ▸ Run     │
├──────────────┬───────────────────────────────────────────────────────────────────────────────┤
│ ● Executions │  refund-bf-3      input: refund_records_dump.csv (12,043 rows)                │
│   Settings   │  handler: golang-refund-backfill 0.1.0   created: 2026-05-22 09:14            │
│ ░Handlers░   │                                                                               │
│              │  ┌─Attempts──Rollup──Bindings──Artifacts────────────────────────────────────┐ │
│              │  │                                                                          │ │
│              │  │ #  State        Started        Run type    success / failed / crashed   │ │
│              │  │ ── ──────────── ───────────── ─────────── ──────────────────────────── ──│ │
│              │  │ 3  ● Running    05-22 14:02   full          7,489  /     12  /     0  ⏵ │ │
│              │  │ 2  ✓ Done       05-22 11:30   retry-failed    412  /      0  /     0  ⏵ │ │
│              │  │ 1  ✗ Aborted    05-22 10:18   full          5,820  /    387  /    24  ⏵ │ │
│              │  │                                                                          │ │
│              │  └──────────────────────────────────────────────────────────────────────────┘ │
└──────────────┴───────────────────────────────────────────────────────────────────────────────┘
```

### W-4 Attempt Detail — Live tab

```
┌──────────────────────────────────────────────────────────────────────────────────────────────┐
│  Executions / refund-bf-3 / Attempt #3 / Live                      ◯ 2 running ▾   ■ Cancel  │
├──────────────┬───────────────────────────────────────────────────────────────────────────────┤
│ ● Executions │  Attempt #3   ● Running    started 05-22 14:02 (12m 04s ago)                  │
│              │                                                                               │
│              │  Phase:   ✓ Init  ✓ Snap  ✓ Start  ◉ Running  ·  Cancel  ·  Persist           │
│              │  ┌─Live──Failed rows──Errors by code──Artifacts──────────────────────────────┐│
│              │  │                                                                           ││
│              │  │ ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓░░░░░░░░░░░░░░░░  7,501 / 12,043   62.3%          ││
│              │  │                                                                           ││
│              │  │   980        950        1m 02s        12         3                       ││
│              │  │   rate/1s    rate/10s   ETA           in-flight  queue                   ││
│              │  │                                                                           ││
│              │  │ ┌─Recent events ───────────────────  [All] (Errors) [Crashes]            ││
│              │  │ │ [#7498]  row 7501  ● BILLING_NOT_FOUND  no billing row for billid   12ms││
│              │  │ │ [#7491]  row 7494  ● BILLING_NOT_FOUND  no billing row for billid   11ms││
│              │  │ │ [#7480]  row 7483  ● DB_ERROR           connection timeout          1.2s││
│              │  │ │ ─── WorkerCrashed  worker_id=2  signal=11  ─ click to expand ─────  ████││
│              │  │ │ [#7420]  row 7423  ● MISSING_BILLID     row has no 'billid'          2ms││
│              │  │ │ ...                                                                    ││
│              │  │ └────────────────────────────────────────────────────────────────────────││
│              │  └───────────────────────────────────────────────────────────────────────────┘│
└──────────────┴───────────────────────────────────────────────────────────────────────────────┘
```

### W-5 Cancelling state (10s threshold reached)

```
│  Attempt #3   ◐ Cancelling    soft cancel issued 11s ago                                     │
│  ┌──────────────────────────────────────────────────────────────────────────────────────┐   │
│  │ ⚠  Cancelling — 4 rows still in flight                                  ◷ 11s        │   │
│  │    Soft cancel is taking longer than expected.                  [ Force kill ]       │   │
│  └──────────────────────────────────────────────────────────────────────────────────────┘   │
│  Phase:   ✓ Init  ✓ Snap  ✓ Start  ✓ Running  ◉ Cancel  ·  Persist                          │
│                                                                                              │
│   click [Force kill]                                                                         │
│   ┌──────────────────────────────────────────────────────────────────────┐                  │
│   │ Force-kill workers?                                                  │                  │
│   │ ─────────────────────────────────────────────────────────────────── │                  │
│   │ Partial outcomes may be lost. This cannot be undone.                │                  │
│   │ Type "refu" (first 4 chars of exec name) to confirm:                │                  │
│   │ [____]                                          [Cancel] [Force kill]│                  │
│   └──────────────────────────────────────────────────────────────────────┘                  │
```

### W-6 Failed rows (one row expanded)

```
│  ┌─Live──Failed rows──Errors by code──Artifacts──────────────────────────────────────────┐  │
│  │  Errors: BILLING_NOT_FOUND 342  ·  DB_ERROR 38  ·  MISSING_BILLID 7      ⊙ Reveal     │  │
│  │  ┌────────────────────────────────────────────────────────────────────────────────┐   │  │
│  │  │ seq    row    kind    error_code         message                   dur_ms     │   │  │
│  │  │ ───── ───── ─────── ─────────────────── ─────────────────────────── ────────── │   │  │
│  │  │ 102    105   ● err   BILLING_NOT_FOUND   no billing row for billid       14   │   │  │
│  │  │ ▼ 198  201   ● err   DB_ERROR            connection timeout            1240   │   │  │
│  │  │   ┌──────────────────────────────────────────────────────────────────────┐    │   │  │
│  │  │   │ raw_record                                                           │    │   │  │
│  │  │   │ {                                                                    │    │   │  │
│  │  │   │   "id": "rec_201",                                                   │    │   │  │
│  │  │   │   "billid": "b0042",                                                 │    │   │  │
│  │  │   │   "channel": null                                                    │    │   │  │
│  │  │   │ }                                                            [Copy]  │    │   │  │
│  │  │   └──────────────────────────────────────────────────────────────────────┘    │   │  │
│  │  │ 241    244   ● err   BILLING_NOT_FOUND   no billing row for billid       11   │   │  │
│  │  │ ...                                                                            │   │  │
│  │  └────────────────────────────────────────────────────────────────────────────────┘   │  │
│  │  Showing 1–200 of unknown        [ Load more ]              [ Retry failed only ▸ ]   │  │
│  └────────────────────────────────────────────────────────────────────────────────────────┘ │
```

### W-7 Empty workspace state

```
┌──────────────────────────────────────────────────────────────────────────────────────────────┐
│  ◇  billing-workspace ▾    Executions                                              + New     │
├──────────────┬───────────────────────────────────────────────────────────────────────────────┤
│ ● Executions │                                                                               │
│              │                                                                               │
│              │                                  ▭ ▭                                          │
│              │                                Inbox                                          │
│              │                                                                               │
│              │                         No executions yet.                                    │
│              │                Start by creating one — or run                                 │
│              │                rowforge exec start in a terminal.                             │
│              │                                                                               │
│              │                       [ + New execution ]                                     │
│              │                                                                               │
│              │                Or [ Open a different workspace ]                              │
│              │                                                                               │
└──────────────┴───────────────────────────────────────────────────────────────────────────────┘
```

### W-8 Orphan attempt banner (ambiguous, idle ≤ 5 min)

```
│  Attempt #3   ⚠ Possibly running externally                                                  │
│  ┌──────────────────────────────────────────────────────────────────────────────────────┐   │
│  │ ⚠  This attempt may still be running externally (e.g. via the CLI).                  │   │
│  │    State shown below may be stale.                                                   │   │
│  │                                            [ Refresh ]    [ Mark aborted manually ]  │   │
│  └──────────────────────────────────────────────────────────────────────────────────────┘   │
```

These wireframes are not normative. They are sketches. The
**normative parts** are §7.3 (page tree), §7.5 (color tokens),
§7.7 (boundary states), §7.10 (must-not list).

## 7.14 Open questions

1. **Active-runs UI when count is high.** v1 caps at 3 (Part 3 §3.4),
   so the header pill is fine. If the limit is raised, does the pill
   become a popover with a search? Defer until users hit the cap.
2. **Failed-row filter UI before v2 index.** Filtering by `error_code`
   without the index requires a full scan. Offer it as a "may be slow"
   action, or hide until v2 (Part 4 §4.4)?
3. **macOS App Nap UX hint.** Spec does not require opt-out (Part 3
   §3.8). Should the UI show a passive hint ("Keep window foregrounded
   for smoothest updates") on first long run, or leave it to docs?
4. **High-friction force-kill confirmation token.** Exec-name prefix or
   the literal string "FORCE KILL"? First is contextual, second is
   universal but more typing.
