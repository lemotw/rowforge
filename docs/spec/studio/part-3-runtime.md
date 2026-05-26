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
        ┌──────────┐
        │ Starting │  Session registered; workers spawning, handlers building / handshaking.
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

Sessions register directly into `Starting`. There is no `Pending` state —
`start_run` inserts the SQLite `attempts` row and spawns the tokio task
atomically, so the session is always at least `Starting` by the time it
is visible.

Persistence at transitions:

- **Starting** (on registration): SQLite `attempts` row inserted with
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

## 3.9 Handler log tee (Plan 9)

When Studio starts a run via `run_pipeline_in_process`, the pool-streaming
layer tees every worker's stdio output to two destinations simultaneously:
`<attempt_dir>/handler_log.log` on disk and a `broadcast::Sender<HandlerLogLine>`
held in the `Session`.

### What is captured

- **stderr** — all lines, unconditionally.
- **stdout** — non-JSON lines only by default (e.g. debug prints). Valid
  outcome JSON lines are excluded unless `Settings.handler_log_capture_raw_stdout`
  is `true` (Part 2 §2.2.9). Outcome JSON is typically high-volume; writing
  it to the log creates large files without adding diagnostic value.

### On-disk format

Each appended line has the form:

```
<rfc3339-timestamp> [handler#<worker_id> <stream>] <content>
```

where `<stream>` is `stdout` or `stderr`. Example:

```
2026-05-25T14:32:01.423Z [handler#2 stderr] panic: nil pointer dereference
```

The format is designed for `cat`/`less`/`grep` without rowforge-specific
tooling. Lines are appended as they arrive; the file is never truncated
during a run. No rotation is performed — callers must manage file size.

### Broadcast channel for live tail

A `broadcast::Sender<HandlerLogLine>` (capacity 4096) is created per
attempt when the run starts and stored in the `Session`. Studio's Tauri
layer subscribes via `StudioCore::handler_log_subscribe(attempt_id)` to
fan out lines to the UI in real time.

Backpressure: when a subscriber's receive buffer fills, the oldest
unread messages are silently dropped by `tokio::sync::broadcast`. The
Tauri event pump carries a `dropped: u64` field in every batch payload
so the UI can surface a warning banner. The file on disk is always
complete — dropping only affects the in-process broadcast path.

### Batching policy

The Tauri event `handler_log:<attempt_id>` is emitted at most every 100 ms
OR when 64 lines have accumulated, whichever comes first. Payload:

```typescript
{ lines: HandlerLogLine[], dropped: number }
```

### CLI back-compat

The pool-streaming tee is additive: when `on_handler_log` is `None` (the
CLI path), stderr is still printed to the terminal via `eprintln!` as
before. The `capture_raw_stdout` flag is irrelevant in the CLI path and
defaults to `false`.

## 3.10 Execution deletion (Plan 10)

### Active-run gate

`StudioCore::execution_delete(exec_id)` first validates that `exec_id`
passes the `is_valid_id_component` check (same regex used everywhere for
id validation), then queries `SessionRegistry::has_active_run_for_exec`.
If any session is alive for that execution the call returns
`UiError::ExecutionInUse { exec_id }` immediately — no partial work is
done.

### SQLite cascade (manual; no `ON DELETE CASCADE`)

The current schema does **not** use `ON DELETE CASCADE` on the foreign
key from `attempts` to `executions`. Deletion is therefore performed
manually inside a single transaction:

1. `DELETE FROM attempts WHERE execution_id = ?`
2. `DELETE FROM executions WHERE id = ?`

Both statements execute atomically. If either fails, the transaction is
rolled back and an error is returned.

### Filesystem cleanup

After the SQLite transaction commits, Studio calls `fs::remove_dir_all`
on `<workspace_root>/executions/<exec_id>/`. This step is **best-effort**:

- If the directory is missing (already removed externally) the error is
  silently ignored.
- If `remove_dir_all` fails for any other reason (permissions, OS error)
  the error is **logged but not returned to the caller** — the SQLite
  record is the authoritative source of truth. The orphaned directory
  will not appear in future `exec_list` results but will remain on disk.

### Idempotency

Attempting to delete an execution that does not exist returns a
`UiError::NotFound { kind: "execution", id }`. This is the only failure
mode after the active-run gate clears (short of IO errors), so callers
can treat a successful delete followed by a repeated delete as a
predictable `NotFound`.

### Bulk deletion

`StudioCore::execution_delete_bulk(exec_ids: Vec<String>)` iterates the
list serially, calling `execution_delete` for each id. Failures are
accumulated into `ExecDeleteBulkResult::failed`; the loop **never aborts
early**. All remaining ids are attempted regardless of earlier failures.
The function always returns `Ok(ExecDeleteBulkResult)` — the `Result`
error arm is only used for argument-validation errors that apply to the
entire call (e.g. empty id list), not per-item failures.

## 3.11 Selective row dispatch — `only_row_ids` (Plan 11)

`rowforge-core::RunRequest` carries an optional `only_row_ids:
Option<Vec<u64>>` field. When `Some`, the reader task filters input rows
**before** the `skip_seqs` check: a row is dispatched only if its `seq`
value appears in the supplied set.

Key semantics:

- The row identifier in `only_row_ids` is the same `seq` (u64) that
  appears on disk in `outcomes.jsonl` envelope fields. The label
  "row_ids" is used at the API surface for clarity; the on-disk JSON
  field is `seq`.
- **`only_row_ids` overrides `skip_seqs`**: if a seq is in
  `only_row_ids`, it is dispatched even if it was previously attempted
  and would otherwise be skipped by `skip_seqs`. This is intentional —
  the re-run-failed flow explicitly wants to re-dispatch rows that have
  been attempted before.
- When `None` (the default), behaviour is unchanged: all rows pass the
  reader unless filtered by `skip_seqs` or `limit`.
- `ReaderConfig.only_row_ids` carries the value through to the reader
  task; the matching is a set-membership check (`HashSet<u64>`).

Primary consumer: Plan 11's re-run-failed flow. `StudioCore::start_run`
accepts `RunOpts.only_row_ids` and plumbs it through to `RunRequest`
before spawning the pipeline.

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
