# Manual smoke check — Plan 02

The agent cannot launch a Tauri desktop window. This file is the human
checklist to verify Plan 02 end-to-end.

Run these from `apps/rowforge-studio/`:

## Initial launch

1. `pnpm install` (idempotent)
2. `pnpm tauri dev`

**Expected on first launch:**
- A 1280×800 window titled "rowforge Studio" appears.
- Workspace Picker shows: "No workspace yet" + Inbox icon + two buttons.
- Header path reads "—" (no workspace open yet).

## Pick a workspace

Click `[Open folder…]`:
- macOS file dialog opens to select a directory.
- Pick any empty directory.
- App routes to Executions page; header path shows the chosen folder.
- Table shows the empty state ("No executions yet. Create one with
  `rowforge exec start` in a terminal.").

OR click `[Use ~/.rowforge]`:
- App creates `~/.rowforge` if missing, opens it, routes to Executions.

## HMR (React)

- Edit `src/pages/ExecList.tsx` — change the header text from
  `"Executions"` to `"Executions (HMR)"`.
- The window updates within ~500ms without losing the workspace state.
- Revert your edit.

## Hot rebuild (Rust)

- Edit `src-tauri/src/commands.rs` — add a `//` comment somewhere.
- Tauri rebuilds (~10s); the window restarts.
- Workspace path was persisted to settings.json so the picker is
  bypassed and the Exec List loads directly.
- Revert your edit.

## Settings persistence

Inspect the settings file on disk:

- macOS: `~/Library/Application Support/com.lemotw.rowforge.studio/rowforge-studio/settings.json`
- Linux: `~/.local/share/com.lemotw.rowforge.studio/rowforge-studio/settings.json`
- Windows: `%APPDATA%\com.lemotw.rowforge.studio\rowforge-studio\settings.json`

The file should contain:
```json
{
  "schema_version": 1,
  "workspace_root": "<your chosen path>",
  "default_workers": null,
  "max_concurrent_runs": null,
  "telemetry_opt_in": false
}
```

## Reopen with bad workspace_root

- Quit the app.
- Edit `settings.json`, set `workspace_root` to `"/does/not/exist"`.
- Relaunch the app:
  - `BootGate`'s autoload fails (open returns `WorkspaceUnavailable`).
  - The Picker shows again so you can recover.
  - The stored path is preserved on failure (no auto-clear in Plan 2).

## Populated exec list

To verify the table renders rows (not just empty state):

- Quit the app.
- Use the CLI to create an execution in your workspace:
  ```bash
  ROWFORGE_HOME=/path/to/your/workspace \
    cargo run -p rowforge-cli -- exec start --csv /path/to/any.csv --name smoke
  ```
- Relaunch the app and verify the row appears in the table with name,
  created timestamp, row count, and "0" attempts.

## What's NOT in Plan 02

The following are spec'd but not yet built — do not expect them:

- Active runs pill in the header.
- Click-on-header to switch workspace mid-session.
- Settings page (the persistence layer ships in Plan 2; the UI page
  arrives in Plan 5).
- Exec detail page (Plan 3).
- Run launcher / start execution from UI (Plan 5).
- Handler list / handler authoring (Plans 6–8).
- Schema-version mismatch refusal (Plan 3).

## Plan 03 additions

After picking a workspace with executions:

- Click any exec row → routes to `/exec/<id>` → Attempts tab lists attempts.
- Click an attempt's "open ⏵" link → routes to `/exec/<id>/attempt/<aid>` → Summary tab.
- Open `Rollup` tab → click `Compute rollup` button → see resolved /
  failed_last / crashed_last / cancelled_last / too_large / never_attempted
  counters + by_error_code table.
- Open `Failed rows` tab → table appears; click a row's `raw` to expand
  the JSON; click `history` to open the side drawer.
- Open `Artifacts` tab → file paths with Reveal buttons that open Finder.
- Click the header workspace path → modal with Reveal in Finder / Reload
  data / Switch workspace…
- For a running attempt (state ≠ done/aborted/crashed), an amber banner
  shows "Snapshot may be stale" with a manual Refresh button. Live
  updates arrive in Plan 4.
- Breadcrumb at top: `Executions / <exec name> / Attempt <aid>` — click
  segments to navigate.

### Schema-version pin

Quit the app. Manually bump the SQLite `schema_version` row in
`<workspace>/executions.db` to 99 via:

```bash
sqlite3 <workspace>/executions.db "UPDATE schema_version SET version = 99;"
```

Relaunch the app: it refuses to open with a `WorkspaceLocked` error
mentioning the schema version.

### Edge cases worth checking

- Open an exec with 0 attempts → "This execution has never been run" message.
- Open Failed rows with > 100 errors → cursor-style "Load more" appears
  for page 2.
- Click "history" on a row that's been resolved in a later attempt →
  drawer shows `✓ resolved at attempt <aid>`.

## Plan 04 additions

After a workspace is picked, runs can be started and watched live.

### Start a run

1. Click an exec row → ExecDetail.
2. Click **Run** in the header.
3. Pick a handler directory in the file dialog.
4. The app automatically navigates to `/exec/:id/attempt/:aid?run=<handle>` — the
   new attempt's Live tab opens immediately (Plan 5 T15, closes Plan 4 limitation).

### Watch live progress

1. The Live tab is active as soon as the run starts (auto-navigation above).
2. Live progress streams in; the attempt state updates in real-time.

### Cancel a live run

1. While a live run is active, click the **Cancel** button
   in the header.
2. Soft confirm dialog appears: "Soft cancel? In-flight rows will finish."
3. Click "Soft cancel". The header switches to an amber "Cancelling…"
   banner with an elapsed counter.
4. After 10 seconds, a red "Force kill" button appears on the right side
   of the banner.
5. Click Force kill → confirmation dialog requiring the first 4 chars of
   the exec name typed in.

### Active runs pill

When ≥ 1 runs are active, the header shows a green
**N running** pill. Click it for a popover listing the active handles
and aggregate counters (total processed / total failed).

### Concurrency limits

1. Try starting a second run on the same exec — should fail with
   "execution X already has an active run" (per-exec limit = 1).
2. Start 3 different runs on 3 different execs. Starting a 4th should fail with
   "workspace concurrent-run limit reached" (workspace limit = 3).

### Orphan recovery

1. While a run is active, kill the Studio process abruptly
   (Cmd+Q won't work because Drop runs — use Activity Monitor → Force
   Quit, or `kill -9 <pid>`).
2. Wait 5+ minutes (or modify the attempt's `outcomes.jsonl` mtime via
   `touch -t YYYYMMDDhhmm <path>` to be > 5 min old).
3. Reopen Studio. The orphan attempt should be auto-marked as
   `aborted` in the Attempts list.

### Schema version pin (from Plan 3)

(Same as Plan 3 — bump SQLite `schema_version` table row > current to
verify Studio refuses to open.)

### Known Plan 4 limitations (to be addressed later)

- ~~**Auto-navigate to ?run= after Run button**: missing; deferred to Plan 5.~~ Closed in Plan 5 (Task 15).
- **Hard cancel (Force kill)**: currently behaves identically to soft
  cancel — rowforge-core doesn't expose per-worker process kill yet.
  The dialog still requires the typed confirm token; spec requires UX
  even if backend isn't fully wired.
- **`total_rate` in active runs pill**: shows 0; SessionRegistry doesn't
  cache per-session rate. Deferred.
- **`slowest_run` in active runs popover**: shows `null`. Same reason.

## Plan 05 additions

### Create an execution (Flow A)

The handler is **not** picked here — it is selected per-Run on ExecDetail
(the data model binds handler to attempt, not to exec).

1. Empty workspace (or with execs) → click **New execution** on Workspace Home (either the empty-state primary button or the header secondary button)
2. Enter name `smoke_test_plan5` (must match `[a-z0-9_-]+`, ≤ 64 chars)
3. Click **Pick…** next to "Input file" → choose any CSV/JSONL/NDJSON file
4. Confirm format chip shows the detected extension
5. Click **Create execution** → routes to `/exec/<id>` (Detail). Click **Run** there when ready to pick a handler and start a run.

#### Negative paths to verify

- **Bad name** — typing characters outside `[a-z0-9_-]` shows the inline regex hint; Create button stays disabled
- **Missing input** — Create stays disabled until input file is picked
- **Unsupported extension** — pick a `.txt` file → backend rejects with `invalid_input` (currently the picker filters extensions, so this requires manually pasting a path)
- **Duplicate name** — submit twice with the same name → second submit shows red error "duplicate exec name" inside the form

### Run button auto-navigate (Plan 4 carry-forward closed)

1. ExecDetail of an exec → click **Run**
2. Pick handler dir → after spinner, app auto-routes to the new attempt's Live tab
3. No manual click on the new attempt row needed

### Export (Flow D)

1. ExecDetail (any exec with at least one Done attempt) → click **Export** in the header
2. Pick output dir (or leave default — backend writes to `<exec_dir>/exports/<timestamp>/`)
3. Choose format: `csv`, `jsonl`, or `both`
4. Optional: check **Require complete**
5. Click **Export** → loading toast "Exporting…" → after a few seconds, success toast with **Reveal** action
6. Click **Reveal** → OS file manager opens at the output directory
7. Verify files present:
   - `csv`: `success.csv`, `failed.csv`, `resolution.json`
   - `jsonl`: `success.jsonl`, `failed.jsonl`, `resolution.json`
   - `both`: all five

#### Negative path

- Exec with unresolved rows (e.g. fresh exec, no runs) + **Require complete** checked → submit
- Should show red toast: "Export incomplete: N rows unresolved — uncheck 'Require complete' or finish the run first."
- No files written

### Known Plan 5 limitations (deferred to Plan 6+)

- **No Settings page UI** — `Settings.max_concurrent_runs` still hardcoded at (3 workspace / 1 per-exec)
- **No Workspace switching UI** — settings.json must be edited by hand to change `workspace_root`
- **No Workspace Picker boot improvements** — empty-state still auto-redirects
- **No handler authoring panel** — Part 8 entirely deferred (Handlers route, manifest editor, Pack, smoke test)
- **Hard cancel still degrades to soft cancel** — needs rowforge-core process-kill API
- **Export blocks UI** — no streaming progress, no cancel during export, no per-file granularity
- **`total_rate` / `slowest_run` still placeholder** — `RunRollupTick` returns `0.0` / `null` for these fields

## Plan 06 additions

### Settings page

1. Click **Settings** in the left sidebar → lands on `/settings`
2. Confirm the 4 sections render:
   - **Workspace** — current path + Switch workspace… button
   - **Concurrency** — Default workers + Max concurrent runs inputs
   - **Telemetry** — Opt-in checkbox
3. Edit **Max concurrent runs** from 3 to 5
4. Confirm the blue banner "ℹ Changes to max concurrent runs apply on next workspace open" appears
5. Click **Save** → no toast (silent success); form retains the new value
6. Click **Cancel** after editing again → form snaps back to the saved value (3 or whatever the last save was)

### Workspace switching — happy path

1. From Settings, with no active runs (verify via header pill = empty), click **Switch workspace…**
2. Pick a different directory (or a new empty one)
3. App routes to `/` showing the new workspace's exec list (likely empty)
4. Sidebar / header shows the new workspace path
5. settings.json on disk now points to the new root

### Workspace switching — blocked

1. Start a long-running attempt (sample 10s sleep handler with ≥1 row)
2. While running, open `/settings`
3. Confirm **Switch workspace…** is disabled with tooltip "Cancel N active run first"
4. Confirm the amber warning text "⚠ N active run — cancel to switch" appears below the button
5. Cancel the run (via the active-runs pill or attempt page)
6. Confirm within 2s the button becomes enabled (the 2s poll fires)

### `last_handler_dir` persistence across restart

1. Run an exec with a handler dir (via RunButton or the Wizard's "Start immediately")
2. Quit Studio entirely (Cmd+Q on macOS / Alt+F4 on Windows / similar)
3. Reopen Studio, navigate back to the same exec
4. Click **Run** (the primary button, NOT the gear icon) → directory picker should NOT appear; the previous handler dir is auto-selected and the run starts directly
5. Confirm via `sqlite3 <workspace>/executions.db "SELECT id, last_handler_dir FROM executions"` that the value is stored

### `max_concurrent_runs` takes effect at next workspace_open

1. Change Settings → Max concurrent runs → 1, Save
2. With no workspace switch yet, try to start 2 concurrent runs on different execs (or two attempts of the same exec — Plan 4 limits are 1/exec / 3/workspace)
3. The second run should NOT be rejected (old limit still in effect — banner said it would apply on next open)
4. Switch workspace (or quit + reopen Studio) → re-enter same workspace
5. Now try the 2 concurrent runs again → second should fail with `run_busy { scope: per_workspace, limit: 1 }`

### `RunRollupTick` real numbers

1. Start 2 concurrent runs with different speeds (e.g. one sleep 10s × 5 rows, one sleep 2s × 5 rows)
2. Wait ~12 seconds (so both sliding windows fill)
3. Open the active-runs pill in the header
4. Confirm `total_rate` shows a non-zero number (sum of both runs' rates per second)
5. Confirm `slowest_run` points to the slower handler's attempt (the 10s-sleep one)

### Known Plan 6 limitations (deferred to Plan 7+)

- Handler authoring panel (Part 8 entirely) → Plan 7
- Hard cancel still degrades to soft cancel — needs rowforge-core process-kill API
- No Settings hot-reload — `max_concurrent_runs` only takes effect on next workspace open (intentional; surfaced via dirty banner)
- No multi-workspace recents / "Recently opened" picker
- `slowest_run` heuristic is min positive rate_10s; ETA-based and stall-aware variants deferred

## Plan 07 additions — Handler authoring (static surface)

### Pre-populate fixtures

Before launching, drop a couple of handler dirs into the workspace so the
list page has rows. From a terminal (replace `<workspace>` with your path):

```bash
WS=<workspace>
mkdir -p "$WS/handlers/alpha"
cat > "$WS/handlers/alpha/rowforge.yaml" <<'EOF'
name: alpha
version: 0.1.0
language: go
kind: row
primary_field: id
entry:
  cmd: ["./handler"]
EOF
touch "$WS/handlers/alpha/handler.go"

# Invalid: missing required field
mkdir -p "$WS/handlers/broken"
echo "kind: row" > "$WS/handlers/broken/rowforge.yaml"

# Missing manifest entirely
mkdir -p "$WS/handlers/no-manifest"
echo "// notes" > "$WS/handlers/no-manifest/notes.txt"
```

### Sidebar link + list page

1. Sidebar **Handlers** entry no longer shows "Coming soon" — clicking it
   routes to `/handlers`.
2. List page renders 3 rows: `alpha` (green "valid"), `broken` (yellow
   "invalid"), `no-manifest` (red "missing").
3. Each row shows mono name, status badge, version, language, relative
   timestamp, and per-row Edit / Reveal buttons.

### Detail page

4. Click `alpha` row → routes to `/handlers/alpha`.
5. Header shows mono name, full path, and 4 action buttons:
   Open in editor / Reveal / Rename… / Delete…
6. **Manifest** section: green "valid" badge + key-value table
   (kind, primary_field, entry.cmd).
7. **Files** section: lists `handler.go` with byte size (`rowforge.yaml` appears in the Manifest section above, not here).
8. Back to list → click `broken` → red error list shows manifest parse
   errors.
9. Back to list → click `no-manifest` → "No rowforge.yaml in this handler
   directory." copy.

### Edit + Reveal

10. Detail page → **Open in editor**. With no Settings.preferred_editor and
    no $VISUAL/$EDITOR set, the resolver falls through to `which` probes
    (code → cursor → nvim → vim → nano). The first match opens the
    handler dir.
11. **Reveal** → OS file manager opens at the handler dir.
12. **Negative path**: clear all editor envs and Settings.preferred_editor;
    move `code` / `cursor` etc. out of PATH (or just unset PATH for the
    Studio session). Open in editor → toast with `editor_not_found` copy:
    "No editor found. Set `$VISUAL` or `$EDITOR`, or configure 'Preferred
    editor' in Settings."

### Scaffold (New Handler)

13. List page → **New Handler** → ScaffoldDialog opens.
14. Type `BadName` → inline regex error; Create disabled.
15. Clear, type `gamma`, leave **Go (row mode)** selected, leave
    primary_field `id` → Create enables.
16. Click **Create** → toast "Handler 'gamma' created"; dialog closes;
    routes to `/handlers/gamma`.
17. Verify the new handler shows: valid manifest, `primary_field: id`,
    `kind: row`. Files section lists `handler.go`, `go.mod` (rowforge.yaml
    appears in the Manifest section, not Files).
18. Open in editor on the new handler → confirm `{{name}}` /
    `{{primary_field}}` were substituted (no literal `{{` left in source).
19. Repeat with **Go (batch mode)** → name it `delta` → manifest shows
    `batch_size: 5`.
20. Repeat with **Empty** → name it `epsilon` → Files section shows only
    `handler.go` (rowforge.yaml appears in the Manifest section, not Files;
    no .mod for the empty template).
21. Try scaffold a name that already exists (`alpha`) → `HandlerExists`
    inline error; dialog stays open.

### Rename

22. Go to `/handlers/gamma` → **Rename…** → dialog opens with `gamma`
    pre-filled. Rename button disabled (unchanged).
23. Edit to `gamma-renamed` → Rename enables.
24. Click **Rename** → toast "Handler renamed to 'gamma-renamed'"; URL
    updates to `/handlers/gamma-renamed`; detail page reflects new path.
25. List page now shows `gamma-renamed`, no `gamma`.
26. **Negative**: rename to `alpha` (existing) → `HandlerExists` banner;
    dialog stays open.
27. **Negative**: rename to `Bad-Name` → inline regex error; button
    disabled.

### Delete (typed-token)

28. Go to `/handlers/delta` → **Delete…**.
29. Type `Delta` (wrong case) → Delete stays disabled (case-sensitive).
30. Type `delta` exactly → Delete enables with red destructive styling.
31. Click **Delete** → toast "Handler 'delta' deleted"; routes to
    `/handlers`; list no longer contains `delta`.
32. **Symlink defense**: create a symlink pointing outside the workspace,
    e.g. `ln -s /tmp/external "$WS/handlers/evil"`. Try to delete via the
    UI → backend rejects with Io error (path-traversal: canonicalize
    starts_with workspace check). No external files touched.

### Settings.preferred_editor

33. Settings page → confirm a new **Editor** section between Concurrency
    and Telemetry.
34. Type `code --wait` → **Save** → toast "Settings saved".
35. Handler detail → Open in editor → resolver now uses `code --wait` (VS
    Code opens; the spawn doesn't return until you close the file).
36. Clear the Editor field → Save → field normalizes back to null. Open in
    editor falls back to $VISUAL/$EDITOR/auto-probe.

### `handlers:list` event refresh

37. Open `/handlers` in the app. From inside the app, trigger a scaffold
    / delete / rename. The list refreshes immediately without manual
    reload (Tauri event `handlers:list` emitted by mutation commands).
38. (Plan 7 is static — no filesystem watcher. External edits via terminal
    do NOT auto-refresh; that's expected.)

### Lazy rename semantics

39. If past executions reference a renamed handler, those rows still show
    the OLD handler name on ExecHistoryPage (lazy semantics —
    `executions.last_handler_dir` snapshot preserved). Rename only touches
    the filesystem; sqlite is untouched.

### Known Plan 7 limitations (deferred to Plan 8+)

- No manifest editor — manifests are read-only in Studio; users edit them
  via Open in editor / external tools
- No Pack / smoke-test surface — handler verification is filesystem-only
- No filesystem watcher — list/detail refresh only on app-side mutations
  or manual reload
- No detail-page file-click-to-open — files are display-only; Open in
  editor on the page header opens the whole dir
