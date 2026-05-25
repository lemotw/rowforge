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
}
```

Plan 7 variant details:

| Variant | Serialized `kind` | Payload | Emitted by | UI rendering |
|---|---|---|---|---|
| `EditorNotFound` | `editor_not_found` | none (`message: null`) | `handler_open_editor` when none of preferred / `$VISUAL` / `$EDITOR` / probes resolved | Toast or inline error; copy points user to Settings → Editor or to set `$VISUAL`/`$EDITOR` |
| `HandlerNotFound { name }` | `handler_not_found` | `{ name }` | `handler_show`, `handler_open_editor`, `handler_reveal`, `handler_delete`, `handler_rename` when target dir is absent | Detail page: "Handler '<name>' not found. It may have been deleted or renamed." with back link to `/handlers` |
| `HandlerExists { name }` | `handler_exists` | `{ name }` | `handler_scaffold` (target dir already exists), `handler_rename` (new name already taken) | Inline banner in the relevant dialog; submit stays disabled until name changes |
| `InvalidHandlerName { name }` | `invalid_handler_name` | `{ name }` | `handler_scaffold`, `handler_rename` when name fails regex `/^[a-z0-9][a-z0-9-]*$/` | Inline field error; validated client-side during typing, server is authoritative |

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

run_start(execution_id, handler_dir,
          row_limit?, workers?,
          dry_run?, skip_attempted?)   -> RunStartedHandle
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
```

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

Events (one-way, core → UI):

```
run:<handle>                          ProgressEvent payload
runs:active                           RunRollupTick payload   (Part 6 §6.6)
handlers:list                         ()                      // Plan 7: coarse refresh hint emitted after scaffold/delete/rename
```

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
