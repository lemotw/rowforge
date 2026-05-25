# Part 2 — Model

This part defines the entities Studio exposes to its UI. Every entity is a
**projection** of on-disk artifacts; Studio never invents fields the CLI
cannot reproduce.

For CLI-side entities (Execution, Attempt, HandlerInstance, RowResolution,
etc.) see [`../cli/part-2-model.md`](../cli/part-2-model.md). This part
references but does not duplicate those.

## 2.1 Entity inventory

| Entity | Source | Purpose | Cost class |
|---|---|---|---|
| `Workspace` | sqlite path + filesystem root | Open / identify a workspace | hot |
| `ExecSummary` | `executions` row + latest attempt `meta.json` | List rows | warm |
| `ExecDetail` | `executions` row + all `attempts` rows + current handler instance | Detail header | warm |
| `AttemptDetail` | `attempts` row + `meta.json` + handler instance | Attempt page | warm |
| `ExecRollup` | streamed fold of all attempts | Cross-attempt resolution counts | cold |
| `FailedRowPage` | paged scan of `outcomes.jsonl` | Failed-row browser | cold |
| `RowHistory` | per-row fold across all attempts | "What happened to row N?" | cold, on demand |
| `RunHandle` | in-memory `SessionRegistry` | Refer to a live run from UI | hot |
| `ProgressEvent` | broadcast channel | Live progress | hot |
| `Settings` | settings file | User preferences | hot |

"Cost class" controls caching (Part 4 §4.3).

## 2.2 Projection types

Concrete fields are normative for v1.

### 2.2.1 `Workspace`
```rust
struct Workspace {
    root: PathBuf,
    schema_version: u8,
}
```

### 2.2.2 `ExecSummary`
```rust
struct ExecSummary {
    id: ExecutionId,
    name: String,
    created_at: DateTime<Utc>,
    input_rows: Option<u64>,             // None if input not yet snapshotted
    attempts_count: u32,
    last_attempt_state: Option<AttemptState>,
    last_attempt_counts: Option<AttemptCounts>,   // success/failed/crashed of last attempt only
    last_handler_dir: Option<PathBuf>,             // Plan 6: handler dir from most recent run; powers RunButton default
}
```
- `last_attempt_counts` is NOT a rollup across attempts. The rollup is
  `ExecRollup` and is cold-computed because it requires scanning all
  attempts.

### 2.2.3 `ExecDetail`
```rust
struct ExecDetail {
    summary: ExecSummary,
    input_path_snapshot: PathBuf,
    input_format: InputFormat,          // Csv / Jsonl / Ndjson
    handler_binding: HandlerBindingView,
    attempts: Vec<AttemptSummary>,      // chronological
    field_mapping: Option<FieldMapping>,
    config_overrides: BTreeMap<String, JsonValue>,
}
```

### 2.2.4 `AttemptDetail`
```rust
struct AttemptDetail {
    id: AttemptId,
    execution_id: ExecutionId,
    state: AttemptState,
    run_type: RunType,                  // see cli part-2 §2.4
    started_at: DateTime<Utc>,
    finished_at: Option<DateTime<Utc>>,
    stats: AttemptStats,                // success/failed/crashed/skipped/avg_dur_ms
    by_error_code: BTreeMap<String, u64>,   // bounded; "OTHER" overflow at 32
    handler_instance: HandlerInstanceView,
    paths: AttemptPaths,                // outcomes.jsonl, meta.json, stderr.log
}
```

### 2.2.5 `ExecRollup`
```rust
struct ExecRollup {
    resolved: u64,
    failed_last: u64,
    crashed_last: u64,
    too_large: u64,
    never_attempted: u64,
    by_error_code: BTreeMap<String, u64>,
}
```
Computed by folding all attempts' outcomes through `compute_resolution`
(see `rowforge-core`). Cold — never cached past the lifetime of one UI
panel render. Cost is linear in total outcomes across attempts. See
Part 4 §4.4 for the sidecar-index plan that bounds this.

### 2.2.6 `FailedRowPage`
```rust
struct FailedPageQuery {
    execution_id: ExecutionId,
    attempt_id: AttemptId,
    offset: u64,
    limit: u32,                         // capped at 500
    error_code_filter: Option<String>,  // v1: None only; v2: optional
}
struct FailedRowPage {
    rows: Vec<FailedRow>,
    next_offset: Option<u64>,
    total_known: Option<u64>,           // populated only if cheap (index present)
}
struct FailedRow {
    seq: u64,
    row_index: u64,
    kind: RowOutcomeKind,               // Error / Crash / TooLarge
    error_code: Option<String>,
    message: Option<String>,
    raw_record: JsonValue,
    dur_ms: u32,
}
```
v1 implements with linear scan from `offset`; v2 layers the index from
Part 4 §4.4.

### 2.2.7 `RowHistory`
```rust
struct RowHistory {
    seq: u64,
    rows: Vec<(AttemptId, RowOutcomeKind, Option<String>)>,
    resolved_at: Option<AttemptId>,     // first attempt that produced Success
}
```
On-demand only; opened when the user clicks a specific row.

### 2.2.8 `RunHandle`, `ProgressEvent`, `RunStatus`
See Part 3 §3.3 for state machine; Part 6 §6.1 for full event taxonomy.
The summary view for this section:
```rust
struct RunHandle(String);                // opaque, serializable, IPC-safe
enum RunStatus { Pending, Starting, Running, Cancelling, Done, Aborted, Crashed }
```

### 2.2.9 `Settings`
```rust
struct Settings {
    schema_version: u8,                  // = 1 in v1
    workspace_root: Option<PathBuf>,
    default_workers: Option<u32>,
    max_concurrent_runs: Option<u32>,    // default 3
    telemetry_opt_in: bool,              // default false; not collected in v1
}
```
Type lives in `studio-core::settings`; load/save path resolution lives in
the Tauri layer (uses Tauri's `app_data_dir`).

## 2.3 What is deliberately not an entity

- **HandlerInstance as a top-level peer.** Treated as a property of the
  attempt. There is no "list all handler instances" surface.
- **Per-row × per-attempt matrix.** For a 1M-row × 5-attempt exec this is
  5M cells, almost all `NeverAttempted` in later attempts. `RowHistory`
  fetches the sparse history on demand instead.
- **Raw `outcomes.jsonl` path escape hatch.** UI cannot bypass projections
  to read the file directly through `studio-core`. If you need the path,
  it is in `AttemptDetail::paths` for a "Reveal in Finder" affordance —
  not for in-process parsing.

## 2.4 Projection contract

- Every projection is `serde::Serialize`.
- Every projection has `#[non_exhaustive]` so future fields are
  non-breaking.
- Every projection is computable from disk artifacts and the SQLite
  registry — no hidden state in memory.
- Projections do not expose error types other than `UiError` (see Part 5
  §5.3).
