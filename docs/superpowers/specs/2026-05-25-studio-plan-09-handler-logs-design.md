# Plan 9 — Handler Log Capture + Live Tail (Logs tab)

**Date:** 2026-05-25
**Branch:** `studio-plan-09-handler-logs` (to be created)
**Builds on:** Plans 3-8

## 1. Purpose

Today (after Plan 8), handler stderr/stdout is captured only via `eprintln!` in `pool_streaming.rs` — CLI users see it inline with rowforge tracing, but it's not persisted and Studio users can't see it at all (it's on the Tauri parent process's stderr). Plan 9 makes handler streams persistable + viewable in Studio with live tail.

Concretely:
- rowforge-core writes a per-attempt `handler_log.log` file as the run progresses
- A broadcast channel lets Studio subscribe to live lines as they're produced
- Studio's attempt detail page gains a **Logs tab** alongside Summary / Rollup / Failed rows
- Logs tab shows: bootstrap (file tail) + live stream + scrollback (virtualized) + worker filter + reveal-file escape hatch

## 2. Scope decisions

| Decision | Choice | Rationale |
|---|---|---|
| Streams captured | stderr (all) + stdout (non-outcome lines only by default) | outcomes.jsonl already holds valid outcome JSON; logging it again wastes disk. Invalid stdout lines (handler accidentally println'd debug) ARE captured because they fail JSON parse. |
| Raw stdout mode | Optional Settings flag `handler_log_capture_raw_stdout: bool` (default false) | Lets advanced users replay the protocol stream when debugging handler protocol issues. |
| UI placement | New Logs tab on attempt detail | Doesn't clutter Live tab; full-width log surface; consistent with existing tab pattern. |
| Log persistence | File on disk: `<exec_dir>/attempts/<attempt_id>/handler_log.log` | Survives Studio restart, CLI users can `cat`/`less` it, naturally captures concurrent worker output via append-only writes. |
| File size cap | Soft: warn at 100 MB (file metadata check at open); hard: none | A pathological handler dumping gigs of stderr is the handler's bug. Document and move on. |
| Bootstrap shape | Tail last 5000 lines (or 256 KB, whichever smaller) | Snappy first paint; user clicks "Load more" or "Reveal log file" for older content. |
| Live stream backpressure | Broadcast channel cap 4096 lines | If Studio not subscribed (UI minimized, slow), oldest drop with sentinel "N lines dropped". File still gets everything (separate write path). |

## 3. File layout

```
<workspace>/executions/<exec_id>/attempts/<attempt_id>/
├── attempt.json                    # existing
├── outcomes.jsonl                  # existing (valid outcome JSON only)
├── handler_log.log                 # NEW (Plan 9)
├── progress.json                   # existing
└── ...
```

**`handler_log.log` format**:
- UTF-8 plain text, append-only
- One line per captured stream line
- Each line prefixed with: `<ISO timestamp> [handler#<worker_id> <stream>] <line>`
  - Example: `2026-05-25T10:00:01.234Z [handler#0 stderr] connecting to db...`
  - Example: `2026-05-25T10:00:01.567Z [handler#2 stdout] DEBUG: cache miss`  (only logged when stdout line is non-JSON OR raw_stdout flag on)
- No trailing newline normalization; preserve whatever handler emits

> The timestamp + worker_id + stream prefix is **part of the on-disk format** so `cat`/`less` users can parse without rowforge-specific tooling.

## 4. rowforge-core changes

### 4.1 `pool_streaming.rs` — tee streams to file + channel

Current behavior (line ~188-205): handler stderr drained line-by-line and `eprintln!`'d. Plan 9 changes this to:

```rust
// pseudocode — actual integration depends on existing types
async fn drain_stream(
    worker_id: usize,
    stream: HandlerStream,             // Stderr | Stdout
    reader: impl AsyncBufReadExt + Unpin,
    file_writer: Arc<Mutex<tokio::fs::File>>,
    broadcast_tx: tokio::sync::broadcast::Sender<HandlerLogLine>,
    is_outcome_line: impl Fn(&str) -> bool,  // only used for stdout
    capture_raw_stdout: bool,
) {
    let mut lines = reader.lines();
    while let Ok(Some(line)) = lines.next_line().await {
        // For stdout: skip if it parses as valid outcome JSON, unless raw mode.
        if matches!(stream, HandlerStream::Stdout) && !capture_raw_stdout && is_outcome_line(&line) {
            continue;  // outcome goes to outcomes.jsonl, not here
        }

        let now = chrono::Utc::now();
        let log_line = HandlerLogLine {
            timestamp: now,
            worker_id,
            stream,
            line: line.clone(),
        };

        // 1. File (sync — buffered writer flushes on each line for crash safety)
        if let Ok(mut f) = file_writer.lock().await {
            let _ = f.write_all(
                format!("{} [handler#{} {}] {}\n",
                    now.to_rfc3339(), worker_id, stream.as_str(), line)
                    .as_bytes()
            ).await;
        }

        // 2. Broadcast (best-effort; backpressure drops oldest)
        let _ = broadcast_tx.send(log_line);

        // 3. CLI still gets eprintln! for backward compat
        if cfg!(feature = "cli-stderr-echo") {  // or always; see §6 CLI
            eprintln!("[handler#{} {}] {}", worker_id, stream.as_str(), line);
        }
    }
}
```

### 4.2 New types (in `rowforge-core::handler_log` module or co-located in pool_streaming.rs)

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[non_exhaustive]
pub enum HandlerStream {
    Stdout,
    Stderr,
}

impl HandlerStream {
    pub fn as_str(&self) -> &'static str {
        match self {
            HandlerStream::Stdout => "stdout",
            HandlerStream::Stderr => "stderr",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct HandlerLogLine {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub worker_id: usize,
    pub stream: HandlerStream,
    pub line: String,
}
```

### 4.3 ProgressCallback extension

`ProgressCallback` (Plan 5 made it `Arc<dyn Fn>`) currently handles row/progress events. Add a parallel `HandlerLogCallback`:

```rust
pub type HandlerLogCallback = Arc<dyn Fn(HandlerLogLine) + Send + Sync>;

pub struct StreamingPoolConfig {
    // ...existing fields...
    pub on_handler_log: Option<HandlerLogCallback>,   // NEW
    pub capture_raw_stdout: bool,                      // NEW (default false)
}
```

CLI passes `None` (writes only to file + still eprintln!s). Studio passes a callback that forwards to a Tauri event.

### 4.4 File path convention

A new helper in rowforge-core:

```rust
pub fn handler_log_path(attempt_dir: &Path) -> PathBuf {
    attempt_dir.join("handler_log.log")
}
```

Used by both the writer and any reader (CLI `tail` subcommand, Studio bootstrap).

## 5. studio-core changes

### 5.1 New API: bootstrap + stream

```rust
impl StudioCore {
    /// Read up to `max_lines` from the tail of the handler log for this attempt.
    /// Returns lines parsed back from the on-disk format. Used for the snapshot
    /// when the Logs tab mounts.
    pub fn handler_log_tail(
        &self,
        exec_id: &str,
        attempt_id: &str,
        max_lines: usize,
    ) -> Result<Vec<HandlerLogLine>, UiError>;

    /// Subscribe to live updates for a running attempt. Backpressure: bounded
    /// channel, oldest dropped if Studio falls behind. The returned receiver
    /// is wrapped into a Tauri event stream at the shell layer.
    pub fn handler_log_subscribe(
        &self,
        exec_id: &str,
        attempt_id: &str,
    ) -> Result<tokio::sync::broadcast::Receiver<HandlerLogLine>, UiError>;
}
```

### 5.2 Routing

Plan 4's `SessionRegistry` keeps live attempts indexed. Extend it to hold one `broadcast::Sender<HandlerLogLine>` per active attempt. `StreamingPoolConfig.on_handler_log` is wired to that sender at attempt-start time. `handler_log_subscribe` returns `sender.subscribe()`.

If the attempt is no longer live (already finished), `handler_log_subscribe` returns a Receiver that closes immediately — UI falls back to "showing static file content".

## 6. CLI

Backward compat: CLI still prints `[handler#N stream] line` to stderr (existing behavior). New flag NOT added in v1 — the file is just there alongside outcomes.jsonl.

Optionally: `rowforge attempt logs <attempt_id>` subcommand to tail the file. Add if scope allows; defer otherwise.

## 7. Tauri shell

### 7.1 New commands

```rust
#[tauri::command]
pub fn handler_log_tail(
    state: State<'_, AppState>,
    exec_id: String,
    attempt_id: String,
    max_lines: Option<usize>,    // default 5000
) -> Result<Vec<HandlerLogLine>, UiError>;

#[tauri::command]
pub async fn handler_log_subscribe(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    exec_id: String,
    attempt_id: String,
) -> Result<(), UiError>;
```

`handler_log_subscribe` spawns a task that pumps the receiver into Tauri events `handler_log:<attempt_id>` (one event per batch of ~50 lines or every 100ms, whichever first). Cancels on attempt-detail unmount via a stop command.

### 7.2 New event

```
event: "handler_log:<attempt_id>"
payload: { lines: HandlerLogLine[], dropped: number }
```

`dropped` carries the count if the broadcast channel dropped lines under backpressure.

### 7.3 Unsubscribe

```rust
#[tauri::command]
pub fn handler_log_unsubscribe(
    state: State<'_, AppState>,
    attempt_id: String,
) -> Result<(), UiError>;
```

Called from the React Logs tab unmount.

## 8. React UI

### 8.1 New Logs tab

Add to attempt detail tabs (alongside Summary / Rollup / Failed rows / Live). Visible for all attempts (running and finished).

### 8.2 Component: `AttemptLogsTab`

```tsx
function AttemptLogsTab({ execId, attemptId, isLive }: Props) {
  // 1. Bootstrap: read tail
  const tail = useHandlerLogTail(execId, attemptId);

  // 2. Live stream: only subscribe if attempt is still running
  const [liveLines, setLiveLines] = useState<HandlerLogLine[]>([]);
  const [dropped, setDropped] = useState(0);
  useEffect(() => {
    if (!isLive) return;
    ipc.handler_log_subscribe({ execId, attemptId });
    const unlisten = listen(`handler_log:${attemptId}`, (event) => {
      setLiveLines(prev => [...prev, ...event.payload.lines]);
      if (event.payload.dropped > 0) setDropped(d => d + event.payload.dropped);
    });
    return () => {
      unlisten.then(fn => fn());
      ipc.handler_log_unsubscribe({ attemptId });
    };
  }, [execId, attemptId, isLive]);

  const allLines = [...(tail.data ?? []), ...liveLines];
  const filtered = useFiltered(allLines, filters);

  return (
    <div className="flex flex-col h-full">
      <Toolbar
        workerFilter={workerFilter}
        streamFilter={streamFilter}
        searchTerm={searchTerm}
        autoScroll={autoScroll}
        onReveal={() => ipc.attempt_reveal_log({ execId, attemptId })}
      />
      {dropped > 0 && <DroppedBanner count={dropped} />}
      <VirtualizedLogList lines={filtered} autoScroll={autoScroll} />
    </div>
  );
}
```

### 8.3 Virtualized log list

Uses `@tanstack/react-virtual` (already a Plan 3-5 dep). Renders ~30 visible rows + buffer. Each row is a single line with:
- Timestamp (subdued)
- Worker badge `#0` / `#1` (color-coded per worker for visual ID)
- Stream chip `stdout` (blue) / `stderr` (yellow)
- The line content (monospace, whitespace-preserving)

Click on a line → copies it to clipboard with timestamp+prefix.

### 8.4 Toolbar

- **Worker filter**: multi-select chip per worker_id seen so far (auto-populated)
- **Stream filter**: stdout / stderr / both
- **Search**: case-insensitive substring filter over the line content
- **Auto-scroll**: toggle (default ON; turns OFF when user scrolls up > 100px from bottom)
- **Reveal log file** button (uses tauri_plugin_shell::open with file path)
- **Pause** button: stops appending live lines to the visible list while keeping the subscription alive (so you can read without it scrolling away). Resume re-appends everything received during pause.

### 8.5 Dropped banner

If the backpressure channel dropped lines (UI lag, app minimized), show:
```
⚠ N log lines dropped during high-throughput period.
  [Reveal log file] for complete capture.
```

The file always has everything — drops only affect the live stream.

### 8.6 Empty / no-log states

- Attempt has no `handler_log.log` (e.g. older attempts pre-Plan 9) → "No log file. This attempt predates Plan 9 log capture." with reveal-file disabled.
- File exists but empty → "Handler has not produced any output yet." with live indicator if attempt running.

## 9. Settings additions

```rust
pub struct Settings {
    // ...existing fields...
    pub handler_log_capture_raw_stdout: bool,    // NEW; default false
}
```

UI: SettingsForm gains a checkbox under a new "Logs" section:

```
Logs
☐ Capture raw stdout (every line, including valid outcome JSON)
  Default off — only non-outcome stdout is logged, since outcomes
  already go to outcomes.jsonl. Turn on to debug protocol issues.
```

Storage: `settings.json` (no schema migration; schema_version stays at 1 since Plan 7 — the field is nullable on read).

The setting is read at attempt-start time (passed into `StreamingPoolConfig.capture_raw_stdout`). Changing it mid-run doesn't affect in-flight attempts; effect on next run.

## 10. Out of scope (explicit)

- Log rotation / size limits beyond a soft 100 MB warning
- Search across multiple attempts
- Server-side regex filter (filter only on the visible lines client-side)
- Log export (use Reveal in finder + manual copy)
- Cross-attempt diff
- Color coding by log level (e.g. parsing `ERROR` / `WARN` keywords) — handler-conventional, not stable
- Stderr from rowforge-core itself (the runtime, not the handler) — handler-only for v1

## 11. Testing

| Suite | Adds | Notes |
|---|---|---|
| rowforge-core | ~6 | tee write to file + channel; stdout JSON skip; raw stdout opt-in; broadcast backpressure drops; HandlerLogLine serde shape; handler_log_path helper |
| rowforge-studio-core | ~4 | handler_log_tail reads existing file; handler_log_subscribe returns receiver; closed receiver for finished attempts; UiError envelopes |
| rowforge-studio (Tauri) | ~2 | ipc_contract: command symbols + event payload shape |
| vitest | ~10 | useHandlerLogTail + useHandlerLog stream hook; AttemptLogsTab states (loading / empty / populated / dropped / live-paused); filter toolbar interactions |

Targets:
- cargo: 344 → ~356 (+12)
- vitest: 121 → ~131 (+10)

## 12. Spec doc updates

- `docs/spec/studio/part-3-runtime.md`: §3 (pool streaming) gains handler_log.log writer + broadcast channel
- `docs/spec/studio/part-4-data.md`: file layout section adds handler_log.log to per-attempt dir
- `docs/spec/studio/part-5-api.md`: §5.3 new UiError variants (if any); §5.5 handler_log_tail / handler_log_subscribe / handler_log_unsubscribe commands; events section adds `handler_log:<attempt_id>`
- `docs/spec/studio/part-7-ui.md`: §7.3 IA — Logs tab added to attempt detail; §7.4 add Logs tab flow
- `docs/spec/studio/part-2-model.md`: Settings.handler_log_capture_raw_stdout
- All mirrored in zh-Hant

## 13. Acceptance criteria

1. `cargo build && cargo test` clean
2. `pnpm tsc -b && pnpm test && pnpm build` clean
3. CLI `rowforge exec run` still prints `[handler#N stream] line` to terminal (backward compat)
4. After a run completes, `cat <exec_dir>/attempts/<attempt_id>/handler_log.log` shows the full transcript
5. Studio attempt detail page has a Logs tab; clicking it shows the tail of the log file
6. While a run is live, new lines appear in the tab within ~200ms of being emitted by the handler
7. Worker / stream / search filters narrow the visible list correctly
8. Auto-scroll toggles off when user scrolls up > 100px from bottom; toggles on when user scrolls back to bottom
9. Reveal log file button opens the file in OS file manager
10. Settings.handler_log_capture_raw_stdout toggle controls whether valid outcome JSON lines appear in the log file (default off → they don't)
11. 100 MB file size soft-warn appears as a banner in the Logs tab when file exceeds threshold (still works; just a notice)
12. Backpressure dropped-lines sentinel appears when UI falls behind a fast handler

## 14. Open questions

None at design time. Implementer may surface clarifications.
