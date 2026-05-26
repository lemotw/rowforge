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

## Plan 08 additions — Handler build

### Pre-populate fixtures

Before starting, ensure you have at least one handler with `entry.build`
and one without. The existing `examples/handlers/golang-apple-refund`
(or similar Go handlers) works well. For a python-only handler, use:

```bash
WS=<workspace>
mkdir -p "$WS/handlers/py-noop"
cat > "$WS/handlers/py-noop/rowforge.yaml" <<'EOF'
name: py-noop
version: 0.1.0
language: python
kind: row
primary_field: id
entry:
  cmd: ["python3", "handler.py"]
EOF
```

### 1. CLI auto-build (closes ENOENT pain)

1. Fresh checkout / clean workspace. Don't pre-build anything.
2. `rowforge exec start --csv path/to/any.csv --name smoke`
3. `rowforge exec run --handler examples/handlers/golang-apple-refund <exec_id> --sample 2 --workers 1`
4. Expected: stderr shows `[rowforge] building golang-apple-refund ...`
   then `[rowforge] build ok (NNN ms)` then the run proceeds. The
   binary is now present in the handler dir.
5. Re-run the same command → no build banner (binary is fresh).
6. `touch examples/handlers/golang-apple-refund/handler.go`
7. Re-run → build banner appears again (source mtime > binary mtime).

### 2. CLI explicit build subcommand

8. `rowforge handler build` — builds every handler under
   `<workspace>/handlers/*` that has `entry.build` AND is stale.
   Per-handler outcomes printed to stderr.
9. `rowforge handler build alpha` — single handler by name.
10. `rowforge handler build --force alpha` — bypasses staleness check;
    rebuilds even when binary is fresh.

### 3. CLI build failure

11. Edit a handler's `rowforge.yaml` to make `entry.build` a failing
    command (e.g. `build: ["sh", "-c", "exit 3"]`).
12. `rowforge exec run --handler <dir> <exec_id>` → expected: stderr
    shows `[rowforge] build failed (exit 3):` followed by any build
    output; CLI exits with code 2.

### 4. Studio Build button (happy path)

13. Open Studio. Navigate to a handler with `entry.build` (e.g.
    `golang-apple-refund` or any Go handler).
14. Detail page header shows a **Build** button between Open in editor
    and Rename…
15. Click Build → label changes to "Building…", button disabled.
16. After completion (~3–10 s for a Go handler): Last build section
    appears between Manifest and Files. Green "success" badge, exit 0,
    duration in ms, timestamp.
17. Click "Show output ▾" → log expands; shows build stdout (usually
    empty for Go) and stderr.
18. Re-click "Hide output ▴" → collapses.

### 5. Studio Build button (failure path)

19. Edit a handler's `rowforge.yaml` to make `entry.build` fail
    (e.g. `build: ["sh", "-c", "echo broken >&2; exit 5"]`).
20. From Studio's detail page, click **Build**.
21. Sonner toast appears: "Build failed for 'NAME' (exit 5). See the
    Last build section for details."
22. Last build section shows: red "failed" badge, exit 5, duration.
    Expand → stderr contains "broken".

### 6. Studio Build button hidden for python/node handlers

23. Navigate to a handler whose `entry.cmd` is `["python3", ...]` with
    no `entry.build`. Detail page header does **NOT** show a Build
    button.

### 7. Toolchain missing

24. Edit a handler so `entry.build` is `["this-tool-xyz-does-not-exist"]`.
25. Click **Build** in Studio → toast: "Build tool
    'this-tool-xyz-does-not-exist' not found in PATH. Install it or
    update entry.build in your manifest."
26. Last build section is NOT populated (no outcome to cache on
    toolchain-missing; verify by reloading detail page — section absent).

### 8. Manifest validation warnings (Plan 8 additions)

27. Detail page of a handler whose `entry.build` first token isn't on
    PATH → Manifest section shows a yellow warning chip:
    "build tool 'X' not found in PATH".
28. Detail page of a handler whose `entry.cmd` points to a missing
    relative file AND `entry.build` is absent → yellow warning:
    "entry.cmd target './X' not found".
29. Same handler setup but WITH `entry.build` present → the
    `CmdTargetMissing` warning is suppressed (build is expected to
    produce the target).

### Known Plan 8 limitations (deferred to later plans)

- No smoke test surface — handler verification still requires running
  an actual exec-run.
- No build cancel — long builds block the Build button until exit.
- stderr not streamed — log appears all at once at completion.
- No build / exec-run interlock — concurrent build + run on the same
  handler can race; user is expected to wait.
- `BuildOutcome` cache lost on Studio quit — re-open shows no "Last
  build" until the next build is triggered.
- Studio's Tauri `handler_build` command is `async` but does not use
  `spawn_blocking` — the Tauri async runtime is blocked for the
  duration of the build. For multi-second builds this may delay other
  Tauri commands. Refactor flagged for a later plan.

---

## Plan 09 — Handler Logs

These steps verify the end-to-end handler log capture: file persistence,
Studio Logs tab states (bootstrap / live / filters / auto-scroll / pause
/ backpressure), capture_raw_stdout toggle, and edge cases.

### 1. Setup

1. Create a fresh workspace and an execution using any input CSV.
   Use the `golang-apple-refund` handler (or any handler you can edit)
   and add `fmt.Fprintln(os.Stderr, "processing row", rowIndex)` inside
   the per-row handler body so every row emits a stderr line.

2. Run the execution with at least a few hundred rows so the log file
   has visible content.

### 2. File persistence

3. After the attempt finishes, locate the attempt directory:
   `<workspace>/executions/<exec_id>/attempts/<attempt_id>/`
   Confirm that `handler_log.log` exists in that directory.

4. Run `cat <path>/handler_log.log | head -5` and verify each line has
   the format:
   ```
   2026-05-25T14:32:01.423Z [handler#0 stderr] processing row 1
   ```
   — RFC 3339 timestamp, `[handler#<N> stderr|stdout]`, then content.

5. In a terminal, run the same execution via the CLI:
   `rowforge exec run --handler <handler_dir> <exec_id>`
   Confirm that handler stderr lines still appear in the terminal
   (`[handler#0 stderr] ...`) — CLI back-compat is preserved.

6. Open the existing `handler_log.log` and grep for any JSON that looks
   like an outcome line (e.g. `{"type":"success"`). With the default
   setting `capture_raw_stdout = false`, no outcome JSON should appear.

### 3. Studio Logs tab

7. Open Studio. Navigate to the execution's Attempt Detail page for the
   attempt you ran. Click the **Logs** tab (between Errors by code and
   Artifacts in the sub-tab bar).

8. The tab shows a tail of the log file — up to 5000 lines are loaded
   on mount. Verify that the lines match what `cat` showed.

9. Each line displays: RFC 3339 timestamp · colored worker badge `#0`
   (or `#N`) · stream chip (yellow label **stderr** / blue label
   **stdout**) · monospace line content.

10. Click the `#0` worker chip in the toolbar filter. The list narrows
    to only lines from worker 0. Click again to deselect.

11. In the toolbar, switch the stream toggle to **stdout only**. The
    list should be empty (or show only stdout lines if any non-JSON
    stdout was emitted). Confirm stderr lines are hidden. Switch back
    to **both**.

12. Type a substring from one of the log lines into the search box.
    The list narrows to matching lines. Clear the search box to restore.

13. Start a new run on the same execution so the attempt is live.
    Navigate to that attempt's Logs tab. Confirm the auto-scroll toggle
    is on; new lines appear at the bottom and the viewport follows.

14. While lines are arriving, scroll up manually. The auto-scroll
    indicator in the toolbar should show as disengaged (e.g. greyed
    out). New lines stop pushing the viewport.

15. Click **Pause** in the toolbar. Live lines continue arriving in the
    background but the visible list does not update. Click **Resume**:
    buffered lines flush in and auto-scroll reactivates.

### 4. Backpressure

16. Modify the handler to emit ~10 000 lines per second to stderr (e.g.
    log every byte or add a tight loop). Run the execution. While it is
    running, watch the Logs tab — an amber banner should appear:
    > ⚠ N log lines dropped — open the log file for full content.
    The `dropped` counter in the banner corresponds to lines lost from
    the broadcast channel, not from the file.

17. After the run, click **Reveal log file** in the Logs toolbar. Your
    OS file manager (Finder on macOS) should open at
    `<attempt_dir>/handler_log.log`. Open the file and verify it
    contains far more lines than the Studio UI showed — the file is
    always complete.

### 5. capture_raw_stdout

18. Go to **Settings** (`/settings`). Scroll to the **Logs** section.
    Enable the **Capture raw stdout** toggle. Click Save.

19. Run a new attempt. After it finishes, open `handler_log.log` and
    grep for outcome JSON lines (e.g. `{"type":"success"`). They should
    now appear, interleaved with the stderr lines, each prefixed with
    the `[handler#N stdout]` label.

### 6. Edge cases

20. Navigate to an old attempt that was created before Plan 9 (i.e.
    before `handler_log.log` was introduced). Open its Logs tab. The
    tab shows:
    > No log file. This attempt predates Plan 9 log capture.

21. Start a new run and navigate to its Logs tab immediately, before
    the handler emits any output. The tab shows:
    > Handler has not produced any output yet.
    (Once the handler starts emitting, the message clears and lines
    appear.)

22. With any attempt open in the Logs tab, apply a filter combination
    that matches nothing (e.g. search for a string that does not exist,
    or select a worker ID that had no output). The list shows:
    > No lines match the current filters.

### Known Plan 9 limitations

- No log rotation — a long-running handler can produce 100 MB+ files;
  disk usage must be managed manually.
- No cross-attempt log search — each attempt's log is a separate file;
  there is no grep-across-attempts surface in the UI.
- No color coding by severity keyword (ERROR / WARN / INFO) — all lines
  use the same stream color (yellow stderr / blue stdout).
- `rowforge-core`'s own internal tracing (the `tracing` crate spans)
  is not captured — `handler_log.log` contains only the handler
  process's own stdout/stderr, not the Studio runtime's logs.
- The broadcast channel cap is 4096 lines; a handler emitting faster
  than the Tauri event pump can drain will see drops. The file is
  unaffected.

---

# Manual smoke check — Plan 10 (execution delete)

Plan 10 adds single and bulk execution deletion, Select mode on the Exec
list, an on-disk Size column, and `rowforge exec delete` CLI subcommands.

## 1. Setup

1. Open a fresh workspace (or use an existing one). Create 3 executions
   via "New execution" — give them distinct names such as `smoke-a`,
   `smoke-b`, `smoke-c`. Use a small CSV input (5–10 rows) so runs
   finish quickly.

2. Start a run on `smoke-a` and let it proceed but do **not** cancel it
   yet — you need an active run for the active-run gate tests below
   (step 11 and step 18). If the run finishes before you reach those
   steps, start it again.

## 2. ExecList column display

3. The ExecList should show columns in this order:
   **Name | Rows | Attempts | Size | Created**
   Verify no "Status" or other column has slipped in, and that "Size"
   appears between Attempts and Created.

4. Hover over any Name cell. A tooltip appears showing the full
   `exec_id` (e.g. `e_01JXXXXXXXXXXXXXXX`). The cell text shows the
   human-readable name, not the id.

5. The Size column shows formatted bytes (e.g. `3.2 KB`, `5.0 MB`).
   Executions that have never run will show a small size (just the
   SQLite row; no attempt dir). If an execution's directory is missing
   from disk, the cell shows `—`.

6. The Created column is rightmost. Values are human-friendly timestamps
   (e.g. "2 minutes ago" or an absolute date — whichever the component
   uses).

## 3. Select mode

7. Click the **Select** button in the Exec list header. The button label
   changes to **Cancel**; a checkbox column appears as the leftmost
   column.

8. Verify every row now has a checkbox on the left. Unselected rows have
   an unchecked box; no rows are pre-selected.

9. Click a non-running row → the checkbox ticks; the row background
   highlights. Click it again → unchecked. Clicking the row no longer
   navigates to the detail page in Select mode.

10. Click **Cancel** in the header → Select mode exits, checkboxes
    disappear, the button label reverts to **Select**, and no rows remain
    highlighted.

11. With `smoke-a` still running: re-enter Select mode. The `smoke-a`
    row's checkbox should be **disabled** (grayed out). Hover over the
    checkbox area → tooltip: "Cancel active run first". You cannot tick
    it.

## 4. Single delete via Select

12. Re-enter Select mode. Select `smoke-b` (the one that is not running).
    The header shows **Delete 1 execution**. Click it.

13. `DeleteExecutionsDialog` opens with title **Delete 1 execution?**.
    It lists `smoke-b`'s name, its size in formatted bytes, and the
    message "Are you sure? This action cannot be undone."

14. Click the destructive **Delete** button. The dialog closes; a Sonner
    toast says "1 execution deleted". The Exec list refreshes and
    `smoke-b` is gone. Verify on disk:
    ```bash
    ls <workspace>/executions/
    ```
    The `e_...` directory for `smoke-b` should no longer exist.

## 5. Bulk delete (happy path)

15. Select both `smoke-c` and any other completed execution (not the
    active one). The header shows **Delete 2 executions**. Click it.

16. The dialog title is **Delete 2 executions?**. It lists both names.
    The total size shown is the sum of both. Confirm → toast "2
    executions deleted"; the list refreshes; both are gone.

17. Verify both directories are removed from disk:
    ```bash
    ls <workspace>/executions/
    ```
    Only `smoke-a`'s directory (the one with the active run) should
    remain, along with any executions you created after step 14.

## 6. Active-run gate (bulk partial fail path)

18. Create two new executions `smoke-d` and `smoke-e`. Start a run on
    `smoke-d` and leave it running. Enter Select mode. Select both
    `smoke-d` (active) and `smoke-e`. Notice `smoke-d`'s checkbox is
    disabled — you cannot select it. This means the UI already prevents
    you from queueing an active exec for deletion.

    (Alternative path if you want to hit the server-side gate directly:
    use the CLI in step 21.)

19. Instead of fighting the UI gate, cancel the run on `smoke-d` first
    (or let it finish), then proceed to delete both via Select mode.
    Both should delete cleanly. This confirms the gate clears after the
    run ends.

## 7. ExecDetail — deleted-elsewhere 404 fallback

20. Open two browser tabs / windows pointing to the same Studio instance.
    In tab 1, navigate to the detail page of an existing execution (e.g.
    `/exec/<id>`). In tab 2, delete that execution via Select mode.

    Tab 1 should react to the `exec_list:refresh` event (the React Query
    cache invalidates). Navigate to the detail page in tab 1 (or refresh
    it). The page renders:

    > This execution has been deleted or is unavailable.

    with a **← Back** link to the Executions list. No crash, no spinner
    stuck forever.

## 8. CLI

21. Run:
    ```bash
    rowforge exec delete <exec_id>
    ```
    where `<exec_id>` is a valid execution in the workspace. Expected:
    - stderr: `[<id>] deleted`
    - exit code: `0`

    Then run with a non-existent id:
    ```bash
    rowforge exec delete e_00000000000000000000000000
    ```
    Expected: stderr error line, exit code 1.

22. Run:
    ```bash
    rowforge exec delete --all-completed
    ```
    This deletes every execution that does not have an active run. Each
    successfully deleted execution prints one stderr line:
    `[<id>] deleted`. Any failures print an error line. The exit code
    equals the number of failures (capped at 125). Verify the workspace's
    `executions/` directory contains only directories for active or
    never-run executions (or is empty if none remain).

### Known Plan 10 limitations

- No trash / undo. Deletion is immediate and permanent.
- Partial-attempt deletion is not supported — deletion is at the
  execution level only (all attempts are removed together).
- Bulk delete is serial, not parallel. Deleting 100 executions takes
  100 × (SQLite round-trip + fs removal) time.
- Active-run refusal requires manual cancel first; there is no
  auto-cancel-then-delete shortcut.
- The Size column value is populated lazily via `walkdir` during
  `exec_list` — it reflects the size at list-load time, not a live
  counter.

# Manual smoke check — Plan 11 (re-run failed rows)

Plan 11 adds the ability to re-run only the rows that failed in a
previous attempt. It introduces `only_row_ids` filtering in the pipeline
and a Re-run dialog on the Attempt Detail Failed rows tab.

## 1. Setup

1. Open (or create) a workspace. Create an execution with a CSV input of
   at least 10 rows. Use a handler that fails some rows deterministically
   — for example, a shell stub that exits non-zero when the input seq
   value is odd, or the built-in `fail-odd-seqs` test-handler behavior.
   Verify the handler dir is accessible and `exec.last_handler_dir` is
   set (i.e., the handler has been used at least once via Studio's Run
   button).

2. Confirm you have at least 10 rows in the input CSV so the run
   produces a mix of successes and failures (e.g. 5 odd-seq failures +
   5 even-seq successes).

## 2. Initial run — observe failures

3. Click **Run** on the execution and start a full run (no row limit).
   Let it complete. The attempt transitions to `Done`.

4. Open **Attempt Detail** → **Failed rows** tab. The counter at the
   top should read "5 failed rows" (or however many odd-seq rows your
   handler fails). Each row shows its `seq` value and error details.

## 3. Re-run flow — happy path

5. The **Re-run N rows** button appears in the Failed rows tab header
   (e.g. "Re-run 5 rows"). Confirm it is enabled: the attempt is Done
   (non-active), the exec has `last_handler_dir`, and N > 0.

6. Click **Re-run 5 rows**. The `RerunFailedDialog` opens. Verify:
   - Title reads "Re-run 5 failed rows?"
   - The handler dir path (`exec.last_handler_dir`) is displayed.
   - The source attempt id is shown (the `attempt_id` of the current
     attempt).

7. Click **Re-run** (confirm). The dialog closes after the new attempt
   starts (mutation success). A Sonner toast confirms the mutation
   succeeded. Studio auto-navigates to the new attempt's **Live** tab.

8. Watch the new attempt's Live tab. The progress counter shows only up
   to 5 rows dispatched — not the full 10. The run completes quickly.

9. After the new attempt finishes, verify on disk:
   ```bash
   ls <workspace>/executions/<exec_id>/attempts/
   ```
   Open the new attempt directory and inspect `outcomes.jsonl`. It
   should contain at most 5 `BatchOutcome` lines, each covering only the
   previously-failed seq values. No even-seq rows should appear.

## 4. Edge cases

10. Navigate to any attempt that has **0 failed rows** (e.g. a
    successful re-run from step 7 where all 5 rows now pass). Open its
    Failed rows tab. The **Re-run N rows** button should be **disabled**.
    Hover over it → tooltip: "No failed rows to re-run".

11. Start a new run on the execution (e.g. full run again) and while it
    is still in `Running` state, navigate to the **current** attempt's
    Failed rows tab. The **Re-run** button should be **disabled**. Hover
    over it → tooltip: "Cancel active run first". (`hasActiveRun` is
    derived from this attempt's non-terminal state.)

12. Cancel the active run (soft cancel; let it drain). Once the attempt
    reaches `Aborted`, the Re-run button re-enables (assuming there are
    failed rows and `last_handler_dir` is set). Confirm the tooltip is
    gone.

13. If you have access to an execution that was created before Plan 6
    (i.e., its `last_handler_dir` is `null` / absent), navigate to one
    of its attempts' Failed rows tab. The Re-run button should be
    **disabled** with tooltip "Source attempt has no handler reference".
    (If no such exec exists, skip this step — it can be simulated by
    nulling `last_handler_dir` in SQLite directly.)

## 5. Rollup consistency after re-run

14. After step 7's re-run completes with 3 of 5 rows now succeeding
    (and 2 still failing): navigate to **Execution Detail → Rollup**.
    Verify:
    - `resolved` count increased by 3.
    - `failed_last` counts only the 2 seq values that are still failing
      (the last attempt for those seqs was an error).
    - `never_attempted` is 0 (all rows were attempted in attempt 1 or
      the re-run).

15. Navigate to the re-run attempt's Failed rows tab. The **Re-run**
    button now reads "Re-run 2 rows" — reflecting only the 2 seq values
    that are still failing in *this* attempt, not the original 5.
    Clicking it would start a third attempt targeting only those 2 rows.

### Known Plan 11 limitations

- No ExecDetail-level "Re-run all currently-failed across exec" button —
  re-run is always scoped to a single attempt's failures.
- No handler picker override in the dialog — the re-run always uses
  `exec.last_handler_dir`. To use a different handler, start a new run
  via the Run button on Execution Detail.
- No individual row selection or preview — the dialog is all-or-none:
  all of this attempt's failed rows, or cancel.
- No CLI `rowforge attempt rerun-failed` subcommand in Plan 11.
- `hasActiveRun` gate is approximate (uses the current attempt's own
  terminal state, not an exec-wide active-run check). If a *different*
  attempt on the same exec is running, the button will appear enabled
  but the backend will refuse with `UiError::RunBusy`, surfaced as a
  toast error.

# Manual smoke check — Plan 12 (handler import + fork)

Plan 12 adds two new ways to create handlers in a workspace:
**Import from folder** (bring any local handler directory into the
workspace) and **Fork** (duplicate an existing workspace handler under a
new name with the manifest `name:` field rewritten).

## Setup

Before starting, ensure you have a workspace open with at least one
existing handler (e.g. `golang-apple-refund` from the examples).

## 1. Import from folder — happy path

1. Prepare a source folder on disk that contains a `rowforge.yaml`. The
   easiest way is to copy an existing example:
   ```bash
   cp -r examples/handlers/golang-uppercase /tmp/test-import
   ```
   Verify `/tmp/test-import/rowforge.yaml` exists before proceeding.

2. Studio → `/handlers` → click **New Handler**.

3. The ScaffoldDialog opens. Select the **"Import from folder…"** radio
   button (fourth option). The `primary_field` input disappears; a
   **"Pick folder…"** button appears.

4. Click **"Pick folder…"**. The OS native file dialog opens. Navigate to
   and select `/tmp/test-import`. The dialog closes and the chosen path
   is displayed next to the button.

5. In the name field, type `imported-handler`. Click **Create**.

6. The dialog closes. `imported-handler` appears in the handler list.
   Click the row to open its detail page. Verify:
   - The **Source** tab shows the same files that were in `/tmp/test-import`.
   - The **Manifest** tab shows the imported `rowforge.yaml` contents.

## 2. Import edge cases

7. Open **New Handler** again. Select "Import from folder…". Create an
   empty directory first:
   ```bash
   mkdir -p /tmp/empty-dir
   ```
   Pick `/tmp/empty-dir` as the source, name the handler `should-fail`.
   Click **Create**. Verify the backend returns a friendly error: the
   dialog stays open and shows an inline error message such as
   "source folder must contain rowforge.yaml" (surfaced from
   `UiError::InvalidArg`).

8. Try to import with the name `imported-handler` again (same name as
   step 5). Pick any valid source folder. Click **Create**. Verify an
   inline error: "Handler 'imported-handler' already exists"
   (`UiError::HandlerExists`).

9. Import a source that contains a `.git` subdirectory (the
   `golang-uppercase` example should not have one, but you can create a
   fake one):
   ```bash
   mkdir -p /tmp/test-import-git
   cp -r examples/handlers/golang-uppercase/. /tmp/test-import-git/
   mkdir /tmp/test-import-git/.git
   echo "fake" > /tmp/test-import-git/.git/HEAD
   ```
   Import `/tmp/test-import-git` as `imported-with-git`. After success,
   verify the `.git` directory was copied along:
   ```bash
   ls <workspace>/handlers/imported-with-git/.git/
   ```
   This confirms the copy-everything semantic — no filter is applied.

## 3. Fork — happy path

10. Navigate to the `imported-handler` detail page (from step 5, or
    `/handlers/imported-handler`).

11. In the page header, locate the **Fork…** button positioned between
    **Rename…** and **Delete…**. Click it. The `ForkHandlerDialog` opens.

12. The name field is pre-filled with `imported-handler-fork`. Leave it
    as-is (or change it; any valid handler name works). Click **Fork**.

13. The dialog closes and Studio navigates to
    `/handlers/imported-handler-fork`. Verify:
    - The handler appears in the list.
    - Opening the **Manifest** tab shows `name: imported-handler-fork`
      in `rowforge.yaml` (the manifest name was rewritten by the serde
      round-trip).
    - Other manifest fields (version, language, entry, etc.) match the
      source handler.

## 4. Fork edge cases

14. Navigate back to `imported-handler`. Click **Fork…** again. Change
    the name to `imported-handler-fork` (the handler created in step 13).
    Click **Fork**. Verify an inline error: "Handler
    'imported-handler-fork' already exists" (`UiError::HandlerExists`).
    The dialog stays open; clicking Cancel dismisses it without changes.

15. Manually add a YAML comment to `imported-handler`'s `rowforge.yaml`
    in an external editor:
    ```yaml
    # This comment should disappear after fork
    name: imported-handler
    version: "0.1.0"
    ```
    Save the file. Now fork `imported-handler` to `comment-test-fork`.
    Open the fork's Manifest tab. The `# This comment should disappear
    after fork` line is **gone** — this is the documented serde
    round-trip limitation. Key ordering may also differ from the
    original. This is expected behavior, not a bug.

### Known Plan 12 limitations

- Copy filter is none — `.git/`, `node_modules/`, build outputs, and any
  other files come along during both import and fork. Clean up large
  directories manually after import if needed.
- Fork loses YAML comments and may reorder keys due to the serde
  round-trip manifest rewrite.
- No cross-workspace import — the OS file dialog sources from the local
  disk only; there is no handler registry.
- Symlinks in the source folder are skipped (neither preserved nor
  followed). A `tracing::warn` is emitted for each skipped entry; no
  UI indication is shown.
- Import requires `rowforge.yaml` in the source. Pure source folders
  without a manifest must use Scaffold + manual paste instead.

---

# Manual smoke check — Plan 13 (handler smoke test)

## 1. Setup

1. Open Studio in a fresh workspace at `/tmp/plan13-smoke-ws/`.
2. Scaffold a `go_stdio` handler named `echo` with primary_field `id`.
3. Click **Open in editor**; replace the row handler body so it echoes
   `{"echoed": <id>}` on every input row. Save.
4. Click **Build** — verify "Last build" shows success.

## 2. Paste mode happy path

5. Scroll to **Smoke test** section. The radio "Paste JSON" is selected.
6. Paste two lines:
   ```
   {"id":"1"}
   {"id":"2"}
   ```
7. Header below textarea shows "2 rows parsed".
8. Click **Run smoke test**. The button switches to "Running…".
9. After a moment, the outcomes table renders 2 rows:
   - seq 1: status `success`, data column shows `{"echoed":"1"}`
   - seq 2: status `success`, data column shows `{"echoed":"2"}`
10. The counts strip shows `Outcomes (2) · ✓ 2 success · <ms> ms · exit 0`.

## 3. Paste mode invalid JSON

11. Replace line 2 with `not json`.
12. Header flips to red error: "line 2: <some parser detail>".
13. **Run smoke test** is disabled.

## 4. Synthetic mode

14. Click "One synthetic row" radio.
15. Description text appears about dispatching `{ "row": 1 }`.
16. Click Run; outcomes table renders 1 row, seq 1, status success.

## 5. Fixtures mode — jsonl

17. Create `/tmp/smoke-fx.jsonl`:
    ```
    {"id":"a"}
    {"id":"b"}
    {"id":"c"}
    ```
18. Click "Fixtures…" radio. Click "Pick file…", choose the file.
19. Code block shows the path; preview shows "3 rows loaded — keys: id".
20. Set "Rows to run" to 2.
21. Click Run; outcomes table has 2 rows.

## 6. Fixtures mode — csv

22. Create `/tmp/smoke-fx.csv`:
    ```
    id,email
    1,a@x.com
    2,b@x.com
    ```
23. Click "Change…" and pick the csv.
24. Preview shows "2 rows loaded — keys: id, email".
25. Click Run; both rows succeed.

## 7. Fixtures mode — directory

26. Create `/tmp/smoke-fx-dir/` containing both files from steps 17 and 22.
27. Click "Change…" and pick the directory.
28. Loaded rows come from the jsonl (precedence: jsonl > csv).

## 8. Fixtures mode — empty

29. Create empty `/tmp/empty.jsonl`.
30. Pick it. Red error appears: "no rows found in fixtures path".

## 9. Row count cap

31. Type `200` in the Rows-to-run input. It clamps to `100`.
32. Type `0`. It clamps to `1`.

## 10. Build failure surface

33. Break the handler source (add `syntax error` line at the top). Save.
34. Click Run smoke test (paste mode, single valid row).
35. An error block shows the BuildFailed message with the handler name
    and exit code. Outcomes table does not render.
36. Fix the source. Smoke runs again successfully.

## 11. Active-run gate (cross-process)

37. In a terminal, start a long-running exec via CLI on the same handler:
    `rowforge run --workspace /tmp/plan13-smoke-ws/ --handler echo` with
    an input file large enough that the run takes >15 seconds.
38. While the exec runs, open Studio, navigate to the handler.
39. Try to smoke test. The Run button works, but the call fails with the
    error message: `Handler "echo" has an active run. Cancel the run first.`
40. Wait for the CLI run to finish. Smoke test now succeeds again.

## 12. Stderr tail

41. Modify the handler to `fmt.Fprintln(os.Stderr, "boot")` before reading
    stdin. Rebuild.
42. Run smoke; expand the "stderr tail" details block; "boot" line
    appears.
43. Modify the handler to write a >5 KiB stderr loop (e.g. 200 lines
    each containing 50 chars). Rebuild and run smoke.
44. Stderr tail still shows ~4096 bytes (the last portion only).

## Known Plan 13 limitations

- One smoke at a time per Studio process (a workspace-wide
  `tokio::sync::Mutex` serializes calls).
- Smoke outcomes are NOT persisted — they vanish on page reload.
- Batch handlers receive rows one at a time during smoke (batch mode is
  not exercised).
- Hard cancel for a wedged smoke is not implemented (the soft cancel
  shipped with exec runs does not extend to smoke; deferred to Plan 14).
