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
4. The button changes to "✓ Started". Click it (or the page header) to refresh.
5. The Attempts table shows a new attempt row.

### Watch live progress

1. Click the new attempt row → AttemptDetail.
2. Append `?run=<handle>` to the URL — the handle comes from the
   `run_start` response (logged to the dev console / accessible via
   future Plan 5 auto-navigation). For now, the URL must be constructed
   manually OR triggered via the Replay path.

   Plan 4 limitation: full auto-routing to `?run=` is deferred to Plan 5
   (the Run button doesn't know the new attempt's id until a follow-up
   query). Use Replay (below) for the easiest end-to-end Live demo.

### Replay a terminal attempt (easiest Live demo)

1. Click a terminal attempt (state = done / aborted / crashed).
2. In the header, see the speed selector (1× / 5× / 10×) + Replay button.
3. Pick 5× or 10× and click **Replay**.
4. The page navigates with `?run=<replay-handle>`.
5. The **Live** tab appears alongside the existing tabs.
6. Click **Live** to see:
   - PhaseChipBar showing current phase
   - ProgressRegion with rate / ETA / in-flight / queue
   - EventTail filling with OutcomeSamples (default: Errors only)
7. The replay reaches Done; status flips, recent events stop updating.

### Cancel a live run

1. While a live run is active (replay or real), click the **Cancel** button
   in the header.
2. Soft confirm dialog appears: "Soft cancel? In-flight rows will finish."
3. Click "Soft cancel". The header switches to an amber "Cancelling…"
   banner with an elapsed counter.
4. After 10 seconds, a red "Force kill" button appears on the right side
   of the banner.
5. Click Force kill → confirmation dialog requiring the first 4 chars of
   the exec name typed in.

### Active runs pill

When ≥ 1 runs (real or replay) are active, the header shows a green
**N running** pill. Click it for a popover listing the active handles
and aggregate counters (total processed / total failed).

### Concurrency limits

1. Try starting a second run on the same exec — should fail with
   "execution X already has an active run" (per-exec limit = 1).
2. Start 3 different runs on 3 different execs (using Replay for the
   easiest reproduction). Starting a 4th should fail with
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

- **Auto-navigate to ?run= after Run button**: missing; deferred to Plan 5.
- **Hard cancel (Force kill)**: currently behaves identically to soft
  cancel — rowforge-core doesn't expose per-worker process kill yet.
  The dialog still requires the typed confirm token; spec requires UX
  even if backend isn't fully wired.
- **`total_rate` in active runs pill**: shows 0; SessionRegistry doesn't
  cache per-session rate. Deferred.
- **`slowest_run` in active runs popover**: shows `null`. Same reason.
