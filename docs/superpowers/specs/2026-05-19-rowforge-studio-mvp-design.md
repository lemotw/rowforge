# rowforge-studio MVP — Design

- **Date**: 2026-05-19
- **Status**: Approved for planning
- **Scope**: First milestone of `apps/rowforge-studio`, focused on **execution management**. Handler authoring (manifest editor, scaffolding, smoke tests, pack) is explicitly deferred.

## 1. Goals & non-goals

### Goals
- Provide a desktop GUI for the existing `rowforge exec` workflow (start, list, show, run, attempts, attempt, export) without requiring users to drop to the terminal.
- Surface real-time progress while an attempt is running, with a lightweight UI (progress counters + recent-events tail).
- Share the same on-disk executions repository as the CLI, so Studio and CLI users see the same data.

### Non-goals (this milestone)
- Handler authoring: scaffolding, manifest editor, in-app build, smoke tests, `pack` packaging.
- Replay / watch of attempts running in another process (no `watch.rs` module).
- Multi-workspace registry, remote workspaces, daemon mode.
- i18n, theming beyond defaults, accessibility tuning beyond Tauri defaults.
- Front-end automated tests; performance benchmarks; visual regression.

## 2. Architecture

```
┌──────────────────────────────────────────────────┐
│  apps/rowforge-studio  (Tauri app)               │
│    src-tauri/src/commands.rs   ← thin glue       │
│    ui/  (React + Vite + TypeScript) ← single SPA │
└───────────────┬──────────────────────────────────┘
                │ method calls, broadcast channels
                ▼
┌──────────────────────────────────────────────────┐
│  crates/rowforge-studio-core   (Tauri-agnostic)  │
│    StudioCore { workspace, sessions }            │
│    ├─ workspace.rs                               │
│    ├─ exec_view.rs                               │
│    ├─ run_session.rs                             │
│    ├─ error.rs (UiError)                         │
│    └─ lib.rs (re-exports rowforge-core types)    │
└───────────────┬──────────────────────────────────┘
                │ consumes only rowforge-core public API
                ▼
┌──────────────────────────────────────────────────┐
│  crates/rowforge-core    (unchanged / minor)     │
└──────────────────────────────────────────────────┘
```

### Architectural decisions

1. **`rowforge-studio-core` is an *extension* of `rowforge-core`, not a wrapper.** If a capability is also useful to the CLI (e.g. computing attempt summaries, locating the default workspace), it belongs in `rowforge-core` and both consumers call it. studio-core only contains things the CLI does not need.
2. **studio-core has no Tauri dependency.** It exposes plain Rust types and `tokio` channels. The Tauri layer subscribes to channels and forwards events via `Window::emit`.
3. **studio-core re-exports the rowforge-core types** that appear in its public API (e.g. `ExecutionId`, `AttemptId`, `RowOutcomeKind`, `Manifest`) so `commands.rs` imports a single crate.
4. **No second consumer is designed for.** TUI / web / remote frontends are out of scope; the public surface is shaped for the Tauri app only. If a second consumer appears later, refactor then.
5. **No separate daemon or worker process.** Everything runs inside the Tauri main process's `tokio` runtime.
6. **`rowforge-cli` is not refactored in this milestone.** A future change may move duplicated workspace-discovery / projection code into `rowforge-core` so both CLI and studio-core consume it.

## 3. `rowforge-studio-core` modules

### 3.1 `workspace.rs`
- `pub struct Workspace { root: PathBuf }`
- `pub fn default_workspace() -> Result<Workspace, UiError>` — uses the same default-root logic as the CLI.
- If the equivalent logic currently lives inside `rowforge-cli`, it is lifted into `rowforge-core` so both crates share it (this is an in-scope minor change to core).

### 3.2 `exec_view.rs` — read-side projections for the UI
- `pub fn list(ws: &Workspace) -> Result<Vec<ExecSummary>, UiError>`
  - Scans the workspace root, reads each execution's `meta.json` plus the latest attempt's `outcomes.jsonl` tail for counts.
  - `ExecSummary { id, name, created_at, input_rows, attempts_count, last_state, success, failed, never_attempted }`
- `pub fn show(ws, exec_id) -> Result<ExecDetail, UiError>` — execution detail + per-attempt summaries.
- `pub fn attempt(ws, exec_id, attempt_id) -> Result<AttemptDetail, UiError>` — paths, stats, `run_type`.
- `pub fn failed_sample(ws, exec_id, attempt_id, offset, limit) -> Result<Vec<FailedRow>, UiError>` — paged sample of failed rows. **Never loads all rows into memory.** UI default is `offset=0, limit=50`.

### 3.3 `run_session.rs` — attempt lifecycle
- `pub struct RunHandle(String)` — serializable; opaque to the front-end.
- `pub struct RunOpts { handler_dir, limit, dry_run, workers, force, retry_failed, config_overrides, mapping }`
- `pub enum ProgressEvent { Tick { processed, total, success, failed }, Outcome { row_index, kind, brief }, Done(RunReport), Aborted(UiError) }`
- `pub enum RunStatus { Running, Done, Aborted }`
- Internally each session owns:
  - a `tokio::task::JoinHandle<()>` running the rowforge-core pipeline,
  - a `tokio::sync::broadcast::Sender<ProgressEvent>` injected into the pipeline in place of stderr progress logging,
  - the existing rowforge-core `CancellationToken` for cooperative cancel.

### 3.4 `error.rs` — UI-facing error type
```rust
#[derive(Debug, thiserror::Error, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UiError {
    #[error("{kind} not found: {id}")]
    NotFound { kind: String, id: String },
    #[error("invalid argument: {0}")]
    InvalidArg(String),
    #[error("handler build failed")]
    HandlerBuildFailed { stderr: String },
    #[error("run aborted: {reason}")]
    RunAborted { reason: String },
    #[error("handle expired or unknown: {0}")]
    UnknownHandle(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("internal: {0}")]
    Internal(String),
}
```
- `From<std::io::Error>` and `From<serde_json::Error>` map to `Io`.
- Errors from `rowforge-core` are **classified explicitly** at each call site (no blanket `From<anyhow::Error>`):
  - "startup timeout" / "all workers crashed" → `RunAborted`
  - handler build failures → `HandlerBuildFailed { stderr }`
  - everything else → `Internal(format!("{e:#}"))`

### 3.5 `lib.rs` — top-level handle
```rust
pub struct StudioCore {
    workspace: Workspace,
    sessions: SessionRegistry,
}

impl StudioCore {
    pub fn open_default() -> Result<Self, UiError>;

    // Read projections
    pub fn list(&self) -> Result<Vec<ExecSummary>, UiError>;
    pub fn show(&self, exec_id: &ExecutionId) -> Result<ExecDetail, UiError>;
    pub fn attempt(&self, exec_id: &ExecutionId, attempt_id: &AttemptId)
        -> Result<AttemptDetail, UiError>;
    pub fn failed_sample(&self, exec_id: &ExecutionId, attempt_id: &AttemptId,
                         offset: usize, limit: usize)
        -> Result<Vec<FailedRow>, UiError>;

    // Run lifecycle
    pub fn start_run(&self, exec_id: &ExecutionId, opts: RunOpts)
        -> Result<RunHandle, UiError>;
    pub fn subscribe(&self, handle: &RunHandle)
        -> Result<broadcast::Receiver<ProgressEvent>, UiError>;
    pub fn cancel(&self, handle: &RunHandle) -> Result<(), UiError>;
    pub fn status(&self, handle: &RunHandle) -> Result<RunStatus, UiError>;

    // Execution lifecycle
    pub fn start_exec(&self, args: StartExecArgs) -> Result<ExecutionId, UiError>;
    pub fn export(&self, exec_id: &ExecutionId, opts: ExportOpts)
        -> Result<ExportReport, UiError>;
}
```
- `SessionRegistry` (internal): `Mutex<HashMap<RunHandle, RunSession>>`.
- Completed handles are reclaimed; calling `cancel`/`subscribe`/`status` on a reclaimed handle returns `UiError::UnknownHandle`. The exact reclaim policy (e.g. "N seconds after `Done`") is an implementation detail and is **not** fixed in this spec.
- Re-exports from `rowforge-core` whose names appear in the API above (`ExecutionId`, `AttemptId`, `Manifest`, `RowOutcomeKind`, etc.).

### Modules deferred to a later milestone
- `watch.rs` — observing attempts running in another process. Not needed while runs are spawned inside the Studio process.
- Handler authoring modules (manifest editor backing, scaffolding, smoke test, pack).

## 4. Data flow

### 4.1 Open Studio → executions list
```
UI mount
  → invoke("list_executions")
  → commands::list_executions(core: State<StudioCore>)
  → core.list()                       // reads filesystem
  → Vec<ExecSummary> (serde-JSON)
  → React renders the table
```
No subscription, no long-poll. Refresh is manual (button + window focus).

### 4.2 Start a run + receive progress
```
UI clicks "Run attempt"
  → invoke("start_run", { exec_id, opts })
  → commands::start_run()
      → core.start_run(...)
          ├─ spawn tokio task running the rowforge-core pipeline
          ├─ register (handle → RunSession) in SessionRegistry
          └─ return RunHandle
  → commands spawns a forwarder task:
        let mut rx = core.subscribe(&handle)?;
        while let Ok(evt) = rx.recv().await {
            window.emit(&format!("run:{handle}"), &evt)?;
            if matches!(evt, Done(_) | Aborted(_)) { break; }
        }

UI listens to "run:<handle>" → updates progress + a 200-entry ring buffer of Outcome events.
```

Cancel: `invoke("cancel_run", { handle })` → `core.cancel(&handle)` → cooperative cancel via the existing rowforge-core `CancellationToken`; the pipeline emits `Aborted` (not `Done`) on its way out and persists attempt state as aborted.

### 4.3 View a finished attempt
```
UI selects an attempt row
  → invoke("show_attempt", { exec_id, attempt_id })
  → core.attempt(...)
  → AttemptDetail { paths, stats, run_type, ... }

For the failed-rows panel:
  → invoke("failed_sample", { exec_id, attempt_id, offset: 0, limit: 50 })
  → core.failed_sample(...)
  → Vec<FailedRow>
  // UI offers a "show next 50" button that bumps offset.
```
No full-attempt load into memory. No virtual scrolling.

### Invariants
1. **`RunHandle` is the only run identifier crossing the IPC boundary.** `RunSession` never leaves studio-core.
2. **Progress events are one-way (core → UI).** UI actions on a run (cancel) go through separate `invoke` calls, not back-channels on the event stream.
3. **Each handle has its own emit channel name** (`"run:<handle>"`). Multiple concurrent runs do not cross-contaminate the front-end listeners.

## 5. Error handling

- `rowforge-core` keeps its existing error types unchanged.
- `studio-core` returns `Result<T, UiError>` from every public method. `UiError` is `Serialize`, so Tauri can return it directly to the front-end.
- `commands.rs` adds no extra error type. It returns `Result<T, UiError>` from each command.
- The front-end uses a single `call<T>(cmd, args)` wrapper around `invoke`; the catch path receives a `UiError` and routes it based on the `kind` tag:
  - `HandlerBuildFailed` — collapsible panel showing `stderr`.
  - `RunAborted` — banner with `reason`, plus a link to the attempt's on-disk logs.
  - `NotFound` / `InvalidArg` — inline form error.
  - `Internal` — generic toast with a "Copy log" affordance.

### Explicitly deferred
- No i18n / error-code table.
- No automatic retry. The UI exposes `--force` and `--retry-failed` as options; the user decides.
- No error aggregation for per-row failures; those are data (in `outcomes.jsonl`), not errors.

## 6. Testing strategy

### studio-core (primary coverage)
- `tests/list_executions.rs` — `tempdir` fixtures with multiple synthetic exec directories; assert `StudioCore::list()` returns correct summaries.
- `tests/attempt_failed_sample.rs` — fixture with mixed `success` / `failed` / `crashed` outcomes; assert pagination is correct and bounded in memory.
- `tests/run_session_happy.rs` — real run against `crates/test-handler` (or `examples/handlers/golang-uppercase`); subscribe to the broadcast channel; assert event sequence `Tick → Outcome* → Done`.
- `tests/run_session_cancel.rs` — `start_run` then immediately `cancel`; assert receiver yields `Aborted` (not `Done`) **and** that the attempt is persisted with aborted state (this is the user-visible invariant that "cancel actually cancelled").
- `tests/error_mapping.rs` — point at a missing handler dir and at a deliberately-broken handler; assert `UiError` lands in `HandlerBuildFailed` rather than `Internal`.

### Tauri commands
- No dedicated unit tests; commands are thin glue and rely on studio-core coverage.
- A single manual smoke pass after `cargo tauri build`: list → start → cancel → start → wait → export. No webdriver / playwright in this milestone.

### Front-end
- No automated tests in this milestone (logic lives in studio-core).
- TypeScript strict mode + ESLint as guard rails.

### CI
- `cargo test --workspace`
- `cargo clippy -- -D warnings`
- `cargo fmt --check`
- `pnpm tsc --noEmit` and `pnpm lint`
- Tauri release build is **not** in CI; it runs locally / per release.

## 7. Out of scope (handler authoring milestone)

Tracked for the next milestone, not this one:
- Manifest editor with validation against the `rowforge.yaml` schema.
- Project scaffolding wizard (Go / Python / raw stdio templates).
- In-app build (running `entry.build`) and smoke test (`run --limit N --dry-run`) wired to UI.
- `pack` packaging UI.
- `watch.rs` for replaying / observing externally-run attempts.

## 8. Open questions

None at design time. Reclaim policy for completed `RunHandle`s is intentionally left to implementation.
