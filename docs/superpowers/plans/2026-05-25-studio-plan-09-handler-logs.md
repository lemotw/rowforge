# Plan 9 — Handler Log Capture + Live Tail Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Persist handler stderr/stdout per attempt to a log file, broadcast lines live to subscribers, surface a new Logs tab on attempt detail with virtualized scrollback, filters, and reveal-file escape hatch.

**Architecture:** rowforge-core's `pool_streaming` tees worker streams to (1) a per-attempt `handler_log.log` file and (2) a `tokio::sync::broadcast` channel. studio-core's `SessionRegistry` holds the channel sender per live attempt; Studio reads the file for snapshot, subscribes to the channel for live. Tauri batches events ~50 lines / 100ms to the frontend. React renders with `@tanstack/react-virtual`.

**Tech stack:** Rust (rowforge-core, rowforge-studio-core, Tauri 2), React 19 + Vite 6 + TanStack Query v5 + react-virtual.

**Design spec:** `docs/superpowers/specs/2026-05-25-studio-plan-09-handler-logs-design.md`

---

## Task 1: rowforge-core handler_log types + path helper

**Files:**
- Create: `crates/rowforge-core/src/handler_log.rs`
- Modify: `crates/rowforge-core/src/lib.rs` (add `pub mod handler_log;`)
- Test: same file

- [ ] **Step 1: Add module declaration**

In `lib.rs`, add `pub mod handler_log;` near `pub mod build;`.

- [ ] **Step 2: Type skeleton**

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
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
    pub timestamp: DateTime<Utc>,
    pub worker_id: usize,
    pub stream: HandlerStream,
    pub line: String,
}

pub fn handler_log_path(attempt_dir: &Path) -> PathBuf {
    attempt_dir.join("handler_log.log")
}

/// Format a line for on-disk persistence and CLI echo.
pub fn format_line(line: &HandlerLogLine) -> String {
    format!(
        "{} [handler#{} {}] {}\n",
        line.timestamp.to_rfc3339(),
        line.worker_id,
        line.stream.as_str(),
        line.line,
    )
}

/// Parse a line back from the on-disk format. Returns None if the line
/// doesn't conform (e.g. plain-text manual edits, or non-prefix lines).
pub fn parse_line(line: &str) -> Option<HandlerLogLine> {
    // Format: "<rfc3339> [handler#<wid> <stream>] <content>"
    let (ts, rest) = line.split_once(" [handler#")?;
    let timestamp = DateTime::parse_from_rfc3339(ts).ok()?.with_timezone(&Utc);
    let (wid_str, rest) = rest.split_once(' ')?;
    let worker_id: usize = wid_str.parse().ok()?;
    let (stream_str, content) = rest.split_once("] ")?;
    let stream = match stream_str {
        "stdout" => HandlerStream::Stdout,
        "stderr" => HandlerStream::Stderr,
        _ => return None,
    };
    Some(HandlerLogLine {
        timestamp,
        worker_id,
        stream,
        line: content.trim_end_matches('\n').to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_and_parse_round_trip() {
        let line = HandlerLogLine {
            timestamp: "2026-05-25T10:00:01.234Z".parse().unwrap(),
            worker_id: 3,
            stream: HandlerStream::Stderr,
            line: "connecting to db...".into(),
        };
        let formatted = format_line(&line);
        let parsed = parse_line(&formatted).expect("parse ok");
        assert_eq!(parsed.worker_id, 3);
        assert_eq!(parsed.stream, HandlerStream::Stderr);
        assert_eq!(parsed.line, "connecting to db...");
    }

    #[test]
    fn parse_returns_none_for_non_conforming() {
        assert!(parse_line("plain text").is_none());
        assert!(parse_line("2026-05-25T10:00:00Z [bad prefix]").is_none());
    }

    #[test]
    fn handler_log_path_appends_filename() {
        let p = handler_log_path(Path::new("/tmp/exec/attempt1"));
        assert_eq!(p, Path::new("/tmp/exec/attempt1/handler_log.log"));
    }
}
```

- [ ] **Step 3: Verify**

```bash
cargo test -p rowforge-core --lib handler_log::tests
```

Expected: 3 PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/rowforge-core/src/handler_log.rs crates/rowforge-core/src/lib.rs
git commit -m "rowforge-core: handler_log module — types + path helper

HandlerStream (Stdout | Stderr) + HandlerLogLine + handler_log_path +
format_line + parse_line. The on-disk format embeds timestamp +
worker_id + stream prefix so cat/less users can parse without
rowforge-specific tooling.

3 unit tests: round-trip serialization, non-conforming reject, path
helper."
```

---

## Task 2: pool_streaming tee writer

**Files:**
- Modify: `crates/rowforge-core/src/pool_streaming.rs`
- Test: same file (or integration test)

- [ ] **Step 1: Locate stderr drain**

```bash
grep -nE 'take_stderr|drain.*stderr|\[handler#' crates/rowforge-core/src/pool_streaming.rs
```

Read the current drain block (~line 188-205 from Plan 8 review). Note: it currently only drains stderr; you need to ALSO drain stdout AND tee.

- [ ] **Step 2: Add HandlerLogCallback to StreamingPoolConfig**

```rust
use crate::handler_log::{HandlerLogLine, HandlerStream, handler_log_path, format_line};

pub type HandlerLogCallback = std::sync::Arc<dyn Fn(HandlerLogLine) + Send + Sync>;

pub struct StreamingPoolConfig {
    // ...existing fields...
    pub on_handler_log: Option<HandlerLogCallback>,
    pub capture_raw_stdout: bool,
}
```

Backward compat: default `None` + `false` in any `..Default::default()` paths.

- [ ] **Step 3: Open the log file at pool start**

Near where `pool_streaming` opens other per-attempt files (search `outcomes.jsonl` opening pattern in the same file for the convention):

```rust
let log_path = handler_log_path(&attempt_dir);
let log_file = tokio::fs::OpenOptions::new()
    .create(true)
    .append(true)
    .open(&log_path)
    .await
    .map_err(|e| /* existing error type */)?;
let log_file = std::sync::Arc::new(tokio::sync::Mutex::new(log_file));
```

- [ ] **Step 4: Replace the stderr drain with a tee'd drainer for both streams**

Refactor the existing stderr drain. New shape:

```rust
async fn drain_stream(
    worker_id: usize,
    stream_kind: HandlerStream,
    reader: impl tokio::io::AsyncBufReadExt + Unpin + Send + 'static,
    log_file: std::sync::Arc<tokio::sync::Mutex<tokio::fs::File>>,
    on_handler_log: Option<HandlerLogCallback>,
    capture_raw_stdout: bool,
) {
    use tokio::io::AsyncWriteExt;
    let mut lines = reader.lines();
    while let Ok(Some(line)) = lines.next_line().await {
        // For stdout, skip valid outcome JSON unless raw_stdout flag on.
        if matches!(stream_kind, HandlerStream::Stdout)
            && !capture_raw_stdout
            && line_is_valid_outcome(&line)
        {
            continue;
        }
        let entry = HandlerLogLine {
            timestamp: chrono::Utc::now(),
            worker_id,
            stream: stream_kind,
            line: line.clone(),
        };
        // 1. File
        if let mut f = log_file.lock().await {
            let _ = f.write_all(format_line(&entry).as_bytes()).await;
            // No explicit flush per line; rely on OS page cache. Flush on close.
        }
        // 2. Broadcast / callback
        if let Some(cb) = &on_handler_log {
            cb(entry.clone());
        }
        // 3. CLI back-compat echo to stderr
        eprintln!("[handler#{} {}] {}", worker_id, stream_kind.as_str(), line);
    }
}

fn line_is_valid_outcome(line: &str) -> bool {
    // Quick JSON-shape check: starts with '{' and parses as JSON object.
    // Don't fully validate the outcome schema here — just "is it JSON".
    line.trim_start().starts_with('{')
        && serde_json::from_str::<serde_json::Value>(line).is_ok()
}
```

Wire stderr drain (replace existing eprintln-only path) AND a parallel stdout drainer at the same worker-spawn site. The existing stdout reader is consumed by the outcome parser — you need to use a `tokio::io::split` or process the reader once and tee. Easiest: instead of teeing the stdout reader (it's already in a parser), have the outcome parser call back when it sees a non-JSON line. Specifically:

- Existing stdout-handling code reads each line, attempts to parse as `Outcome` JSON.
- On parse success → push to outcomes channel as today
- On parse failure → call `on_handler_log` with `HandlerStream::Stdout` + write to log file (this captures "handler printed debug to stdout by mistake")
- If `capture_raw_stdout` is true → ALSO call on_handler_log on parse success (file gets the valid outcome JSON too)

Look for the outcome parsing in `pool_streaming.rs` or a sibling file (worker.rs reads stdout?). Wire the log-tee at the parse step, not via a separate drainer.

- [ ] **Step 5: Tests**

Integration tests in `crates/rowforge-core/tests/` (new file `handler_log_integration.rs`):

```rust
// Setup: a tiny pool_streaming run with a stub handler that emits a few
// stderr lines + a malformed stdout line, run synchronously to completion,
// then verify the log file exists with the expected lines AND the broadcast
// callback received them.

use std::path::Path;
use std::sync::{Arc, Mutex};
use rowforge_core::handler_log::{HandlerLogLine, HandlerStream, handler_log_path, parse_line};

#[tokio::test]
async fn pool_streaming_writes_handler_log_file() {
    // Use a stub handler (sh -c '... echo to stderr ... echo invalid stdout ...')
    // Run pool_streaming for 1 row.
    // After completion: read handler_log.log; assert non-empty and
    // contains the stderr line + invalid stdout line.
    todo!("wire to existing test helpers in this crate")
}

#[tokio::test]
async fn pool_streaming_broadcasts_handler_log_lines() {
    let received = Arc::new(Mutex::new(Vec::<HandlerLogLine>::new()));
    let received_clone = received.clone();
    let cb: HandlerLogCallback = Arc::new(move |line| {
        received_clone.lock().unwrap().push(line);
    });
    // Run pool_streaming with on_handler_log = Some(cb).
    // After completion: assert received contains the stderr line.
    todo!()
}

#[tokio::test]
async fn raw_stdout_flag_captures_valid_outcome_json() {
    // capture_raw_stdout = true; run; assert log file contains stdout lines
    // that are valid outcome JSON (which would normally be skipped).
    todo!()
}
```

Look at existing tests in `crates/rowforge-core/tests/` for the test helper patterns — there's likely a fixture builder or `TestHandler` mock.

- [ ] **Step 6: Verify**

```bash
cargo test -p rowforge-core --test handler_log_integration
cargo test -p rowforge-core   # no regressions
```

- [ ] **Step 7: Commit**

```bash
git add crates/rowforge-core/src/pool_streaming.rs crates/rowforge-core/tests/handler_log_integration.rs
git commit -m "rowforge-core: pool_streaming tees handler streams to log + broadcast

pool_streaming now writes per-attempt handler_log.log alongside
outcomes.jsonl. Each line carries timestamp + worker_id + stream
prefix for cat/less compatibility.

StreamingPoolConfig gains:
- on_handler_log: Option<HandlerLogCallback> — Studio subscribes
  via SessionRegistry (Plan 9 T4)
- capture_raw_stdout: bool (default false) — when false, valid
  outcome JSON lines on stdout are NOT logged (already in
  outcomes.jsonl); when true, raw stdout is captured for protocol
  debugging

CLI back-compat: still eprintln!s '[handler#N stream] line' so
existing terminal workflows are unchanged.

3 integration tests."
```

---

## Task 3: ProgressCallback extension review

**Files:**
- Possibly modify: existing types in rowforge-core if `StreamingPoolConfig` shape doesn't accept the new fields cleanly
- This is a buffer task — primarily to verify T2's API extension didn't break callers (CLI and Studio)

- [ ] **Step 1: Audit callers**

```bash
grep -rn 'StreamingPoolConfig\|pool_streaming::' crates/ apps/rowforge-studio/src-tauri/ 2>/dev/null
```

Verify:
- `rowforge-cli` callers: pass `on_handler_log: None`, `capture_raw_stdout: false`. If they use a struct literal vs builder, struct literal needs the new fields explicitly.
- `rowforge-studio-core` callers: Plan 5 wired RunProgressEvent handling via `on_row_done` etc.; here we'll inject the new fields at attempt-start.

- [ ] **Step 2: Update CLI call sites**

If CLI uses `StreamingPoolConfig { ... }` literal, add the two new fields. If it uses a builder, no change.

- [ ] **Step 3: Cargo build clean**

```bash
cargo build
cargo test -p rowforge-cli
```

No regressions.

- [ ] **Step 4: Commit (if any changes)**

If T2 didn't break callers (e.g. struct uses `..Default::default()`), this task can be empty — just verify and move on. If it broke callers:

```bash
git add crates/rowforge-cli/src/
git commit -m "rowforge-cli: pass None for new StreamingPoolConfig handler-log fields

Plan 9 T2 added on_handler_log + capture_raw_stdout to
StreamingPoolConfig. CLI doesn't subscribe to live log events
(it eprintln!s as before via the back-compat path in pool_streaming),
so both fields are None / false."
```

---

## Task 4: studio-core SessionRegistry + handler_log API

**Files:**
- Modify: `crates/rowforge-studio-core/src/session.rs` (or wherever SessionRegistry lives)
- Modify: `crates/rowforge-studio-core/src/lib.rs` (add handler_log_tail + handler_log_subscribe methods)
- Test: `crates/rowforge-studio-core/tests/foundation.rs`

- [ ] **Step 1: Locate SessionRegistry**

```bash
grep -rn 'pub struct SessionRegistry\|impl SessionRegistry' crates/rowforge-studio-core/src/
```

Read enough to understand how it stores per-attempt state. Plan 4-6 hold things like `active_runs: HashMap<AttemptId, RunHandle>`.

- [ ] **Step 2: Add broadcast sender per attempt**

```rust
struct RunHandle {
    // ...existing fields...
    handler_log_tx: tokio::sync::broadcast::Sender<HandlerLogLine>,
}

impl SessionRegistry {
    pub fn handler_log_subscribe(&self, attempt_id: &str)
        -> Option<tokio::sync::broadcast::Receiver<HandlerLogLine>>
    {
        self.active_runs.lock().unwrap()
            .get(attempt_id)
            .map(|h| h.handler_log_tx.subscribe())
    }
}
```

Channel capacity: 4096 lines (oldest dropped under backpressure).

- [ ] **Step 3: Wire the sender into StreamingPoolConfig at attempt-start**

Where `studio-core::start_run` (or similar — find via `grep StreamingPoolConfig crates/rowforge-studio-core/src/`) constructs the config:

```rust
let (handler_log_tx, _) = tokio::sync::broadcast::channel(4096);
let tx_clone = handler_log_tx.clone();
let config = StreamingPoolConfig {
    // ...existing fields...
    on_handler_log: Some(Arc::new(move |line| {
        let _ = tx_clone.send(line);   // ignore lag/no-subscriber errors
    })),
    capture_raw_stdout: settings.handler_log_capture_raw_stdout,  // Plan 9 T5
};
// store handler_log_tx in the RunHandle so subscribers can find it
```

- [ ] **Step 4: handler_log_tail (file read for snapshot)**

```rust
impl StudioCore {
    pub fn handler_log_tail(
        &self,
        exec_id: &str,
        attempt_id: &str,
        max_lines: usize,
    ) -> Result<Vec<HandlerLogLine>, UiError> {
        let attempt_dir = self.workspace.root.as_path()
            .join("executions").join(exec_id)
            .join("attempts").join(attempt_id);
        let path = rowforge_core::handler_log::handler_log_path(&attempt_dir);
        if !path.exists() {
            return Ok(vec![]);  // no log yet
        }
        // Read last max_lines (or last 256 KB, whichever smaller).
        let content = std::fs::read_to_string(&path)
            .map_err(|e| UiError::Io(format!("read handler log: {}", e)))?;
        let lines: Vec<_> = content
            .lines()
            .rev()
            .take(max_lines)
            .filter_map(rowforge_core::handler_log::parse_line)
            .collect();
        // Reverse back to chronological order.
        Ok(lines.into_iter().rev().collect())
    }

    pub fn handler_log_subscribe(
        &self,
        attempt_id: &str,
    ) -> Result<tokio::sync::broadcast::Receiver<rowforge_core::handler_log::HandlerLogLine>, UiError> {
        self.session_registry.handler_log_subscribe(attempt_id)
            .ok_or_else(|| UiError::Io(format!("attempt {} is not active", attempt_id)))
    }
}
```

- [ ] **Step 5: Tests**

Append to `foundation.rs`:

```rust
#[test]
fn handler_log_tail_returns_empty_when_no_file() {
    let tmp = TempDir::new().unwrap();
    let core = open_test_workspace(tmp.path());
    let result = core.handler_log_tail("e_nonexistent", "att_x", 100).unwrap();
    assert!(result.is_empty());
}

#[test]
fn handler_log_tail_parses_lines_from_disk() {
    let tmp = TempDir::new().unwrap();
    let core = open_test_workspace(tmp.path());
    let attempt_dir = tmp.path().join("executions/e_test/attempts/att_test");
    std::fs::create_dir_all(&attempt_dir).unwrap();
    let log = attempt_dir.join("handler_log.log");
    std::fs::write(&log, "2026-05-25T10:00:00Z [handler#0 stderr] hello\n\
                          2026-05-25T10:00:01Z [handler#1 stdout] garbage\n").unwrap();
    let lines = core.handler_log_tail("e_test", "att_test", 100).unwrap();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].line, "hello");
}

#[test]
fn handler_log_subscribe_fails_for_inactive_attempt() {
    let tmp = TempDir::new().unwrap();
    let core = open_test_workspace(tmp.path());
    let result = core.handler_log_subscribe("att_not_running");
    assert!(result.is_err());
}

#[test]
fn handler_log_subscribe_returns_receiver_for_active_attempt() {
    // Bootstrap an attempt via SessionRegistry directly (or via a test helper
    // that doesn't require running a real pool). Verify subscribe returns Ok.
    todo!("requires SessionRegistry test fixture")
}
```

- [ ] **Step 6: Verify**

```bash
cargo test -p rowforge-studio-core
```

Expected: all PASS, +4 new tests.

- [ ] **Step 7: Commit**

```bash
git add crates/rowforge-studio-core/src/
git commit -m "studio-core: handler_log_tail + handler_log_subscribe + per-attempt broadcast

SessionRegistry now holds a tokio broadcast sender per active attempt
(cap 4096 lines; oldest dropped under backpressure). At attempt-start,
StreamingPoolConfig.on_handler_log forwards each line into that sender.

StudioCore API:
- handler_log_tail(exec, attempt, max_lines) -> reads handler_log.log
  from disk, parses last N lines in chronological order. Used for
  the Logs tab's snapshot on mount.
- handler_log_subscribe(attempt) -> Receiver. Returns Err for inactive
  attempts (UI falls back to static file view).

4 new integration tests."
```

---

## Task 5: Settings.handler_log_capture_raw_stdout

**Files:**
- Modify: `crates/rowforge-studio-core/src/settings.rs` (or wherever Settings is defined)
- Modify: `apps/rowforge-studio/src-tauri/src/commands.rs` (settings_save needs to refresh the field if any) — or none if Settings is read fresh each attempt
- Modify: `apps/rowforge-studio/src/ipc/types.ts`
- Modify: `apps/rowforge-studio/src/components/SettingsForm.tsx`

- [ ] **Step 1: Add the field**

```rust
pub struct Settings {
    // ...existing fields...
    #[serde(default)]
    pub handler_log_capture_raw_stdout: bool,
}
```

Default false. `#[serde(default)]` so existing settings.json files without the field deserialize cleanly.

- [ ] **Step 2: Wire into attempt-start**

In studio-core's start_run path (T4 step 3), read `settings.handler_log_capture_raw_stdout` and pass to `StreamingPoolConfig.capture_raw_stdout`.

- [ ] **Step 3: TS mirror**

In types.ts:
```ts
export interface Settings {
  // ...existing fields...
  handler_log_capture_raw_stdout: boolean;
}
```

- [ ] **Step 4: SettingsForm "Logs" section**

Between Telemetry section and the bottom of the form:

```tsx
<Section title="Logs">
  <label className="flex items-start gap-2 text-sm">
    <input
      type="checkbox"
      checked={form.handler_log_capture_raw_stdout}
      onChange={(e) =>
        setForm({ ...form, handler_log_capture_raw_stdout: e.target.checked })
      }
      className="mt-1"
    />
    <div>
      <div>Capture raw stdout in handler log</div>
      <div className="text-xs text-muted-foreground">
        Default off — only non-outcome stdout is logged, since outcomes
        already go to outcomes.jsonl. Turn on to debug protocol issues.
      </div>
    </div>
  </label>
</Section>
```

- [ ] **Step 5: Tests**

Append a SettingsForm vitest:

```tsx
it("renders raw stdout capture toggle", () => {
  // mock settings, render, assert checkbox is present
});

it("save sends handler_log_capture_raw_stdout", () => {
  // mock invoke, render, toggle, click save, assert payload
});
```

- [ ] **Step 6: Verify + commit**

```bash
cargo test -p rowforge-studio-core
cd apps/rowforge-studio
pnpm tsc -b && pnpm test
```

Commit message:
```
studio: Settings.handler_log_capture_raw_stdout

New nullable Settings field controlling whether valid outcome JSON
stdout lines are duplicated into handler_log.log. Default false
(off): outcomes go only to outcomes.jsonl; protocol-debugging users
flip it on.

Read at attempt-start (via StudioCore start_run path); changes
mid-run don't affect in-flight attempts.

SettingsForm gains a Logs section below Telemetry. 2 new vitest.
```

---

## Task 6: Tauri commands + event batching

**Files:**
- Modify: `apps/rowforge-studio/src-tauri/src/commands.rs`
- Modify: `apps/rowforge-studio/src-tauri/src/lib.rs` (register invoke_handler!)
- Modify: `apps/rowforge-studio/src-tauri/tests/ipc_contract.rs`

- [ ] **Step 1: Three commands**

```rust
use rowforge_core::handler_log::HandlerLogLine;

#[tauri::command]
pub fn handler_log_tail(
    state: State<'_, AppState>,
    exec_id: String,
    attempt_id: String,
    max_lines: Option<usize>,
) -> Result<Vec<HandlerLogLine>, UiError> {
    let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
    let core = guard.as_ref()
        .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
    core.handler_log_tail(&exec_id, &attempt_id, max_lines.unwrap_or(5000))
}

#[tauri::command]
pub async fn handler_log_subscribe(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    exec_id: String,
    attempt_id: String,
) -> Result<(), UiError> {
    let rx_result = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        let core = guard.as_ref()
            .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
        core.handler_log_subscribe(&attempt_id)
    };
    let mut rx = rx_result?;
    let event_name = format!("handler_log:{}", attempt_id);

    // Track the task so unsubscribe can stop it.
    let cancel_token = state.handler_log_cancels
        .entry(attempt_id.clone())
        .or_insert_with(|| tokio_util::sync::CancellationToken::new())
        .clone();

    tokio::spawn(async move {
        use tokio::sync::broadcast::error::RecvError;
        let mut batch = Vec::<HandlerLogLine>::with_capacity(64);
        let mut dropped = 0u64;
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(100));
        loop {
            tokio::select! {
                _ = cancel_token.cancelled() => break,
                msg = rx.recv() => match msg {
                    Ok(line) => batch.push(line),
                    Err(RecvError::Lagged(n)) => dropped += n,
                    Err(RecvError::Closed) => break,
                },
                _ = interval.tick() => {
                    if !batch.is_empty() || dropped > 0 {
                        let _ = app.emit(&event_name, serde_json::json!({
                            "lines": batch,
                            "dropped": dropped,
                        }));
                        batch.clear();
                        dropped = 0;
                    }
                },
            }
        }
    });

    Ok(())
}

#[tauri::command]
pub fn handler_log_unsubscribe(
    state: State<'_, AppState>,
    attempt_id: String,
) -> Result<(), UiError> {
    if let Some(token) = state.handler_log_cancels.remove(&attempt_id) {
        token.cancel();
    }
    Ok(())
}
```

- [ ] **Step 2: AppState extension**

```rust
pub struct AppState {
    // ...existing fields...
    pub handler_log_cancels: dashmap::DashMap<String, tokio_util::sync::CancellationToken>,
}
```

Add `dashmap = "6"` and `tokio-util = { version = "0.7", features = ["sync"] }` to Cargo.toml if missing.

- [ ] **Step 3: Register**

In `lib.rs`'s `generate_handler![...]`: `commands::handler_log_tail, commands::handler_log_subscribe, commands::handler_log_unsubscribe`.

- [ ] **Step 4: ipc_contract tests**

```rust
#[test]
fn plan9_handler_log_commands_registered() {
    let _ = crate::commands::handler_log_tail;
    let _ = crate::commands::handler_log_subscribe;
    let _ = crate::commands::handler_log_unsubscribe;
}

#[test]
fn plan9_handler_log_line_json_shape() {
    let json = serde_json::json!({
        "timestamp": "2026-05-25T10:00:00Z",
        "worker_id": 3,
        "stream": "stderr",
        "line": "hello",
    });
    let parsed: rowforge_core::handler_log::HandlerLogLine =
        serde_json::from_value(json).unwrap();
    assert_eq!(parsed.worker_id, 3);
}
```

- [ ] **Step 5: Verify + commit**

```bash
cargo build
cargo test -p rowforge-studio --test ipc_contract
```

Commit message:
```
studio-shell: handler_log Tauri commands (tail / subscribe / unsubscribe)

Three new commands wrap StudioCore's handler_log_tail and
handler_log_subscribe APIs.

handler_log_subscribe spawns a batching task that pumps the broadcast
receiver into 'handler_log:<attempt_id>' events, max one event per
100ms or per 64-line batch (whichever first). Backpressure drop
counts are carried in the event payload as `dropped`.

handler_log_unsubscribe uses a CancellationToken in AppState to
stop the pump cleanly on UI unmount.

ipc_contract +2.
```

---

## Task 7: TS mirrors + hooks

**Files:**
- Modify: `apps/rowforge-studio/src/ipc/types.ts`
- Modify: `apps/rowforge-studio/src/ipc/client.ts`
- Create: `apps/rowforge-studio/src/ipc/use-handler-log.ts`

- [ ] **Step 1: Types**

```ts
export type HandlerStream = "stdout" | "stderr";

export interface HandlerLogLine {
  timestamp: string;       // ISO 8601
  worker_id: number;
  stream: HandlerStream;
  line: string;
}
```

- [ ] **Step 2: ipc client**

```ts
handler_log_tail: (args: { exec_id: string; attempt_id: string; max_lines?: number }) =>
  invoke<HandlerLogLine[]>("handler_log_tail", args),
handler_log_subscribe: (args: { exec_id: string; attempt_id: string }) =>
  invoke<void>("handler_log_subscribe", args),
handler_log_unsubscribe: (args: { attempt_id: string }) =>
  invoke<void>("handler_log_unsubscribe", args),
```

- [ ] **Step 3: Hooks**

`use-handler-log.ts`:

```ts
import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import { ipc } from "./client";
import type { HandlerLogLine } from "./types";

export function useHandlerLogTail(execId: string, attemptId: string) {
  return useQuery({
    queryKey: ["handler_log_tail", execId, attemptId],
    queryFn: () => ipc.handler_log_tail({ exec_id: execId, attempt_id: attemptId }),
  });
}

interface LiveStream {
  lines: HandlerLogLine[];
  dropped: number;
}

export function useHandlerLogLive(execId: string, attemptId: string, enabled: boolean): LiveStream {
  const [state, setState] = useState<LiveStream>({ lines: [], dropped: 0 });

  useEffect(() => {
    if (!enabled) return;
    let unlisten: UnlistenFn | null = null;
    let cancelled = false;

    ipc.handler_log_subscribe({ exec_id: execId, attempt_id: attemptId })
      .then(() => {
        if (cancelled) return;
        return listen<{ lines: HandlerLogLine[]; dropped: number }>(
          `handler_log:${attemptId}`,
          (event) => {
            setState((prev) => ({
              lines: [...prev.lines, ...event.payload.lines],
              dropped: prev.dropped + event.payload.dropped,
            }));
          },
        );
      })
      .then((un) => { if (un) unlisten = un; });

    return () => {
      cancelled = true;
      if (unlisten) unlisten();
      ipc.handler_log_unsubscribe({ attempt_id: attemptId }).catch(() => {});
    };
  }, [execId, attemptId, enabled]);

  return state;
}
```

- [ ] **Step 4: Tests**

Add to existing ipc test file or create `use-handler-log.test.ts`:

```ts
it("useHandlerLogTail fetches tail", async () => {
  // mock invoke, assert query fires
});

it("useHandlerLogLive subscribes and accumulates lines from event", async () => {
  // mock invoke + listen, fire event, assert state grows
});
```

Target +3 vitest.

- [ ] **Step 5: Commit**

---

## Task 8: AttemptLogsTab component

**Files:**
- Create: `apps/rowforge-studio/src/pages/AttemptLogsTab.tsx`
- Create: `apps/rowforge-studio/src/components/LogsToolbar.tsx`
- Create: `apps/rowforge-studio/src/components/LogsVirtualList.tsx`
- Create: `apps/rowforge-studio/src/pages/__tests__/AttemptLogsTab.test.tsx`

- [ ] **Step 1: Toolbar**

`LogsToolbar.tsx` — worker filter chips (multi-select), stream filter (stdout/stderr/both), search input, auto-scroll toggle, Reveal button, Pause/Resume button. Pure presentational; state lives in the parent tab.

- [ ] **Step 2: Virtualized list**

`LogsVirtualList.tsx` — uses `@tanstack/react-virtual` (verify dep in Cargo... I mean package.json — check first):

```tsx
import { useVirtualizer } from "@tanstack/react-virtual";

export function LogsVirtualList({ lines, autoScroll }: Props) {
  const parentRef = useRef<HTMLDivElement>(null);
  const rowVirtualizer = useVirtualizer({
    count: lines.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 24,
    overscan: 20,
  });
  useEffect(() => {
    if (autoScroll && parentRef.current) {
      parentRef.current.scrollTop = parentRef.current.scrollHeight;
    }
  }, [lines.length, autoScroll]);

  return (
    <div ref={parentRef} className="flex-1 overflow-auto font-mono text-xs">
      <div style={{ height: rowVirtualizer.getTotalSize(), position: "relative" }}>
        {rowVirtualizer.getVirtualItems().map((vItem) => {
          const line = lines[vItem.index];
          return (
            <div
              key={vItem.key}
              style={{
                position: "absolute",
                top: 0,
                left: 0,
                width: "100%",
                transform: `translateY(${vItem.start}px)`,
              }}
              className="px-2 py-0.5 hover:bg-zinc-800/40 cursor-text"
              title={`${line.timestamp} [handler#${line.worker_id} ${line.stream}]`}
            >
              <span className="text-muted-foreground">
                {new Date(line.timestamp).toLocaleTimeString()}
              </span>{" "}
              <WorkerBadge id={line.worker_id} />{" "}
              <StreamChip stream={line.stream} />{" "}
              <span className="whitespace-pre-wrap">{line.line}</span>
            </div>
          );
        })}
      </div>
    </div>
  );
}
```

WorkerBadge: color-coded by worker_id (e.g. hsl(line.worker_id * 60, 50%, 70%)). StreamChip: blue for stdout, yellow for stderr.

- [ ] **Step 3: Tab component**

`AttemptLogsTab.tsx` — wires:
- `useHandlerLogTail(execId, attemptId)` for bootstrap
- `useHandlerLogLive(execId, attemptId, isLive)` for live stream (only if attempt is running)
- Filters state (worker, stream, search) and `autoScroll` boolean
- `useMemo` filter
- `revealLog` calls `tauri_plugin_shell::ShellExt::open(file_path)` via a new ipc command `handler_log_reveal_file` (or pass the path from `handler_log_tail`'s response and reveal client-side via the existing reveal shell)

To get the file path: either add `handler_log_path` to a new Tauri command, OR have `handler_log_tail` return `{ lines: [...], file_path: "..." }`. Simplest: new `handler_log_reveal(exec, attempt)` command that uses ShellExt.

- [ ] **Step 4: Tests**

5+ tests covering states:
- Loading
- Empty (no log)
- Populated (from tail)
- Live (lines arrive via event)
- Filters narrow visible list
- Dropped banner appears when dropped > 0
- Reveal button calls handler_log_reveal

- [ ] **Step 5: Verify + commit**

---

## Task 9: AttemptDetailPage tab integration

**Files:**
- Modify: `apps/rowforge-studio/src/pages/AttemptDetailPage.tsx` (find the tabs section)
- Test: existing AttemptDetailPage tests

- [ ] **Step 1: Add Logs tab**

In the tabs section (alongside Summary / Rollup / Failed rows / Live):

```tsx
{tab === "logs" && (
  <AttemptLogsTab execId={execId} attemptId={attemptId} isLive={attemptIsLive} />
)}
```

Add `Logs` to the tab list selector.

- [ ] **Step 2: Verify tab routes preserved on existing tests**

Existing AttemptDetailPage tests assert tab visibility / clicks. Ensure adding the Logs tab doesn't break them.

- [ ] **Step 3: Commit**

---

## Task 10: Spec docs + HUMAN_SMOKE Plan 09

**Files:**
- Modify: `docs/spec/studio/part-2-model.md` (en + zh-Hant) — Settings field
- Modify: `docs/spec/studio/part-3-runtime.md` (en + zh-Hant) — pool_streaming tee
- Modify: `docs/spec/studio/part-4-data.md` (en + zh-Hant) — file layout
- Modify: `docs/spec/studio/part-5-api.md` (en + zh-Hant) — commands + events
- Modify: `docs/spec/studio/part-7-ui.md` (en + zh-Hant) — Logs tab IA + flows
- Modify: `apps/rowforge-studio/HUMAN_SMOKE.md` — Plan 09 section

Per design §12. Document at minimum:
- Settings.handler_log_capture_raw_stdout default false
- File path `<attempt>/handler_log.log` and on-disk format
- Broadcast channel cap 4096; dropped sentinel
- Tail snapshot K=5000
- 3 new Tauri commands + 1 event channel
- Logs tab in attempt detail IA
- Bootstrap+live flow

HUMAN_SMOKE Plan 09 walkthrough (≥20 numbered steps):
1. Run a handler that prints to stderr; verify CLI still shows `[handler#0 stderr] ...`
2. After completion: `cat <attempt>/handler_log.log` shows everything
3. Studio Logs tab opens; bootstrap shows recent lines
4. Filter by worker → list narrows
5. Filter by stream → list narrows
6. Search "ERROR" → only matching lines visible
7. Reveal file → OS file manager opens at path
8. Run a long handler in Studio; new lines appear within 200ms
9. Scroll up; auto-scroll toggle goes off
10. Scroll back to bottom; auto-scroll re-enables
11. Pause button: live lines don't re-render until resumed
12. Fast handler emitting 10k lines/sec → dropped banner appears
13. Toggle Settings.capture_raw_stdout on → next run logs valid outcome JSON too
14. Old attempt (pre-Plan 9) → "No log file" empty state
15. Attempt with file but empty (handler emitted nothing) → "No output yet" state
... etc

---

## Final verification + PR

```bash
cargo build && cargo test
cd apps/rowforge-studio && pnpm tsc -b && pnpm test && pnpm build
```

Expected:
- Cargo: 344 → ~356 (+12)
- Vitest: 121 → ~131 (+10)

PR:
```bash
git push -u origin studio-plan-09-handler-logs
gh pr create --title "studio Plan 9: handler log capture + live tail" --body "..."
```

---

## Order dependency

T1 (types) → T2 (pool_streaming, needs T1) → T3 (audit, needs T2) → T4 (studio-core, needs T2) → T5 (Settings, needs T4 to wire) → T6 (Tauri, needs T4) → T7 (TS hooks, needs T6) → T8 (component, needs T7) → T9 (page integration, needs T8) → T10 (docs).

T3 is a buffer task — may be empty if T2's API extension doesn't break callers.
