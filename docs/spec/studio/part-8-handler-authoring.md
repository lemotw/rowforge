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

`rowforge-core::Manifest` gains two fields. CLI and Studio share the
type. The change is co-released with v1.

```rust
struct Manifest {
    // ... existing fields ...
    build: Option<String>,    // e.g. "go build -o bin/handler ./..."
    run:   String,            // e.g. "bin/handler"   (relative to handler dir)
}
```

Semantics:

- `build` is optional. When present, both CLI and Studio invoke it via
  the OS shell with `cwd = <handler_dir>` before spawning `run`.
- `run` is required. Same `cwd` semantics. Studio resolves and spawns
  this for both smoke-test and the CLI-shared exec-run path.
- Both commands are split via shell-words. `PATH` lookup applies to the
  first token. Triggers `UiError::ToolchainMissing` if the first token
  resolves to nothing.
- Adding `build` is forward-compatible (`Option`, default `None`; Part 4
  §4.6 tolerant reader). Adding `run` as required is a breaking
  manifest change — the migration writes `run` for every CLI fixture
  in the same v1 release.

Validation path: `validate_manifest` (Part 5 §5.4) is extended to
verify both fields parse and the first token of each resolves in
`PATH`. PATH resolution failure becomes a `ManifestWarning`, not an
error (the command may still run on machines with a different `PATH`).

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
    last_build: Option<BuildRecord>,        // in-memory; see §8.4.7
    has_fixtures_dir: bool,                 // anchor for v1.1 (§8.9 Q1)
}

struct SourceFileSummary {
    name: String,
    size_bytes: u64,
    is_directory: bool,
}

struct BuildRecord {
    started_at: DateTime<Utc>,
    finished_at: DateTime<Utc>,
    exit_code: i32,
    command: String,                       // copy of manifest.build at run
    stderr_tail: String,                   // ≤ 64 KiB
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
- `BuildRecord`, `SmokeTestReport`: **hot** in-memory; not persisted
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
                  ↘ BuildCancelled
```

- `Building`: subprocess running; stderr streams as
  `BuildEvent::StderrLine` on `handler:build:<name>` (§8.5.2).
- Terminal states write into in-memory `BuildRecord`, kept per handler
  until Studio restart.
- Cancel: 3-second soft threshold (vs Part 3 §3.5's 10 s for exec-run;
  build is cheap and repeatable). Force-kill below that threshold is
  available without the typed-token friction Part 7 §7.6.3 requires
  for exec-run hard cancel.

### 8.4.3 Smoke-test lifecycle

```
Pending → (Building →) Handshaking → Running → Done
                                             ↘ Aborted
                                             ↘ TimedOut
                                             ↘ BuildFailed
```

Pipeline:

1. If `manifest.build` is present and `args.skip_build = false`, run
   build first. On build failure: `BuildFailed`, outcomes empty,
   `build_failed = true`. Stop.
2. Spawn `manifest.run` with `cwd = handler_dir`.
3. Standard rowforge handshake. On failure: `Aborted { reason:
   HandshakeFailed }`.
4. For each row in `args.rows`, write one JSON-Lines payload to stdin,
   await one outcome from stdout. Total wall-clock capped by
   `timeout_secs`.
5. After last row, send EOF, wait ≤ 2 s for graceful exit. Else
   force-kill, report `TimedOut`.

Cancel: 3 s soft, then hard kill.

### 8.4.4 Concurrency

| Limit | Default | Surfaced as |
|---|---|---|
| Build on the same handler | 1 | `HandlerBusy { reason: BuildInFlight }` |
| Smoke test on the same handler | 1 | `HandlerBusy { reason: SmokeInFlight }` |
| Build + smoke on same handler | mutually exclusive | smoke auto-builds; concurrent build refused |
| Smoke tests across workspace | 2 | `HandlerBusy { reason: WorkspaceLimit }` |
| Build/smoke during an exec run on same handler | refused | `HandlerBusy { reason: ExecRunInFlight }` |

### 8.4.5 Interlock with exec-runs

While an exec run holds a handler in flight (Part 3), Studio refuses
build / smoke on the same handler name to avoid rewriting the binary
mid-run. Symmetrically, the Run launcher (Part 7 §7.3) refuses to
start an exec run on a handler with an active build / smoke. The
interlock lives in `SessionRegistry` (Part 5 §5.2 commentary) and is
the source of truth for both directions.

### 8.4.6 Scaffold templates

v1 ships only templates that match the existing example handlers
(`examples/handlers/`):

- `GoStdio` — single-row stdio handler. Mirrors
  `golang-apple-refund`.
- `GoBatch` — batch handler. Mirrors `golang-billing-channel`.
- `Empty` — bare `manifest.json` + empty source dir.

Scaffolds write to `<workspace>/handlers/<name>/`. Existing dir →
`UiError::HandlerScaffoldConflict { name }`.

Templates are baked into the Studio binary in v1. Future template
sources (registry, URL) are not designed and not anchored — that
deserves its own brainstorm.

### 8.4.7 Cleanup at shutdown

On Studio quit (Part 3 §3.6):

1. Active build / smoke subprocesses soft-cancelled with 1-second
   deadline, then hard-killed.
2. In-memory `BuildRecord` / `SmokeTestReport` are discarded. UI must
   not show a stale "last build" after restart.

## 8.5 API

> **Plan 7 shipped.** All items in §8.5.1–§8.5.3 are landed. Landed file
> paths:
>
> - `crates/rowforge-studio-core/src/handler.rs` — module home
>   (`handler_list`, `handler_show`, `handler_open_editor`, `handler_reveal`,
>   `handler_scaffold`, `handler_delete`, `handler_rename`, `resolve_editor`)
> - `crates/rowforge-studio-core/src/handler_templates/` — embedded scaffold
>   templates (GoStdio, GoBatch, Empty)
> - `crates/rowforge-studio-core/src/error.rs` — `UiError` variants incl.
>   Plan 7 additions
> - `apps/rowforge-studio/src-tauri/src/commands.rs` — Tauri command shells
>   for all 7 new commands
> - `apps/rowforge-studio/src/ipc/types.ts` — TypeScript mirrors
> - `apps/rowforge-studio/src/ipc/use-handlers.ts` — TanStack Query hooks
> - `apps/rowforge-studio/src/pages/HandlersPage.tsx`
> - `apps/rowforge-studio/src/pages/HandlerDetailPage.tsx`
> - `apps/rowforge-studio/src/components/ScaffoldDialog.tsx`
> - `apps/rowforge-studio/src/components/RenameHandlerDialog.tsx`
> - `apps/rowforge-studio/src/components/DeleteHandlerDialog.tsx`

### 8.5.1 `StudioCore` additions

```rust
impl StudioCore {
    pub fn handler_list(&self) -> Result<Vec<HandlerSummary>, UiError>;
    pub fn handler_show(&self, name: &str) -> Result<HandlerDetail, UiError>;
    pub fn handler_open_editor(&self, name: &str) -> Result<(), UiError>;
    pub fn handler_reveal(&self, name: &str) -> Result<(), UiError>;

    pub fn handler_build(&self, name: &str) -> Result<BuildHandle, UiError>;
    pub fn handler_smoke_test(&self, args: SmokeTestArgs)
        -> Result<SmokeTestHandle, UiError>;
    pub fn handler_cancel_build(&self, h: &BuildHandle, mode: CancelMode)
        -> Result<(), UiError>;
    pub fn handler_cancel_smoke(&self, h: &SmokeTestHandle, mode: CancelMode)
        -> Result<(), UiError>;
    pub fn handler_subscribe_build(&self, h: &BuildHandle)
        -> Result<BuildStream, UiError>;
    pub fn handler_subscribe_smoke(&self, h: &SmokeTestHandle)
        -> Result<SmokeStream, UiError>;

    pub fn handler_scaffold(&self, args: ScaffoldArgs) -> Result<String, UiError>;
    pub fn handler_delete(&self, name: &str) -> Result<(), UiError>;
    pub fn handler_rename(&self, old: &str, new: &str) -> Result<(), UiError>;
}
```

`BuildHandle` and `SmokeTestHandle` are opaque IDs analogous to
`RunHandle` (Part 5 §5.2). Two separate handle types so the type
system rules out crossed cancels.

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

```
handler_list()                              -> Vec<HandlerSummary>
handler_show(name)                          -> HandlerDetail
handler_open_editor(name)                   -> ()
handler_reveal(name)                        -> ()
handler_build(name)                         -> BuildHandle
handler_smoke_test(args)                    -> SmokeTestHandle
handler_cancel_build(handle, mode)          -> ()
handler_cancel_smoke(handle, mode)          -> ()
handler_scaffold(args)                      -> String
handler_delete(name)                        -> ()
handler_rename(old, new)                    -> ()
```

### 8.5.4 New `UiError` variants

Extending Part 5 §5.3:

```rust
EditorNotFound,
HandlerBusy { name: String, reason: HandlerBusyReason },
HandlerScaffoldConflict { name: String },
ToolchainMissing { cmd: String, expected_for: String },  // e.g. cmd="go" for "build"
SmokeRowsTooMany { limit: u32 },                          // > 100 in v1

enum HandlerBusyReason {
    BuildInFlight,
    SmokeInFlight,
    ExecRunInFlight,
    WorkspaceLimit,
}
```

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
    smoke_test_default_timeout_secs: Option<u32>,  // default 30             [deferred]
}
```

> **Implementer correction (Plan 7):** `preferred_editor` was added without
> bumping `schema_version`. The original design above specified a bump from 1
> to 2; Plan 7 instead landed it as a tolerant-reader addition at schema
> version 1. The authoritative description is in Part 2 §2.2.9.
> `smoke_test_default_timeout_secs` is deferred; not shipped in Plan 7.

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
5. **No persistence of `BuildRecord` / `SmokeTestReport` across
   restarts.** v1 in-memory; UI must not display stale "last build"
   after a Studio relaunch.

## 8.9 Open questions

1. **Fixture-file / from-exec smoke inputs.** v1 paste-only; paste-100
   ceiling will frustrate users with large fixtures. v1.1 candidate
   (§8.1 deferred).
2. **In-Studio diff viewer.** After external editor save, would
   "what changed since last build?" help, or is the editor's own diff
   enough?
3. **Smoke-test history on disk.** Persist last N reports per handler
   so a restart does not erase debugging context.
4. **`BuildRecord` on disk.** Same question for builds. Tied to Q3.
5. **`rowforge pack` from Studio.** Currently CLI-only.
6. **Manifest write-back / structured editor.** Original
   `ManifestSource::Draft` anchor (Part 5 §5.4) was for this. Needs a
   real editor surface before it pays off.
