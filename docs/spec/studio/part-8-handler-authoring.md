# Part 8 — Handler Authoring

Defines the handler management panel: how users discover, edit, scaffold,
build, smoke-test, and delete handler programs from within Studio.

This part **supersedes** the "anchored, implemented later" position on
handler authoring previously taken by Part 1 §1.4 and Part 5 §5.4. v1
now ships handler authoring at the scope below.

Cross-references inline; consolidated table in §8.7.

## 8.1 Purpose and scope

In v1, Studio covers two user goals (Part 1 §1.1):

1. Manage executions — Parts 2–7.
2. **Manage handler implementation — this part.**

### In v1
- Discover handlers from `<workspace>/handlers/*` (single source).
- List, view, scaffold, delete, rename handler directories.
- Edit via external editor (Studio launches; no in-app code editor).
- Reveal in Finder / Explorer.
- Manifest validation surfaced first-class.
- Build via `manifest.build` command.
- Smoke test with user-pasted input rows (≤ 100).

### Deferred
- In-Studio code editor (Monaco / CodeMirror): explicit non-goal; external
  editor is the v1 contract.
- Fixture-file and from-exec smoke-test inputs (§8.9 Q1).
- `rowforge pack` from Studio (§8.9 Q5).
- Structured manifest editor (writes back to disk; §8.9 Q6).
- Cross-workspace handler registry.

## 8.2 Manifest extension

`rowforge-core::Manifest::entry` carries two fields that drive execution
and build. CLI and Studio share the type. The shape was established in
prior plans; Plan 8 makes `entry.build` actually execute.

```rust
struct Entry {
    cmd:   Vec<String>,              // e.g. ["./handler"]  or  ["python3", "handler.py"]
    build: Option<Vec<String>>,      // e.g. ["go", "build", "-o", "handler", "./..."]
    // ...other entry fields...
}
```

Semantics:

- `entry.build` is optional. When present, CLI and Studio run it via
  `std::process::Command` with `cwd = <handler_dir>` before spawning
  `entry.cmd`.
- `entry.cmd` is required. Same `cwd` semantics.
- `PATH` lookup applies to the first token of each field. Triggers
  `UiError::ToolchainMissing` if the first token of `entry.build`
  resolves to nothing.
- No shell interpolation — tokens are passed directly to `exec`; no
  quoting or glob expansion.

Validation path: `validate_manifest` (in `rowforge-studio-core`, per
Plan 7 detail) is extended with two new `ManifestWarning` variants
(§8.4.2). PATH resolution failure is a warning, not an error, because
the command may run on machines with a different `PATH`.

## 8.3 Model

Projections live in `studio-core`. All carry `#[non_exhaustive]` per
Part 5 §5.7.

```rust
struct HandlerSummary {
    name: String,                       // dir name under handlers/
    path: PathBuf,
    manifest_status: ManifestStatus,    // Valid | Invalid | Missing
    last_modified: DateTime<Utc>,       // max(mtime) over handler dir
    version: Option<String>,            // manifest.version
    language: Option<String>,           // manifest.language (display only)
}

enum ManifestStatus { Valid, Invalid, Missing }

struct HandlerDetail {
    summary: HandlerSummary,
    manifest: Option<Manifest>,
    manifest_errors: Vec<ManifestError>,
    manifest_warnings: Vec<ManifestWarning>,
    source_files: Vec<SourceFileSummary>,  // top-level only
    last_build: Option<BuildOutcome>,       // in-memory; see §8.4.8
    has_fixtures_dir: bool,                 // anchor for v1.1 (§8.9 Q1)
}

struct SourceFileSummary {
    name: String,
    size_bytes: u64,
    is_directory: bool,
}

/// Plan 8: lives in rowforge-core::build; re-exported by studio-core.
struct BuildOutcome {
    started_at: DateTime<Utc>,
    finished_at: DateTime<Utc>,
    exit_code: i32,
    command: Vec<String>,                  // copy of entry.build at run time
    stdout: String,                        // full stdout captured
    stderr: String,                        // full stderr captured
}

struct SmokeTestArgs {
    handler_name: String,
    rows: Vec<JsonValue>,                  // user-pasted; v1 cap = 100
    timeout_secs: u32,                     // default 30, max 300
    skip_build: bool,                      // default false
}

struct SmokeTestReport {
    started_at: DateTime<Utc>,
    finished_at: DateTime<Utc>,
    build_skipped: bool,
    build_failed: bool,                    // outcomes empty if true
    outcomes: Vec<RowOutcome>,             // len == args.rows.len()
    stderr_tail: String,                   // ≤ 64 KiB
    handler_version: Option<String>,       // from handshake
}

struct ScaffoldArgs {
    name: String,                          // ^[a-z0-9][a-z0-9-]*$
    template: ScaffoldTemplate,
    primary_field: String,                 // ^[a-zA-Z_][a-zA-Z0-9_]*$ — input column the example expects
}
enum ScaffoldTemplate { GoStdio, GoBatch, Empty }
```

**Scaffold field validation:**
- `name` must match `^[a-z0-9][a-z0-9-]*$` — enforced server-side by
  `handler_scaffold` and `handler_rename`; emits `InvalidHandlerName`
  on failure.
- `primary_field` must be a valid identifier: `^[a-zA-Z_][a-zA-Z0-9_]*$`
  (letters, digits, underscores; cannot start with a digit) — enforced
  server-side by `handler_scaffold`; emits `InvalidArg` on failure.
  This constraint prevents YAML/Go injection in scaffolded files.

Cost classes (Part 2 §2.1):

- `HandlerSummary` list: **warm** (dir scan + per-manifest read; cached
  with mtime probe identical to `ExecSummary`).
- `HandlerDetail`: **warm**.
- `BuildOutcome`, `SmokeTestReport`: **hot** in-memory; not persisted
  across Studio restarts (§8.9 Q3/Q4).

## 8.4 Runtime

### 8.4.1 Edit launcher

`handler_open_editor(name)` resolves an external editor in order:

1. `Settings.preferred_editor` (§8.6.4).
2. `$VISUAL`, then `$EDITOR`.
3. Probe `code`, `cursor`, `nvim`, `vim`, `nano` in `PATH`.
4. Fail with `UiError::EditorNotFound`.

The chosen command is spawned detached with the handler dir as the sole
argument. Studio does not wait for the editor process and does not
track its lifecycle.

`handler_reveal(name)` uses Tauri `shell::open(handler_dir)` and lets
the OS file manager handle it.

### 8.4.2 Build lifecycle

```
Pending → Building → BuildSucceeded
                  ↘ BuildFailed
```

Build is synchronous from the caller's perspective. The CLI runs on
its main thread; Studio's Tauri command is `async` but currently does
not use `spawn_blocking` — the runtime is blocked for the duration of
a build. (Refactor flagged for later; typical builds complete in
seconds.)

No mid-flight cancel in v1. Full `stdout` + `stderr` captured and
returned in `BuildOutcome`.

`needs_build` (caller-side staleness check, used by CLI):
- Returns `false` when `entry.build` is `None`.
- Returns `false` when `entry.cmd[0]` is an absolute path OR a
  PATH-resolvable bare name (interpreter case: no binary concept).
- Otherwise treats `entry.cmd[0]` as a relative binary in `handler_dir`.
  Returns `true` when the binary is missing OR when the max source
  mtime (`.go .rs .py .js .ts .mjs .java .c .cpp .h .hpp`, top-level
  only) exceeds binary mtime.

CLI `exec run` honors `needs_build` before spawning workers; build
failure exits the CLI with code 2. CLI `handler build` subcommand exits
with the count of failures (capped at 125). Studio always forces (no
staleness check) on Build button click.

Terminal states write into an in-memory `BuildOutcome` cache
(`StudioCore.build_cache: Mutex<HashMap<String, BuildOutcome>>`),
kept per handler until Studio restart.

**Validator warnings** (`validate_manifest` in `rowforge-studio-core`):
- `BuildToolNotInPath { tool }` — first token of `entry.build` not
  found in `PATH`.
- `CmdTargetMissing { path }` — first token of `entry.cmd` is a
  relative path that doesn't exist on disk. Suppressed when
  `entry.build` is `Some` (the build step is expected to produce it).

### 8.4.3 Smoke test

> **Implemented in Plan 13.** The deferred lifecycle state machine from
> Plan 8 has been superseded by the simpler synchronous implementation
> described below. See Part 7 §7.3 for UI placement and Part 5 §5.5 for
> the Tauri command signatures.

Studio surfaces a Smoke test section on each handler's detail page. The user
can paste JSON lines, pick a fixtures file, or dispatch one synthetic row,
and observe outcomes inline without creating an execution.

- Bounded to 1–100 rows per smoke run
- Forced row mode (batch handlers still receive rows one at a time)
- Reuses the Plan 8 build gate — rebuilds when `needs_build` is true
- Refuses when an exec attempt is already running against this handler
  (cross-process sqlite gate via
  `ExecutionStore::has_active_attempt_for_handler_dir`)
- Per-process serialization via an internal `tokio::sync::Mutex` —
  one smoke at a time per Studio process

Outcomes are ephemeral (not persisted to `outcomes.jsonl`). stderr is
captured as a 4 KiB tail and surfaced in a collapsible details panel.

Per-row timeout: `smoke_timeout_per_row_secs` from `Settings` (default 30 s;
0 means no timeout / capped at 1 hour). The smoke runner uses
`rowforge_core::worker::Worker` directly in row mode with a single worker.

API: see Part 5 §5.2 `handler_smoke_run` and `handler_smoke_load_fixtures`.

### 8.4.4 Concurrency

| Limit | Default | Surfaced as |
|---|---|---|
| Build on the same handler | 1 | `HandlerBusy { reason: BuildInFlight }` |
| Smoke test on the same handler | 1 | `HandlerBusy { reason: SmokeInFlight }` |
| Build + smoke on same handler | mutually exclusive | smoke auto-builds; concurrent build refused |
| Smoke tests across workspace | 2 | `HandlerBusy { reason: WorkspaceLimit }` |
| Build/smoke during an exec run on same handler | refused | `HandlerBusy { reason: ExecRunInFlight }` |

### 8.4.5 Interlock with exec-runs

> **Deferred from Plan 8** — see design doc §10. Smoke test and
> exec-run interlock will land in a later plan.

While an exec run holds a handler in flight (Part 3), Studio refuses
build / smoke on the same handler name to avoid rewriting the binary
mid-run. Symmetrically, the Run launcher (Part 7 §7.3) refuses to
start an exec run on a handler with an active build / smoke. The
interlock lives in `SessionRegistry` (Part 5 §5.2 commentary) and is
the source of truth for both directions.

### 8.4.6 Scaffold sources

The "New Handler" wizard (`ScaffoldDialog`) offers four sources. The first
three are templates baked into the Studio binary; the fourth (added in
Plan 12) imports an existing local folder.

#### Templates

v1 ships only templates that match the existing example handlers
(`examples/handlers/`):

- `GoStdio` — single-row stdio handler. Mirrors `golang-apple-refund`.
- `GoBatch` — batch handler. Mirrors `golang-billing-channel`.
- `Empty` — bare `manifest.json` + empty source dir.

Scaffolds write to `<workspace>/handlers/<name>/`. Existing dir →
`UiError::HandlerScaffoldConflict { name }`.

Templates are baked into the Studio binary in v1. Future template
sources (registry, URL) are not designed and not anchored — that
deserves its own brainstorm.

#### Import from folder (Plan 12)

The fourth radio option — **"Import from folder…"** — lets a user pick any
local directory that already contains a `rowforge.yaml` and copy it
verbatim into `<workspace>/handlers/<name>/`.

- The source folder **must** contain `rowforge.yaml`; the backend rejects
  folders without it via `UiError::InvalidArg`. Pure-source folders
  without a manifest should go through Scaffold + manual paste instead.
- Copy semantics: **no filter** — `.git/`, `node_modules/`, build
  outputs, and any other files all come along. This matches
  `copy_dir_recursive`'s behavior (§8.5.1).
- Symlinks in the source are **skipped** (not preserved, not followed); a
  `tracing::warn` is emitted for each skipped entry.
- When the import succeeds, Studio emits `handlers:list` and navigates to
  the new handler's detail page.
- UI: selecting "Import from folder…" hides the `primary_field` input and
  shows a "Pick folder…" button that opens Tauri's native OS file dialog.
  The name field remains visible and required.

### 8.4.7 Handler fork (Plan 12)

Fork duplicates an existing workspace handler under a new name. It uses
the same `copy_dir_recursive` helper as import (§8.5.1) so the copy
semantics are identical — no filter, symlinks skipped with warning.

After copying, the fork rewrites the manifest's `name:` field via a
**serde round-trip** (load YAML → update `manifest.name` → serialize back
to disk). This is best-effort:

- If the manifest fails to load, the rewrite is skipped with a
  `tracing::warn` and the fork still succeeds (the caller gets a handler
  directory that is otherwise a faithful copy).
- **YAML comments are not preserved.** The serde round-trip drops all
  `# comment` lines.
- **Key ordering may change.** serde_yaml serializes keys in struct
  declaration order, not the original file order.

These are documented limitations, not bugs. Users who care about a
hand-formatted `rowforge.yaml` should edit the fork's manifest manually
after forking.

**UI:** A **Fork…** button appears in the `HandlerDetailPage` header,
between **Rename…** and **Delete…**. Clicking it opens `ForkHandlerDialog`,
which pre-fills the name field with `<source_name>-fork`. On success, Studio
navigates to `/handlers/<new_name>`.

**`StudioCore` addition (Plan 12):**

```rust
impl StudioCore {
    pub fn handler_import_from_folder(
        &self,
        source_path: &Path,
        name: &str,
    ) -> Result<(), UiError>;

    pub fn handler_fork(
        &self,
        source_name: &str,
        new_name: &str,
    ) -> Result<(), UiError>;
}
```

**`copy_dir_recursive` helper (Plan 12):**

```rust
// rowforge-studio-core::handler (pub(crate))
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), UiError>;
```

Walks `src` with `walkdir`. Creates `dst` and all intermediate
directories. Copies regular files only; skips non-regular entries
(symlinks, sockets, devices) with `tracing::warn`. No filter — every
regular file is copied regardless of name.

### 8.4.8 Cleanup at shutdown

On Studio quit (Part 3 §3.6):

1. Active build / smoke subprocesses soft-cancelled with 1-second
   deadline, then hard-killed.
2. In-memory `BuildOutcome` / `SmokeTestReport` are discarded. UI must
   not show a stale "last build" after restart.

## 8.5 API

> **Plan 7 shipped.** All items in §8.5.1–§8.5.3 are landed. Landed file
> paths:
>
> - `crates/rowforge-studio-core/src/handler.rs` — module home
>   (`handler_list`, `handler_show`, `handler_open_editor`, `handler_reveal`,
>   `handler_scaffold`, `handler_delete`, `handler_rename`, `resolve_editor`,
>   `copy_dir_recursive` [Plan 12], `handler_import_from_folder` [Plan 12],
>   `handler_fork` [Plan 12])
> - `crates/rowforge-studio-core/src/smoke.rs` — smoke runner
>   (`handler_smoke_run`, `handler_smoke_load_fixtures`) [Plan 13]
> - `crates/rowforge-studio-core/src/handler_templates/` — embedded scaffold
>   templates (GoStdio, GoBatch, Empty)
> - `crates/rowforge-studio-core/src/error.rs` — `UiError` variants incl.
>   Plan 7 additions + `HandlerBusy` [Plan 13]
> - `apps/rowforge-studio/src-tauri/src/commands.rs` — Tauri command shells
>   for all 7 Plan 7 commands + `handler_import_from_folder` + `handler_fork`
>   [Plan 12] + `handler_smoke_run` + `handler_smoke_load_fixtures` [Plan 13]
> - `apps/rowforge-studio/src/ipc/types.ts` — TypeScript mirrors
> - `apps/rowforge-studio/src/ipc/use-handlers.ts` — TanStack Query hooks;
>   `useHandlerImportFromFolder` + `useHandlerFork` added in Plan 12;
>   `useHandlerSmokeRun` + `useHandlerSmokeLoadFixtures` added in Plan 13
> - `apps/rowforge-studio/src/pages/HandlersPage.tsx`
> - `apps/rowforge-studio/src/pages/HandlerDetailPage.tsx`
> - `apps/rowforge-studio/src/components/ScaffoldDialog.tsx` — gains 4th
>   radio "Import from folder…" in Plan 12
> - `apps/rowforge-studio/src/components/ForkHandlerDialog.tsx` — Plan 12
> - `apps/rowforge-studio/src/components/SmokeSection.tsx` — Plan 13
> - `apps/rowforge-studio/src/components/RenameHandlerDialog.tsx`
> - `apps/rowforge-studio/src/components/DeleteHandlerDialog.tsx`

### 8.5.1 `StudioCore` additions

```rust
impl StudioCore {
    pub fn handler_list(&self) -> Result<Vec<HandlerSummary>, UiError>;
    pub fn handler_show(&self, name: &str) -> Result<HandlerDetail, UiError>;
    pub fn handler_open_editor(&self, name: &str) -> Result<(), UiError>;
    pub fn handler_reveal(&self, name: &str) -> Result<(), UiError>;

    /// Plan 8: synchronous; caches outcome in build_cache for handler_show.
    pub fn handler_build(&self, name: &str) -> Result<BuildOutcome, UiError>;

    // Plan 13 — smoke test (see §8.4.3)
    pub async fn handler_smoke_run(&self, req: SmokeRunRequest)
        -> Result<SmokeRunResult, UiError>;
    pub fn handler_smoke_load_fixtures(&self, path: &Path, limit: usize)
        -> Result<Vec<Map<String, Value>>, UiError>;

    // Deferred to a later plan:
    pub fn handler_cancel_build(&self, h: &BuildHandle, mode: CancelMode)
        -> Result<(), UiError>;
    pub fn handler_subscribe_build(&self, h: &BuildHandle)
        -> Result<BuildStream, UiError>;

    pub fn handler_scaffold(&self, args: ScaffoldArgs) -> Result<String, UiError>;
    pub fn handler_delete(&self, name: &str) -> Result<(), UiError>;
    pub fn handler_rename(&self, old: &str, new: &str) -> Result<(), UiError>;

    // Plan 12 — import and fork (see §8.4.6–§8.4.7)
    // Full signatures documented in §8.4.7; repeated here for completeness.
    pub fn handler_import_from_folder(&self, source_path: &Path, name: &str)
        -> Result<(), UiError>;
    pub fn handler_fork(&self, source_name: &str, new_name: &str)
        -> Result<(), UiError>;
}
```

`StudioCore.build_cache: Mutex<HashMap<String, BuildOutcome>>` — in-memory
per-session store; `handler_show` injects the cached outcome into
`HandlerDetail.last_build`. Lost on Studio restart (§8.4.8).

`BuildHandle` and `SmokeTestHandle` are opaque IDs analogous to
`RunHandle` (Part 5 §5.2). Two separate handle types so the type
system rules out crossed cancels. (Used only by the deferred smoke-test
path.)

### 8.5.2 Events

```
handler:build:<name>          BuildEvent
handler:smoke:<name>          SmokeEvent
handlers:list                 ()                      // coarse refresh hint
```

```rust
enum BuildEvent {
    Started { command: String, at_ms: u64 },
    StderrLine { line: String, at_ms: u64 },
    Done { exit_code: i32, dur_ms: u32, stderr_tail: String },
    Cancelled,
}

enum SmokeEvent {
    BuildPhase(BuildEvent),
    Handshake { handler_version: Option<String>, dur_ms: u32 },
    Outcome { row_index: u32, outcome: RowOutcome },
    Done(SmokeTestReport),
    Aborted { reason: SmokeAbortReason },
    TimedOut { row_index: Option<u32>, elapsed_ms: u32 },
}

enum SmokeAbortReason {
    UserCancelled,
    HandshakeFailed { stderr_tail: String },
    HandlerCrashed { stderr_tail: String, signal: Option<i32> },
    BuildFailed,
    Internal { message: String },
}
```

Smoke test does **not** apply the 4 Hz / 20 Hz coalescing budgets from
Part 6 §6.2 because N ≤ 100. Every outcome is emitted.

`StderrLine` events apply a per-handler 20 lines / sec token bucket
(Part 6 §6.2 pattern) so a noisy build cannot saturate the broadcast
channel.

### 8.5.3 Tauri commands

> **Plan 7 shipped:** `handler_list`, `handler_show`, `handler_open_editor`,
> `handler_reveal`, `handler_scaffold`, `handler_delete`, `handler_rename`.
>
> **Plan 8 adds:** `handler_build`.
>
> **Plan 12 adds:** `handler_import_from_folder`, `handler_fork`.
>
> **Plan 13 adds:** `handler_smoke_run`, `handler_smoke_load_fixtures`.

```
handler_list()                              -> Vec<HandlerSummary>
handler_show(name)                          -> HandlerDetail
handler_open_editor(name)                   -> ()
handler_reveal(name)                        -> ()
handler_build(name: String)                 -> BuildOutcome     // Plan 8
handler_scaffold(args)                      -> String
handler_delete(name)                        -> ()
handler_rename(old, new)                    -> ()
handler_import_from_folder(source_path, name) -> ()            // Plan 12; emits handlers:list
handler_fork(source_name, new_name)           -> ()            // Plan 12; emits handlers:list
handler_smoke_run(request: SmokeRunRequest)   -> SmokeRunResult  // Plan 13; async; no events
handler_smoke_load_fixtures(path, limit)      -> Vec<Map>        // Plan 13; sync; limit 1..=100
```

`handler_build` side effect: emits `handlers:list` event after build
(success or failure) so `HandlerSummary.last_modified` picks up the
new binary mtime.

`handler_build` is declared `async` in Tauri but currently blocks the
async runtime during the build (no `spawn_blocking`). Known limitation;
typical builds complete in seconds. Refactor flagged for a later plan.

`handler_import_from_folder` and `handler_fork` both emit `handlers:list`
on success (coarse refresh hint, same pattern as scaffold/delete/rename).
No new `UiError` variants are introduced — they reuse `InvalidArg`,
`HandlerExists`, `HandlerNotFound`, and `InvalidHandlerName` (Part 5 §5.3).

### 8.5.4 New `UiError` variants

Extending Part 5 §5.3:

**Plan 7 variants:**

```rust
EditorNotFound,
HandlerBusy { name: String, reason: HandlerBusyReason },
HandlerScaffoldConflict { name: String },
ToolchainMissing { name: String, tool: String },  // Plan 8 reworked payload
SmokeRowsTooMany { limit: u32 },                   // > 100 in v1

enum HandlerBusyReason {
    BuildInFlight,
    SmokeInFlight,
    ExecRunInFlight,
    WorkspaceLimit,
}
```

**Plan 8 variants:**

```rust
/// Build subprocess exited non-zero.
BuildFailed { name: String, exit_code: i32 },

/// First token of entry.build not resolvable via `which`.
ToolchainMissing { name: String, tool: String },

/// Build attempted on a handler whose manifest has no entry.build.
NoBuildCommand { name: String },
```

Plan 8 variant details:

| Variant | Serialized `kind` | Payload | Emitted by | UI copy |
|---|---|---|---|---|
| `BuildFailed { name, exit_code }` | `build_failed` | `{ name, exit_code }` | `handler_build` when build exits non-zero | "Build failed for 'NAME' (exit N). See the Last build section for details." |
| `ToolchainMissing { name, tool }` | `toolchain_missing` | `{ name, tool }` | `handler_build` when `entry.build[0]` is missing from `PATH` | "Build tool 'TOOL' not found in PATH. Install it or update entry.build in your manifest." |
| `NoBuildCommand { name }` | `no_build_command` | `{ name }` | `handler_build` when manifest has no `entry.build` | "Handler 'NAME' has no entry.build command in rowforge.yaml." |

**Plan 13 variants:**

```rust
/// An exec attempt is currently active for this handler; smoke run refused.
HandlerBusy { name: String },
```

Plan 13 variant details:

| Variant | Serialized `kind` | Payload | Emitted by | UI copy |
|---|---|---|---|---|
| `HandlerBusy { name }` | `handler_busy` | `{ name }` | `handler_smoke_run` when `ExecutionStore::has_active_attempt_for_handler_dir` returns `true` | Inline error in SmokeSection: "Handler 'NAME' has an active run. Cancel the run first." |

All carry `#[non_exhaustive]` per Part 5 §5.7.

## 8.6 UI (extends Part 7)

> **Plan 7 shipped.** `/handlers` and `/handlers/:name` are active routes.
> See Part 7 §7.3 for the IA update and §7.4 Flows H–J for scaffold/rename/
> delete user flows.

Sidebar / shell from Part 7 §7.3 is otherwise unchanged. The
**Authoring** group is no longer disabled.

### 8.6.1 IA additions

- Sidebar `AUTHORING / ● Handlers` becomes active (Plan 7: shipped).
- Routes (Plan 7: all active):
  - `/handlers` — Handler list (`HandlersPage.tsx`).
  - `/handlers/:name` — Handler detail (`HandlerDetailPage.tsx`). Tabs: **Source** (file list),
    **Manifest** (validation report), **Smoke test**, **Build log**.
  - `/handlers/new` — Scaffold wizard (modal-as-route; `ScaffoldDialog.tsx`).
- Run launcher (Part 7 §7.3): `HandlerSource` picker becomes a dropdown
  populated from `handler_list()`. "Browse external folder…" remains
  as a fallback. Internally still constructs `HandlerSource::Dir`
  (Part 5 §5.4 anchor unchanged).

### 8.6.2 Primary flows

**Flow E — Edit existing handler**

| # | Step | Command |
|---|---|---|
| 1 | Sidebar → Handlers | `handler_list` |
| 2 | Row → `[Edit]` | `handler_open_editor(name)` |
| 3 | External editor opens; Studio shows toast | — |
| 4 | After saving, Smoke test tab → paste rows → `[Run smoke]` | `handler_smoke_test` |
| 5 | Subscribe `handler:smoke:<name>` | event |

**Flow F — New handler (scaffold)**

| # | Step | Command |
|---|---|---|
| 1 | Handlers → `[+ New handler]` | — |
| 2 | Wizard: name + template + primary field | — |
| 3 | Submit | `handler_scaffold` |
| 4 | Route to `/handlers/:name`; hint "Click Edit to start" | `handler_show` |

**Flow G — Build + smoke test**

| # | Step | Command |
|---|---|---|
| 1 | Detail → Smoke test tab → paste JSON rows | — |
| 2 | `[Run smoke]` | `handler_smoke_test` |
| 3 | UI: Build phase log → Handshake → per-row outcomes | events |
| 4 | Failure → stderr tail in right Sheet | — |

### 8.6.3 Boundary states (extending Part 7 §7.7)

| # | State | Trigger | Display |
|---|---|---|---|
| H1 | Empty `handlers/` | `handler_list → []` | Empty state + `[+ New handler]` + "Handlers live in `<workspace>/handlers/*`" |
| H2 | Missing manifest | dir lacks `manifest.json` | Row badge `⚠ no manifest`; Smoke / Build disabled |
| H3 | Manifest invalid | `manifest_errors` non-empty | Inline red marks; Manifest tab lists errors |
| H4 | `EditorNotFound` | All editors missing | Toast + "Set `$EDITOR` or install `code` CLI" + Reveal-in-Finder fallback |
| H5 | `HandlerBusy` | Build/smoke or exec-run lock | Inline disabled button + tooltip naming the lock |
| H6 | `ToolchainMissing` | First word of `manifest.build` absent in PATH | Modal naming the missing command + install hint |
| H7 | Smoke timeout | Exceeded `timeout_secs` | Banner + "Retry with longer timeout" |
| H8 | `HandlerScaffoldConflict` | Name exists | Wizard inline error; submit disabled |
| H9 | `SmokeRowsTooMany` | > 100 pasted rows | Inline error + count display |
| H10 | Editor opened, save unconfirmed | always | Soft hint "Saved your edits? Smoke test below" (non-blocking) |

### 8.6.4 Settings additions

Extending Part 2 §2.2.9 and Part 5 §5.6:

```rust
struct Settings {
    // ... existing
    preferred_editor: Option<String>,              // e.g. "code", "cursor"  [Plan 7: shipped]
    smoke_default_rows: usize,                     // default 5, clamped 1..=100  [Plan 13]
    smoke_timeout_per_row_secs: u64,               // default 30; 0 = no timeout  [Plan 13]
}
```

> **Implementer correction (Plan 7):** `preferred_editor` was added without
> bumping `schema_version`. The original design above specified a bump from 1
> to 2; Plan 7 instead landed it as a tolerant-reader addition at schema
> version 1. The authoritative description is in Part 2 §2.2.9.

> **Plan 13:** `smoke_default_rows` and `smoke_timeout_per_row_secs` are
> now shipped. They are threaded through `OpenOpts` into `StudioCore`
> atomic fields. Per-row timeout of 0 is interpreted as "no timeout"
> (capped internally at 1 hour). The `SmokeSection` UI reads
> `smoke_default_rows` to pre-fill the "Rows to run" input.

### 8.6.5 Wireframes (illustrative)

ASCII; same caveat as Part 7 §7.13.

#### W-H1 Handler list

```
┌──────────────────────────────────────────────────────────────────────────────────────────────┐
│  Handlers                                                                  [+ New handler]   │
├──────────────────────────────────────────────────────────────────────────────────────────────┤
│  ┌────────────────────────────────────────────────────────────────────────────────────────┐  │
│  │ Name                          Lang   Version   Manifest    Modified                    │  │
│  ├────────────────────────────────────────────────────────────────────────────────────────┤  │
│  │ golang-apple-refund           go     0.1.0     ✓ valid     2026-05-22 09:14   [Edit] ⏵│  │
│  │ golang-billing-channel        go     0.1.0     ✓ valid     2026-05-21 17:02   [Edit] ⏵│  │
│  │ golang-refund-backfill        go     0.1.0     ✓ valid     2026-05-21 11:30   [Edit] ⏵│  │
│  │ scratchpad                    go     —         ⚠ missing   2026-05-22 12:01   [Edit] ⏵│  │
│  └────────────────────────────────────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────────────────────────────────┘
```

#### W-H2 Handler detail — Smoke test tab

```
┌──────────────────────────────────────────────────────────────────────────────────────────────┐
│  Handlers / golang-billing-channel                          [Edit] [Reveal] [Delete]         │
├──────────────────────────────────────────────────────────────────────────────────────────────┤
│  ┌─Source──Manifest──Smoke test──Build log────────────────────────────────────────────────┐ │
│  │                                                                                         │ │
│  │  Input rows (paste JSON, one per line; max 100)                                         │ │
│  │  ┌─────────────────────────────────────────────────────────────────────────────────┐   │ │
│  │  │ {"billid":"b0001"}                                                              │   │ │
│  │  │ {"billid":"b0042"}                                                              │   │ │
│  │  │ {"billid":""}                                                                   │   │ │
│  │  └─────────────────────────────────────────────────────────────────────────────────┘   │ │
│  │  Timeout: [30] s         [ ] Skip build                                                 │ │
│  │                                                                            [ Run smoke ]│ │
│  │                                                                                         │ │
│  │  Last run · 2026-05-22 14:08 · 1.2 s                                                    │ │
│  │  ┌─Outcomes──────────────────────────────────────────────────────────────────────────┐ │ │
│  │  │ row 0   ● success   {"billid":"b0001","channel":"alipay"}                  142 ms │ │ │
│  │  │ row 1   ● error     BILLING_NOT_FOUND                                       11 ms │ │ │
│  │  │ row 2   ● error     MISSING_BILLID                                           2 ms │ │ │
│  │  └────────────────────────────────────────────────────────────────────────────────────┘│ │
│  │  stderr (tail) · [Open full log]                                                        │ │
│  └─────────────────────────────────────────────────────────────────────────────────────────┘│
└──────────────────────────────────────────────────────────────────────────────────────────────┘
```

## 8.7 Cross-references

| §8.x | Depends on |
|---|---|
| 8.1 scope | Part 1 §1.1, §1.4 (superseded); Part 5 §5.4 (anchors realized) |
| 8.2 manifest | Part 4 §4.6 schema versioning; Part 5 §5.4 |
| 8.3 model | Part 2 §2.1 cost classes; §2.4 projection contract |
| 8.4 runtime | Part 3 §3.5 cancel pattern (shorter threshold); §3.6 cleanup; §3.4 concurrency |
| 8.4.5 interlock | Part 5 §5.2 SessionRegistry |
| 8.4.3 smoke test | Part 5 §5.2 `handler_smoke_run` / `handler_smoke_load_fixtures` (Plan 13); Part 5 §5.3 `handler_busy`; Part 7 §7.3 SmokeSection |
| 8.4.6 scaffold sources | Part 5 §5.5 `handler_import_from_folder` (Plan 12) |
| 8.4.7 handler fork | Part 5 §5.5 `handler_fork` (Plan 12); Part 5 §5.3 error variants |
| 8.5 API | Part 5 §5.2, §5.3 errors, §5.5 commands, §5.7 stability |
| 8.5.2 events | Part 6 §6.1 taxonomy; §6.2 (notes why smoke test does not coalesce) |
| 8.6 UI | Part 7 §7.3 IA; §7.7 boundary states; §7.13 wireframe convention |
| 8.6.4 settings | Part 2 §2.2.9; Part 5 §5.6 |

## 8.8 Things the UI must NOT do (handler-specific)

Extending Part 7 §7.10:

1. **No in-Studio code editor.** External editor only (§8.4.1).
2. **No silent overwrite during scaffold.** Conflicts surface as
   `HandlerScaffoldConflict`.
3. **No build / smoke during an exec run on the same handler.**
   Interlock §8.4.5.
4. **No smoke-test event coalescing.** Every outcome must render
   (§8.5.2).
5. **No persistence of `BuildOutcome` / `SmokeTestReport` across
   restarts.** v1 in-memory; UI must not display stale "last build"
   after a Studio relaunch.
6. **No import without rowforge.yaml in source.** Pure-source folders
   must go through Scaffold + manual paste (§8.4.6).
7. **No implicit YAML comment preservation on fork.** The serde
   round-trip limitation (§8.4.7) must not be hidden from the user —
   if the UI ever surfaces a "manifest preview" for a fork, it must
   render the post-round-trip content, not the original.

## 8.9 Open questions

1. **Fixture-file / from-exec smoke inputs.** ~~v1 paste-only~~ **Resolved in
   Plan 13** — fixture file picking (.jsonl / .ndjson / .json / .csv /
   directory) is now shipped. The paste-100 row ceiling remains; larger
   fixtures are truncated to the `limit` (1..=100).
2. **In-Studio diff viewer.** After external editor save, would
   "what changed since last build?" help, or is the editor's own diff
   enough?
3. **Smoke-test history on disk.** Persist last N reports per handler
   so a restart does not erase debugging context.
4. **`BuildOutcome` on disk.** Same question for builds. Tied to Q3.
5. **`rowforge pack` from Studio.** Currently CLI-only.
6. **Manifest write-back / structured editor.** Original
   `ManifestSource::Draft` anchor (Part 5 §5.4) was for this. Needs a
   real editor surface before it pays off.
