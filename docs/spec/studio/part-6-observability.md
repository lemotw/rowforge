# Part 6 — Observability

Describes the event stream from a running attempt to the UI: taxonomy,
throughput safety, live vs replay, live metrics, failure diagnostics,
and multi-run roll-up.

## 6.1 `ProgressEvent` taxonomy

```rust
#[non_exhaustive]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProgressEvent {
    // Lifecycle
    PhaseChanged { phase: Phase, at_ms: u64 },
    WorkerSpawned { worker_id: u32 },
    HandlerReady { worker_id: u32, handler_version: String, startup_ms: u32 },
    WorkerCrashed {
        worker_id: u32,
        last_seq: Option<u64>,
        signal: Option<i32>,
        stderr_tail: String,            // ≤ 64 KiB, head+tail kept on overflow
    },
    StallWarning { silent_secs: u32 },

    // Hot-path progress
    Tick {
        seq: u64,                       // monotonic per run; UI detects drops
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
    OutcomeSample {                     // sampled; not exhaustive
        row_index: u64,
        kind: RowOutcomeKind,
        code: Option<String>,
        message: Option<String>,
        dur_ms: u32,
    },
    BatchSummary {                      // batch-mode runs only
        first_seq: u64,
        n: u32,
        success: u32,
        failed: u32,
        dur_ms: u32,
    },

    // Distinct from row failures
    PipelineWarning { code: String, message: String },
    HandlerStderr { worker_id: u32, line: String },     // sampled

    // Terminal
    Done(RunReport),
    Aborted { reason: AbortReason, at_phase: Phase, partial_report: RunReport },
}

enum Phase {
    Initializing, Snapshotting, Starting, Running, Cancelling, Persisting
}
```

UI rendering of non-essential variants (`PhaseChanged`, `WorkerSpawned`,
`HandlerReady`, `WorkerCrashed`, `StallWarning`, `PipelineWarning`,
`HandlerStderr`, `BatchSummary`) is optional. The events exist so we do
not have to break the enum to add UI later.

`OutcomeSample` is documented as lossy. Anyone who needs every outcome
reads `outcomes.jsonl`.

## 6.2 Throughput safety (coalescing)

At 10K rows/sec a per-row event is one event every 100 µs. A naive
`broadcast::Sender` overflows in 100 ms; React reconciliation collapses
much earlier. So coalescing happens in `studio-core`, before the
broadcast send, in a small `ProgressAggregator`.

Emission budgets:

| Event | Budget | Strategy |
|---|---|---|
| `Tick` | 4 Hz (every 250 ms) | Driven by both wall-clock timer and count-delta threshold; receiver cannot tell which |
| `OutcomeSample` | 20 / sec | Token-bucket; 90% of budget reserved for errors/crashes, 10% for successes |
| `HandlerStderr` | 20 / sec / worker, line ≤ 2 KiB | Burst overflow collapses into `"... n more lines dropped"` |

Channel sizing:
- `broadcast::channel(256)`. Receivers that lag emit one
  `PipelineWarning { code: "EVENT_LAG", message: "n events dropped" }`
  and continue.

Cancel + backlog:
- Cancel is a separate `invoke("run_cancel", ...)` call, never an
  event-stream back-channel. It hits the `CancellationToken` directly
  and is unaffected by event-stream depth.
- On `Aborted` / `Done`, the forwarder drains one final `Tick` before
  unsubscribing, so the user always sees the final counters even if
  earlier Ticks dropped.

What the UI sees at sustained 10K rows/sec:
- A smooth progress bar driven by 4 Hz Ticks.
- A 1s / 10s rate readout.
- A 200-entry ring buffer of `OutcomeSample`s, dominated by errors.
- They never see "every row." That is `outcomes.jsonl`.

Latency floor: 250 ms of visual lag. At 10K rows/sec that is 2500 rows
of visible lag. Acceptable for a desktop GUI.

## 6.3 Where in the pipeline coalescing happens

`rowforge-core` is the source of truth; it must not lose outcomes. It
emits granular events to a `ProgressSink` trait. The CLI's sink writes
stderr lines (today's behavior). Studio's sink is a
`ProgressAggregator` that:

1. Receives every outcome (this is the durable count).
2. Updates internal counters / rate buffers / per-error histogram.
3. On a 250 ms tick, emits a `Tick`.
4. On error / crash outcomes, runs token-bucket sampling and may emit
   an `OutcomeSample`.

This keeps coalescing out of `rowforge-core` (CLI stays unchanged) and
out of the Tauri layer (which would have less context).

## 6.4 Live

The Tauri layer subscribes to a running attempt directly via
`core.subscribe(handle)`, which returns the aggregator's broadcast
receiver. `snapshot()` returns the aggregator's current counters;
`events()` is the broadcast receiver. Used while a run is alive in
this Studio process.

### 6.4.1 React subscription bootstrap (snapshot fallback)

Tauri's `app.emit(channel, payload)` is fire-and-forget — payloads
emitted before any webview listener attaches are discarded, not
queued. The React `useRun` hook therefore can't rely on
`listen("run:<handle>")` alone: events that fired between
`run_start` returning and `listen()` taking effect (50–300 ms in
practice) would be lost.

**Bootstrap protocol:**

1. `useRun` attaches the `listen()` first so subsequent events are
   captured into the reducer.
2. Once `listen()` is attached, it calls `run_snapshot(handle)` and
   dispatches a synthetic `_bootstrap` action carrying the returned
   `ProgressSnapshot`. The reducer applies counter / phase / status
   from the snapshot in one shot.
3. Real events arriving between steps 1 and 2 update the reducer
   normally. The `_bootstrap` dispatch may briefly overwrite with
   slightly older snapshot values; the next real `Tick` (≤ 250 ms
   later) corrects this.
4. If `run_snapshot` rejects with `UnknownHandle` (the run finished
   before the listener attached — common for sub-200 ms runs), the
   hook dispatches a `_terminal_before_listen` action setting
   `phantomBootstrap = true`. The page reacts by hiding the Live
   tab, refetching `attempt_show`, and switching to Summary.

This protocol is invisible at the Tauri command surface; it's a
property of the React `useRun` hook and the supporting commands
`run_snapshot` (Part 5 §5.5) and `attempt_active_handle`. Tests in
`apps/rowforge-studio/src/__tests__/run-state.test.ts` lock both
the `_bootstrap` and `_terminal_before_listen` actions.

## 6.5 Failure diagnostics

`Aborted` carries structured context:

```rust
enum AbortReason {
    UserCancelled,
    HandlerStartupTimeout { failed_workers: u32, last_stderr: String },
    AllWorkersCrashed { crashes: Vec<WorkerCrashRecord> },
    Stalled { silent_secs: u32, last_seq: Option<u64> },
    MissingRequiredInput { columns: Vec<String> },
    SnapshotHashMismatch { path: PathBuf, expected: String, actual: String },
    OrphanedOnRestart,
    Crashed { panic_message: String },
    Internal { message: String },
}

struct WorkerCrashRecord {
    worker_id: u32,
    last_seq: Option<u64>,
    exit_code: Option<i32>,
    signal: Option<i32>,
    stderr_tail: String,                // ≤ 64 KiB
}
```

Handler stderr has dual sinks:
- **Live tail** to UI as `HandlerStderr` events (sampled per §6.2).
- **Persistent file** `attempts/<id>/handler.stderr.log`,
  append-only, no rate limit. UI has an "Open log" affordance whose
  path is in `AttemptDetail::paths`.

Crash mid-row vs handler-reported failure: the wire protocol already
distinguishes (`type=error` is reported, `type=crash` is mid-row death;
CLI Part 4 §7.4). Studio preserves this:
- `error` outcome → `OutcomeSample { kind: Error, ... }`.
- `crash` outcome → `OutcomeSample { kind: Crash, code: "WORKER_CRASH" }`
  **and** a `WorkerCrashed` lifecycle event with the stderr tail.

## 6.6 Multi-run roll-up

Isolation invariants:
- Each `RunHandle` gets its own broadcast channel.
- Each handle gets its own Tauri event name: `"run:<handle>"`.
- Aggregator state never leaks across runs.

Cross-run aggregate (`runs:active`):

```rust
struct RunRollupTick {
    active_runs: u32,
    total_processed: u64,
    total_failed: u64,
    total_rate: f32,
    slowest_run: Option<RunHandle>,
}
```
Emitted at 1 Hz. Built by `SessionRegistry` polling each
session's aggregator snapshot — strictly a counters view, no per-row
data crosses runs. Used by the global header / dock badge and the
"running" dropdown.

What we explicitly do **not** offer:
- Cross-run merged timeline / comparison views (out of scope; BI
  territory).
- Persistent active-runs roll-up across Studio restarts (the
  `executions.db` registry already records terminal state).

## 6.7 Live metrics

Counters always emitted (cheap):
- `processed`, `success`, `failed`, `crashed`.
- `in_flight`, `queue_depth`.
- `rate_1s`, `rate_10s` from sampled ring buffers at the 250 ms tick.
- `eta_ms` = `(total − processed) / rate_10s`; show "—" until the
  10 s buffer has filled.
- Worker utilization (per-worker `busy_ms / total_ms`; aggregated
  in `Tick` if cheap to do so).

Opt-in, not in v1:
- Per-row latency histograms (HDR-histogram). Cost is sub-µs per
  insert but per-tick percentile snapshots cost more memory than
  counters. Adding this requires a `RunOpts::observe_latencies` flag
  and an additional event variant; v1 does not include it.

## 6.8 Open questions

1. Does `progress.jsonl` for non-row events earn its place soon enough
   to schedule, or wait for demand to manifest?
2. Stderr ring policy: head+tail or contiguous? Affects "feels truncated"
   vs "feels lossy."
3. Should `runs:active` survive a Studio restart by re-scanning SQLite
   for any `state = running` rows, or stay strictly in-memory? The
   answer interacts with §3.7 crash recovery.
