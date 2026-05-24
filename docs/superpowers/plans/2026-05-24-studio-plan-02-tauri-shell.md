# Studio Plan 02 — Tauri Shell + Exec List Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** First runnable Studio desktop window. Boot screen lets the user pick a workspace, then renders the exec list (W-1 wireframe) backed by `rowforge-studio-core` from Plan 01.

**Architecture:** Tauri 2 (existing partial scaffold at `apps/rowforge-studio/src-tauri/gen/`) hosting a Vite + React 19 + TypeScript + Tailwind 3 + shadcn/ui webview. The Tauri layer (`commands.rs`) is thin glue — argument translation, `tauri::State<StudioCore>` lifecycle, error pass-through. All projection logic stays in `studio-core`.

**Tech Stack:**
- Tauri 2 (Rust backend, system webview)
- Vite 6 + React 19 + TypeScript strict
- Tailwind CSS 3 + shadcn/ui (copy-paste components)
- TanStack Query v5 for all `invoke` calls (cache + loading)
- react-router v6 HashRouter
- pnpm

**Spec references:** Part 1 §1.3 architecture; Part 5 §5.1 crate boundary, §5.5 Tauri command surface, §5.6 settings file location; Part 7 §7.1 stack, §7.3 IA + sidebar layout, §7.4 Flow A steps 1–3, §7.13 wireframes W-1 + W-7.

---

## Decisions resolved during brainstorm

| Decision | Choice | Why |
|---|---|---|
| Boot UX when `~/.rowforge` missing | Full-page Workspace Picker (W-7) | Explicit; avoids surprise file creation; user choice |
| Tauri command `Err` shape | `Result<T, UiError>` structured JSON | Aligns with spec §5.3; frontend can branch by `kind` |
| Package manager | pnpm | Tauri example default; workspace-friendly |
| Tailwind major | v3 | shadcn/ui's stable surface; v4 still ecosystem-rough |
| React version | 19 | Latest stable; concurrent features useful for Live tab later |
| Routing | `react-router-dom` HashRouter | Webview-safe; no server-side routing concerns |
| State / IPC | TanStack Query v5 | Cache + loading + retry out of the box |

---

## File structure

### New — Rust backend (`apps/rowforge-studio/src-tauri/`)
- `Cargo.toml`
- `tauri.conf.json` — app identifier, window size, build pipeline
- `build.rs` — Tauri 2 build hook (`tauri_build::build()`)
- `src/main.rs` — entry, runtime, command registration
- `src/commands.rs` — Tauri commands wrapping `StudioCore`
- `src/state.rs` — `AppState { core: tokio::sync::RwLock<Option<StudioCore>> }` (Option because no workspace open at boot)
- `src/settings.rs` — file-path resolution via `app.path().app_data_dir()` + load/save IO
- `capabilities/main.json` — refactor of existing `gen/schemas/capabilities.json` into source

### New — `rowforge-studio-core` extensions
- `crates/rowforge-studio-core/src/settings.rs` — `Settings`, `Settings::load_from(Read)`, `Settings::save_to(Write)` (filesystem-policy-free per spec §5.6)
- `crates/rowforge-studio-core/src/lib.rs` — `pub mod settings;` + re-export

### New — Frontend (`apps/rowforge-studio/`)
- `package.json`
- `pnpm-lock.yaml` (generated)
- `vite.config.ts`
- `tsconfig.json`, `tsconfig.node.json`
- `tailwind.config.ts`
- `postcss.config.js`
- `index.html`
- `src/main.tsx` — React entry
- `src/App.tsx` — Router root
- `src/ipc/client.ts` — typed `invoke` wrapper + UiError type
- `src/ipc/types.ts` — TypeScript mirrors of `Workspace` / `ExecSummary` / `Settings` / `UiError`
- `src/ipc/queries.ts` — TanStack Query hooks: `useWorkspace`, `useExecList`, `useSettings`
- `src/pages/WorkspacePicker.tsx`
- `src/pages/ExecList.tsx`
- `src/layout/AppShell.tsx`
- `src/layout/Sidebar.tsx`
- `src/layout/Header.tsx`
- `src/components/ui/*.tsx` — shadcn primitives we use (button, table, etc.)
- `src/styles/globals.css` — Tailwind directives + CSS variables
- `src/lib/utils.ts` — shadcn's `cn()` helper
- `src/__tests__/exec-list.test.tsx` — Vitest smoke test
- `vitest.config.ts`

### Modified
- `Cargo.toml` (workspace root) — add `apps/rowforge-studio/src-tauri` to `members`
- `crates/rowforge-studio-core/src/lib.rs` — `pub mod settings;`
- `crates/rowforge-studio-core/Cargo.toml` — drop unused `dirs` dep (Plan 1 carry-forward)

### Out of scope for Plan 02
- Active runs pill (no live runs yet — Plan 4)
- Sidebar's Authoring group (Plan 6)
- Breadcrumb (only one page route in Plan 2)
- Exec detail / Attempt detail (Plan 3)
- Run launcher (Plan 5)
- Settings page UI (Plan 5 — only the persistence layer ships in Plan 2)
- Schema-version pin enforcement on workspace open (Plan 3)
- Active runs subscription / Tauri events (Plan 4)

---

## Task 1: Tauri backend Cargo scaffold

**Files:**
- Create: `apps/rowforge-studio/src-tauri/Cargo.toml`
- Create: `apps/rowforge-studio/src-tauri/build.rs`
- Create: `apps/rowforge-studio/src-tauri/src/main.rs`
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1.1: Add the new crate to workspace members**

Open root `Cargo.toml`. Update `[workspace] members`:

```toml
[workspace]
resolver = "2"
members = [
    "crates/rowforge-core",
    "crates/rowforge-cli",
    "crates/rowforge-studio-core",
    "crates/test-handler",
    "apps/rowforge-studio/src-tauri",
]
```

Add Tauri 2 deps to `[workspace.dependencies]`:

```toml
tauri = { version = "2", features = [] }
tauri-build = { version = "2" }
tauri-plugin-shell = "2"
tauri-plugin-dialog = "2"
```

- [ ] **Step 1.2: Create `apps/rowforge-studio/src-tauri/Cargo.toml`**

```toml
[package]
name = "rowforge-studio"
version = "0.1.0"
description = "rowforge Studio (Tauri shell)"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish = false

[lib]
name = "rowforge_studio_lib"
crate-type = ["staticlib", "cdylib", "rlib"]

[build-dependencies]
tauri-build = { workspace = true }

[dependencies]
tauri = { workspace = true, features = [] }
tauri-plugin-shell = { workspace = true }
tauri-plugin-dialog = { workspace = true }
rowforge-studio-core = { path = "../../../crates/rowforge-studio-core" }
serde = { workspace = true }
serde_json = { workspace = true }
tokio = { workspace = true }
```

- [ ] **Step 1.3: Create `apps/rowforge-studio/src-tauri/build.rs`**

```rust
fn main() {
    tauri_build::build()
}
```

- [ ] **Step 1.4: Create `apps/rowforge-studio/src-tauri/src/main.rs` skeleton**

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    rowforge_studio_lib::run()
}
```

- [ ] **Step 1.5: Create `apps/rowforge-studio/src-tauri/src/lib.rs` skeleton**

```rust
//! rowforge Studio Tauri shell.
//!
//! See `docs/spec/studio/part-5-api.md` §5.1 for crate-boundary contract:
//! this layer is thin glue; all projection logic lives in
//! `rowforge-studio-core`.

mod state;
mod commands;
mod settings;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(state::AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::workspace_open,
            commands::exec_list,
            commands::workspace_settings_load,
            commands::workspace_settings_save,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

Create the three module stubs so it compiles:

`src/state.rs`:
```rust
//! App state — stub until Task 2 fills it in.
#[derive(Default)]
pub struct AppState;
```

`src/commands.rs`:
```rust
//! Tauri commands — stubs until later tasks fill them in.
use serde::Serialize;

#[derive(Serialize)]
pub struct StubWorkspace;

#[tauri::command]
pub fn workspace_open() -> Result<StubWorkspace, String> {
    Err("not implemented".into())
}
#[tauri::command]
pub fn exec_list() -> Result<Vec<()>, String> {
    Err("not implemented".into())
}
#[tauri::command]
pub fn workspace_settings_load() -> Result<(), String> {
    Err("not implemented".into())
}
#[tauri::command]
pub fn workspace_settings_save() -> Result<(), String> {
    Err("not implemented".into())
}
```

`src/settings.rs`:
```rust
//! Settings file-path resolution + IO — stub until Task 4.
```

- [ ] **Step 1.6: Verify the crate compiles**

Run: `cargo build -p rowforge-studio`
Expected: PASS (warnings about unused stubs are OK).

If `tauri-build` complains about a missing `tauri.conf.json`, the next task creates it; for now this build will fail at link time but compile-check should succeed. If it errors hard, skip ahead to Task 2 step 2.1 first then come back.

Actually `tauri_build::build()` does require `tauri.conf.json`. Reorder: do Task 2's `tauri.conf.json` creation **first**, then come back for Step 1.6.

- [ ] **Step 1.7: Commit**

```bash
git add Cargo.toml apps/rowforge-studio/src-tauri/Cargo.toml apps/rowforge-studio/src-tauri/build.rs apps/rowforge-studio/src-tauri/src/
git commit -m "studio-shell: scaffold tauri backend skeleton

Adds apps/rowforge-studio/src-tauri to workspace with stub
main.rs / lib.rs / state.rs / commands.rs / settings.rs. Real
implementations land in later tasks."
```

---

## Task 2: Tauri config + capabilities

**Files:**
- Create: `apps/rowforge-studio/src-tauri/tauri.conf.json`
- Create: `apps/rowforge-studio/src-tauri/capabilities/main.json`
- Move/rewrite: `apps/rowforge-studio/src-tauri/gen/schemas/capabilities.json` content into source-controlled `capabilities/main.json`

- [ ] **Step 2.1: Create `apps/rowforge-studio/src-tauri/tauri.conf.json`**

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "rowforge Studio",
  "version": "0.1.0",
  "identifier": "com.lemotw.rowforge.studio",
  "build": {
    "beforeDevCommand": "pnpm dev",
    "devUrl": "http://localhost:1420",
    "beforeBuildCommand": "pnpm build",
    "frontendDist": "../dist"
  },
  "app": {
    "windows": [
      {
        "title": "rowforge Studio",
        "width": 1280,
        "height": 800,
        "minWidth": 960,
        "minHeight": 600,
        "label": "main"
      }
    ],
    "security": {
      "csp": null
    }
  },
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": [
      "icons/32x32.png",
      "icons/128x128.png",
      "icons/128x128@2x.png",
      "icons/icon.icns",
      "icons/icon.ico"
    ]
  }
}
```

Note: the icons block references files we do not yet have. Tauri allows running `tauri dev` without them; bundling requires them. Plan 2 only needs `tauri dev` to work — add a `.gitkeep` placeholder under `icons/` and leave bundle for a future plan. To avoid a hard build break:

- [ ] **Step 2.2: Create placeholder icon dir**

```bash
mkdir -p apps/rowforge-studio/src-tauri/icons
touch apps/rowforge-studio/src-tauri/icons/.gitkeep
```

Tauri will warn during `tauri dev` but not fail.

- [ ] **Step 2.3: Create `apps/rowforge-studio/src-tauri/capabilities/main.json`**

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Capability for the main window",
  "local": true,
  "windows": ["main"],
  "permissions": [
    "core:default",
    "shell:default",
    "shell:allow-open",
    "dialog:default",
    "dialog:allow-open"
  ]
}
```

These four permissions cover Plan 2 + foreseeable near-term:
- `core:default` — Tauri core (event, invoke, window)
- `shell:allow-open` — for future "Reveal in Finder" (Plan 3+); `shell:default` for general shell access
- `dialog:allow-open` — Workspace Picker file dialog (this Plan)

The existing `gen/schemas/capabilities.json` is auto-generated from the schema. Tauri 2 expects capability sources under `capabilities/` and regenerates `gen/schemas/` at build. Do not edit `gen/schemas/capabilities.json` by hand.

- [ ] **Step 2.4: Update `tauri.conf.json` to register the capability**

Add to `tauri.conf.json` under `"app"`:

```json
"app": {
  ...,
  "security": {
    "csp": null,
    "capabilities": ["default"]
  }
}
```

(Replace the existing `"security"` block.)

- [ ] **Step 2.5: Verify backend compiles**

Run: `cargo build -p rowforge-studio`
Expected: PASS. `tauri_build` should pick up the config and regenerate `gen/schemas/`.

- [ ] **Step 2.6: Commit**

```bash
git add apps/rowforge-studio/src-tauri/tauri.conf.json apps/rowforge-studio/src-tauri/capabilities apps/rowforge-studio/src-tauri/icons
git commit -m "studio-shell: tauri config + main capability

App identifier, 1280x800 window, dev server on :1420.
Capability grants shell:allow-open + dialog:allow-open for
Reveal-in-Finder and Workspace Picker."
```

---

## Task 3: `Settings` in `studio-core`

Spec §5.6 says `Settings` type lives in `studio-core`; file-path resolution lives in the Tauri layer. Plan 2 only needs the `workspace_root` field; other fields stay None until later plans.

**Files:**
- Create: `crates/rowforge-studio-core/src/settings.rs`
- Modify: `crates/rowforge-studio-core/src/lib.rs`
- Modify: `crates/rowforge-studio-core/Cargo.toml` (drop unused `dirs` dep — Plan 1 carry-forward)

- [ ] **Step 3.1: Write failing tests**

Create `crates/rowforge-studio-core/src/settings.rs`:

```rust
//! User settings type — filesystem-policy-free.
//!
//! `Settings::load_from(impl Read)` / `Settings::save_to(impl Write)` take
//! arbitrary streams so this crate never depends on a specific file
//! location. The Tauri layer resolves `<app_data_dir>/rowforge-studio/
//! settings.json` and feeds the bytes through.
//!
//! Spec: `docs/spec/studio/part-2-model.md` §2.2.9,
//!       `docs/spec/studio/part-5-api.md` §5.6.

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::PathBuf;

use crate::UiError;

const CURRENT_SCHEMA_VERSION: u8 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct Settings {
    pub schema_version: u8,
    pub workspace_root: Option<PathBuf>,
    pub default_workers: Option<u32>,
    pub max_concurrent_runs: Option<u32>,
    pub telemetry_opt_in: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            schema_version: CURRENT_SCHEMA_VERSION,
            workspace_root: None,
            default_workers: None,
            max_concurrent_runs: None,
            telemetry_opt_in: false,
        }
    }
}

impl Settings {
    pub fn load_from<R: Read>(reader: R) -> Result<Self, UiError> {
        serde_json::from_reader(reader)
            .map_err(|e| UiError::Io(format!("settings parse: {e}")))
    }

    pub fn save_to<W: Write>(&self, writer: W) -> Result<(), UiError> {
        serde_json::to_writer_pretty(writer, self)
            .map_err(|e| UiError::Io(format!("settings write: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_schema_version_1() {
        assert_eq!(Settings::default().schema_version, 1);
    }

    #[test]
    fn roundtrip_preserves_workspace_root() {
        let mut s = Settings::default();
        s.workspace_root = Some(PathBuf::from("/tmp/ws"));
        let mut buf = Vec::new();
        s.save_to(&mut buf).unwrap();
        let parsed = Settings::load_from(buf.as_slice()).unwrap();
        assert_eq!(parsed.workspace_root, Some(PathBuf::from("/tmp/ws")));
    }

    #[test]
    fn tolerant_to_missing_fields() {
        let json = br#"{"schema_version": 1}"#;
        let parsed = Settings::load_from(json.as_slice()).unwrap();
        assert_eq!(parsed.schema_version, 1);
        assert_eq!(parsed.workspace_root, None);
        assert!(!parsed.telemetry_opt_in);
    }
}
```

- [ ] **Step 3.2: Run — failure (module not registered)**

Run: `cargo test -p rowforge-studio-core --lib settings::tests`
Expected: FAIL — "unresolved module `settings`".

- [ ] **Step 3.3: Register the module**

Edit `crates/rowforge-studio-core/src/lib.rs`. Add `pub mod settings;` after the other `pub mod` lines, and `pub use settings::Settings;` after the other `pub use` lines.

- [ ] **Step 3.4: Drop the unused `dirs` dep**

In `crates/rowforge-studio-core/Cargo.toml`, remove the `dirs = { workspace = true }` line under `[dependencies]`. (Plan 1 carry-forward — confirmed unused.)

- [ ] **Step 3.5: Run tests — pass**

Run: `cargo test -p rowforge-studio-core`
Expected: PASS. All 4 foundation tests from Plan 1 still pass; 3 new settings tests pass.

- [ ] **Step 3.6: Commit**

```bash
git add crates/rowforge-studio-core
git commit -m "studio-core: Settings type with stream load/save

Plan 2 needs persisted workspace_root. Type defined here per spec
§2.2.9; filesystem-policy-free per spec §5.6 (Tauri layer resolves
app_data_dir). Drops unused dirs dep (Plan 1 carry-forward)."
```

---

## Task 4: Tauri-side Settings file IO

**Files:**
- Modify: `apps/rowforge-studio/src-tauri/src/settings.rs`
- Modify: `apps/rowforge-studio/src-tauri/src/state.rs`

- [ ] **Step 4.1: Implement settings file path + IO**

Open `apps/rowforge-studio/src-tauri/src/settings.rs` and replace:

```rust
//! Settings file path resolution + IO using Tauri's app_data_dir.
//!
//! Path: `<app_data_dir>/rowforge-studio/settings.json` per spec §5.6.

use std::fs;
use std::path::PathBuf;

use rowforge_studio_core::{Settings, UiError};
use tauri::{Manager, Runtime};

fn settings_path<R: Runtime>(app: &tauri::AppHandle<R>) -> Result<PathBuf, UiError> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| UiError::Io(format!("app_data_dir: {e}")))?;
    let ws_dir = dir.join("rowforge-studio");
    fs::create_dir_all(&ws_dir).map_err(|e| UiError::Io(e.to_string()))?;
    Ok(ws_dir.join("settings.json"))
}

pub fn load<R: Runtime>(app: &tauri::AppHandle<R>) -> Result<Settings, UiError> {
    let p = settings_path(app)?;
    if !p.exists() {
        return Ok(Settings::default());
    }
    let f = fs::File::open(&p).map_err(|e| UiError::Io(e.to_string()))?;
    Settings::load_from(f)
}

pub fn save<R: Runtime>(
    app: &tauri::AppHandle<R>,
    settings: &Settings,
) -> Result<(), UiError> {
    let p = settings_path(app)?;
    let f = fs::File::create(&p).map_err(|e| UiError::Io(e.to_string()))?;
    settings.save_to(f)
}
```

- [ ] **Step 4.2: Verify it compiles**

Run: `cargo build -p rowforge-studio`
Expected: PASS.

- [ ] **Step 4.3: Commit**

```bash
git add apps/rowforge-studio/src-tauri/src/settings.rs
git commit -m "studio-shell: resolve settings file path via app_data_dir

Path: <app_data_dir>/rowforge-studio/settings.json per spec §5.6.
Missing file ⇒ default Settings. Creates parent dirs on save."
```

---

## Task 5: `AppState` + Tauri commands wiring `StudioCore`

**Files:**
- Modify: `apps/rowforge-studio/src-tauri/src/state.rs`
- Modify: `apps/rowforge-studio/src-tauri/src/commands.rs`

- [ ] **Step 5.1: `AppState` with locked `Option<StudioCore>`**

Open `apps/rowforge-studio/src-tauri/src/state.rs` and replace:

```rust
//! App state: the lazily-opened StudioCore.
//!
//! `core` is None until the user picks a workspace via Workspace Picker
//! (Plan 2) or the boot autoload finds settings.workspace_root.

use rowforge_studio_core::StudioCore;
use tokio::sync::RwLock;

#[derive(Default)]
pub struct AppState {
    pub core: RwLock<Option<StudioCore>>,
}
```

`StudioCore` is not `Sync` by default (holds an SQLite connection internally). `RwLock<Option<StudioCore>>` makes it externally Send/Sync; only one writer at a time. List queries are read-side but core's `list()` takes `&self` — so reads are fine concurrently. Practical concurrency is limited (≤ 1 user, sequential UI clicks); the RwLock is forward-prep.

- [ ] **Step 5.2: Commands wired to StudioCore**

Open `apps/rowforge-studio/src-tauri/src/commands.rs` and replace:

```rust
//! Tauri commands wrapping the `StudioCore` surface.
//!
//! Every command returns `Result<T, UiError>`; the structured error is
//! serialized to JSON for the React layer to classify by `kind` (spec
//! §5.3 / §5.5).

use std::path::PathBuf;

use rowforge_studio_core::{
    ExecSummary, ListFilter, OpenOpts, Settings, StudioCore, UiError, Workspace,
};
use tauri::State;

use crate::settings as settings_io;
use crate::state::AppState;

#[tauri::command]
pub async fn workspace_open(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    path: Option<PathBuf>,
) -> Result<Workspace, UiError> {
    let opts = match path {
        Some(p) => OpenOpts::new().with_workspace(p),
        None => OpenOpts::new(),
    };
    let core = StudioCore::open(opts)?;
    let workspace = core.workspace().clone();

    // Persist the chosen path to settings so next boot autoloads.
    let mut s = settings_io::load(&app)?;
    s.workspace_root = Some(workspace.root.clone());
    settings_io::save(&app, &s)?;

    *state.core.write().await = Some(core);
    Ok(workspace)
}

#[tauri::command]
pub async fn exec_list(
    state: State<'_, AppState>,
) -> Result<Vec<ExecSummary>, UiError> {
    let guard = state.core.read().await;
    let core = guard
        .as_ref()
        .ok_or_else(|| UiError::WorkspaceUnavailable("no workspace open".into()))?;
    core.list(ListFilter::default())
}

#[tauri::command]
pub fn workspace_settings_load(app: tauri::AppHandle) -> Result<Settings, UiError> {
    settings_io::load(&app)
}

#[tauri::command]
pub fn workspace_settings_save(
    app: tauri::AppHandle,
    settings: Settings,
) -> Result<(), UiError> {
    settings_io::save(&app, &settings)
}
```

- [ ] **Step 5.3: Verify backend compiles**

Run: `cargo build -p rowforge-studio`
Expected: PASS.

- [ ] **Step 5.4: Commit**

```bash
git add apps/rowforge-studio/src-tauri/src/state.rs apps/rowforge-studio/src-tauri/src/commands.rs
git commit -m "studio-shell: wire workspace_open / exec_list / settings commands

AppState holds RwLock<Option<StudioCore>>. workspace_open persists
the path into settings so next boot autoloads. exec_list errors
WorkspaceUnavailable if no workspace open."
```

---

## Task 6: Frontend scaffold (Vite + React 19 + TS)

**Files:**
- Create: `apps/rowforge-studio/package.json`
- Create: `apps/rowforge-studio/vite.config.ts`
- Create: `apps/rowforge-studio/tsconfig.json`
- Create: `apps/rowforge-studio/tsconfig.node.json`
- Create: `apps/rowforge-studio/index.html`
- Create: `apps/rowforge-studio/src/main.tsx`
- Create: `apps/rowforge-studio/src/App.tsx`
- Create: `apps/rowforge-studio/.gitignore`

- [ ] **Step 6.1: Create `package.json`**

```json
{
  "name": "rowforge-studio",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc -b && vite build",
    "preview": "vite preview",
    "test": "vitest run",
    "tauri": "tauri"
  },
  "dependencies": {
    "@tanstack/react-query": "^5.59.0",
    "@tauri-apps/api": "^2.1.0",
    "@tauri-apps/plugin-dialog": "^2.0.0",
    "@tauri-apps/plugin-shell": "^2.0.0",
    "class-variance-authority": "^0.7.0",
    "clsx": "^2.1.1",
    "lucide-react": "^0.460.0",
    "react": "^19.0.0",
    "react-dom": "^19.0.0",
    "react-router-dom": "^6.28.0",
    "tailwind-merge": "^2.5.0"
  },
  "devDependencies": {
    "@tauri-apps/cli": "^2.1.0",
    "@testing-library/jest-dom": "^6.5.0",
    "@testing-library/react": "^16.0.0",
    "@types/node": "^22.0.0",
    "@types/react": "^19.0.0",
    "@types/react-dom": "^19.0.0",
    "@vitejs/plugin-react": "^4.3.0",
    "autoprefixer": "^10.4.20",
    "jsdom": "^25.0.0",
    "postcss": "^8.4.49",
    "tailwindcss": "^3.4.15",
    "typescript": "^5.6.0",
    "vite": "^6.0.0",
    "vitest": "^2.1.0"
  }
}
```

- [ ] **Step 6.2: Create `vite.config.ts`**

```ts
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "node:path";

const port = 1420;

export default defineConfig(async () => ({
  plugins: [react()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  clearScreen: false,
  server: {
    port,
    strictPort: true,
    host: process.env.TAURI_DEV_HOST || false,
    hmr: process.env.TAURI_DEV_HOST
      ? { protocol: "ws", host: process.env.TAURI_DEV_HOST, port: port + 1 }
      : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
}));
```

- [ ] **Step 6.3: Create `tsconfig.json`**

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "useDefineForClassFields": true,
    "lib": ["ES2022", "DOM", "DOM.Iterable"],
    "module": "ESNext",
    "skipLibCheck": true,
    "moduleResolution": "bundler",
    "allowImportingTsExtensions": true,
    "resolveJsonModule": true,
    "isolatedModules": true,
    "moduleDetection": "force",
    "noEmit": true,
    "jsx": "react-jsx",
    "strict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "noFallthroughCasesInSwitch": true,
    "noUncheckedSideEffectImports": true,
    "baseUrl": ".",
    "paths": { "@/*": ["src/*"] }
  },
  "include": ["src"],
  "references": [{ "path": "./tsconfig.node.json" }]
}
```

- [ ] **Step 6.4: Create `tsconfig.node.json`**

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "lib": ["ES2023"],
    "module": "ESNext",
    "skipLibCheck": true,
    "moduleResolution": "bundler",
    "allowSyntheticDefaultImports": true,
    "strict": true,
    "noEmit": true,
    "composite": true
  },
  "include": ["vite.config.ts", "vitest.config.ts", "tailwind.config.ts"]
}
```

- [ ] **Step 6.5: Create `index.html`**

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <link rel="icon" type="image/svg+xml" href="/vite.svg" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>rowforge Studio</title>
  </head>
  <body class="bg-neutral-950 text-neutral-100 antialiased">
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
```

- [ ] **Step 6.6: Create `src/main.tsx`**

```tsx
import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { HashRouter } from "react-router-dom";
import App from "./App";
import "./styles/globals.css";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: { retry: false, refetchOnWindowFocus: false },
  },
});

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <HashRouter>
        <App />
      </HashRouter>
    </QueryClientProvider>
  </React.StrictMode>
);
```

- [ ] **Step 6.7: Create `src/App.tsx`**

```tsx
import { Route, Routes } from "react-router-dom";
import { BootGate } from "./pages/BootGate";

export default function App() {
  return (
    <Routes>
      <Route path="*" element={<BootGate />} />
    </Routes>
  );
}
```

`BootGate` is created in Task 9. Compilation will fail until then. Skip the build check until Task 9 lands.

- [ ] **Step 6.8: Create `.gitignore`**

```
node_modules
dist
.vite
*.tsbuildinfo
```

- [ ] **Step 6.9: Install deps**

```bash
cd apps/rowforge-studio
pnpm install
```

Expected: lockfile created, no errors. (Network required.)

- [ ] **Step 6.10: Commit (intentionally without lockfile pass first)**

```bash
git add apps/rowforge-studio/package.json apps/rowforge-studio/vite.config.ts apps/rowforge-studio/tsconfig.json apps/rowforge-studio/tsconfig.node.json apps/rowforge-studio/index.html apps/rowforge-studio/src apps/rowforge-studio/.gitignore apps/rowforge-studio/pnpm-lock.yaml
git commit -m "studio-shell: vite + react 19 + ts frontend scaffold

package.json + vite.config + tsconfig + entry. HashRouter +
TanStack Query providers. BootGate referenced (created Task 9)."
```

---

## Task 7: Tailwind + shadcn/ui setup

**Files:**
- Create: `apps/rowforge-studio/tailwind.config.ts`
- Create: `apps/rowforge-studio/postcss.config.js`
- Create: `apps/rowforge-studio/src/styles/globals.css`
- Create: `apps/rowforge-studio/src/lib/utils.ts`
- Create: `apps/rowforge-studio/components.json` (shadcn config)
- Create: 4 baseline shadcn primitives we need for W-1 and W-7

- [ ] **Step 7.1: Create `tailwind.config.ts`**

```ts
import type { Config } from "tailwindcss";

export default {
  darkMode: ["class"],
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      fontFamily: {
        mono: ["JetBrains Mono", "SF Mono", "Menlo", "ui-monospace", "monospace"],
      },
      colors: {
        background: "hsl(var(--background))",
        foreground: "hsl(var(--foreground))",
        muted: { DEFAULT: "hsl(var(--muted))", foreground: "hsl(var(--muted-foreground))" },
        border: "hsl(var(--border))",
        primary: { DEFAULT: "hsl(var(--primary))", foreground: "hsl(var(--primary-foreground))" },
      },
    },
  },
  plugins: [],
} satisfies Config;
```

- [ ] **Step 7.2: Create `postcss.config.js`**

```js
export default {
  plugins: {
    tailwindcss: {},
    autoprefixer: {},
  },
};
```

- [ ] **Step 7.3: Create `src/styles/globals.css`**

```css
@tailwind base;
@tailwind components;
@tailwind utilities;

@layer base {
  :root {
    --background: 0 0% 4%;
    --foreground: 0 0% 96%;
    --muted: 240 5% 14%;
    --muted-foreground: 240 5% 65%;
    --border: 240 5% 18%;
    --primary: 142 71% 45%;
    --primary-foreground: 0 0% 100%;
  }
  body {
    @apply bg-background text-foreground;
    font-variant-numeric: tabular-nums;
  }
}
```

- [ ] **Step 7.4: Create `src/lib/utils.ts`**

```ts
import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}
```

- [ ] **Step 7.5: Create `components.json`**

```json
{
  "$schema": "https://ui.shadcn.com/schema.json",
  "style": "new-york",
  "rsc": false,
  "tsx": true,
  "tailwind": {
    "config": "tailwind.config.ts",
    "css": "src/styles/globals.css",
    "baseColor": "neutral",
    "cssVariables": true
  },
  "aliases": {
    "components": "@/components",
    "utils": "@/lib/utils"
  }
}
```

- [ ] **Step 7.6: Add 4 shadcn primitives by hand (Button, Card, Table, Skeleton)**

We add these by hand to avoid the shadcn CLI's interactivity. Use the canonical shadcn "new-york" style.

Create `apps/rowforge-studio/src/components/ui/button.tsx`:

```tsx
import * as React from "react";
import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";

const buttonVariants = cva(
  "inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-md text-sm font-medium transition-colors disabled:pointer-events-none disabled:opacity-50",
  {
    variants: {
      variant: {
        default: "bg-primary text-primary-foreground hover:bg-primary/90",
        secondary: "bg-muted text-foreground hover:bg-muted/80",
        ghost: "hover:bg-muted",
        outline: "border border-border bg-transparent hover:bg-muted",
      },
      size: {
        default: "h-9 px-4 py-2",
        sm: "h-8 rounded-md px-3 text-xs",
        lg: "h-10 rounded-md px-6",
        icon: "h-9 w-9",
      },
    },
    defaultVariants: { variant: "default", size: "default" },
  }
);

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {
  asChild?: boolean;
}

export const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, asChild = false, ...props }, ref) => {
    const Comp = asChild ? Slot : "button";
    return (
      <Comp
        ref={ref}
        className={cn(buttonVariants({ variant, size }), className)}
        {...props}
      />
    );
  }
);
Button.displayName = "Button";

export { buttonVariants };
```

This requires `@radix-ui/react-slot`. Add it to `package.json` dependencies:
```
"@radix-ui/react-slot": "^1.1.0",
```
Then `pnpm install` again.

Create `apps/rowforge-studio/src/components/ui/card.tsx`:

```tsx
import * as React from "react";
import { cn } from "@/lib/utils";

export const Card = React.forwardRef<HTMLDivElement, React.HTMLAttributes<HTMLDivElement>>(
  ({ className, ...props }, ref) => (
    <div ref={ref} className={cn("rounded-lg border border-border bg-neutral-900", className)} {...props} />
  )
);
Card.displayName = "Card";
```

Create `apps/rowforge-studio/src/components/ui/skeleton.tsx`:

```tsx
import { cn } from "@/lib/utils";
export function Skeleton({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return <div className={cn("animate-pulse rounded-md bg-muted", className)} {...props} />;
}
```

Create `apps/rowforge-studio/src/components/ui/table.tsx` (minimal):

```tsx
import * as React from "react";
import { cn } from "@/lib/utils";

export const Table = React.forwardRef<HTMLTableElement, React.HTMLAttributes<HTMLTableElement>>(
  ({ className, ...props }, ref) => (
    <table ref={ref} className={cn("w-full text-sm", className)} {...props} />
  )
);
export const Thead: typeof Table = React.forwardRef(({ className, ...props }: any, ref) => (
  <thead ref={ref} className={cn("border-b border-border text-muted-foreground", className)} {...props} />
)) as any;
export const Tr: typeof Table = React.forwardRef(({ className, ...props }: any, ref) => (
  <tr ref={ref} className={cn("border-b border-border last:border-0 hover:bg-muted/40", className)} {...props} />
)) as any;
export const Th: typeof Table = React.forwardRef(({ className, ...props }: any, ref) => (
  <th ref={ref} className={cn("h-9 px-3 text-left font-medium", className)} {...props} />
)) as any;
export const Td: typeof Table = React.forwardRef(({ className, ...props }: any, ref) => (
  <td ref={ref} className={cn("h-9 px-3", className)} {...props} />
)) as any;
```

- [ ] **Step 7.7: Install added deps**

```bash
cd apps/rowforge-studio
pnpm install
```

- [ ] **Step 7.8: Commit**

```bash
git add apps/rowforge-studio/tailwind.config.ts apps/rowforge-studio/postcss.config.js apps/rowforge-studio/src/styles apps/rowforge-studio/src/lib apps/rowforge-studio/src/components apps/rowforge-studio/components.json apps/rowforge-studio/package.json apps/rowforge-studio/pnpm-lock.yaml
git commit -m "studio-shell: tailwind + shadcn baseline (Button, Card, Table, Skeleton)

Dark-mode tokens from spec §7.5. Tabular-nums on body. Four primitives
needed for W-1 (exec list) and W-7 (empty state)."
```

---

## Task 8: IPC client + typed bindings + TanStack Query hooks

**Files:**
- Create: `apps/rowforge-studio/src/ipc/types.ts`
- Create: `apps/rowforge-studio/src/ipc/client.ts`
- Create: `apps/rowforge-studio/src/ipc/queries.ts`

- [ ] **Step 8.1: Hand-write TypeScript type mirrors**

Create `apps/rowforge-studio/src/ipc/types.ts`:

```ts
// Hand-written mirrors of rowforge-studio-core public types.
// Keep in sync until Plan 3 introduces auto-gen via specta or
// tauri-specta. Cross-reference: `crates/rowforge-studio-core/src/*.rs`.

export interface Workspace {
  root: string;
  schema_version: number;
}

export interface ExecSummary {
  id: string;
  name: string;
  created_at: string; // ISO 8601 UTC
  input_rows: number | null;
  attempts_count: number;
  last_attempt_state: string | null;
  last_attempt_counts: AttemptCountsStub | null;
}

export interface AttemptCountsStub {
  success: number;
  failed: number;
  crashed: number;
}

export interface Settings {
  schema_version: number;
  workspace_root: string | null;
  default_workers: number | null;
  max_concurrent_runs: number | null;
  telemetry_opt_in: boolean;
}

export type UiErrorKind =
  | "workspace_unavailable"
  | "io"
  | "internal";

export interface UiError {
  kind: UiErrorKind;
  // serde tagged variants put unnamed inner data under "0"
  0?: string;
}

export function uiErrorMessage(e: unknown): string {
  if (e && typeof e === "object" && "kind" in e) {
    const ue = e as UiError;
    return `[${ue.kind}] ${ue[0] ?? ""}`;
  }
  return String(e);
}
```

- [ ] **Step 8.2: Typed `invoke` wrapper**

Create `apps/rowforge-studio/src/ipc/client.ts`:

```ts
import { invoke } from "@tauri-apps/api/core";
import type { ExecSummary, Settings, Workspace } from "./types";

export const ipc = {
  workspace_open: (args: { path: string | null }) =>
    invoke<Workspace>("workspace_open", args),
  exec_list: () => invoke<ExecSummary[]>("exec_list"),
  workspace_settings_load: () => invoke<Settings>("workspace_settings_load"),
  workspace_settings_save: (args: { settings: Settings }) =>
    invoke<void>("workspace_settings_save", args),
};
```

- [ ] **Step 8.3: TanStack Query hooks**

Create `apps/rowforge-studio/src/ipc/queries.ts`:

```ts
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ipc } from "./client";
import type { Settings } from "./types";

export const useSettings = () =>
  useQuery({
    queryKey: ["settings"],
    queryFn: ipc.workspace_settings_load,
  });

export const useExecList = (enabled: boolean) =>
  useQuery({
    queryKey: ["exec_list"],
    queryFn: ipc.exec_list,
    enabled,
  });

export const useOpenWorkspace = () => {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (path: string | null) => ipc.workspace_open({ path }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["exec_list"] });
      qc.invalidateQueries({ queryKey: ["settings"] });
    },
  });
};

export const useSaveSettings = () => {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (settings: Settings) =>
      ipc.workspace_settings_save({ settings }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["settings"] }),
  });
};
```

- [ ] **Step 8.4: Verify TS compiles**

```bash
cd apps/rowforge-studio
pnpm tsc -b
```

Expected: PASS for types/client/queries (App.tsx still fails because BootGate not yet created; Task 9 fixes).

- [ ] **Step 8.5: Commit**

```bash
git add apps/rowforge-studio/src/ipc
git commit -m "studio-shell: IPC client + TanStack Query hooks

Hand-written TS mirrors of studio-core types (Plan 3 may introduce
specta auto-gen). Four typed invoke wrappers + four React hooks
covering workspace open / exec list / settings load+save."
```

---

## Task 9: BootGate + Workspace Picker page (W-7)

**Files:**
- Create: `apps/rowforge-studio/src/pages/BootGate.tsx`
- Create: `apps/rowforge-studio/src/pages/WorkspacePicker.tsx`

- [ ] **Step 9.1: BootGate route logic**

Create `apps/rowforge-studio/src/pages/BootGate.tsx`:

```tsx
import { useEffect, useState } from "react";
import { useSettings, useOpenWorkspace } from "@/ipc/queries";
import { WorkspacePicker } from "./WorkspacePicker";
import { ExecListPage } from "./ExecList";
import { uiErrorMessage } from "@/ipc/types";

type Phase = "loading" | "picker" | "ready" | "error";

export function BootGate() {
  const settings = useSettings();
  const openMut = useOpenWorkspace();
  const [phase, setPhase] = useState<Phase>("loading");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (settings.isLoading) return;
    if (settings.isError) {
      setError(uiErrorMessage(settings.error));
      setPhase("error");
      return;
    }
    const stored = settings.data?.workspace_root ?? null;
    if (stored) {
      openMut.mutate(stored, {
        onSuccess: () => setPhase("ready"),
        onError: (e) => {
          // Stored workspace bad; fall back to picker.
          console.warn("autoload failed:", uiErrorMessage(e));
          setPhase("picker");
        },
      });
    } else {
      setPhase("picker");
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settings.isLoading, settings.isError]);

  if (phase === "loading") {
    return (
      <div className="grid h-screen place-items-center text-muted-foreground">
        Loading…
      </div>
    );
  }
  if (phase === "error") {
    return (
      <div className="grid h-screen place-items-center text-red-400">
        {error ?? "unknown error"}
      </div>
    );
  }
  if (phase === "picker") {
    return <WorkspacePicker onPicked={() => setPhase("ready")} />;
  }
  return <ExecListPage />;
}
```

- [ ] **Step 9.2: WorkspacePicker page**

Create `apps/rowforge-studio/src/pages/WorkspacePicker.tsx`:

```tsx
import { open } from "@tauri-apps/plugin-dialog";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { useOpenWorkspace } from "@/ipc/queries";
import { uiErrorMessage } from "@/ipc/types";
import { Inbox } from "lucide-react";

export function WorkspacePicker({ onPicked }: { onPicked: () => void }) {
  const openMut = useOpenWorkspace();

  const pickFolder = async () => {
    const selected = await open({ directory: true, multiple: false });
    if (typeof selected !== "string") return;
    openMut.mutate(selected, { onSuccess: onPicked });
  };

  const useDefault = () => {
    openMut.mutate(null, { onSuccess: onPicked });
  };

  return (
    <div className="grid h-screen place-items-center">
      <Card className="flex w-[480px] flex-col items-center gap-6 p-10">
        <Inbox className="h-12 w-12 text-muted-foreground" />
        <div className="text-center">
          <h1 className="text-xl font-medium">No workspace yet</h1>
          <p className="mt-2 text-sm text-muted-foreground">
            rowforge stores executions and per-row outcomes on disk. Pick
            an existing workspace or create one at <code>~/.rowforge</code>.
          </p>
        </div>
        <div className="flex w-full flex-col gap-2">
          <Button onClick={pickFolder} disabled={openMut.isPending}>
            Open folder…
          </Button>
          <Button onClick={useDefault} variant="outline" disabled={openMut.isPending}>
            Use ~/.rowforge
          </Button>
        </div>
        {openMut.isError && (
          <p className="text-sm text-red-400">{uiErrorMessage(openMut.error)}</p>
        )}
      </Card>
    </div>
  );
}
```

- [ ] **Step 9.3: TS compile check**

```bash
cd apps/rowforge-studio
pnpm tsc -b
```

Expected: ExecListPage import still missing — Task 10 fixes. Use `--noEmit` and continue.

Actually skip the check; Task 10 closes the loop.

- [ ] **Step 9.4: Commit**

```bash
git add apps/rowforge-studio/src/pages/BootGate.tsx apps/rowforge-studio/src/pages/WorkspacePicker.tsx
git commit -m "studio-shell: boot gate + workspace picker (W-7)

BootGate autoloads settings.workspace_root if set; else shows
Workspace Picker. Picker offers Open folder dialog or use default
~/.rowforge."
```

---

## Task 10: AppShell layout + Exec list page (W-1)

**Files:**
- Create: `apps/rowforge-studio/src/layout/AppShell.tsx`
- Create: `apps/rowforge-studio/src/layout/Sidebar.tsx`
- Create: `apps/rowforge-studio/src/layout/Header.tsx`
- Create: `apps/rowforge-studio/src/pages/ExecList.tsx`

- [ ] **Step 10.1: Sidebar**

Create `apps/rowforge-studio/src/layout/Sidebar.tsx`:

```tsx
import { cn } from "@/lib/utils";
import { Activity, Settings as SettingsIcon } from "lucide-react";

export function Sidebar() {
  return (
    <aside className="w-44 border-r border-border bg-neutral-925">
      <nav className="flex flex-col gap-1 p-3 text-sm">
        <div className="px-2 pb-1 pt-2 text-xs uppercase text-muted-foreground">
          Workspace
        </div>
        <SideLink icon={<Activity className="h-4 w-4" />} label="Executions" active />
        <SideLink icon={<SettingsIcon className="h-4 w-4" />} label="Settings" disabled />

        <div className="mt-4 px-2 pb-1 text-xs uppercase text-muted-foreground">
          Authoring
        </div>
        <SideLink label="Handlers" disabled hint="Coming soon" />
      </nav>
    </aside>
  );
}

function SideLink({
  icon,
  label,
  active,
  disabled,
  hint,
}: {
  icon?: React.ReactNode;
  label: string;
  active?: boolean;
  disabled?: boolean;
  hint?: string;
}) {
  return (
    <div
      className={cn(
        "flex items-center gap-2 rounded px-2 py-1.5",
        active && "bg-primary/20 text-foreground",
        !active && !disabled && "text-muted-foreground hover:bg-muted/40",
        disabled && "text-muted-foreground/50"
      )}
    >
      {icon}
      <span>{label}</span>
      {hint && <span className="ml-auto text-[10px]">{hint}</span>}
    </div>
  );
}
```

- [ ] **Step 10.2: Header**

Create `apps/rowforge-studio/src/layout/Header.tsx`:

```tsx
import type { Workspace } from "@/ipc/types";

export function Header({ workspace }: { workspace: Workspace | null }) {
  return (
    <header className="flex h-12 items-center border-b border-border px-4 text-sm">
      <span className="font-mono text-muted-foreground">
        {workspace?.root ?? "—"}
      </span>
      <span className="ml-2 text-xs text-muted-foreground/70">
        {workspace ? `schema v${workspace.schema_version}` : ""}
      </span>
    </header>
  );
}
```

- [ ] **Step 10.3: AppShell**

Create `apps/rowforge-studio/src/layout/AppShell.tsx`:

```tsx
import { Header } from "./Header";
import { Sidebar } from "./Sidebar";
import type { Workspace } from "@/ipc/types";

export function AppShell({
  workspace,
  children,
}: {
  workspace: Workspace | null;
  children: React.ReactNode;
}) {
  return (
    <div className="grid h-screen grid-cols-[auto_1fr] grid-rows-[auto_1fr]">
      <div className="col-span-2">
        <Header workspace={workspace} />
      </div>
      <Sidebar />
      <main className="overflow-auto">{children}</main>
    </div>
  );
}
```

- [ ] **Step 10.4: ExecList page (W-1)**

Create `apps/rowforge-studio/src/pages/ExecList.tsx`:

```tsx
import { useExecList } from "@/ipc/queries";
import { AppShell } from "@/layout/AppShell";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, Thead, Tr, Th, Td } from "@/components/ui/table";
import { uiErrorMessage } from "@/ipc/types";
import { useQuery } from "@tanstack/react-query";
import { ipc } from "@/ipc/client";

function useWorkspace() {
  // workspace_open also returns Workspace; we surface the last-opened
  // value via React state owned by BootGate. For simplicity here, we
  // requery settings.workspace_root and trust BootGate's autoload.
  return useQuery({
    queryKey: ["settings_workspace_root"],
    queryFn: async () => (await ipc.workspace_settings_load()).workspace_root,
  });
}

export function ExecListPage() {
  const wsRoot = useWorkspace();
  const list = useExecList(true);

  const workspace = wsRoot.data
    ? { root: wsRoot.data, schema_version: 1 }
    : null;

  return (
    <AppShell workspace={workspace}>
      <div className="p-6">
        <div className="mb-4 flex items-center justify-between">
          <h1 className="text-lg font-medium">Executions</h1>
        </div>

        {list.isLoading && (
          <div className="space-y-2">
            <Skeleton className="h-9 w-full" />
            <Skeleton className="h-9 w-full" />
            <Skeleton className="h-9 w-full" />
          </div>
        )}

        {list.isError && (
          <div className="rounded border border-red-500/40 bg-red-500/10 p-3 text-sm text-red-300">
            {uiErrorMessage(list.error)}
          </div>
        )}

        {list.data && list.data.length === 0 && (
          <div className="rounded-lg border border-dashed border-border p-10 text-center text-sm text-muted-foreground">
            No executions yet. Create one with{" "}
            <code>rowforge exec start</code> in a terminal.
          </div>
        )}

        {list.data && list.data.length > 0 && (
          <Table>
            <Thead>
              <Tr>
                <Th>Name</Th>
                <Th>Created</Th>
                <Th>Rows</Th>
                <Th>Attempts</Th>
              </Tr>
            </Thead>
            <tbody>
              {list.data.map((e) => (
                <Tr key={e.id}>
                  <Td className="font-mono">{e.name || "—"}</Td>
                  <Td className="font-mono">
                    {new Date(e.created_at).toISOString().replace("T", " ").slice(0, 16)}
                  </Td>
                  <Td className="text-right">{e.input_rows ?? "—"}</Td>
                  <Td className="text-right">{e.attempts_count}</Td>
                </Tr>
              ))}
            </tbody>
          </Table>
        )}
      </div>
    </AppShell>
  );
}
```

- [ ] **Step 10.5: TS compile check end-to-end**

```bash
cd apps/rowforge-studio
pnpm tsc -b
```

Expected: 0 errors.

- [ ] **Step 10.6: Frontend build check**

```bash
pnpm build
```

Expected: Vite produces `dist/` with no errors.

- [ ] **Step 10.7: Commit**

```bash
git add apps/rowforge-studio/src/layout apps/rowforge-studio/src/pages/ExecList.tsx
git commit -m "studio-shell: AppShell + Exec list page (W-1)

Persistent sidebar (Executions active, Settings/Handlers disabled),
header showing workspace path + schema version, exec table from
useExecList(). Skeleton on load, empty state, error banner."
```

---

## Task 11: Rust contract test for `workspace_open` JSON shape

Catch a future spec drift where `Workspace` field names change without
updating the TS mirror.

**Files:**
- Create: `apps/rowforge-studio/src-tauri/tests/ipc_contract.rs`

- [ ] **Step 11.1: Write the test**

```rust
//! Lock the JSON shape that crosses the Tauri IPC boundary.
//!
//! Hand-written TS mirrors at `apps/rowforge-studio/src/ipc/types.ts`
//! depend on these keys. Any rename here without updating TS is a UI
//! breakage; this test forces them to move together.

use rowforge_studio_core::{ExecSummary, Workspace};
use std::path::PathBuf;

#[test]
fn workspace_json_keys() {
    let w = Workspace { root: PathBuf::from("/tmp/ws"), schema_version: 1 };
    let v = serde_json::to_value(&w).unwrap();
    assert!(v.get("root").is_some(), "root key");
    assert!(v.get("schema_version").is_some(), "schema_version key");
}

#[test]
fn exec_summary_json_keys() {
    // Construct via From<&Execution>? Execution is private to core's exec store.
    // For shape-only test, construct directly using exhaustive struct
    // literal (this test will need to be updated when fields are added —
    // intentional).
    let json = r#"{
        "id":"e1","name":"x","created_at":"2026-05-24T12:00:00Z",
        "input_rows":42,"attempts_count":0,
        "last_attempt_state":null,"last_attempt_counts":null
    }"#;
    let parsed: ExecSummary = serde_json::from_str(json).expect("deserialize");
    assert_eq!(parsed.id, "e1");
    assert_eq!(parsed.input_rows, Some(42));
}
```

Note: `ExecSummary` is `#[non_exhaustive]` — we can deserialize but cannot construct externally. The JSON-string approach above tests the deserialization shape; if the spec adds a required field (no `#[serde(default)]`), this test breaks loudly. Acceptable: the test's job is to catch shape drift.

- [ ] **Step 11.2: Run**

```bash
cargo test -p rowforge-studio --test ipc_contract
```

Expected: PASS (2 tests).

- [ ] **Step 11.3: Commit**

```bash
git add apps/rowforge-studio/src-tauri/tests/ipc_contract.rs
git commit -m "studio-shell: IPC JSON contract test

Locks the field names of Workspace and ExecSummary so a rename in
rowforge-studio-core forces a matching update in TS mirrors
(apps/rowforge-studio/src/ipc/types.ts)."
```

---

## Task 12: React smoke test (Vitest)

**Files:**
- Create: `apps/rowforge-studio/vitest.config.ts`
- Create: `apps/rowforge-studio/src/__tests__/exec-list.test.tsx`
- Create: `apps/rowforge-studio/src/__tests__/setup.ts`

- [ ] **Step 12.1: Vitest config**

Create `apps/rowforge-studio/vitest.config.ts`:

```ts
import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import path from "node:path";

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: { "@": path.resolve(__dirname, "./src") },
  },
  test: {
    environment: "jsdom",
    setupFiles: ["./src/__tests__/setup.ts"],
  },
});
```

- [ ] **Step 12.2: Setup file**

Create `apps/rowforge-studio/src/__tests__/setup.ts`:

```ts
import "@testing-library/jest-dom";
import { vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));
```

- [ ] **Step 12.3: Exec list smoke test**

Create `apps/rowforge-studio/src/__tests__/exec-list.test.tsx`:

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { ExecListPage } from "@/pages/ExecList";

describe("ExecList", () => {
  let qc: QueryClient;

  beforeEach(() => {
    vi.clearAllMocks();
    qc = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
  });

  function wrap(node: React.ReactNode) {
    return <QueryClientProvider client={qc}>{node}</QueryClientProvider>;
  }

  it("renders empty state when list is []", async () => {
    (invoke as any)
      .mockImplementationOnce(() => Promise.resolve({ workspace_root: "/tmp/ws", schema_version: 1, default_workers: null, max_concurrent_runs: null, telemetry_opt_in: false }))
      .mockImplementationOnce(() => Promise.resolve([]));
    render(wrap(<ExecListPage />));
    expect(await screen.findByText(/No executions yet/i)).toBeInTheDocument();
  });

  it("renders rows from invoke result", async () => {
    (invoke as any)
      .mockImplementationOnce(() => Promise.resolve({ workspace_root: "/tmp/ws", schema_version: 1, default_workers: null, max_concurrent_runs: null, telemetry_opt_in: false }))
      .mockImplementationOnce(() =>
        Promise.resolve([
          {
            id: "e1",
            name: "smoke",
            created_at: "2026-05-24T12:00:00Z",
            input_rows: 5,
            attempts_count: 0,
            last_attempt_state: null,
            last_attempt_counts: null,
          },
        ])
      );
    render(wrap(<ExecListPage />));
    expect(await screen.findByText("smoke")).toBeInTheDocument();
  });
});
```

- [ ] **Step 12.4: Run**

```bash
cd apps/rowforge-studio
pnpm test
```

Expected: 2 passed.

- [ ] **Step 12.5: Commit**

```bash
git add apps/rowforge-studio/vitest.config.ts apps/rowforge-studio/src/__tests__
git commit -m "studio-shell: vitest + ExecListPage smoke tests

Two tests covering empty state and populated table. Mocks
@tauri-apps/api/core invoke."
```

---

## Task 13: HMR sanity (human verification only)

This task documents the manual smoke for `pnpm tauri dev`. No code, no commits. The user must execute these themselves — the agent cannot launch a Tauri window.

- [ ] **Step 13.1: Print the smoke procedure**

Produce a `apps/rowforge-studio/HUMAN_SMOKE.md` with this content:

```markdown
# Manual smoke check (Plan 02)

Run these from `apps/rowforge-studio/`:

1. `pnpm install` (idempotent)
2. `pnpm tauri dev`

Expected on first launch:
- A 1280×800 window titled "rowforge Studio" appears.
- Workspace Picker shows: "No workspace yet" + Inbox icon + two buttons.
- Header path reads "—" (no workspace open yet).

Click [Open folder…]:
- macOS file dialog opens to select a directory.
- Pick any empty directory.
- App routes to Executions page; header path shows the chosen folder.
- Table shows the empty state "No executions yet."

Test HMR:
- Edit `src/pages/ExecList.tsx` header text from "Executions" to "Executions (HMR)".
- The window updates within ~500ms without losing the workspace state.

Test Rust hot rebuild:
- Edit `src-tauri/src/commands.rs` comment.
- Tauri rebuilds (~10s); window restarts; workspace path was persisted to
  settings.json so picker is bypassed and Exec List page reloads directly.

Inspect settings on disk (macOS):
- `~/Library/Application\ Support/com.lemotw.rowforge.studio/rowforge-studio/settings.json`
- Should contain `"workspace_root": "<your path>"`.

Test reopen with bad workspace_root:
- Quit app.
- Edit the JSON, set `workspace_root` to "/does/not/exist".
- Relaunch: BootGate's autoload fails, Picker shows again. (Stored path is
  preserved on failure — no auto-clear in Plan 2.)
```

- [ ] **Step 13.2: Commit**

```bash
git add apps/rowforge-studio/HUMAN_SMOKE.md
git commit -m "studio-shell: human smoke procedure for tauri dev

Agent cannot launch a Tauri window; this file is the human checklist
for verifying Plan 2 end-to-end."
```

---

## Task 14: Workspace-wide build + test smoke

- [ ] **Step 14.1: Cargo build**

```bash
cd /Users/lemo/code/lemo/repo/rowforge
cargo build
```

Expected: 5 crates (added: `rowforge-studio`) compile. 0 errors. Warnings about unused `tauri-plugin-shell` import in main.rs are OK if any.

- [ ] **Step 14.2: Cargo test**

```bash
cargo test
```

Expected: 161 (Plan 1) + 3 (settings) + 2 (ipc_contract) = 166 passed.

- [ ] **Step 14.3: Frontend build**

```bash
cd apps/rowforge-studio && pnpm build
```

Expected: dist/ produced.

- [ ] **Step 14.4: Frontend tests**

```bash
pnpm test
```

Expected: 2 passed.

- [ ] **Step 14.5: No commits**

Just verification. If anything fails, escalate.

---

## Plan 02 acceptance

1. `cargo test` workspace-wide passes (166+ tests).
2. `cd apps/rowforge-studio && pnpm build` produces a dist.
3. `pnpm test` passes (2 tests).
4. `pnpm tauri dev` launches a window. Workspace Picker appears on first
   launch with no `~/Library/.../settings.json`.
5. After picking a folder, the Exec list page renders (empty state if
   workspace has no execs; rows if it does).
6. Settings persist across app restart.
7. Tauri command surface matches spec §5.5:
   - `workspace_open(opts)` → `Workspace`
   - `exec_list(filter)` → `Vec<ExecSummary>`
   - `workspace_settings_load()` → `Settings`
   - `workspace_settings_save(s)` → `()`

## What lands in Plan 03 next

- `StudioCore::show(execution_id)` → `ExecDetail`
- `StudioCore::attempt(exec, attempt)` → `AttemptDetail`
- `StudioCore::rollup(execution_id)` → `ExecRollup`
- `StudioCore::failed_page(query)` → `FailedRowPage` (linear scan; v2 index later)
- `StudioCore::row_history(exec, seq)` → `RowHistory`
- Backfill `ExecSummary.attempts_count` / `last_attempt_state` / `last_attempt_counts`
- Warm-tier mtime probe + 30s TTL caching
- Workspace `schema_version` hard pin
- Rename `UiError::WorkspaceUnavailable` → `WorkspaceLocked`; classify CoreError per call site
- Introduce `ExecutionId` newtype
- Frontend routes `/exec/:id` and `/exec/:id/attempt/:aid` (Tabs: Attempts, Rollup, Bindings; Sub-tabs: Live disabled, Failed rows, Errors by code, Artifacts)

## Open questions Plan 02 deliberately punts

1. **`UiError` JSON shape — first actual contact in Plan 2.** Plan 1 defined
   `UiError` with tuple variants under `#[serde(tag = "kind")]`. Serde
   accepts this at compile time but the runtime JSON shape for tuple
   variants under internal tagging is unusual (the inner String may
   appear under key `"0"` or trigger an error). Task 11's `ipc_contract`
   test should add a `UiError` serialization round-trip; if the shape
   isn't `{ "kind": "...", "0": "..." }`, adjust `src/ipc/types.ts`
   accordingly **before** Task 9 lands. Plan 3 should refactor `UiError`
   to struct variants (`WorkspaceLocked { by: String }` per spec §5.3),
   which sidesteps this entirely.
2. **Workspace switching mid-session.** Header path is read-only in Plan 2.
   Plan 3 should let user click it to reopen Picker.
2. **Persistence of bad workspace_root.** Currently if autoload fails the
   stored path is preserved (Picker shows again). Could auto-clear with
   user consent.
3. **Theme.** Dark-only in Plan 2 (no light-mode tokens published yet).
4. **Workspace locking against parallel CLI use.** Plan 1 / Plan 2 don't
   acquire an exclusive lock. Plan 3+ may want a `flock(2)`-style guard
   via `fs2` (already a workspace dep).
5. **`StudioCore` Drop semantics.** Currently no explicit close. Plan 4
   adds Drop to soft-cancel active runs at shutdown (spec §3.6); for now
   process exit is sufficient.
