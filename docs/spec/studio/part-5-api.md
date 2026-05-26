# Part 5 — API

Defines the public surface of `rowforge-studio-core`, the Tauri command
layer that consumes it, the error model, settings, and versioning.

## 5.1 Crate boundary contract

Three crates, three responsibilities:

### `rowforge-core` (engine)
Owns: streaming pipeline, worker pool, handler protocol, SQLite
migrations, all on-disk artifact parsing and writing, `RowResolution`
computation, manifest validation, workspace discovery.

The following items, if today they live in `rowforge-cli`, are lifted
into `rowforge-core` as part of v1:
- `default_workspace_root()`
- SQLite `open_with_migrations()`
- `compute_resolution` counts-only entry point
- `validate_manifest(source)` (returning a structured report)
- `outcomes.jsonl` line iteration as a public utility

Rationale: the CLI and `studio-core` are both legitimate consumers.

### `rowforge-studio-core` (GUI-only extension)
Owns: `UiError`, `SessionRegistry`, `ProgressAggregator` (event
sampling/coalescing), `ExecRollup` orchestration, `Settings` types and
file-format-agnostic load/save, replay adapter (v2).

Does **not** own: Tauri types, IPC concerns, app-data-dir resolution,
window event handling, manifest schema (re-exports the core type).

### `apps/rowforge-studio` (Tauri layer)
Owns: command translation, `tauri::State<StudioCore>` lifecycle, event
emit forwarding, settings file path resolution (via Tauri's
`app_data_dir`), startup wiring, telemetry hooks (if/when added).

May not bypass `studio-core` to call `rowforge-core` directly.

## 5.2 `StudioCore` public API (v1)

```rust
impl StudioCore {
    pub fn open(opts: OpenOpts) -> Result<Self, UiError>;
    pub fn workspace(&self) -> &Workspace;

    // Handler log (Plan 9; see Part 3 §3.9)
    pub fn handler_log_tail(&self, exec: &ExecutionId, attempt: &AttemptId, max_lines: Option<usize>)
        -> Result<Vec<HandlerLogLine>, UiError>;
    pub fn handler_log_subscribe(&self, attempt: &AttemptId)
        -> Result<broadcast::Receiver<HandlerLogLine>, UiError>;  // Err if attempt not active
    pub fn set_handler_log_capture_raw_stdout(&self, v: bool);

    // Read projections (Part 2)
    pub fn list(&self, filter: ListFilter) -> Result<Vec<ExecSummary>, UiError>;
    pub fn show(&self, id: &ExecutionId) -> Result<ExecDetail, UiError>;
    pub fn attempt(&self, e: &ExecutionId, r: &AttemptId)
        -> Result<AttemptDetail, UiError>;
    pub fn rollup(&self, e: &ExecutionId) -> Result<ExecRollup, UiError>;
    pub fn failed_page(&self, q: FailedPageQuery) -> Result<FailedRowPage, UiError>;
    pub fn row_history(&self, e: &ExecutionId, seq: u64)
        -> Result<RowHistory, UiError>;

    // Run lifecycle (Part 3 §3.3)
    pub fn start_run(&self, e: &ExecutionId, opts: RunOpts)
        -> Result<RunStartedHandle, UiError>;
    // RunStartedHandle = { handle: RunHandle, attempt_id: String }
    // — returning attempt_id lets the UI build the
    //   /exec/:id/attempt/:aid?run=<handle> URL in one round-trip.
    pub fn subscribe(&self, h: &RunHandle) -> Result<RunStream, UiError>;
    pub fn cancel(&self, h: &RunHandle, mode: CancelMode) -> Result<(), UiError>;
    pub fn status(&self, h: &RunHandle) -> Result<RunStatus, UiError>;
    pub fn active_runs(&self) -> Vec<RunHandle>;
    pub fn active_runs_stream(&self) -> ActiveRunsStream;  // Part 6 §6.6

    // Execution lifecycle
    pub fn start_exec(&self, args: StartExecArgs) -> Result<ExecutionId, UiError>;
    pub fn export(&self, e: &ExecutionId, opts: ExportOpts)
        -> Result<ExportReport, UiError>;

    // Plan 11 — re-run failed rows
    pub fn attempt_failed_row_ids(&self, exec_id: &ExecutionId, attempt_id: &AttemptId)
        -> Result<Vec<u64>, UiError>;
    // Reads outcomes.jsonl for the given attempt; collects seq values from
    // BatchOutcome envelopes where the nested outcome type is "error" or "crash".
    // Returns a deduped, ascending-sorted Vec<u64>. The seq field is the
    // row identifier (u64) used throughout the pipeline (on-disk field name: seq).

    // Execution deletion (Plan 10; see Part 3 §3.10)
    pub fn execution_delete(&self, exec_id: &str) -> Result<(), UiError>;
    pub fn execution_delete_bulk(&self, exec_ids: Vec<String>)
        -> Result<ExecDeleteBulkResult, UiError>;

    // Handler-authoring anchor points (Part 5 §5.4)
    pub fn validate_manifest(&self, source: ManifestSource)
        -> Result<ManifestReport, UiError>;
}
```

Supporting types:

```rust
struct OpenOpts { workspace: Option<PathBuf> }
struct ListFilter { /* v1: none; reserved for future */ }
struct RunOpts {
    handler: HandlerSource,
    limit: Option<u64>,
    dry_run: bool,
    workers: Option<u32>,
    force: bool,
    retry_failed: bool,
    config_overrides: BTreeMap<String, JsonValue>,
    mapping: Option<FieldMapping>,
    sync_data: bool,
    only_row_ids: Option<Vec<u64>>,  // Plan 11: when Some, dispatch only these seqs
}
enum HandlerSource {
    Dir(PathBuf),
    // v2: Sandbox { manifest: ManifestDraft, source_dir: PathBuf },
}
enum CancelMode { Soft, Hard }
struct RunStream {
    handle: RunHandle,
    rx: broadcast::Receiver<ProgressEvent>,
    snapshot: ProgressSnapshot,         // counters captured at subscribe time
}
struct StartExecArgs {
    input_path: PathBuf,
    name: String,
    csv_id: Option<String>,
    pinned_handler_instance: Option<HandlerInstanceId>,
}
struct ExportOpts {
    output_dir: Option<PathBuf>,
    format: ExportFormat,               // Csv | Jsonl | Both
    require_complete: bool,
}
enum ManifestSource {
    Path(PathBuf),
    // v2: Draft(ManifestDraft),
}
struct ManifestReport {
    manifest: Manifest,                 // parsed, if successful
    errors: Vec<ManifestError>,
    warnings: Vec<ManifestWarning>,
}
```

What is **deliberately not** in the API:
- `raw_outcomes_path(&self, ...)` — no escape hatch around projections.
- `sql_query(&self, ...)` — no direct SQL access.
- A `subscribe_all_runs()` that multiplexes per-run streams onto one
  channel — that would break per-handle event-name isolation (Part 6
  §6.6). Use `active_runs_stream()` instead, which is a counters-only
  aggregate.

## 5.3 Error model

```rust
#[derive(Debug, thiserror::Error, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum UiError {
    #[error("{kind} not found: {id}")]
    NotFound { kind: String, id: String },

    #[error("invalid argument: {0}")]
    InvalidArg(String),

    #[error("handler build failed")]
    HandlerBuildFailed { stderr: String },

    #[error("run aborted: {reason:?}")]
    RunAborted { reason: AbortReason },     // structured; see Part 6 §6.5

    #[error("handle expired or unknown: {0}")]
    UnknownHandle(String),

    #[error("workspace locked or incompatible: {by}")]
    WorkspaceLocked { by: String },

    #[error("manifest invalid")]
    ManifestInvalid { errors: Vec<ManifestError> },

    #[error("run cannot start: another run is active for {execution_id}")]
    RunBusy { execution_id: String, scope: BusyScope }, // PerExec | Workspace

    #[error("io error: {0}")]
    Io(String),

    #[error("internal: {0}")]
    Internal(String),

    // Plan 7 — handler management variants (see Part 8 §8.5.4 for full handler error set)
    #[error("editor not found")]
    EditorNotFound,

    #[error("handler not found: {name}")]
    HandlerNotFound { name: String },

    #[error("handler already exists: {name}")]
    HandlerExists { name: String },

    #[error("invalid handler name: {name}")]
    InvalidHandlerName { name: String },

    // Plan 8 — build variants (see Part 8 §8.5.4 for details)
    #[error("build failed for handler '{name}' (exit {exit_code})")]
    BuildFailed { name: String, exit_code: i32 },

    #[error("build tool '{tool}' for handler '{name}' not found in PATH")]
    ToolchainMissing { name: String, tool: String },

    #[error("handler '{name}' has no entry.build in its manifest")]
    NoBuildCommand { name: String },

    // Plan 10 — execution deletion
    #[error("execution is in use: {exec_id}")]
    ExecutionInUse { exec_id: String },
}
```

Plan 7 variant details:

| Variant | Serialized `kind` | Payload | Emitted by | UI rendering |
|---|---|---|---|---|
| `EditorNotFound` | `editor_not_found` | none (`message: null`) | `handler_open_editor` when none of preferred / `$VISUAL` / `$EDITOR` / probes resolved | Toast or inline error; copy points user to Settings → Editor or to set `$VISUAL`/`$EDITOR` |
| `HandlerNotFound { name }` | `handler_not_found` | `{ name }` | `handler_show`, `handler_open_editor`, `handler_reveal`, `handler_delete`, `handler_rename` when target dir is absent | Detail page: "Handler '<name>' not found. It may have been deleted or renamed." with back link to `/handlers` |
| `HandlerExists { name }` | `handler_exists` | `{ name }` | `handler_scaffold` (target dir already exists), `handler_rename` (new name already taken) | Inline banner in the relevant dialog; submit stays disabled until name changes |
| `InvalidHandlerName { name }` | `invalid_handler_name` | `{ name }` | `handler_scaffold`, `handler_rename` when name fails regex `/^[a-z0-9][a-z0-9-]*$/` | Inline field error; validated client-side during typing, server is authoritative |
| `InvalidArg(String)` | `invalid_arg` | `{ message }` | `handler_scaffold` when `primary_field` fails identifier regex `^[a-zA-Z_][a-zA-Z0-9_]*$` | Inline field error on the primary_field input; prevents YAML/Go injection in scaffolded files |

Plan 8 variant details:

| Variant | Serialized `kind` | Payload | Emitted by | UI rendering |
|---|---|---|---|---|
| `BuildFailed { name, exit_code }` | `build_failed` | `{ name, exit_code }` | `handler_build` when build exits non-zero | Sonner toast: "Build failed for 'NAME' (exit N). See the Last build section for details." |
| `ToolchainMissing { name, tool }` | `toolchain_missing` | `{ name, tool }` | `handler_build` when `entry.build[0]` not in `PATH` | Toast: "Build tool 'TOOL' not found in PATH. Install it or update entry.build in your manifest." |
| `NoBuildCommand { name }` | `no_build_command` | `{ name }` | `handler_build` when manifest has no `entry.build` | Toast: "Handler 'NAME' has no entry.build command in rowforge.yaml." |

Plan 10 variant details:

| Variant | Serialized `kind` | Payload | Emitted by | UI rendering |
|---|---|---|---|---|
| `ExecutionInUse { exec_id }` | `execution_in_use` | `{ exec_id }` | `execution_delete` / `execution_delete_bulk` (per-item) when `SessionRegistry::has_active_run_for_exec` returns `true` | Checkbox disabled in ExecList select mode with tooltip "Cancel active run first"; bulk-fail yellow alert above the list showing which exec_ids could not be deleted |

Composition rules:
- No blanket `From<anyhow::Error> for UiError`.
- Each call site classifies root cause and picks the right variant.
- `From<std::io::Error>` and `From<serde_json::Error>` map to `Io`.
- `Internal` is reserved for "could not classify"; UI shows a
  generic toast with a copy-details button.

## 5.4 Extension surface for handler authoring (anchor points)

> **Realized in Part 8.** Handler authoring is now v1 scope. The
> anchors below remain valid but their v2-only labels (`Sandbox`,
> `Draft`) refer to features still deferred. See Part 8 §8.5 for the
> full handler API added on top of these anchors.

v1 reserves three anchor points so handler-authoring features land
without breaking changes:

1. **`HandlerSource` enum** — v1 has only `Dir(PathBuf)`. v2 will add
   `Sandbox { manifest: ManifestDraft, source_dir: PathBuf }` so smoke
   tests can run against an unsaved draft.

2. **`ManifestSource` enum** — same shape: `Path(PathBuf)` in v1,
   `Draft(ManifestDraft)` added in v2.

3. **`validate_manifest`** — v1 implementation is a thin wrapper over
   `rowforge-core`'s existing manifest validator, returning a
   structured `ManifestReport` instead of the CLI's text output. The
   editor in v2 calls this on every save / on the fly without further
   API change.

`Manifest`, `ManifestDraft`, `ManifestError`, `ManifestWarning`, and
`ManifestSource` all live in `rowforge-core` and are re-exported by
`studio-core`.

## 5.5 Tauri command surface

Names are `noun_verb`, snake_case (Tauri's JS binding camelCases
automatically; we do not configure overrides). Every command returns
`Result<T, UiError>` directly; no `{ data, meta }` envelope in v1.

```
workspace_open(opts)                  -> Workspace
workspace_settings_load()             -> Settings
workspace_settings_save(s)            -> ()

exec_list(filter)                     -> Vec<ExecSummary>
exec_show(id)                         -> ExecDetail
exec_rollup(id)                       -> ExecRollup
exec_start(args)                      -> ExecutionId
exec_export(id, opts)                 -> ExportReport

attempt_show(execution_id, attempt_id)            -> AttemptDetail
attempt_failed_page(query)                        -> FailedRowPage
attempt_row_history(execution_id, seq)            -> RowHistory
attempt_failed_row_ids(execution_id, attempt_id)  -> Vec<u64>
    // Plan 11. Reads outcomes.jsonl; returns deduped ascending seq values
    // where BatchOutcome outcome type is "error" or "crash". The outcome
    // type field is named "type" (not "status"). Returns [] if the attempt
    // has no failures. Returns NotFound if the attempt does not exist.

run_start(execution_id, handler_dir,
          row_limit?, workers?,
          dry_run?, skip_attempted?,
          only_row_ids?)               -> RunStartedHandle
    // only_row_ids (Option<Vec<u64>>, Plan 11): when provided, the pipeline
    // dispatches only the listed seq values, bypassing skip_seqs for those rows.
run_cancel(handle, mode)              -> ()
run_status(handle)                    -> RunStatus
run_active()                          -> Vec<RunHandle>
run_snapshot(handle)                  -> ProgressSnapshot
attempt_active_handle(attempt_id)     -> Option<RunHandle>

manifest_validate(source)             -> ManifestReport

// Plan 7 — handler management commands (see Part 8 §8.5.3 for full list)
handler_list()                        -> Vec<HandlerSummary>
handler_show(name)                    -> HandlerDetail
handler_open_editor(name)             -> ()
handler_reveal(name)                  -> ()
handler_scaffold(args)                -> String          // returns new handler name
handler_delete(name)                  -> ()
handler_rename(old, new)              -> ()

// Plan 8 — build command (see Part 8 §8.5.3 for details)
handler_build(name: String)           -> BuildOutcome    // async; emits handlers:list

// Plan 9 — handler log commands (see Part 3 §3.9)
handler_log_tail(exec_id, attempt_id, max_lines?)   -> Vec<HandlerLogLine>
    // Reads up to max_lines (default 5000) from handler_log.log on disk.
    // Returns empty vec if the file does not exist (pre-Plan-9 attempt).
handler_log_subscribe(exec_id, attempt_id)          -> ()
    // Async. Spawns a batching pump that emits handler_log:<attempt_id>
    // events (100 ms / 64-line batch). Errors if attempt is not active.
handler_log_unsubscribe(attempt_id)                 -> ()
    // Cancels the pump started by handler_log_subscribe.

// Plan 10 — execution deletion (see Part 3 §3.10)
execution_delete(exec_id)                           -> ()
    // Deletes one execution. Emits exec_list:refresh on success.
    // Returns ExecutionInUse if a session is active for the exec.
    // Returns NotFound if the exec does not exist.
execution_delete_bulk(exec_ids)                     -> ExecDeleteBulkResult
    // Deletes multiple executions serially. Emits exec_list:refresh after
    // any successful delete. Never aborts early; partial failures are
    // returned in ExecDeleteBulkResult.failed.
```

`handler_build` note: the command is declared `async` but currently
blocks the Tauri async runtime for the duration of the build (no
`spawn_blocking`). Refactor flagged for a later plan; typical Go/Rust
builds complete in < 30 s.

`BuildOutcome` type (lives in `rowforge-core::build`):

```rust
struct BuildOutcome {
    started_at: DateTime<Utc>,
    finished_at: DateTime<Utc>,
    exit_code: i32,
    command: Vec<String>,   // copy of entry.build at run time
    stdout: String,
    stderr: String,
}
```

See Part 8 §8.3 for full type context.

Notes on the run lifecycle commands (Plan 5):

- `run_start` returns a `RunStartedHandle { handle, attempt_id }`
  so the UI can build the `/exec/:id/attempt/:aid?run=<handle>` URL
  in one round-trip.
- `row_limit` (`Option<u64>`) caps how many rows are dispatched.
  Combined with `skip_attempted` it enables successive sampling of
  fresh rows across runs.
- `skip_attempted` (`Option<bool>`) — when true, `RowResolution` for
  the execution is computed and every already-attempted seq
  (anything not `NeverAttempted`) is passed as `skip_seqs` to the
  pipeline. Used by the UI's "sample fresh rows across runs" path.
- `run_snapshot` returns the live `ProgressSnapshot` for a handle in
  the registry. Used by the React `useRun` hook to bootstrap state
  after `listen()` attaches — Tauri events are fire-and-forget, so
  events emitted before subscription would otherwise be lost.
  Returns `UnknownHandle` if the run already terminated (the React
  side treats that as "fall back to attempt_show static data").
- `attempt_active_handle` resolves an `AttemptId` to its live
  `RunHandle` if a session exists in the registry. Used so a user
  who navigates into an in-flight attempt without `?run=` in the URL
  can see a "Watch live" affordance.
- `only_row_ids` (Plan 11, `Option<Vec<u64>>`) — when provided, the
  pipeline reader dispatches only the rows whose `seq` values are in
  the list, overriding any `skip_seqs` filter. The TypeScript binding
  uses `onlyRowIds` (camelCased automatically by Tauri). The
  `seq` identifier is the same u64 used in `outcomes.jsonl` envelopes
  (on-disk JSON field name: `seq`). Supplied by `useRunStart` when
  called from the Re-run failed dialog.

Events (one-way, core → UI):

```
run:<handle>                          ProgressEvent payload
runs:active                           RunRollupTick payload   (Part 6 §6.6)
handlers:list                         ()                      // Plan 7: coarse refresh hint emitted after scaffold/delete/rename
handler_log:<attempt_id>              HandlerLogBatch payload // Plan 9: batched 100 ms / 64-line
exec_list:refresh                     ()                      // Plan 10: emitted after any successful execution_delete / execution_delete_bulk; React invalidates exec_list query
```

`HandlerLogBatch` payload:
```typescript
interface HandlerLogBatch {
  lines: HandlerLogLine[];
  dropped: number;          // lines lost due to broadcast backpressure since last batch
}
```

**`HandlerLogLine` and `HandlerStream` types (Plan 9):**

```typescript
type HandlerStream = "stdout" | "stderr";

interface HandlerLogLine {
  timestamp: string;         // RFC 3339, e.g. "2026-05-25T14:32:01.423Z"
  worker_id: number;
  stream: HandlerStream;
  line: string;
}
```

These types are mirrored in Rust as:

```rust
// rowforge_core::handler_log
pub enum HandlerStream { Stdout, Stderr }
pub struct HandlerLogLine {
    pub timestamp: DateTime<Utc>,
    pub worker_id: u32,
    pub stream: HandlerStream,
    pub line: String,
}
```

**`ExecDeleteBulkResult` and `ExecDeleteFailure` types (Plan 10):**

```typescript
interface ExecDeleteBulkResult {
  deleted: string[];             // exec_ids successfully removed
  failed: ExecDeleteFailure[];
}

interface ExecDeleteFailure {
  exec_id: string;
  reason: string;                // e.g. "execution is in use" or "not found"
}
```

TypeScript mirrors of Part 2 §2.2.10. The `useExecutionDeleteBulk` hook
wraps `execution_delete_bulk` and invalidates the `exec_list` query on
any successful delete (including partial success where `deleted.length >
0`).

## 5.6 Settings

- File path: `<app_data_dir>/rowforge-studio/settings.json`.
- Format: JSON, schema-versioned.
- Type lives in `studio-core::settings`; path resolution and IO live in
  the Tauri layer.
- `studio-core` exposes `Settings::load_from(reader)` and
  `Settings::save_to(writer)` taking `Read`/`Write` to keep itself
  filesystem-policy-free.

**`max_concurrent_runs` reload semantic:** the value is read at
`workspace_open` time and passed to `SessionRegistry::new` as the
workspace-scoped limit (Part 3 §3.4). Changing it via
`workspace_settings_save` does NOT affect the active SessionRegistry —
the new limit only takes effect on the next `workspace_open` (which
happens during boot autoload or via the Settings page's "Switch
workspace" button). The Settings page surfaces this with a
"Will apply on next workspace open" dirty banner when the form
value differs from the loaded value.

## 5.7 Versioning and API stability

- `rowforge-studio-core` is an **internal** crate; not published to
  crates.io. Version travels with the app.
- `rowforge-core` is referenced by path (`{ path = "..." }`); same-tree
  lockstep. Any breaking change in core requires a same-PR update of
  studio-core.
- All public `enum`s in `studio-core` carry `#[non_exhaustive]`.
- All public `struct`s with growable field sets carry
  `#[non_exhaustive]`.
- API versioning policy: `studio-core` does not promise stability to
  external code. The Tauri app and `studio-core` are released together.
