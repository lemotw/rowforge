# Studio Plan 04 — Live Runs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Start runs from Studio, watch them live (4 Hz Tick + sampled OutcomeSample event tail), cancel them in two phases, see concurrent runs in a header pill. Replay terminal attempts at variable speed.

**Architecture:** In-process `rowforge_core::run::execute` spawned on tokio. A `ProgressSink` impl in `studio-core` (the `ProgressAggregator`) coalesces granular per-row events into 4 Hz `Tick` + 20/s `OutcomeSample`, sends through a `tokio::broadcast` channel per `RunHandle`. Tauri forwards as `run:<handle>` events. React listens via `@tauri-apps/api/event` and updates a small in-memory reducer.

**Tech Stack:** Same as Plan 3. Adds `tokio::broadcast` + `tokio_util::sync::CancellationToken` (workspace dep already), `tauri::Manager` for event emission, `@tanstack/react-virtual` for the event tail (already added Plan 2).

**Spec references:** Part 3 §3.1 in-process model, §3.3 state machine, §3.4 concurrency, §3.5 cancel two-phase, §3.6 shutdown cleanup, §3.7 orphan recovery; Part 5 §5.2 run APIs, §5.3 `RunAborted` / `RunBusy` / `UnknownHandle` variants, §5.5 Tauri commands + events; Part 6 entire (event taxonomy, throughput safety, live vs replay, failure diagnostics, multi-run roll-up); Part 7 §7.6.1-6 progress region / event tail / cancel / banners.

---

## Decisions resolved during brainstorm

| Decision | Choice | Why |
|---|---|---|
| User starts run via | Minimal Run button on ExecDetail | Plan 4 needs a path to demonstrate live; full launcher is Plan 5 |
| Live tab replay | Plan 4 includes replay (LiveAttemptStream + ReplayAttemptStream traits) | One pass to do both — they share the AttemptStream abstraction |
| Concurrency limit config | Hard-coded spec defaults (1/exec, 3/workspace) | Settings UI is Plan 5; manual settings.json edit can override `max_concurrent_runs` (the field exists from Plan 2) |
| Process model | In-process (spec §3.1 default) | Sidecar runner is v2 |
| Coalescing budgets | Full spec §6.2 (4 Hz Tick, 20/s OutcomeSample, 256 broadcast slots) | UI would be unusable without it at 10K rows/sec |

---

## File structure

### New — `rowforge-studio-core`
- `crates/rowforge-studio-core/src/events.rs` — `ProgressEvent`, `Phase`, `AbortReason`, `WorkerCrashRecord`, `RunReport`, `BatchSummary`
- `crates/rowforge-studio-core/src/run_handle.rs` — `RunHandle` opaque id, `RunStatus` enum, `CancelMode`
- `crates/rowforge-studio-core/src/session.rs` — `SessionRegistry`, `Session` entry, registration / lookup / cleanup
- `crates/rowforge-studio-core/src/aggregator.rs` — `ProgressAggregator` (coalesces incoming events into Tick/OutcomeSample), `ProgressSnapshot` (current counters)
- `crates/rowforge-studio-core/src/run.rs` — `StudioCore::start_run` / `cancel` / `subscribe` / `status` / `active_runs` / `active_runs_stream`
- `crates/rowforge-studio-core/src/attempt_stream.rs` — `trait AttemptStream`, `LiveAttemptStream`, `ReplayAttemptStream`

### Modified — `rowforge-studio-core`
- `src/exec_view.rs`, `src/failed.rs` — add `#[non_exhaustive]` to `AttemptCountsStub` enum siblings if any (carry-forward), plus `InputFormat` / `RowOutcomeKind` enum annotations
- `src/error.rs` — add `RunAborted { reason: AbortReason }`, `RunBusy { execution_id: String, scope: BusyScope }`, `UnknownHandle(String)` variants
- `src/lib.rs` — register new modules, `StudioCore` gains `sessions: SessionRegistry`

### Modified — `rowforge-core`
- Possibly: `src/run.rs` — add `ProgressSink` trait if not already public, plus a `run_with_sink(...)` entry that studio can hook. Verify before assuming.

### New — Tauri commands
- `apps/rowforge-studio/src-tauri/src/commands.rs` — `run_start`, `run_cancel`, `run_status`, `run_active`, plus `attempt_replay_start` for replay
- `apps/rowforge-studio/src-tauri/src/events.rs` — bridge from broadcast channel to `app.emit_to("main", "run:<handle>", ...)`

### New — React UI
- `apps/rowforge-studio/src/ipc/events.ts` — `listenRun(handle)` / `listenActiveRuns()` helpers
- `apps/rowforge-studio/src/ipc/run-state.ts` — small reducer mapping `ProgressEvent` to UI state
- `apps/rowforge-studio/src/components/ProgressRegion.tsx` — 3-column live progress
- `apps/rowforge-studio/src/components/EventTail.tsx` — virtualized event list
- `apps/rowforge-studio/src/components/PhaseChipBar.tsx`
- `apps/rowforge-studio/src/components/LifecycleBanner.tsx` — WorkerCrashed / StallWarning / PipelineWarning / EVENT_LAG
- `apps/rowforge-studio/src/components/CancelDialog.tsx` — soft → 10 s → force-kill flow
- `apps/rowforge-studio/src/components/ActiveRunsPill.tsx` — header pill + popover
- `apps/rowforge-studio/src/components/RunButton.tsx` — minimal "Run" on ExecDetail
- `apps/rowforge-studio/src/components/ReplayToggle.tsx` — terminal attempt → replay mode
- `apps/rowforge-studio/src/components/ui/popover.tsx` — shadcn primitive (needed for ActiveRunsPill)

### Modified — React UI
- `src/pages/AttemptDetail.tsx` — Live tab enabled when running; replay toggle when terminal
- `src/pages/ExecDetail.tsx` — RunButton on header
- `src/layout/Header.tsx` — ActiveRunsPill
- `src/ipc/queries.ts` — `useRunStart`, `useRunCancel`, `useRunStatus`, `useActiveRuns` mutation/query hooks

### Out of scope for Plan 04
- Full Run launcher (handler picker UI, retry-failed, dry-run, sample, config overrides) — Plan 5
- Settings UI page — Plan 5
- `start_exec` / `export` Tauri commands — Plan 5
- Handler authoring — Plans 6-8
- Sidecar runner process — v2

---

## Task 1: Plan 3 carry-forward + extend `UiError`

**Files:**
- Modify: `crates/rowforge-studio-core/src/exec_detail.rs` (InputFormat)
- Modify: `crates/rowforge-studio-core/src/failed.rs` (RowOutcomeKind)
- Modify: `crates/rowforge-studio-core/src/error.rs`
- Modify: `apps/rowforge-studio/src/ipc/types.ts`

- [ ] **Step 1.1: Add `#[non_exhaustive]` to two enums**

`InputFormat` in exec_detail.rs and `RowOutcomeKind` in failed.rs. Both currently `#[derive(...)] pub enum {...}`. Just add the attribute.

- [ ] **Step 1.2: Extend `UiError` with run-lifecycle variants**

In `crates/rowforge-studio-core/src/error.rs`:

```rust
#[derive(Debug, Error, Serialize)]
#[serde(tag = "kind", content = "message", rename_all = "snake_case")]
#[non_exhaustive]
pub enum UiError {
    // ... existing 5 variants
    #[error("run aborted: {0}")]
    RunAborted(String),

    #[error("run cannot start: {0}")]
    RunBusy(String),

    #[error("handle expired or unknown: {0}")]
    UnknownHandle(String),
}
```

Note: spec §5.3 shows `RunAborted { reason: AbortReason }` and `RunBusy { execution_id, scope }` as struct variants. Plan 4 keeps the simpler `String` payload (adjacent tagging works with `String`), with `to_string()` formatting of the structured info inside. Plan 5 may upgrade to struct variants once we wire structured error inspection on the React side.

- [ ] **Step 1.3: Update TS mirror**

In `apps/rowforge-studio/src/ipc/types.ts`, extend `UiErrorKind`:

```ts
export type UiErrorKind =
  | "workspace_locked"
  | "not_found"
  | "invalid_arg"
  | "io"
  | "internal"
  | "run_aborted"
  | "run_busy"
  | "unknown_handle";
```

- [ ] **Step 1.4: Run tests + commit**

```bash
cargo test -p rowforge-studio-core
cargo test -p rowforge-studio --test ipc_contract
cd apps/rowforge-studio && pnpm tsc -b && pnpm test
```

Expected: all green (no behavior change yet, just type additions).

```bash
git add crates/rowforge-studio-core apps/rowforge-studio/src/ipc/types.ts
git commit -m "studio-core: Plan 3 carry-forward + UiError run variants

- #[non_exhaustive] on InputFormat and RowOutcomeKind
- Three new UiError variants (RunAborted/RunBusy/UnknownHandle)
  for Plan 4 run lifecycle
- TS mirror kind union extended"
```

---

## Task 2: `ProgressEvent` enum + supporting types

**Files:**
- Create: `crates/rowforge-studio-core/src/events.rs`
- Modify: `crates/rowforge-studio-core/src/lib.rs`

- [ ] **Step 2.1: Define the event taxonomy per spec §6.1**

Create `crates/rowforge-studio-core/src/events.rs`:

```rust
//! Live progress events. Spec part-6 §6.1 (full taxonomy).
//!
//! These cross the IPC boundary as `run:<handle>` Tauri events.
//! adjacently tagged JSON shape: `{ "type": "...", ... }`.

use crate::failed::RowOutcomeKind;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProgressEvent {
    // Lifecycle
    PhaseChanged { phase: Phase, at_ms: u64 },
    WorkerSpawned { worker_id: u32 },
    HandlerReady { worker_id: u32, handler_version: String, startup_ms: u32 },
    WorkerCrashed(WorkerCrashRecord),
    StallWarning { silent_secs: u32 },

    // Hot path
    Tick {
        seq: u64,
        at_ms: u64,
        processed: u64,
        total: Option<u64>,
        success: u64,
        failed: u64,
        crashed: u64,
        in_flight: u32,
        queue_depth: u32,
        rate_1s: f32,
        rate_10s: f32,
        eta_ms: Option<u64>,
    },
    OutcomeSample {
        row_index: u64,
        kind: RowOutcomeKind,
        code: Option<String>,
        message: Option<String>,
        dur_ms: u32,
    },
    BatchSummary {
        first_seq: u64,
        n: u32,
        success: u32,
        failed: u32,
        dur_ms: u32,
    },

    // Distinct from row failures
    PipelineWarning { code: String, message: String },
    HandlerStderr { worker_id: u32, line: String },

    // Terminal
    Done(RunReport),
    Aborted {
        reason: AbortReason,
        at_phase: Phase,
        partial_report: RunReport,
    },
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Phase {
    Initializing,
    Snapshotting,
    Starting,
    Running,
    Cancelling,
    Persisting,
}

#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct WorkerCrashRecord {
    pub worker_id: u32,
    pub last_seq: Option<u64>,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub stderr_tail: String, // ≤ 64 KiB
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum AbortReason {
    UserCancelled,
    HandlerStartupTimeout { failed_workers: u32, last_stderr: String },
    AllWorkersCrashed { crashes: Vec<WorkerCrashRecord> },
    Stalled { silent_secs: u32, last_seq: Option<u64> },
    MissingRequiredInput { columns: Vec<String> },
    SnapshotHashMismatch { path: std::path::PathBuf, expected: String, actual: String },
    OrphanedOnRestart,
    Crashed { panic_message: String },
    Internal { message: String },
}

#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct RunReport {
    pub processed: u64,
    pub success: u64,
    pub failed: u64,
    pub crashed: u64,
    pub dur_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tick_serializes_with_type_field() {
        let ev = ProgressEvent::Tick {
            seq: 1,
            at_ms: 250,
            processed: 100,
            total: Some(1000),
            success: 95,
            failed: 5,
            crashed: 0,
            in_flight: 4,
            queue_depth: 12,
            rate_1s: 400.0,
            rate_10s: 380.0,
            eta_ms: Some(2_250),
        };
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(v.get("type").and_then(|t| t.as_str()), Some("tick"));
        assert_eq!(v.get("processed").and_then(|p| p.as_u64()), Some(100));
    }

    #[test]
    fn abort_reason_serializes_as_kind_tagged() {
        let r = AbortReason::UserCancelled;
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v.get("kind").and_then(|k| k.as_str()), Some("user_cancelled"));
    }
}
```

- [ ] **Step 2.2: Register + run tests + commit**

```bash
# add `pub mod events;` to lib.rs
# `pub use events::{ProgressEvent, Phase, AbortReason, WorkerCrashRecord, RunReport};`

cargo test -p rowforge-studio-core --lib events::tests
git add crates/rowforge-studio-core
git commit -m "studio-core: ProgressEvent taxonomy (spec §6.1)

16 variants across lifecycle / hot-path / pipeline / terminal.
Phase enum, AbortReason discriminated union, WorkerCrashRecord
struct. All #[non_exhaustive]. JSON shapes locked by 2 tests."
```

---

## Task 3: `RunHandle` / `RunStatus` / `CancelMode`

**Files:**
- Create: `crates/rowforge-studio-core/src/run_handle.rs`
- Modify: `lib.rs`

- [ ] **Step 3.1: Types**

```rust
//! RunHandle: opaque session ID returned by start_run; passed back to
//! cancel/subscribe/status. Serializable so React side can store it.
//!
//! Spec part-2 §2.2.8, part-3 §3.3.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RunHandle(String);

impl RunHandle {
    pub fn new() -> Self {
        Self(format!("run-{}", ulid::Ulid::new()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RunHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RunStatus {
    Pending,
    Starting,
    Running,
    Cancelling,
    Done,
    Aborted,
    Crashed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CancelMode {
    Soft,
    Hard,
}
```

- [ ] **Step 3.2: Register + tests + commit**

```bash
# pub mod run_handle; + re-export
cargo test -p rowforge-studio-core
git add crates/rowforge-studio-core
git commit -m "studio-core: RunHandle / RunStatus / CancelMode (spec §3.3)"
```

`ulid` is already in workspace deps.

---

## Task 4: `ProgressAggregator` — coalesce events for the broadcast channel

**Files:**
- Create: `crates/rowforge-studio-core/src/aggregator.rs`
- Modify: lib.rs

- [ ] **Step 4.1: Aggregator with 4 Hz Tick + 20/s OutcomeSample**

```rust
//! Coalesces per-row outcome events into 4 Hz Tick + 20/s OutcomeSample
//! before broadcasting. Spec part-6 §6.2.
//!
//! Two timer loops: (1) every 250 ms, emit Tick from current snapshot;
//! (2) on each outcome, token-bucket sample for OutcomeSample.

use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tokio::time;

use crate::events::{Phase, ProgressEvent, RunReport};
use crate::failed::RowOutcomeKind;

const TICK_INTERVAL: Duration = Duration::from_millis(250);
const OUTCOME_TOKENS_PER_SEC: u32 = 20;
const ERROR_BUDGET_RATIO: f32 = 0.9;
const BROADCAST_CAPACITY: usize = 256;

#[derive(Debug, Clone, Default)]
pub struct ProgressSnapshot {
    pub seq: u64,                  // tick sequence
    pub processed: u64,
    pub total: Option<u64>,
    pub success: u64,
    pub failed: u64,
    pub crashed: u64,
    pub in_flight: u32,
    pub queue_depth: u32,
    pub phase: Option<Phase>,
}

pub struct ProgressAggregator {
    inner: Mutex<Inner>,
    tx: broadcast::Sender<ProgressEvent>,
    started: Instant,
}

struct Inner {
    snapshot: ProgressSnapshot,
    rate_1s_buf: ringbuf::Ringbuf<f32>,
    rate_10s_buf: ringbuf::Ringbuf<f32>,
    error_tokens: f32,
    success_tokens: f32,
    last_token_at: Instant,
    tick_seq: u64,
}

// (simplified — real impl uses an actual rate-limit lib or
// hand-rolled token bucket)

impl ProgressAggregator {
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            inner: Mutex::new(Inner {
                snapshot: ProgressSnapshot::default(),
                rate_1s_buf: ringbuf::Ringbuf::new(4),  // 4 samples = 1 s at 250 ms tick
                rate_10s_buf: ringbuf::Ringbuf::new(40),
                error_tokens: (OUTCOME_TOKENS_PER_SEC as f32) * ERROR_BUDGET_RATIO,
                success_tokens: (OUTCOME_TOKENS_PER_SEC as f32) * (1.0 - ERROR_BUDGET_RATIO),
                last_token_at: Instant::now(),
                tick_seq: 0,
            }),
            tx,
            started: Instant::now(),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ProgressEvent> {
        self.tx.subscribe()
    }

    pub fn snapshot(&self) -> ProgressSnapshot {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).snapshot.clone()
    }

    /// Called per row outcome by rowforge-core's ProgressSink.
    pub fn on_outcome(&self, row_index: u64, kind: RowOutcomeKind, code: Option<String>, message: Option<String>, dur_ms: u32) {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        // Update counters
        inner.snapshot.processed += 1;
        match kind {
            RowOutcomeKind::Error => inner.snapshot.failed += 1,
            RowOutcomeKind::Crash => inner.snapshot.crashed += 1,
            RowOutcomeKind::TooLarge => inner.snapshot.failed += 1,
            // success is handled by on_outcome_success below
        }
        // Token bucket
        // ... (refill since last_token_at; if budget left, emit OutcomeSample)
        let _ = (row_index, code, message, dur_ms); // wiring shown in full impl
    }

    pub fn on_outcome_success(&self, _dur_ms: u32) {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.snapshot.processed += 1;
        inner.snapshot.success += 1;
        // Token bucket may emit a sample for success too (10% budget)
    }

    /// Drive the 4 Hz Tick timer; should be spawned as a tokio task.
    pub async fn tick_loop(self: std::sync::Arc<Self>, mut stop: tokio::sync::watch::Receiver<bool>) {
        let mut interval = time::interval(TICK_INTERVAL);
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let event = self.compose_tick();
                    let _ = self.tx.send(event);
                }
                _ = stop.changed() => break,
            }
        }
    }

    fn compose_tick(&self) -> ProgressEvent {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.tick_seq += 1;
        let seq = inner.tick_seq;
        let snap = inner.snapshot.clone();
        // ... compute rate_1s / rate_10s / eta_ms from buffers
        ProgressEvent::Tick {
            seq,
            at_ms: self.started.elapsed().as_millis() as u64,
            processed: snap.processed,
            total: snap.total,
            success: snap.success,
            failed: snap.failed,
            crashed: snap.crashed,
            in_flight: snap.in_flight,
            queue_depth: snap.queue_depth,
            rate_1s: 0.0, // wired up
            rate_10s: 0.0,
            eta_ms: None,
        }
    }

    pub fn emit(&self, event: ProgressEvent) {
        let _ = self.tx.send(event);
    }
}

// Helper ringbuf — minimal in-place implementation or pull a tiny crate
mod ringbuf {
    pub struct Ringbuf<T> { buf: Vec<T>, idx: usize, cap: usize }
    impl<T: Clone + Default> Ringbuf<T> {
        pub fn new(cap: usize) -> Self { Self { buf: vec![T::default(); cap], idx: 0, cap } }
        pub fn push(&mut self, x: T) { self.buf[self.idx] = x; self.idx = (self.idx + 1) % self.cap; }
        pub fn iter(&self) -> impl Iterator<Item = &T> { self.buf.iter() }
    }
}
```

- [ ] **Step 4.2: Tests for tick spacing + token bucket**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn tick_loop_emits_at_4hz() {
        let agg = Arc::new(ProgressAggregator::new());
        let mut rx = agg.subscribe();
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        let h = tokio::spawn(agg.clone().tick_loop(stop_rx));
        tokio::time::sleep(Duration::from_millis(600)).await;
        let _ = stop_tx.send(true);
        h.await.unwrap();
        // Drain receiver — expect ≥ 2 ticks in 600 ms.
        let mut count = 0;
        while let Ok(_) = rx.try_recv() { count += 1; }
        assert!(count >= 2, "got {count} ticks in 600 ms");
    }

    #[test]
    fn outcome_counter_increments() {
        let agg = ProgressAggregator::new();
        agg.on_outcome_success(10);
        agg.on_outcome_success(11);
        agg.on_outcome(1, RowOutcomeKind::Error, Some("X".into()), None, 5);
        let s = agg.snapshot();
        assert_eq!(s.processed, 3);
        assert_eq!(s.success, 2);
        assert_eq!(s.failed, 1);
    }
}
```

- [ ] **Step 4.3: Register + commit**

```bash
# pub mod aggregator; + re-export ProgressAggregator, ProgressSnapshot

cargo test -p rowforge-studio-core
git add crates/rowforge-studio-core
git commit -m "studio-core: ProgressAggregator (4 Hz Tick + 20/s sample)

Token-bucket sampling per spec §6.2 (90/10 error/success split).
Tick driver runs on tokio interval, broadcasts via 256-slot channel.
Snapshot accessor for subscribe-time initial state."
```

---

## Task 5: `SessionRegistry` — track active runs

**Files:**
- Create: `crates/rowforge-studio-core/src/session.rs`

- [ ] **Step 5.1: Registry with concurrent run limits**

```rust
//! In-memory registry of active runs. One entry per RunHandle.
//! Enforces concurrency limits (spec §3.4): 1 per exec, 3 per workspace.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::aggregator::ProgressAggregator;
use crate::run_handle::{RunHandle, RunStatus};

pub struct Session {
    pub handle: RunHandle,
    pub execution_id: String,
    pub aggregator: Arc<ProgressAggregator>,
    pub cancel_token: CancellationToken,
    pub status: RunStatus,
    pub started_at: std::time::Instant,
}

pub struct SessionRegistry {
    inner: Mutex<HashMap<RunHandle, Arc<Session>>>,
    workspace_limit: u32,
    per_exec_limit: u32,
}

impl Default for SessionRegistry {
    fn default() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            workspace_limit: 3,
            per_exec_limit: 1,
        }
    }
}

impl SessionRegistry {
    pub fn new(workspace_limit: u32, per_exec_limit: u32) -> Self {
        Self { inner: Mutex::new(HashMap::new()), workspace_limit, per_exec_limit }
    }

    pub fn can_start(&self, execution_id: &str) -> Result<(), BusyReason> {
        let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if inner.len() as u32 >= self.workspace_limit {
            return Err(BusyReason::Workspace);
        }
        if inner.values().any(|s| s.execution_id == execution_id) {
            return Err(BusyReason::PerExec);
        }
        Ok(())
    }

    pub fn register(&self, session: Arc<Session>) {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.insert(session.handle.clone(), session);
    }

    pub fn get(&self, h: &RunHandle) -> Option<Arc<Session>> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).get(h).cloned()
    }

    pub fn remove(&self, h: &RunHandle) -> Option<Arc<Session>> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).remove(h)
    }

    pub fn handles(&self) -> Vec<RunHandle> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).keys().cloned().collect()
    }

    pub fn snapshots(&self) -> Vec<(RunHandle, crate::aggregator::ProgressSnapshot)> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
            .values()
            .map(|s| (s.handle.clone(), s.aggregator.snapshot()))
            .collect()
    }
}

pub enum BusyReason { PerExec, Workspace }

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_session(id: &str, exec: &str) -> Arc<Session> {
        Arc::new(Session {
            handle: RunHandle::new(),
            execution_id: exec.into(),
            aggregator: Arc::new(ProgressAggregator::new()),
            cancel_token: CancellationToken::new(),
            status: RunStatus::Running,
            started_at: std::time::Instant::now(),
        })
    }

    #[test]
    fn per_exec_limit_enforced() {
        let r = SessionRegistry::default();
        r.register(fake_session("a", "e1"));
        assert!(matches!(r.can_start("e1"), Err(BusyReason::PerExec)));
    }

    #[test]
    fn workspace_limit_enforced() {
        let r = SessionRegistry::default();
        r.register(fake_session("a", "e1"));
        r.register(fake_session("b", "e2"));
        r.register(fake_session("c", "e3"));
        assert!(matches!(r.can_start("e4"), Err(BusyReason::Workspace)));
    }
}
```

- [ ] **Step 5.2: Register + commit**

```bash
cargo test -p rowforge-studio-core
git add crates/rowforge-studio-core
git commit -m "studio-core: SessionRegistry with concurrency limits

1/exec + 3/workspace hard-coded (spec §3.4). can_start() check
returns BusyReason so the call site maps to UiError::RunBusy with
the right scope text."
```

---

## Task 6: `StudioCore::start_run` + `subscribe` + `status` + `active_runs`

**Files:**
- Create: `crates/rowforge-studio-core/src/run.rs`
- Modify: lib.rs (`StudioCore` gains `sessions: SessionRegistry`)

- [ ] **Step 6.1: Inspect `rowforge_core::run::execute`**

Find what signature it takes. Need a way to:
- Pass a ProgressSink that the aggregator implements
- Pass a CancellationToken
- Spawn it on tokio and get a JoinHandle to detect panic

```bash
grep -n "pub fn execute\|pub struct RunRequest\|pub trait ProgressSink\|pub type ProgressCallback" crates/rowforge-core/src/run.rs | head
```

If no `ProgressSink` trait exists, this task includes adding one to `rowforge-core` so studio can hook (carry-forward to spec §5.1 lift). Report what you found.

- [ ] **Step 6.2: Implement `StudioCore::start_run`**

```rust
//! Run lifecycle in studio-core. Spawn the rowforge-core pipeline, plug
//! the ProgressAggregator as the ProgressSink, register the session,
//! return the RunHandle.

use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use crate::aggregator::ProgressAggregator;
use crate::ids::ExecutionId;
use crate::run_handle::{CancelMode, RunHandle, RunStatus};
use crate::session::{BusyReason, Session, SessionRegistry};
use crate::{StudioCore, UiError};

pub struct RunOpts {
    pub handler_dir: std::path::PathBuf,
    pub workers: Option<u32>,
    pub retry_failed: bool,
    // Plan 5 adds full RunOpts; Plan 4 ships the minimum.
}

pub struct RunStream {
    pub handle: RunHandle,
    pub rx: tokio::sync::broadcast::Receiver<crate::events::ProgressEvent>,
    pub snapshot: crate::aggregator::ProgressSnapshot,
}

impl StudioCore {
    pub fn start_run(
        &self,
        execution_id: &ExecutionId,
        opts: RunOpts,
    ) -> Result<RunHandle, UiError> {
        // 1. Concurrency check
        self.sessions
            .can_start(execution_id.as_str())
            .map_err(|reason| match reason {
                BusyReason::PerExec => UiError::RunBusy(format!(
                    "execution {} already has an active run", execution_id
                )),
                BusyReason::Workspace => UiError::RunBusy(
                    "workspace concurrent-run limit reached".into()
                ),
            })?;

        // 2. Allocate handle + aggregator + cancel token
        let handle = RunHandle::new();
        let aggregator = Arc::new(ProgressAggregator::new());
        let cancel_token = CancellationToken::new();

        // 3. Spawn the rowforge-core pipeline on tokio.
        //    Bridge core's progress callbacks into aggregator.on_outcome*().
        //    On completion or panic, emit Done or Aborted then remove
        //    from registry.
        let session = Arc::new(Session {
            handle: handle.clone(),
            execution_id: execution_id.as_str().to_string(),
            aggregator: aggregator.clone(),
            cancel_token: cancel_token.clone(),
            status: RunStatus::Pending,
            started_at: std::time::Instant::now(),
        });
        self.sessions.register(session.clone());

        // 4. Spawn the tick loop (250 ms ticker) for this aggregator.
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        let agg_for_tick = aggregator.clone();
        tokio::spawn(agg_for_tick.tick_loop(stop_rx));
        // stop_tx held in the session somewhere so we can shut down tick
        // loop on Done/Aborted. (Plan 4 wiring detail; revisit.)

        // 5. Spawn the run task.
        let registry = self.sessions_ref();
        let h2 = handle.clone();
        tokio::spawn(async move {
            // ... call rowforge_core::run::execute with the agg as sink
            //     and cancel_token as the abort signal.
            //
            // On Done: aggregator.emit(ProgressEvent::Done(RunReport{...}))
            // On panic: convert to Aborted { reason: Crashed { panic_message } }
            //
            // Finally: registry.remove(&h2)
            let _ = stop_tx; // ensure tick loop stops when this task ends
            let _ = registry;
        });

        Ok(handle)
    }

    pub fn subscribe(&self, h: &RunHandle) -> Result<RunStream, UiError> {
        let session = self.sessions.get(h)
            .ok_or_else(|| UiError::UnknownHandle(h.to_string()))?;
        Ok(RunStream {
            handle: h.clone(),
            rx: session.aggregator.subscribe(),
            snapshot: session.aggregator.snapshot(),
        })
    }

    pub fn cancel(&self, h: &RunHandle, mode: CancelMode) -> Result<(), UiError> {
        let session = self.sessions.get(h)
            .ok_or_else(|| UiError::UnknownHandle(h.to_string()))?;
        match mode {
            CancelMode::Soft => session.cancel_token.cancel(),
            CancelMode::Hard => {
                // Plan 4: trigger force-kill via core's child process kill.
                // Detailed wiring deferred to actual impl — uses Child::kill
                // through whatever handle rowforge-core exposes.
                session.cancel_token.cancel();
                // ... + force kill
            }
        }
        Ok(())
    }

    pub fn status(&self, h: &RunHandle) -> Result<RunStatus, UiError> {
        self.sessions.get(h)
            .map(|s| s.status)
            .ok_or_else(|| UiError::UnknownHandle(h.to_string()))
    }

    pub fn active_runs(&self) -> Vec<RunHandle> {
        self.sessions.handles()
    }

    fn sessions_ref(&self) -> Arc<SessionRegistry> {
        // SessionRegistry will need to live in an Arc on StudioCore.
        unimplemented!("wire after lib.rs change in Step 6.3")
    }
}
```

This is a sketch — the implementer fleshes out the actual pipeline-spawn integration. Step 6.1's grep finding determines exactly how to hook `rowforge_core::run::execute`.

- [ ] **Step 6.3: Update `StudioCore` to hold `Arc<SessionRegistry>`**

In lib.rs:

```rust
pub struct StudioCore {
    workspace: Workspace,
    store: rowforge_core::execution_store::ExecutionStore,
    exec_list_cache: crate::cache::Cache<crate::cache::ExecListKey, Vec<ExecSummary>>,
    pub(crate) sessions: std::sync::Arc<crate::session::SessionRegistry>,
}
```

`open` initialises `sessions: Arc::new(SessionRegistry::default())`.

- [ ] **Step 6.4: Tests + commit**

Integration test: `start_run` + `subscribe` returns a stream that receives at least one Tick within 1 second. Use a real `ExecutionStore` fixture + a tiny handler (test-handler crate already exists in workspace).

```bash
cargo test -p rowforge-studio-core
git add crates/rowforge-studio-core crates/rowforge-core
git commit -m "studio-core: start_run + subscribe + cancel + status + active_runs

Plan 4 core run lifecycle. In-process pipeline spawn with
ProgressAggregator as the ProgressSink. SessionRegistry tracks
active sessions; concurrency limits enforced per spec §3.4."
```

---

## Task 7: Orphan recovery on `StudioCore::open`

**Files:**
- Modify: `crates/rowforge-studio-core/src/lib.rs` (StudioCore::open)

Per spec §3.7: scan workspace SQLite for attempts in state `starting | running` whose mtime is > 5 min. Mark as `aborted` with reason `OrphanedOnRestart`.

- [ ] **Step 7.1: Add scan to `open` + test that simulates orphan**

```rust
fn scan_for_orphans(store: &mut rowforge_core::execution_store::ExecutionStore) -> Result<u32, UiError> {
    // Query for attempts in starting/running state
    // For each: stat outcomes.jsonl mtime
    //   - if > 5 min ago: mark aborted with OrphanedOnRestart reason
    //   - if ≤ 5 min: leave alone (CLI may be running externally)
    // Return count of orphans marked
    todo!("implement")
}
```

Test: create attempt in "running" state, set its outcomes.jsonl mtime to 10 min ago, open StudioCore, verify it's been marked aborted.

- [ ] **Step 7.2: Commit**

```bash
git commit -m "studio-core: orphan recovery on workspace open (spec §3.7)

Scans for attempts stuck in starting/running with outcomes.jsonl
mtime > 5 min; marks them aborted with OrphanedOnRestart reason.
≤ 5 min left alone (may be live CLI run)."
```

---

## Task 8: Shutdown cleanup (`StudioCore::Drop`)

Per spec §3.6: on app quit, soft-cancel all active sessions with 1 s deadline, then hard-kill.

- [ ] **Step 8.1: impl Drop**

```rust
impl Drop for StudioCore {
    fn drop(&mut self) {
        for handle in self.sessions.handles() {
            if let Some(session) = self.sessions.get(&handle) {
                session.cancel_token.cancel();
            }
        }
        // 1 s wait could be done via blocking sleep + force-kill,
        // but in Drop we can't await. Mark as best-effort; Tauri
        // shutdown hooks handle graceful drain.
    }
}
```

- [ ] **Step 8.2: Commit**

```bash
git commit -m "studio-core: Drop cancels active sessions (spec §3.6)"
```

---

## Task 9: `trait AttemptStream` + `LiveAttemptStream`

**Files:**
- Create: `crates/rowforge-studio-core/src/attempt_stream.rs`

- [ ] **Step 9.1: Trait + Live impl**

```rust
//! AttemptStream abstraction: unifies live (from SessionRegistry) and
//! replay (from outcomes.jsonl). Spec part-6 §6.4.

use futures::Stream;
use std::pin::Pin;

use crate::aggregator::ProgressSnapshot;
use crate::events::ProgressEvent;
use crate::run_handle::RunHandle;
use crate::session::Session;
use std::sync::Arc;

pub trait AttemptStream {
    fn snapshot(&self) -> ProgressSnapshot;
    fn events(self: Box<Self>) -> Pin<Box<dyn Stream<Item = ProgressEvent> + Send>>;
}

pub struct LiveAttemptStream {
    session: Arc<Session>,
}

impl LiveAttemptStream {
    pub fn new(session: Arc<Session>) -> Self {
        Self { session }
    }
}

impl AttemptStream for LiveAttemptStream {
    fn snapshot(&self) -> ProgressSnapshot {
        self.session.aggregator.snapshot()
    }
    fn events(self: Box<Self>) -> Pin<Box<dyn Stream<Item = ProgressEvent> + Send>> {
        let rx = self.session.aggregator.subscribe();
        Box::pin(tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(|r| async { r.ok() }))
    }
}
```

Need to add `futures`, `tokio-stream` to deps.

- [ ] **Step 9.2: Commit**

```bash
git commit -m "studio-core: AttemptStream trait + LiveAttemptStream"
```

---

## Task 10: `ReplayAttemptStream`

**Files:**
- Modify: `crates/rowforge-studio-core/src/attempt_stream.rs`

- [ ] **Step 10.1: Replay impl**

Reads `meta.json` for the initial snapshot, then streams `outcomes.jsonl` synthesizing 4 Hz Tick events with progress derived from each outcome's `dur_ms`. Speed configurable (1x / 5x / 10x).

```rust
pub struct ReplayAttemptStream {
    snapshot: ProgressSnapshot,
    outcomes_path: std::path::PathBuf,
    speed: f32,
}

impl ReplayAttemptStream {
    pub fn from_attempt(
        attempt_dir: &std::path::Path,
        speed: f32,
    ) -> Result<Self, std::io::Error> {
        // Read meta.json for snapshot
        // Stash outcomes.jsonl path
        todo!()
    }
}

impl AttemptStream for ReplayAttemptStream {
    fn snapshot(&self) -> ProgressSnapshot {
        self.snapshot.clone()
    }
    fn events(self: Box<Self>) -> Pin<Box<dyn Stream<Item = ProgressEvent> + Send>> {
        // async-stream macro to walk outcomes.jsonl:
        // - Aggregate cumulative counters
        // - Every 250 ms (scaled by `speed`), yield a Tick
        // - Token-bucket sample OutcomeSamples
        // - At EOF, yield Done(RunReport)
        todo!()
    }
}
```

- [ ] **Step 10.2: Test + commit**

```bash
git commit -m "studio-core: ReplayAttemptStream for terminal attempts

Streams outcomes.jsonl synthesizing 4 Hz Tick events.
Configurable speed (1x default; 5x / 10x available)."
```

---

## Task 11: `active_runs_stream` — 1 Hz workspace rollup

**Files:**
- Modify: `crates/rowforge-studio-core/src/run.rs`

Per spec §6.6:

```rust
#[derive(Debug, Clone, Serialize)]
pub struct RunRollupTick {
    pub active_runs: u32,
    pub total_processed: u64,
    pub total_failed: u64,
    pub total_rate: f32,
    pub slowest_run: Option<RunHandle>,
}

pub struct ActiveRunsStream {
    sessions: Arc<SessionRegistry>,
    interval: tokio::time::Interval,
}

impl Stream for ActiveRunsStream { /* poll interval, build tick */ }

impl StudioCore {
    pub fn active_runs_stream(&self) -> ActiveRunsStream { ... }
}
```

- [ ] **Step 11.1: Implement + test (just verify 2 ticks in 2.5 s)**
- [ ] **Step 11.2: Commit**

```bash
git commit -m "studio-core: active_runs_stream (1 Hz workspace rollup)"
```

---

## Task 12: Tauri commands (`run_start` / `run_cancel` / `run_status` / `run_active`)

**Files:**
- Modify: `apps/rowforge-studio/src-tauri/src/commands.rs`
- Modify: `apps/rowforge-studio/src-tauri/src/lib.rs`

- [ ] **Step 12.1: Add 4 commands**

```rust
#[tauri::command]
pub async fn run_start(
    state: State<'_, AppState>,
    execution_id: ExecutionId,
    handler_dir: PathBuf,
) -> Result<RunHandle, UiError> { ... }

#[tauri::command]
pub async fn run_cancel(
    state: State<'_, AppState>,
    handle: RunHandle,
    mode: CancelMode,
) -> Result<(), UiError> { ... }

#[tauri::command]
pub async fn run_status(
    state: State<'_, AppState>,
    handle: RunHandle,
) -> Result<RunStatus, UiError> { ... }

#[tauri::command]
pub async fn run_active(
    state: State<'_, AppState>,
) -> Result<Vec<RunHandle>, UiError> { ... }
```

Also: `attempt_replay_start(execution_id, attempt_id, speed)` returning a fresh `RunHandle` whose events stream from the ReplayAttemptStream. Use the same `run:<handle>` event channel for symmetry.

- [ ] **Step 12.2: Register + commit**

```bash
git commit -m "studio-shell: 5 Tauri commands for run lifecycle + replay"
```

---

## Task 13: Tauri event bridge (`run:<handle>` + `runs:active`)

**Files:**
- Create: `apps/rowforge-studio/src-tauri/src/events.rs`
- Modify: `lib.rs` to spawn the forwarder

- [ ] **Step 13.1: Forwarder task**

After `start_run` returns a `RunHandle`, spawn a task in the Tauri main runtime that:

```rust
async fn forward_run_events(
    app: tauri::AppHandle,
    handle: RunHandle,
    mut rx: broadcast::Receiver<ProgressEvent>,
) {
    while let Ok(event) = rx.recv().await {
        let _ = app.emit_to(EventTarget::any(), &format!("run:{}", handle), &event);
    }
}
```

Similarly for `runs:active` — spawn one task in `lib.rs::run()` startup that polls `active_runs_stream()` and emits.

- [ ] **Step 13.2: Commit**

```bash
git commit -m "studio-shell: bridge broadcast events to Tauri emit"
```

---

## Task 14: TS mirrors for ProgressEvent + AbortReason + RunHandle

Mechanical. Add interfaces matching the Rust shapes.

- [ ] **Step 14.1: Append to types.ts**
- [ ] **Step 14.2: Add client.ts wrappers**
- [ ] **Step 14.3: Commit**

---

## Task 15: React `useRun` hook

**Files:**
- Create: `apps/rowforge-studio/src/ipc/events.ts`
- Create: `apps/rowforge-studio/src/ipc/run-state.ts`
- Create: `apps/rowforge-studio/src/ipc/use-run.ts`

- [ ] **Step 15.1: Reducer + hook**

```ts
// run-state.ts
export interface RunState {
  status: RunStatus;
  snapshot: ProgressSnapshot;
  recentEvents: OutcomeSample[];  // ring buffer of 200
  lifecycleBanners: LifecycleBanner[];
  abortReason?: AbortReason;
}

export function reduceRun(state: RunState, event: ProgressEvent): RunState { ... }
```

```ts
// use-run.ts
export function useRun(handle: RunHandle | null) {
  const [state, dispatch] = useReducer(reduceRun, initial);
  useEffect(() => {
    if (!handle) return;
    const unlisten = listen(`run:${handle}`, (e) => dispatch(e.payload));
    return () => { unlisten.then(f => f()); };
  }, [handle]);
  return state;
}
```

- [ ] **Step 15.2: Commit**

---

## Task 16: ProgressRegion (3-column live progress)

- [ ] Spec part-7 §7.6.1 implementation
- [ ] Commit

---

## Task 17: EventTail (virtualized)

- [ ] @tanstack/react-virtual; per spec §7.6.2 (errors filter default-on)
- [ ] Commit

---

## Task 18: PhaseChipBar + LifecycleBanner

- [ ] Spec part-7 §7.5 (Phase chips) + §7.6.4 (banners)
- [ ] Commit

---

## Task 19: CancelDialog (two-phase + typed token)

- [ ] Spec §7.6.3 implementation
- [ ] Commit

---

## Task 20: AttemptDetail Live tab integration

- [ ] Replace the stale-banner path with Live tab when status is non-terminal
- [ ] Add `useRun(handle)` to subscribe to events
- [ ] Replay toggle for terminal attempts
- [ ] Commit

---

## Task 21: ActiveRunsPill in header

- [ ] Popover primitive (shadcn)
- [ ] Subscribe to `runs:active` event
- [ ] Commit

---

## Task 22: Minimal Run button on ExecDetail

- [ ] Button calls `run_start` with handler_dir from last attempt's binding
- [ ] On success, navigate to `/exec/:id/attempt/:newAid` (need to compute new attempt id from response)
- [ ] Commit

---

## Task 23: Replay button on terminal attempts

- [ ] Reuses AttemptDetail Live tab path with ReplayAttemptStream
- [ ] Speed selector (1x / 5x / 10x)
- [ ] Commit

---

## Task 24: Backend integration tests

Round-out for the new APIs. Real-pipeline test with test-handler crate.

- [ ] Commit

---

## Task 25: Vitest for new components

- [ ] ProgressRegion (rate update), EventTail (filter), CancelDialog (two-phase)
- [ ] Commit

---

## Task 26: Final smoke + HUMAN_SMOKE.md additions

- [ ] All tests + HUMAN_SMOKE Plan 4 section
- [ ] Commit

---

## Plan 04 acceptance

1. `cargo test` workspace-wide pass count + ~20 new tests
2. `pnpm tsc -b` clean
3. `pnpm build` produces dist/
4. `pnpm test` increased count (Plan 3 had 7; Plan 4 adds ~4)
5. New Tauri commands: `run_start`, `run_cancel`, `run_status`, `run_active`, `attempt_replay_start`
6. New events: `run:<handle>`, `runs:active`
7. Concurrency limits enforced (1/exec + 3/workspace) → `RunBusy`
8. Soft cancel: token cancellation → in-flight drain → `Aborted { UserCancelled }`
9. Hard cancel: available after 10 s soft-pending; force-kill via Child::kill
10. Orphan recovery on open: attempts with mtime > 5 min auto-aborted
11. Active runs pill renders + updates at 1 Hz
12. Replay terminal attempt at 1x / 5x / 10x speed
13. **(human)** HUMAN_SMOKE.md Plan 4 walkthrough

## Carry-forward to Plan 5

- Full Run launcher (handler picker, retry-failed, sample, dry-run, config overrides, field mapping, sync_data)
- New Execution wizard
- Export dialog
- Settings UI page (consumes `max_concurrent_runs` to actually override Plan 4 hard-coded limits)
- `start_exec` Tauri command

## Open questions

1. **Hard cancel mechanism**: cancel_token + Child::kill works for the worker subprocess but not for in-process tokio tasks blocking on IO. Worst case (handler in infinite loop): user must force-kill the Tauri app. Acceptable per spec §3.5 last paragraph.
2. **Replay speed quantization**: 1x/5x/10x exact, or continuous? Spec doesn't specify. Plan 4 ships discrete options for simpler UI.
3. **Live tab + workspace switch**: if user switches workspace mid-run, what happens to the active subscription? Plan 4: assume not allowed (WorkspaceMenu's Switch button could disable when active runs exist) OR drain active runs first. Decide during T20.
4. **`start_exec` and `export`** — these are Plan 5, but the Run button (T22) needs to know an existing execution exists. Plan 4 only enables Run for already-created execs.
