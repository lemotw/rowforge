# Studio Plan 07 — Handler Authoring (Static Surface) design

> **Status:** brainstorm output for Plan 7. Builds on Plans 1-6. This is the *design* document; the implementation plan lives at `docs/superpowers/plans/2026-05-25-studio-plan-07-handler-authoring-static.md`.

## 1. Goal

Let users **manage handlers from inside Studio**:
- See the list of handlers in `<workspace>/handlers/`
- View a handler's manifest + source files
- Open the handler dir in an external editor
- Reveal in Finder / Explorer
- Scaffold a new handler from a template (3 templates)
- Rename a handler (filesystem move, lazy on attempt references)
- Delete a handler (with typed-token friction)

No build runtime, no smoke test runtime, no in-Studio code editor. Those are Plans 8 and 9.

## 2. Scope

### In scope
- `StudioCore` APIs: `handler_list`, `handler_show`, `handler_open_editor`, `handler_reveal`, `handler_scaffold`, `handler_delete`, `handler_rename`
- New `UiError` variants: `EditorNotFound`, `HandlerNotFound`, `HandlerExists`, `InvalidHandlerName`
- `Settings.preferred_editor: Option<String>` — re-added (legitimate this time; consumed by the editor resolver)
- 4-tier editor resolver per spec 8.4.1 (preferred_editor → `$VISUAL` → `$EDITOR` → probe `code`/`cursor`/`subl`/`zed`)
- 3 scaffold templates: `GoStdio`, `GoBatch`, `Empty`
- Tauri command surface (7 commands)
- React UI: Handlers list + detail pages, Scaffold modal, Rename dialog, Delete typed-token dialog
- Settings page Editor section (extends Plan 6 Settings)
- Sidebar Handlers link enabled (was anchored disabled in Plans 1-6)
- Spec docs (en + zh-Hant): part-2 (Settings + ExecSummary footnote on lazy rename), part-5 (UiError variants), part-7 (IA + flows), part-8 cross-refs

### Out of scope (Plans 8+ / future)
- **Build runtime** (`manifest.build` subprocess, stderr stream, cancel) → Plan 8
- **Smoke test runtime** (handshake + per-row dispatch, ≤ 100 rows) → Plan 9
- In-Studio code editor (Monaco / CodeMirror) — explicit non-goal per spec 8.1
- Fixture-file / from-exec smoke inputs (spec 8.9 Q1)
- `rowforge pack` from Studio (spec 8.9 Q5)
- Structured manifest editor that writes back to disk (spec 8.9 Q6)
- Cross-workspace handler registry

## 3. Decisions locked during brainstorm

| Question | Choice | Rationale |
|---|---|---|
| Plan 7 subsystem | A (Static surface) only | Each of A / B / C is a coherent 1-cycle unit; ship them as separate plans for reviewable PR scope |
| A scope | Read + Create + Destroy 全集 | All 7 verbs in one plan — leaving Destroy for later would force users back to the CLI for `rm -rf` |
| Editor resolution | 全 4-tier per spec 8.4.1 | `preferred_editor` is the legitimate Settings field (unlike `default_workers` which we just dropped) |
| Delete friction | Typed-token (handler name) | Matches spec part 7 §7.2 explicit-friction rule for destructive actions. Reuses `CancelDialog` typed-token pattern from Plan 4. |
| Rename + existing attempts | Lazy: change dir; don't touch `handler_instance.source_snapshot_dir` rows | `handler_instance` is content-addressed; the path field is informational. Touching it would require a sqlite migration for marginal benefit. Old attempts may surface old paths in artifact links — documented as expected. |

## 4. Backend design

### 4.1 New module `crates/rowforge-studio-core/src/handler.rs`

```rust
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandlerSummary {
    pub name: String,
    pub path: PathBuf,
    pub manifest_status: ManifestStatus,
    pub last_modified: chrono::DateTime<chrono::Utc>,
    pub version: Option<String>,
    pub language: Option<String>,
}

#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManifestStatus { Valid, Invalid, Missing }

#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandlerDetail {
    pub summary: HandlerSummary,
    /// Parsed manifest if status == Valid. None otherwise.
    pub manifest: Option<rowforge_core::manifest::Manifest>,
    pub manifest_errors: Vec<crate::manifest::ManifestError>,
    pub manifest_warnings: Vec<crate::manifest::ManifestWarning>,
    pub source_files: Vec<SourceFileSummary>,
    pub has_fixtures_dir: bool,
}

#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceFileSummary {
    pub name: String,
    pub size_bytes: u64,
    pub is_directory: bool,
}

#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScaffoldArgs {
    pub name: String,
    pub template: ScaffoldTemplate,
    /// Input column the example handler reads. e.g. "email", "order_id".
    pub primary_field: String,
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScaffoldTemplate { GoStdio, GoBatch, Empty }
```

### 4.2 `StudioCore` APIs

```rust
impl StudioCore {
    pub fn handler_list(&self) -> Result<Vec<HandlerSummary>, UiError>;
    pub fn handler_show(&self, name: &str) -> Result<HandlerDetail, UiError>;
    pub fn handler_open_editor(&self, name: &str) -> Result<(), UiError>;
    pub fn handler_reveal(&self, name: &str) -> Result<(), UiError>;
    pub fn handler_scaffold(&self, args: ScaffoldArgs) -> Result<String, UiError>;
    pub fn handler_delete(&self, name: &str) -> Result<(), UiError>;
    pub fn handler_rename(&self, old: &str, new: &str) -> Result<(), UiError>;
}
```

**Discovery (`handler_list`):**
- Scan `<workspace>/handlers/*` (single source per spec 8.1)
- For each subdirectory, attempt `crate::manifest::validate_manifest` (Plan 5)
- ManifestStatus: `Valid` if no errors; `Invalid` if errors but file present; `Missing` if no rowforge.yaml
- last_modified: max(mtime) over the dir's top-level entries

**Detail (`handler_show`):**
- HandlerSummary + manifest + errors/warnings (from Plan 5's ManifestReport)
- source_files: top-level entries only (size, is_dir). Excludes the parsed manifest itself (separate panel).
- has_fixtures_dir: `<handler_dir>/fixtures` exists — anchor for v1.1 smoke fixtures

**Editor (`handler_open_editor`):**
- 4-tier resolver:
  1. `Settings.preferred_editor` (split via `shell_words`)
  2. `$VISUAL` env var
  3. `$EDITOR` env var
  4. Probe in order: `code`, `cursor`, `subl`, `zed` (via `which::which`)
- All 4 fail → `UiError::EditorNotFound`
- Spawn detached: `Command::new(cmd0).args(rest).arg(handler_dir).spawn()` — don't wait, don't track process

**Reveal (`handler_reveal`):**
- Returns `handler_dir` path; Tauri command wraps with `tauri::api::shell::open(handler_dir.to_string_lossy())`
- Or studio-core itself spawns the OS file manager (platform-specific: `open` on macOS, `explorer` on Windows, `xdg-open` on Linux)

**Scaffold (`handler_scaffold`):**
- Validate `args.name` against `^[a-z0-9-]+$` regex; reject with `InvalidHandlerName` on miss
- Validate `<workspace>/handlers/<name>` doesn't exist; `HandlerExists` if it does
- Create dir + write template files (see §4.5)
- Return the canonical name (same as input, but normalized)

**Delete (`handler_delete`):**
- Validate `name` regex
- Canonicalize `<workspace>/handlers/<name>` and assert parent equals canonicalized `<workspace>/handlers`
- `fs::remove_dir_all` — propagates IO errors as `UiError::Io`
- Does NOT touch handler_instances table (lazy)

**Rename (`handler_rename`):**
- Validate both names regex
- Source must exist, destination must not (`HandlerNotFound` / `HandlerExists`)
- `fs::rename` — atomic on same filesystem
- Does NOT touch handler_instances (lazy decision per §3)

### 4.3 New `UiError` variants

Extending Part 5 §5.3:

```rust
EditorNotFound,
HandlerNotFound { name: String },
HandlerExists { name: String },
InvalidHandlerName { name: String },
```

These join Plan 5's `ManifestInvalid`, `ToolchainMissing` etc. All struct-payload variants get TS mirrors with proper types (Plan 6 review fix established this discipline).

### 4.4 `Settings.preferred_editor`

```rust
#[non_exhaustive]
pub struct Settings {
    pub schema_version: u8,
    pub workspace_root: Option<PathBuf>,
    pub max_concurrent_runs: Option<u32>,
    pub telemetry_opt_in: bool,
    /// Plan 7: argv command string for handler_open_editor; first
    /// token PATH-probed via `which`. Example: "code -w", "cursor --new-window".
    /// None → fall back to $VISUAL / $EDITOR / probes.
    pub preferred_editor: Option<String>,
}
```

Re-added after Plan 6 dropped `default_workers`. This one IS consumed (by the resolver in §4.2), unlike the dead `default_workers`.

### 4.5 Scaffold templates

3 templates embedded via `include_str!()` in `crates/rowforge-studio-core/src/handler_templates/`:

**`GoStdio`** — row-mode echo template:
- `rowforge.yaml`: `runtime.mode: row`, `entry.cmd: ["./bin/handler"]`, `entry.build: ["go", "build", "-o", "bin/handler"]`, `required_input: ["{{primary_field}}"]`
- `handler.go`: minimal stdio loop reading 1 row → echo `{{primary_field}}` → write 1 outcome
- `go.mod`: `module {{name}}`, Go 1.22

**`GoBatch`** — batch-mode template:
- Same as GoStdio but `runtime.mode: batch`, `batch_size: 5`
- `handler.go` reads N rows in one batch envelope, returns batch outcome

**`Empty`** — skeleton only:
- `rowforge.yaml` with `entry.cmd: ["./handler"]` and `entry.build: null`
- Empty `handler.go` stub (`package main; func main() {}`)
- User fills in everything else

Template variables (`{{name}}`, `{{primary_field}}`) replaced at scaffold time via simple string replace — no Tera/Handlebars dep.

## 5. Tauri layer

```
handler_list()                       -> Vec<HandlerSummary>
handler_show(name)                   -> HandlerDetail
handler_open_editor(name)            -> ()
handler_reveal(name)                 -> ()
handler_scaffold(args)               -> String
handler_delete(name)                 -> ()
handler_rename(old, new)             -> ()
```

Plus event:
```
handlers:list                        ()       // coarse refresh hint
```

Emitted after `handler_scaffold`, `handler_delete`, `handler_rename` so the UI's list query invalidates without polling.

`handler_reveal` is the only command that needs the Tauri layer's shell plugin (`shell::open(path)`). Other commands stay studio-core-pure.

## 6. Frontend design

### 6.1 IA changes

- **Sidebar:** Handlers group enabled (was anchored disabled in Plans 1-6 per spec part 7 §7.3). New `NavLink` to `/handlers`.
- **`/handlers`** — list page
- **`/handlers/:name`** — detail page
- **Modals overlaid on either:** Scaffold, Rename, Delete

### 6.2 Handlers list page (`/handlers`)

```
┌─ Handlers ──────────────────────────────────────────────┐
│ <workspace>/handlers/                  [+ New handler…] │
├─────────────────────────────────────────────────────────┤
│ ● golang-billing-channel  v0.1.0  go    2d ago     ⋮    │
│ ● golang-apple-txn-id     v0.1.0  go    1h ago     ⋮    │
│ ⚠ broken-experiment       —       —    5min ago    ⋮    │
│ ○ no-manifest-yet         —       —    just now    ⋮    │
└─────────────────────────────────────────────────────────┘

● = manifest_status: valid (green dot)
⚠ = invalid (amber)
○ = missing (gray)
⋮ = row menu: Edit / Reveal / Rename / Delete
```

Empty state: "No handlers in `<workspace>/handlers/`. [Create your first handler] button."

### 6.3 Handler detail page (`/handlers/:name`)

```
┌─ Handlers / golang-billing-channel ─────────────────────┐
│ ● Valid · v0.1.0 · go              [Edit] [Reveal] ⋮    │
│                                                         │
│ ┌─ Manifest ────────────────────────────────────────┐   │
│ │ ✓ Manifest valid     v0.1.0  go                   │   │
│ │ entry.cmd: ["./golang-billing-channel"]           │   │
│ │ entry.build: ["go", "build", "-o", "..."]         │   │
│ │ runtime.mode: batch · batch_size: 5               │   │
│ └───────────────────────────────────────────────────┘   │
│                                                         │
│ ┌─ Source files ────────────────────────────────────┐   │
│ │  handler.go      4.3 KB                  [Reveal] │   │
│ │  go.mod          372 B                   [Reveal] │   │
│ │  go.sum          672 B                   [Reveal] │   │
│ │  bin/            (directory)             [Reveal] │   │
│ └───────────────────────────────────────────────────┘   │
│                                                         │
│ ┌─ Build status ────────────────────────────────────┐   │
│ │ Build runtime ships in Plan 8.                    │   │
│ └───────────────────────────────────────────────────┘   │
│                                                         │
│ ⋮ menu: Rename · Delete (typed-token friction)         │
└─────────────────────────────────────────────────────────┘
```

ManifestReportView reuses Plan 5's component verbatim.

### 6.4 Scaffold modal

Triggered by "New handler…" on `/handlers` or the empty-state CTA.

```
┌─ New handler ───────────────────────────────────┐
│ Name              [ my-new-handler          ]   │
│   ⓘ lowercase letters, digits, hyphens only    │
│                                                 │
│ Template          ( ) GoStdio                   │
│                   (•) GoBatch                   │
│                   ( ) Empty                     │
│                                                 │
│ Primary input field   [ order_id            ]   │
│   ⓘ column name the template reads from CSV    │
│                                                 │
│              [Cancel]  [Create handler]         │
└─────────────────────────────────────────────────┘
```

Submit → `handler_scaffold` → navigate to `/handlers/<name>`.

### 6.5 Rename dialog

```
┌─ Rename handler ────────────────────────────────┐
│ Current   golang-billing-channel                │
│ New name  [ golang-billing-channel-v2      ]    │
│           ⓘ regex [a-z0-9-]+                   │
│                                                 │
│ ⚠ Past attempts referencing this handler        │
│   will keep their original path in artifacts.   │
│                                                 │
│                    [Cancel]  [Rename]           │
└─────────────────────────────────────────────────┘
```

Spec part-7 destructive guidance: rename is **mutating** but not destructive. Plain confirm is fine; no typed-token.

### 6.6 Delete dialog (typed-token)

Reuses `CancelDialog` pattern from Plan 4:

```
┌─ Delete handler ─────────────────────────────────┐
│ ⚠ This will permanently delete                  │
│   <workspace>/handlers/golang-billing-channel   │
│   and all files inside.                         │
│                                                 │
│ To confirm, type the handler name:              │
│ [ golang-billing-channel                    ]   │
│                                                 │
│                    [Cancel]  [Delete handler]   │
│                              (disabled until    │
│                               name matches)     │
└─────────────────────────────────────────────────┘
```

### 6.7 Settings page Editor section

Add a 4th section to the form (Plan 6 has 3: Workspace / Concurrency / Telemetry):

```
┌─ Editor ──────────────────────────────────────────────┐
│ Preferred editor command                              │
│ [ code -w                                          ]  │
│ ⓘ argv string; first token must be on PATH.          │
│   Examples: "code -w", "cursor --new-window".         │
│   Blank → fall back to $VISUAL, $EDITOR, then         │
│   probe code/cursor/subl/zed.                         │
└───────────────────────────────────────────────────────┘
```

## 7. Risks / open questions

1. **Delete safety.** `fs::remove_dir_all` on a user-controllable name. Mitigations:
   - Regex validate `name` before any path work (`[a-z0-9-]+` rejects `..`, `/`, `\`)
   - Canonicalize the constructed path AFTER joining; assert parent equals canonicalized `<workspace>/handlers`
   - Refuse if any segment is `..` after canonicalization
   - Test: invalid names rejected; valid names that resolve outside `handlers/` (e.g. via symlink) rejected

2. **Symlinks in handlers/.** What if `<workspace>/handlers/foo` is a symlink to `/etc/`? Same canonicalize-and-check defense applies. Document as "we only operate inside `<workspace>/handlers/`; symlinks pointing outside are rejected."

3. **Rename collision races.** TOCTOU between `path.exists()` check and `fs::rename`. fs::rename is atomic at the syscall level so the race window is narrow; worst case is a clean error. Document as accepted.

4. **Editor probes on different OSes.** Windows: `code.cmd`, PATHEXT handling — `which@6` does this correctly. Linux: depends on package manager install location. macOS: `code` is the VS Code CLI shim, installed via Command Palette → "Shell Command: Install 'code' command in PATH". Document the prereq in HUMAN_SMOKE.

5. **Scaffold template Go versions.** Hardcoding `go 1.22` in template `go.mod` will date. Mitigate: comment in the template tells users to bump.

6. **`preferred_editor` validation.** User pastes `code -w` — fine. Pastes `code` — also fine (no extra args). Pastes empty string — treat as None. Pastes garbage with unclosed quotes — `shell_words::split` returns an error; surface as `UiError::InvalidArg { reason }` at Settings save time? Or at resolver time? Decision: validate at save (better UX — fail at write rather than later open).

7. **Source files list.** Top-level only per spec 8.3. `bin/` shows as a directory; user can't expand from Studio. That's fine for v1 — Reveal in Finder is the escape hatch.

## 8. Acceptance criteria

1. `cargo build` + `cargo test` clean
2. `pnpm tsc -b` + `pnpm test` + `pnpm build` clean
3. Sidebar Handlers link enabled; `/handlers` route renders
4. `handler_list` returns all dirs under `<workspace>/handlers/` with correct ManifestStatus
5. `handler_show` includes parsed manifest + source files
6. `handler_open_editor` opens VS Code (or `$EDITOR`) at the handler dir; fails cleanly with `EditorNotFound` when all 4 tiers miss
7. `handler_reveal` opens the OS file manager at the handler dir
8. `handler_scaffold` creates 3 different templates correctly:
   - GoStdio: 3 files, row-mode manifest, primary_field replaced
   - GoBatch: 3 files, batch-mode manifest
   - Empty: 2 files (yaml + stub `.go`)
9. `handler_delete` refuses paths outside `<workspace>/handlers/` (symlink defense); typed-token UI prevents accidental delete
10. `handler_rename` updates the dir; `handler_instances.source_snapshot_dir` rows unchanged (verify via sqlite query in test)
11. Settings page has Editor section; `preferred_editor` persists across restart
12. Spec docs (en + zh-Hant) updated: part-2 Settings + ExecSummary footnote, part-5 §5.3 new variants, part-7 IA + flows, part-8 cross-refs
13. **(human)** HUMAN_SMOKE Plan 7 walkthrough

## 9. Out-of-scope captured for future plans

| Item | Target plan |
|---|---|
| Build runtime (`manifest.build` subprocess + stream) | Plan 8 |
| Smoke test runtime (handshake + per-row dispatch) | Plan 9 |
| In-Studio code editor (Monaco / CodeMirror) | Non-goal v1 |
| Fixture-file smoke inputs | Plan 9 v1.1 |
| `rowforge pack` from Studio | Future |
| Structured manifest editor (write-back) | Future |
| Cross-workspace handler registry | Future |
| Hard cancel actually killing workers (carry-forward) | Future (needs rowforge-core API) |
