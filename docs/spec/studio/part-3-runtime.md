# Part 3 — Runtime

Describes how runs execute under Studio: process model, state machine,
concurrency policy, cancel semantics, crash recovery, handler subprocess
cleanup. CLI-side runtime (worker pool internals, dispatch loop, batch
protocol) is at [`../cli/part-3-runtime.md`](../cli/part-3-runtime.md);
this part references but does not duplicate it.

## 3.1 Process model — in-process

Studio runs the `rowforge-core` pipeline inside the Tauri main process's
tokio runtime. There is no sidecar runner process in v1.

Risk containment:

- **Panic isolation.** Every run-level `tokio::spawn` is awaited via a
  `JoinHandle` whose `JoinError::is_panic` path is mapped to
  `ProgressEvent::Aborted { reason: AbortReason::Crashed { panic_message } }`.
  Panics do not propagate to the process root.
- **CPU isolation.** Tokio is configured multi-threaded. CPU-bound work
  inside `studio-core` (CSV parsing, `outcomes.jsonl` scanning) goes
  through `tokio::task::spawn_blocking` so it never starves the reactor
  serving UI commands.
- **Memory bound.** Each run has a `max_in_flight` configured to
  `workers × 2` by default. Queues are bounded; the pipeline applies
  back-pressure on dispatch.

Out-of-scope risk: a native crash from the handler subprocess
(segfault) cannot take Studio down because handlers run in their own
process. A native crash inside `rowforge-core` itself is treated as a
bug and not a designed failure mode.

A sidecar runner process is documented as a v2 option if any of:
panic-from-native-handler becomes common, UI starvation under heavy CPU
proves unsolvable, or memory isolation becomes required.

## 3.2 Worker pool ownership

Workers (handler subprocesses) are owned by `rowforge-core` per run; the
pool is not shared across runs. Studio enforces
`workers × concurrent_runs ≤ logical_cpus × 2` and surfaces a UI warning
if a user-configured override would violate it.

## 3.3 Run state machine

```
        ┌─────────┐
        │ Pending │  RunHandle allocated, tokio task not yet spawned.
        └────┬────┘
             ▼
        ┌──────────┐
        │ Starting │  Workers spawning, handlers building / handshaking.
        └────┬─────┘
             │  on first row dispatched
             ▼
        ┌─────────┐  cancel
        │ Running │ ────────────────┐
        └────┬────┘                 ▼
             │                ┌────────────┐
             │                │ Cancelling │
             │                └─────┬──────┘
             │                      │
   pipeline drained                  │ token observed, in-flight drained
             ▼                      ▼
        ┌──────┐               ┌──────────┐
        │ Done │               │ Aborted  │
        └──────┘               └──────────┘
                              ▲
                              │  panic in run task
                              │
                         ┌─────────┐
                         │ Crashed │
                         └─────────┘
```

Persistence at transitions:

- **Pending → Starting**: SQLite `attempts` row inserted with
  `state = starting`.
- **Starting → Running**: row updated `state = running`.
- **Running → Done**: outcomes flush completed; `meta.json` written;
  SQLite row updated `state = done` with final stats. `Done` event is
  emitted **after** all three.
- **Any → Aborted**: SQLite row updated `state = aborted` with
  partial stats; outcomes flushed up to last batch boundary.
- **Any → Crashed**: best-effort identical to Aborted, with
  `reason = Crashed`. If the panic prevents writes, recovery in §3.7
  fixes it on next launch.

Live row counters (success, failed, in_flight) are not persisted per
event; they are computed from `outcomes.jsonl` on demand and tracked in
memory in the `ProgressAggregator` (Part 6 §6.2).

## 3.4 Multi-run concurrency

Defaults (user-overridable in Settings):

| Limit | Default | Rationale |
|---|---|---|
| Concurrent runs per execution | 1 | Concurrent attempts on the same exec destabilize `RowResolution` folding |
| Concurrent runs per workspace | 3 | Laptop-friendly default; prevents IO contention with sqlite writes |
| Workers per run | core default | Unchanged from CLI |
| `workers × concurrent_runs` | ≤ cpus × 2 | Enforced as a soft warning, hard cap configurable |

`StudioCore::start_run` returns `UiError::RunBusy { execution_id }` when
the per-execution limit is hit, and `UiError::RunBusy { ... }` with a
workspace-level reason when the workspace limit is hit. The UI is
expected to surface the limit, not silently queue.

## 3.5 Cancel semantics

Two modes:

### Soft cancel (default)
1. `StudioCore::cancel(handle, CancelMode::Soft)` sets the core
   `CancellationToken`.
2. Pipeline stops dispatching new rows.
3. In-flight rows finish (typically sub-second per row; bounded by the
   handler's per-row work).
4. `outcomes.jsonl` is flushed to the last batch boundary.
5. SQLite row transitions to `aborted`.
6. `ProgressEvent::Aborted { reason: UserCancelled, ... }` emitted.

### Hard cancel (force kill)
Available only after soft cancel has been outstanding for an
implementation-defined threshold (recommended: 10 seconds). Calls
`Child::kill()` on handler subprocesses. Partial outcomes may be lost;
UI must warn explicitly before invoking.

UI states during cancel:
- `RunStatus::Cancelling` with a per-second progress indicator "n
  rows in flight"
- After threshold: "Force kill" button appears with a destructive-style
  prompt

Worst case: a handler in an infinite loop inside a single row dispatch.
Soft cancel never completes; hard cancel is the only out. This is
documented; there is no automatic escalation.

## 3.6 Resource cleanup at shutdown

On normal app quit:

1. `StudioCore::Drop` iterates active sessions and issues
   `cancel(Soft)`.
2. Waits up to 1 second per session.
3. Any still-alive workers are hard-killed.

On abnormal app exit (crash, OS kill):

- macOS / Linux: child processes are reaped by the OS when the parent
  dies (default behavior of subprocess inheritance).
- Windows: child processes do NOT die with the parent unless added to a
  Job Object. `rowforge-core` must place worker processes in a Job
  Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. (This is a CLI-side
  fix shared with Studio; it lives in core, not studio-core.)

## 3.7 Crash recovery

On `StudioCore::open`, the workspace is scanned for **orphan attempts**:
SQLite rows where `state ∈ {starting, running}` whose owning Studio
process is no longer alive.

Detection heuristic (no Studio pid file in v1):

| `outcomes.jsonl` mtime | Action |
|---|---|
| Idle > 5 minutes | Auto-mark `aborted` with `reason = OrphanedOnRestart`; write partial stats from on-disk outcomes |
| Idle ≤ 5 minutes | Ambiguous (could be a CLI run from a terminal). UI surfaces "possibly still running externally" and offers manual mark-aborted |

The mtime threshold is implementation-tunable; the spec only requires
that the heuristic exists and that the user is never silently shown a
stale `running` state more than once.

Studio does **not** offer "resume" of an orphaned attempt. The
canonical reset operation is `rowforge exec run --retry-failed` on a new
attempt — simpler, auditable, already supported by the CLI.

Responsibility split:
- Detection: `studio-core::workspace::open_default` invokes a scan via
  `rowforge-core::workspace::scan_for_orphans`.
- Repair (writing SQLite + meta): `rowforge-core::workspace::mark_aborted`.

## 3.8 Background and idle behaviour

- App Nap (macOS) is not opted out by default. Long-running attempts
  with Studio in the background may experience delayed UI updates but
  not delayed actual work (worker subprocesses are not affected by App
  Nap).
- Tokio timer drift in the background is irrelevant for run mechanics
  (no time-sensitive scheduling); it affects only `ETA` and `rate_*`
  display, which already use wall-clock deltas.
- The spec does not require Studio to remain in the foreground. Users
  are advised in docs (not the spec) that long runs are happier when
  the app is foregrounded.
