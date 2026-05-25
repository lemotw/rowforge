# Studio Plan 07 — Handler Authoring (Static Surface) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let users manage handlers from Studio — list, view, edit in external editor, reveal in Finder, scaffold from 3 templates, rename (lazy), delete (typed-token). No new runtimes (build/smoke are Plans 8/9).

**Architecture:** New `handler` module in studio-core; 7 new Tauri commands; Handlers route with list + detail pages; 3 scaffold templates embedded via `include_str!`; `Settings.preferred_editor` re-added (legitimate this time) for the 4-tier editor resolver.

**Tech Stack:** Same as Plan 6. Reuses `which` (Plan 5), `shell-words`-style parsing (write inline since `shell_words` was dropped after Plan 5's manifest.rs refactor — re-add as workspace dep). Reuses Plan 5's `ManifestReport` types + `ManifestReportView` component. Reuses Plan 4's `CancelDialog` typed-token pattern.

**Spec references:** Design doc at `docs/superpowers/specs/2026-05-25-studio-plan-07-handler-authoring-static-design.md`. Spec part-8 §8.1, §8.3, §8.4.1, §8.4.6, §8.5, §8.6.

---

## Decisions resolved during brainstorm

| Decision | Choice |
|---|---|
| Plan 7 subsystem from Part 8 | A (Static surface) only — B (Build) → Plan 8, C (Smoke) → Plan 9 |
| Inside A scope | Read + Create + Destroy 全集 |
| Editor resolution | 4-tier: `Settings.preferred_editor` → `$VISUAL` → `$EDITOR` → probe (code/cursor/subl/zed) → `EditorNotFound` |
| Delete friction | Typed-token (handler name) |
| Rename + existing attempts | Lazy — change dir only, don't touch `handler_instance.source_snapshot_dir` |

---

## File structure

### New — `rowforge-studio-core`
- `crates/rowforge-studio-core/src/handler.rs` — types (HandlerSummary / HandlerDetail / ScaffoldArgs / ScaffoldTemplate / SourceFileSummary / ManifestStatus); 7 free functions (list / show / open_editor / reveal / scaffold / delete / rename); editor resolver helper
- `crates/rowforge-studio-core/src/handler_templates/go_stdio/` — 3 files (rowforge.yaml, handler.go, go.mod) embedded via `include_str!`
- `crates/rowforge-studio-core/src/handler_templates/go_batch/` — 3 files
- `crates/rowforge-studio-core/src/handler_templates/empty/` — 2 files

### Modified — `rowforge-studio-core`
- `src/lib.rs` — register `handler` module; 7 new methods on `StudioCore`
- `src/error.rs` — 4 new `UiError` variants: `EditorNotFound`, `HandlerNotFound`, `HandlerExists`, `InvalidHandlerName`
- `src/settings.rs` — `preferred_editor: Option<String>` field + Default
- `Cargo.toml` — re-add `shell-words` (dropped in Plan 5's manifest.rs refactor)

### Modified — Tauri
- `apps/rowforge-studio/src-tauri/src/commands.rs` — 7 new commands
- `apps/rowforge-studio/src-tauri/src/lib.rs` — register in `invoke_handler!`

### New — React UI
- `apps/rowforge-studio/src/pages/HandlerList.tsx` — `/handlers` route
- `apps/rowforge-studio/src/pages/HandlerDetail.tsx` — `/handlers/:name` route
- `apps/rowforge-studio/src/components/ScaffoldHandlerDialog.tsx`
- `apps/rowforge-studio/src/components/RenameHandlerDialog.tsx`
- `apps/rowforge-studio/src/components/DeleteHandlerDialog.tsx` — typed-token
- `apps/rowforge-studio/src/ipc/use-handlers.ts` — TanStack hooks for the 7 ops

### Modified — React UI
- `apps/rowforge-studio/src/App.tsx` — register `/handlers` + `/handlers/:name` routes
- `apps/rowforge-studio/src/layout/Sidebar.tsx` — enable Handlers link (anchored disabled in Plans 1-6)
- `apps/rowforge-studio/src/ipc/types.ts` — TS mirrors for handler types + new UiError variants + Settings.preferred_editor
- `apps/rowforge-studio/src/ipc/client.ts` — 7 new ipc wrappers
- `apps/rowforge-studio/src/components/SettingsForm.tsx` — Editor section (new row in the form)
- `apps/rowforge-studio/HUMAN_SMOKE.md` — Plan 7 walkthrough

### Modified — Spec docs (en + zh-Hant)
- `docs/spec/studio/part-2-model.md` — Settings gains `preferred_editor`; footnote on lazy rename in `ExecSummary` / `handler_instance` discussion
- `docs/spec/studio/part-5-api.md` — §5.3 four new UiError variants; §5.6 mentions `preferred_editor`
- `docs/spec/studio/part-7-ui.md` — IA: `/handlers` + `/handlers/:name` move from "anchored" to active routes; flows for scaffold/rename/delete
- `docs/spec/studio/part-8-handler-authoring.md` — cross-refs from §8.5 / §8.6 to actual paths now that Plan 7 ships them

### Out of scope for Plan 07
- Build runtime → Plan 8
- Smoke test runtime → Plan 9
- In-Studio code editor (non-goal v1)
- Fixture-file smoke inputs
- `rowforge pack` from Studio
- Structured manifest editor
- Cross-workspace handler registry

---

## Task 1: `handler` module scaffold

**Files:** Create `crates/rowforge-studio-core/src/handler.rs`; modify `src/lib.rs`

- [ ] **Step 1: Add `shell-words` dep**

```toml
# crates/rowforge-studio-core/Cargo.toml
shell-words = "1"
```

(Re-add — Plan 5 had it then removed after the manifest.rs refactor delegated to rowforge-core.)

- [ ] **Step 2: Create `handler.rs` with public types**

```rust
//! Handler management — discover, view, scaffold, rename, delete handlers
//! under `<workspace>/handlers/*`. Spec part 8 §8.3 / §8.5.
//!
//! Build + smoke-test runtimes are Plans 8 + 9.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManifestStatus { Valid, Invalid, Missing }

#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandlerDetail {
    pub summary: HandlerSummary,
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
    pub primary_field: String,
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScaffoldTemplate { GoStdio, GoBatch, Empty }

/// Regex check: handler names allowed [a-z0-9-]+.
pub(crate) fn validate_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_validator_accepts_canonical() {
        assert!(validate_name("golang-billing-channel"));
        assert!(validate_name("my-handler-v2"));
        assert!(validate_name("h1"));
    }

    #[test]
    fn name_validator_rejects_uppercase_and_path_chars() {
        assert!(!validate_name(""));
        assert!(!validate_name("UpperCase"));
        assert!(!validate_name("with space"));
        assert!(!validate_name("../etc"));
        assert!(!validate_name("a/b"));
        assert!(!validate_name("with_underscore"));
    }
}
```

- [ ] **Step 3: Register module in lib.rs**

```rust
pub mod handler;
pub use handler::{
    HandlerSummary, HandlerDetail, SourceFileSummary,
    ManifestStatus, ScaffoldArgs, ScaffoldTemplate,
};
```

- [ ] **Step 4: Build + test**

```bash
cd /Users/lemo/code/lemo/repo/rowforge
cargo build -p rowforge-studio-core
cargo test -p rowforge-studio-core handler
```

Expected: 2 tests pass; build clean.

- [ ] **Step 5: Commit**

```
studio-core: handler module scaffold + types

Plan 7 T1. Public types only — HandlerSummary, HandlerDetail,
SourceFileSummary, ManifestStatus, ScaffoldArgs, ScaffoldTemplate.
Plus a validate_name helper enforcing [a-z0-9-]+ for handler names
(used by scaffold / rename / delete to reject path-traversal).

Free functions (list / show / scaffold / etc) land in T2-T8.
```

---

## Task 2: New `UiError` variants

**Files:** Modify `crates/rowforge-studio-core/src/error.rs`; TS mirrors

- [ ] **Step 1: Tests first**

Append to `error.rs` test module:

```rust
#[test]
fn editor_not_found_serializes() {
    let e = UiError::EditorNotFound;
    let v = serde_json::to_value(&e).unwrap();
    assert_eq!(v["kind"], json!("editor_not_found"));
}

#[test]
fn handler_not_found_carries_name() {
    let e = UiError::HandlerNotFound { name: "foo".into() };
    let v = serde_json::to_value(&e).unwrap();
    assert_eq!(v["kind"], json!("handler_not_found"));
    assert_eq!(v["message"]["name"], json!("foo"));
}

#[test]
fn handler_exists_carries_name() {
    let e = UiError::HandlerExists { name: "foo".into() };
    let v = serde_json::to_value(&e).unwrap();
    assert_eq!(v["kind"], json!("handler_exists"));
    assert_eq!(v["message"]["name"], json!("foo"));
}

#[test]
fn invalid_handler_name_carries_name() {
    let e = UiError::InvalidHandlerName { name: "Bad Name".into() };
    let v = serde_json::to_value(&e).unwrap();
    assert_eq!(v["kind"], json!("invalid_handler_name"));
    assert_eq!(v["message"]["name"], json!("Bad Name"));
}
```

- [ ] **Step 2: Add the variants**

```rust
#[derive(Debug, Error, Serialize)]
#[serde(tag = "kind", content = "message", rename_all = "snake_case")]
#[non_exhaustive]
pub enum UiError {
    // … existing variants …

    /// 4-tier editor resolution exhausted (preferred_editor + VISUAL +
    /// EDITOR + probes all failed).
    #[error("editor not found")]
    EditorNotFound,

    /// `<workspace>/handlers/<name>` doesn't exist.
    #[error("handler not found: {name}")]
    HandlerNotFound { name: String },

    /// Scaffold target / rename destination already exists.
    #[error("handler already exists: {name}")]
    HandlerExists { name: String },

    /// Handler name doesn't match `[a-z0-9-]+`.
    #[error("invalid handler name: {name}")]
    InvalidHandlerName { name: String },
}
```

- [ ] **Step 3: TS mirror**

Add to `apps/rowforge-studio/src/ipc/types.ts`:

```ts
export type UiError =
  // ... existing variants ...
  | { kind: "editor_not_found"; message: string }     // tuple unit serializes as ""; or check actual serde output
  | { kind: "handler_not_found"; message: { name: string } }
  | { kind: "handler_exists"; message: { name: string } }
  | { kind: "invalid_handler_name"; message: { name: string } };
```

Update `uiErrorMessage` switch to handle each:

```ts
case "editor_not_found":
  return `[editor_not_found] No editor found — set Settings.preferred_editor or VISUAL/EDITOR`;
case "handler_not_found":
  return `[handler_not_found] '${e.message.name}' is not under <workspace>/handlers/`;
case "handler_exists":
  return `[handler_exists] '${e.message.name}' already exists`;
case "invalid_handler_name":
  return `[invalid_handler_name] '${e.message.name}' must match [a-z0-9-]+`;
```

Extend `UiErrorKind` union.

- [ ] **Step 4: Verify**

```bash
cargo test -p rowforge-studio-core error
cd apps/rowforge-studio && pnpm tsc -b
```

- [ ] **Step 5: Commit**

```
studio: UiError gains EditorNotFound / Handler{NotFound,Exists,InvalidName}

Plan 7 T2. Four new variants for the handler-management surface
landing in T3+. EditorNotFound is unit (no payload); the other
three carry { name } structs. TS mirror + uiErrorMessage updated
to render each.

Plus 4 serde tests locking the JSON shapes.
```

---

## Task 3: `handler_list` + `handler_show`

**Files:** Modify `crates/rowforge-studio-core/src/handler.rs`; tests in `tests/foundation.rs`

- [ ] **Step 1: Tests first**

Append to `tests/foundation.rs`:

```rust
#[test]
fn handler_list_finds_dirs_under_workspace_handlers() {
    let tmp = tempdir::TempDir::new("rfs-plan7-hl").unwrap();
    let handlers_dir = tmp.path().join("handlers");
    std::fs::create_dir_all(&handlers_dir).unwrap();

    // Three test handlers: valid, no-manifest, invalid-manifest.
    let valid = handlers_dir.join("alpha");
    std::fs::create_dir_all(&valid).unwrap();
    std::fs::write(valid.join("rowforge.yaml"),
        "name: alpha\nversion: 0.1.0\nlanguage: go\nentry:\n  cmd: [\"./alpha\"]\n").unwrap();

    let no_manifest = handlers_dir.join("bravo");
    std::fs::create_dir_all(&no_manifest).unwrap();

    let invalid = handlers_dir.join("charlie");
    std::fs::create_dir_all(&invalid).unwrap();
    std::fs::write(invalid.join("rowforge.yaml"), "this: is: bad: yaml: :::").unwrap();

    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().into()),
    ).unwrap();
    let mut list = core.handler_list().unwrap();
    list.sort_by(|a, b| a.name.cmp(&b.name));

    assert_eq!(list.len(), 3);
    assert_eq!(list[0].name, "alpha");
    assert_eq!(list[0].manifest_status, rowforge_studio_core::ManifestStatus::Valid);
    assert_eq!(list[1].name, "bravo");
    assert_eq!(list[1].manifest_status, rowforge_studio_core::ManifestStatus::Missing);
    assert_eq!(list[2].name, "charlie");
    assert_eq!(list[2].manifest_status, rowforge_studio_core::ManifestStatus::Invalid);
}

#[test]
fn handler_list_handles_missing_handlers_dir() {
    let tmp = tempdir::TempDir::new("rfs-plan7-no-hd").unwrap();
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().into()),
    ).unwrap();
    // No `handlers/` directory → empty list, not an error.
    assert_eq!(core.handler_list().unwrap().len(), 0);
}

#[test]
fn handler_show_returns_manifest_and_source_files() {
    let tmp = tempdir::TempDir::new("rfs-plan7-hs").unwrap();
    let h = tmp.path().join("handlers").join("alpha");
    std::fs::create_dir_all(&h).unwrap();
    std::fs::write(h.join("rowforge.yaml"),
        "name: alpha\nversion: 0.1.0\nentry:\n  cmd: [\"./alpha\"]\n").unwrap();
    std::fs::write(h.join("handler.go"), "package main").unwrap();
    std::fs::write(h.join("go.mod"), "module alpha\ngo 1.22").unwrap();

    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().into()),
    ).unwrap();
    let detail = core.handler_show("alpha").unwrap();
    assert_eq!(detail.summary.name, "alpha");
    assert_eq!(detail.summary.manifest_status, rowforge_studio_core::ManifestStatus::Valid);
    assert!(detail.manifest.is_some());
    let names: Vec<&str> = detail.source_files.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"handler.go"));
    assert!(names.contains(&"go.mod"));
    // rowforge.yaml is the manifest, not "source" — excluded from source_files.
    assert!(!names.contains(&"rowforge.yaml"));
}

#[test]
fn handler_show_errors_on_unknown_name() {
    let tmp = tempdir::TempDir::new("rfs-plan7-hs-nf").unwrap();
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().into()),
    ).unwrap();
    let err = core.handler_show("ghost").unwrap_err();
    assert!(matches!(err, rowforge_studio_core::UiError::HandlerNotFound { .. }));
}
```

Run: `cargo test -p rowforge-studio-core handler_list handler_show`
Expected: compile errors (methods don't exist).

- [ ] **Step 2: Implement `handler_list` + `handler_show`**

Append to `handler.rs`:

```rust
use crate::UiError;
use std::path::Path;

pub fn list(workspace_root: &Path) -> Result<Vec<HandlerSummary>, UiError> {
    let handlers_dir = workspace_root.join("handlers");
    if !handlers_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&handlers_dir).map_err(|e| UiError::Io(e.to_string()))? {
        let entry = entry.map_err(|e| UiError::Io(e.to_string()))?;
        let path = entry.path();
        if !path.is_dir() { continue; }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
        if name.is_empty() { continue; }
        out.push(build_summary(&path, name)?);
    }
    Ok(out)
}

fn build_summary(path: &Path, name: String) -> Result<HandlerSummary, UiError> {
    let manifest_path = path.join("rowforge.yaml");
    let (status, manifest_opt) = if !manifest_path.is_file() {
        (ManifestStatus::Missing, None)
    } else {
        match rowforge_core::manifest::Manifest::load_from_dir(path) {
            Ok((m, _)) => (ManifestStatus::Valid, Some(m)),
            Err(_) => (ManifestStatus::Invalid, None),
        }
    };
    let metadata = std::fs::metadata(path).map_err(|e| UiError::Io(e.to_string()))?;
    let last_modified = metadata
        .modified()
        .map(|t| chrono::DateTime::<chrono::Utc>::from(t))
        .map_err(|e| UiError::Io(e.to_string()))?;
    Ok(HandlerSummary {
        name,
        path: path.to_path_buf(),
        manifest_status: status,
        last_modified,
        version: manifest_opt.as_ref().and_then(|m| Some(m.version.clone())),
        language: manifest_opt.as_ref().and_then(|m| {
            if m.language.is_empty() { None } else { Some(m.language.clone()) }
        }),
    })
}

pub fn show(workspace_root: &Path, name: &str) -> Result<HandlerDetail, UiError> {
    if !validate_name(name) {
        return Err(UiError::InvalidHandlerName { name: name.to_string() });
    }
    let path = workspace_root.join("handlers").join(name);
    if !path.is_dir() {
        return Err(UiError::HandlerNotFound { name: name.to_string() });
    }
    let summary = build_summary(&path, name.to_string())?;

    // Run validate_manifest for errors/warnings (separate from status which
    // is the binary derived state for the summary).
    let report = crate::manifest::validate_manifest(
        &crate::manifest::ManifestSource::Path { path: path.clone() },
    );

    // Source files: top-level entries, excluding the manifest itself.
    let mut source_files = Vec::new();
    for entry in std::fs::read_dir(&path).map_err(|e| UiError::Io(e.to_string()))? {
        let entry = entry.map_err(|e| UiError::Io(e.to_string()))?;
        let entry_path = entry.path();
        let entry_name = entry_path
            .file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
        if entry_name.is_empty() || entry_name == "rowforge.yaml" { continue; }
        let metadata = std::fs::metadata(&entry_path).map_err(|e| UiError::Io(e.to_string()))?;
        source_files.push(SourceFileSummary {
            name: entry_name,
            size_bytes: metadata.len(),
            is_directory: metadata.is_dir(),
        });
    }
    source_files.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(HandlerDetail {
        summary,
        manifest: report.manifest,
        manifest_errors: report.errors,
        manifest_warnings: report.warnings,
        source_files,
        has_fixtures_dir: path.join("fixtures").is_dir(),
    })
}
```

- [ ] **Step 3: StudioCore method wrappers**

In `lib.rs`:

```rust
impl StudioCore {
    pub fn handler_list(&self) -> Result<Vec<HandlerSummary>, UiError> {
        crate::handler::list(self.workspace.root.as_path())
    }
    pub fn handler_show(&self, name: &str) -> Result<HandlerDetail, UiError> {
        crate::handler::show(self.workspace.root.as_path(), name)
    }
}
```

(Adjust `self.workspace.root` to whatever field name exists for workspace root path on StudioCore.)

- [ ] **Step 4: Verify**

```bash
cargo test -p rowforge-studio-core handler_list handler_show
cargo test -p rowforge-studio-core
```

All pass. Test count grows by 4.

- [ ] **Step 5: Commit**

```
studio-core: handler_list + handler_show

Plan 7 T3. handler_list scans <workspace>/handlers/* (missing dir
returns empty list, not error). handler_show returns the manifest
+ source files (excluding rowforge.yaml itself, which has its own
panel in the UI).

Tests cover: valid/invalid/missing manifest classification, empty
list when handlers/ absent, source file enumeration, unknown name
→ HandlerNotFound.
```

---

## Task 4: Editor resolver + `handler_open_editor` + `handler_reveal`

**Files:** Modify `crates/rowforge-studio-core/src/handler.rs`; tests

- [ ] **Step 1: Tests first**

Append to `handler.rs` `mod tests`:

```rust
#[test]
fn resolver_uses_settings_preferred_editor_first() {
    let resolved = resolve_editor(
        Some("/usr/bin/true"),  // explicit, valid absolute path
        None,
        None,
        &[],
    );
    assert_eq!(resolved.unwrap(), vec!["/usr/bin/true".to_string()]);
}

#[test]
fn resolver_falls_back_to_visual_then_editor() {
    let r = resolve_editor(None, Some("/bin/echo arg1"), Some("/bin/cat"), &[]);
    // VISUAL beats EDITOR.
    assert_eq!(r.unwrap(), vec!["/bin/echo".to_string(), "arg1".to_string()]);

    let r = resolve_editor(None, None, Some("/bin/cat"), &[]);
    assert_eq!(r.unwrap(), vec!["/bin/cat".to_string()]);
}

#[test]
fn resolver_errors_when_all_tiers_miss() {
    let r = resolve_editor(None, None, None, &["__no_such_tool_xyz__"]);
    assert!(matches!(r, Err(UiError::EditorNotFound)));
}
```

Run: `cargo test -p rowforge-studio-core resolver`
Expected: compile error (function doesn't exist).

- [ ] **Step 2: Implement the resolver**

```rust
/// Resolve an editor command per spec 8.4.1. Returns the parsed argv
/// (cmd + args). `Err(UiError::EditorNotFound)` if all tiers miss.
///
/// `probes` parameter exists for testability — production calls with
/// the standard list `["code", "cursor", "subl", "zed"]`.
pub(crate) fn resolve_editor(
    preferred: Option<&str>,
    visual: Option<&str>,
    editor: Option<&str>,
    probes: &[&str],
) -> Result<Vec<String>, UiError> {
    // Tier 1: Settings.preferred_editor (caller-supplied).
    if let Some(cmd) = preferred {
        if !cmd.trim().is_empty() {
            return parse_argv(cmd);
        }
    }
    // Tier 2 & 3: env vars.
    if let Some(cmd) = visual {
        if !cmd.trim().is_empty() {
            return parse_argv(cmd);
        }
    }
    if let Some(cmd) = editor {
        if !cmd.trim().is_empty() {
            return parse_argv(cmd);
        }
    }
    // Tier 4: probe well-known names.
    for name in probes {
        if which::which(name).is_ok() {
            return Ok(vec![name.to_string()]);
        }
    }
    Err(UiError::EditorNotFound)
}

fn parse_argv(cmd: &str) -> Result<Vec<String>, UiError> {
    shell_words::split(cmd).map_err(|e| {
        UiError::InvalidArg(format!("invalid editor command '{}': {}", cmd, e))
    })
}
```

- [ ] **Step 3: `handler_open_editor` + `handler_reveal`**

```rust
pub fn open_editor(
    workspace_root: &Path,
    name: &str,
    settings_preferred: Option<&str>,
) -> Result<(), UiError> {
    if !validate_name(name) {
        return Err(UiError::InvalidHandlerName { name: name.to_string() });
    }
    let handler_dir = workspace_root.join("handlers").join(name);
    if !handler_dir.is_dir() {
        return Err(UiError::HandlerNotFound { name: name.to_string() });
    }
    let visual = std::env::var("VISUAL").ok();
    let editor = std::env::var("EDITOR").ok();
    let argv = resolve_editor(
        settings_preferred,
        visual.as_deref(),
        editor.as_deref(),
        &["code", "cursor", "subl", "zed"],
    )?;
    let (cmd, args) = argv.split_first().ok_or(UiError::EditorNotFound)?;
    std::process::Command::new(cmd)
        .args(args)
        .arg(&handler_dir)
        .spawn()
        .map_err(|e| UiError::Io(format!("spawn editor: {}", e)))?;
    Ok(())
}

pub fn reveal_path(workspace_root: &Path, name: &str) -> Result<PathBuf, UiError> {
    if !validate_name(name) {
        return Err(UiError::InvalidHandlerName { name: name.to_string() });
    }
    let handler_dir = workspace_root.join("handlers").join(name);
    if !handler_dir.is_dir() {
        return Err(UiError::HandlerNotFound { name: name.to_string() });
    }
    Ok(handler_dir)
}
```

(`reveal_path` just returns the path; the Tauri layer calls `shell::open` with it. Keeps studio-core OS-agnostic.)

- [ ] **Step 4: StudioCore methods**

```rust
impl StudioCore {
    pub fn handler_open_editor(&self, name: &str) -> Result<(), UiError> {
        crate::handler::open_editor(
            self.workspace.root.as_path(),
            name,
            self.preferred_editor.as_deref(),
        )
    }
    pub fn handler_reveal_path(&self, name: &str) -> Result<PathBuf, UiError> {
        crate::handler::reveal_path(self.workspace.root.as_path(), name)
    }
}
```

`StudioCore` needs to hold `preferred_editor: Option<String>` — pass via `OpenOpts.with_preferred_editor()` (the Tauri layer loads Settings before `workspace_open`, same pattern as Plan 6's `max_concurrent_runs`).

- [ ] **Step 5: Verify + commit**

```bash
cargo test -p rowforge-studio-core resolver
cargo test -p rowforge-studio-core
```

Test count grows by 3.

```
studio-core: editor resolver + handler_open_editor + handler_reveal

Plan 7 T4. 4-tier resolver per spec 8.4.1:
preferred (Settings) → $VISUAL → $EDITOR → probe code/cursor/subl/zed
→ EditorNotFound.

handler_open_editor spawns detached (doesn't wait, doesn't track).
handler_reveal returns the dir path; Tauri layer wraps with
shell::open.

StudioCore.preferred_editor sourced from Settings via OpenOpts at
workspace_open (same pattern as Plan 6 max_concurrent_runs).
```

---

## Task 5: Scaffold templates

**Files:** Create `crates/rowforge-studio-core/src/handler_templates/`; modify `handler.rs`

- [ ] **Step 1: Write template files**

Create `crates/rowforge-studio-core/src/handler_templates/` directory with subdirs:

**`go_stdio/rowforge.yaml`:**
```yaml
name: {{name}}
version: 0.1.0
description: "Auto-scaffolded handler"
language: go

entry:
  cmd: ["./{{name}}"]
  build: ["go", "build", "-o", "{{name}}", "handler.go"]
  startup_timeout_ms: 10000

runtime:
  mode: row
  idempotent: true

required_input: ["{{primary_field}}"]
```

**`go_stdio/handler.go`:** minimal row-mode stdio loop (boilerplate per existing example handlers — copy from `examples/handlers/golang-billing-channel` and reduce).

**`go_stdio/go.mod`:**
```
module {{name}}

go 1.22
```

**`go_batch/`:** same shape but `mode: batch` + `batch_size: 5` + handler.go reads batch envelopes.

**`empty/rowforge.yaml`:** skeleton with placeholder entry.cmd.

**`empty/handler.go`:** just `package main; func main() {}`.

- [ ] **Step 2: Embed via `include_str!`**

In `handler.rs`:

```rust
const TPL_GO_STDIO_YAML: &str = include_str!("handler_templates/go_stdio/rowforge.yaml");
const TPL_GO_STDIO_GO:   &str = include_str!("handler_templates/go_stdio/handler.go");
const TPL_GO_STDIO_MOD:  &str = include_str!("handler_templates/go_stdio/go.mod");

const TPL_GO_BATCH_YAML: &str = include_str!("handler_templates/go_batch/rowforge.yaml");
const TPL_GO_BATCH_GO:   &str = include_str!("handler_templates/go_batch/handler.go");
const TPL_GO_BATCH_MOD:  &str = include_str!("handler_templates/go_batch/go.mod");

const TPL_EMPTY_YAML: &str = include_str!("handler_templates/empty/rowforge.yaml");
const TPL_EMPTY_GO:   &str = include_str!("handler_templates/empty/handler.go");

fn template_files(template: ScaffoldTemplate) -> Vec<(&'static str, &'static str)> {
    match template {
        ScaffoldTemplate::GoStdio => vec![
            ("rowforge.yaml", TPL_GO_STDIO_YAML),
            ("handler.go",    TPL_GO_STDIO_GO),
            ("go.mod",        TPL_GO_STDIO_MOD),
        ],
        ScaffoldTemplate::GoBatch => vec![
            ("rowforge.yaml", TPL_GO_BATCH_YAML),
            ("handler.go",    TPL_GO_BATCH_GO),
            ("go.mod",        TPL_GO_BATCH_MOD),
        ],
        ScaffoldTemplate::Empty => vec![
            ("rowforge.yaml", TPL_EMPTY_YAML),
            ("handler.go",    TPL_EMPTY_GO),
        ],
    }
}

fn render(template: &str, name: &str, primary_field: &str) -> String {
    template
        .replace("{{name}}", name)
        .replace("{{primary_field}}", primary_field)
}
```

- [ ] **Step 3: Commit (templates + helpers; scaffold function lands in T6)**

```
studio-core: 3 scaffold templates embedded via include_str!

Plan 7 T5. GoStdio, GoBatch, Empty. Files copy-reduced from
examples/handlers/golang-billing-channel. Two template variables:
{{name}}, {{primary_field}} — simple string replace, no dep on
Tera / Handlebars.

handler_scaffold lands in T6.
```

---

## Task 6: `handler_scaffold` + tests

**Files:** Modify `handler.rs`; tests

- [ ] **Step 1: Test first**

```rust
#[test]
fn scaffold_writes_go_stdio_template() {
    let tmp = tempdir::TempDir::new("rfs-plan7-sc").unwrap();
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().into()),
    ).unwrap();

    let name = core.handler_scaffold(rowforge_studio_core::ScaffoldArgs {
        name: "my-handler".into(),
        template: rowforge_studio_core::ScaffoldTemplate::GoStdio,
        primary_field: "email".into(),
    }).unwrap();
    assert_eq!(name, "my-handler");

    let dir = tmp.path().join("handlers").join("my-handler");
    assert!(dir.is_dir());
    let yaml = std::fs::read_to_string(dir.join("rowforge.yaml")).unwrap();
    assert!(yaml.contains("name: my-handler"));
    assert!(yaml.contains("- email"));    // required_input
    assert!(dir.join("handler.go").is_file());
    assert!(dir.join("go.mod").is_file());
}

#[test]
fn scaffold_rejects_invalid_name() {
    let tmp = tempdir::TempDir::new("rfs-plan7-sc-bn").unwrap();
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().into()),
    ).unwrap();
    let err = core.handler_scaffold(rowforge_studio_core::ScaffoldArgs {
        name: "Has Space".into(),
        template: rowforge_studio_core::ScaffoldTemplate::Empty,
        primary_field: "x".into(),
    }).unwrap_err();
    assert!(matches!(err, rowforge_studio_core::UiError::InvalidHandlerName { .. }));
}

#[test]
fn scaffold_errors_when_name_already_exists() {
    let tmp = tempdir::TempDir::new("rfs-plan7-sc-ex").unwrap();
    std::fs::create_dir_all(tmp.path().join("handlers").join("taken")).unwrap();
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().into()),
    ).unwrap();
    let err = core.handler_scaffold(rowforge_studio_core::ScaffoldArgs {
        name: "taken".into(),
        template: rowforge_studio_core::ScaffoldTemplate::Empty,
        primary_field: "x".into(),
    }).unwrap_err();
    assert!(matches!(err, rowforge_studio_core::UiError::HandlerExists { .. }));
}
```

- [ ] **Step 2: Implement**

```rust
pub fn scaffold(workspace_root: &Path, args: ScaffoldArgs) -> Result<String, UiError> {
    if !validate_name(&args.name) {
        return Err(UiError::InvalidHandlerName { name: args.name });
    }
    let dir = workspace_root.join("handlers").join(&args.name);
    if dir.exists() {
        return Err(UiError::HandlerExists { name: args.name });
    }
    std::fs::create_dir_all(&dir).map_err(|e| UiError::Io(e.to_string()))?;
    for (filename, template) in template_files(args.template) {
        let content = render(template, &args.name, &args.primary_field);
        std::fs::write(dir.join(filename), content).map_err(|e| UiError::Io(e.to_string()))?;
    }
    Ok(args.name)
}
```

Add `StudioCore::handler_scaffold` wrapper.

- [ ] **Step 3: Verify + commit**

```bash
cargo test -p rowforge-studio-core scaffold
```

3 tests pass.

```
studio-core: handler_scaffold

Plan 7 T6. Writes the chosen template's files under
<workspace>/handlers/<name>/. Validates name regex before any
path work; refuses if destination exists.

Tests cover the GoStdio template happy path (variable replacement
in yaml, all 3 files written) plus the InvalidHandlerName +
HandlerExists error paths.
```

---

## Task 7: `handler_delete` + safety hardening

**Files:** Modify `handler.rs`; tests

- [ ] **Step 1: Tests first** — including path-traversal defense

```rust
#[test]
fn delete_removes_handler_dir() {
    let tmp = tempdir::TempDir::new("rfs-plan7-del").unwrap();
    let dir = tmp.path().join("handlers").join("doomed");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("rowforge.yaml"), "x").unwrap();

    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().into()),
    ).unwrap();
    core.handler_delete("doomed").unwrap();
    assert!(!dir.exists());
}

#[test]
fn delete_rejects_invalid_name_before_any_fs_op() {
    let tmp = tempdir::TempDir::new("rfs-plan7-del-bn").unwrap();
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().into()),
    ).unwrap();
    // The regex rejects "../*" before fs::remove_dir_all is ever called.
    let err = core.handler_delete("../etc").unwrap_err();
    assert!(matches!(err, rowforge_studio_core::UiError::InvalidHandlerName { .. }));
}

#[test]
fn delete_rejects_symlinked_dir_pointing_outside_workspace() {
    let tmp = tempdir::TempDir::new("rfs-plan7-del-sym").unwrap();
    let outside = tempdir::TempDir::new("rfs-plan7-del-out").unwrap();
    std::fs::create_dir_all(outside.path().join("victim")).unwrap();
    std::fs::create_dir_all(tmp.path().join("handlers")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        outside.path().join("victim"),
        tmp.path().join("handlers").join("evil"),
    ).unwrap();

    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().into()),
    ).unwrap();
    // Symlink defense: canonicalized parent must equal <workspace>/handlers.
    let err = core.handler_delete("evil");
    // Either Err with InvalidArg/Io OR success but only the symlink (not the
    // target dir) is removed. Outside dir must still exist.
    let _ = err;
    assert!(outside.path().join("victim").is_dir(),
        "delete must NOT recurse through a symlink pointing outside workspace");
}
```

- [ ] **Step 2: Implement with canonicalize-and-check**

```rust
pub fn delete(workspace_root: &Path, name: &str) -> Result<(), UiError> {
    if !validate_name(name) {
        return Err(UiError::InvalidHandlerName { name: name.to_string() });
    }
    let handlers_dir = workspace_root.join("handlers");
    let dir = handlers_dir.join(name);
    if !dir.exists() {
        return Err(UiError::HandlerNotFound { name: name.to_string() });
    }
    // Symlink defense: canonicalize and verify the target dir is INSIDE
    // <workspace>/handlers (after symlink resolution). If `dir` is a
    // symlink to /etc/, this check fails and we abort.
    let canon_dir = dir.canonicalize().map_err(|e| UiError::Io(e.to_string()))?;
    let canon_handlers = handlers_dir.canonicalize().map_err(|e| UiError::Io(e.to_string()))?;
    if !canon_dir.starts_with(&canon_handlers) {
        return Err(UiError::InvalidArg(
            format!("handler '{}' resolves outside workspace", name),
        ));
    }
    std::fs::remove_dir_all(&dir).map_err(|e| UiError::Io(e.to_string()))?;
    Ok(())
}
```

- [ ] **Step 3: Verify + commit**

Tests above + full crate.

```
studio-core: handler_delete with symlink + path-traversal defense

Plan 7 T7. Three lines of defense:
1. validate_name([a-z0-9-]+) rejects "../" segments before any fs op
2. canonicalize() resolves symlinks; we then verify the result
   starts_with(<workspace>/handlers/) so a symlink-out can't escape
3. fs::remove_dir_all only runs after both checks pass

Test that a symlink to /tmp/victim cannot be used to wipe the
target dir.
```

---

## Task 8: `handler_rename`

**Files:** Modify `handler.rs`; tests

- [ ] **Step 1: Tests**

```rust
#[test]
fn rename_moves_handler_dir() {
    let tmp = tempdir::TempDir::new("rfs-plan7-rn").unwrap();
    let src = tmp.path().join("handlers").join("old");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("rowforge.yaml"), "name: old\nversion: 0.1.0\nentry:\n  cmd: [\"./old\"]\n").unwrap();

    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().into()),
    ).unwrap();
    core.handler_rename("old", "new").unwrap();
    assert!(!src.is_dir());
    assert!(tmp.path().join("handlers").join("new").is_dir());
}

#[test]
fn rename_errors_when_destination_exists() {
    let tmp = tempdir::TempDir::new("rfs-plan7-rn-ex").unwrap();
    std::fs::create_dir_all(tmp.path().join("handlers").join("a")).unwrap();
    std::fs::create_dir_all(tmp.path().join("handlers").join("b")).unwrap();
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().into()),
    ).unwrap();
    let err = core.handler_rename("a", "b").unwrap_err();
    assert!(matches!(err, rowforge_studio_core::UiError::HandlerExists { .. }));
}

#[test]
fn rename_rejects_invalid_target_name() {
    let tmp = tempdir::TempDir::new("rfs-plan7-rn-bn").unwrap();
    std::fs::create_dir_all(tmp.path().join("handlers").join("ok")).unwrap();
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().into()),
    ).unwrap();
    let err = core.handler_rename("ok", "Bad Name").unwrap_err();
    assert!(matches!(err, rowforge_studio_core::UiError::InvalidHandlerName { .. }));
}
```

- [ ] **Step 2: Implement**

```rust
pub fn rename(workspace_root: &Path, old: &str, new: &str) -> Result<(), UiError> {
    if !validate_name(old) {
        return Err(UiError::InvalidHandlerName { name: old.to_string() });
    }
    if !validate_name(new) {
        return Err(UiError::InvalidHandlerName { name: new.to_string() });
    }
    let handlers = workspace_root.join("handlers");
    let src = handlers.join(old);
    let dst = handlers.join(new);
    if !src.is_dir() {
        return Err(UiError::HandlerNotFound { name: old.to_string() });
    }
    if dst.exists() {
        return Err(UiError::HandlerExists { name: new.to_string() });
    }
    std::fs::rename(&src, &dst).map_err(|e| UiError::Io(e.to_string()))?;
    Ok(())
}
```

Note: per the locked decision, we do NOT touch `handler_instances.source_snapshot_dir` — that field stays pointing at the old path on existing attempts.

- [ ] **Step 3: Verify + commit**

```
studio-core: handler_rename (lazy on attempt references)

Plan 7 T8. fs::rename the dir; do NOT update sqlite
handler_instances.source_snapshot_dir on existing attempts.
Rationale: handler_instance is content-addressed (binary_hash +
manifest_hash); the dir path is informational. Touching it would
need a schema migration for a cosmetic win.

Doc note added in spec part-2: artifacts on old attempts may show
the pre-rename path; this is expected behavior.
```

---

## Task 9: Tauri commands + register

**Files:** Modify `apps/rowforge-studio/src-tauri/src/commands.rs`, `lib.rs`

- [ ] **Step 1: 7 commands**

```rust
use rowforge_studio_core::{
    // … existing …
    HandlerSummary, HandlerDetail, ScaffoldArgs,
};

#[tauri::command]
pub fn handler_list(state: State<'_, AppState>) -> Result<Vec<HandlerSummary>, UiError> {
    let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
    let core = guard.as_ref().ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
    core.handler_list()
}

#[tauri::command]
pub fn handler_show(state: State<'_, AppState>, name: String) -> Result<HandlerDetail, UiError> {
    let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
    let core = guard.as_ref().ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
    core.handler_show(&name)
}

#[tauri::command]
pub fn handler_open_editor(state: State<'_, AppState>, name: String) -> Result<(), UiError> {
    let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
    let core = guard.as_ref().ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
    core.handler_open_editor(&name)
}

#[tauri::command]
pub fn handler_reveal(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    name: String,
) -> Result<(), UiError> {
    let path = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        let core = guard.as_ref().ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
        core.handler_reveal_path(&name)?
    };
    use tauri_plugin_shell::ShellExt;
    app.shell()
        .open(path.to_string_lossy().to_string(), None)
        .map_err(|e| UiError::Io(e.to_string()))?;
    Ok(())
}

#[tauri::command]
pub fn handler_scaffold(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    args: ScaffoldArgs,
) -> Result<String, UiError> {
    let name = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        let core = guard.as_ref().ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
        core.handler_scaffold(args)?
    };
    let _ = app.emit("handlers:list", ());
    Ok(name)
}

#[tauri::command]
pub fn handler_delete(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    name: String,
) -> Result<(), UiError> {
    {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        let core = guard.as_ref().ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
        core.handler_delete(&name)?;
    }
    let _ = app.emit("handlers:list", ());
    Ok(())
}

#[tauri::command]
pub fn handler_rename(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    old: String,
    new: String,
) -> Result<(), UiError> {
    {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        let core = guard.as_ref().ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
        core.handler_rename(&old, &new)?;
    }
    let _ = app.emit("handlers:list", ());
    Ok(())
}
```

- [ ] **Step 2: Register in `lib.rs`**

```rust
.invoke_handler(tauri::generate_handler![
    // … existing …
    commands::handler_list,
    commands::handler_show,
    commands::handler_open_editor,
    commands::handler_reveal,
    commands::handler_scaffold,
    commands::handler_delete,
    commands::handler_rename,
])
```

- [ ] **Step 3: ipc_contract test**

Add the 7 names to the contract assertion.

- [ ] **Step 4: Commit**

```
studio-shell: 7 handler Tauri commands wired

Plan 7 T9. handler_list / show / open_editor / reveal / scaffold /
delete / rename registered in invoke_handler. handler_reveal uses
tauri_plugin_shell::open at the layer boundary. Scaffold + delete
+ rename emit handlers:list event so the UI list query
invalidates without polling.
```

---

## Task 10: TS mirrors + ipc client + hooks

**Files:** Modify `types.ts`, `client.ts`; create `use-handlers.ts`

- [ ] **Step 1: Types**

```ts
export type ManifestStatus = "valid" | "invalid" | "missing";
export type ScaffoldTemplate = "go_stdio" | "go_batch" | "empty";

export interface HandlerSummary {
  name: string;
  path: string;
  manifest_status: ManifestStatus;
  last_modified: string;   // ISO 8601 UTC
  version: string | null;
  language: string | null;
}

export interface SourceFileSummary {
  name: string;
  size_bytes: number;
  is_directory: boolean;
}

export interface HandlerDetail {
  summary: HandlerSummary;
  manifest: Manifest | null;
  manifest_errors: ManifestError[];
  manifest_warnings: ManifestWarning[];
  source_files: SourceFileSummary[];
  has_fixtures_dir: boolean;
}

export interface ScaffoldArgs {
  name: string;
  template: ScaffoldTemplate;
  primary_field: string;
}
```

- [ ] **Step 2: ipc client wrappers**

```ts
handler_list: () => invoke<HandlerSummary[]>("handler_list"),
handler_show: (args: { name: string }) => invoke<HandlerDetail>("handler_show", args),
handler_open_editor: (args: { name: string }) => invoke<void>("handler_open_editor", args),
handler_reveal: (args: { name: string }) => invoke<void>("handler_reveal", args),
handler_scaffold: (args: ScaffoldArgs) => invoke<string>("handler_scaffold", { args }),
handler_delete: (args: { name: string }) => invoke<void>("handler_delete", args),
handler_rename: (args: { old: string; new: string }) => invoke<void>("handler_rename", args),
```

- [ ] **Step 3: TanStack hooks**

Create `use-handlers.ts`:

```ts
export const useHandlerList = () =>
  useQuery({ queryKey: ["handler_list"], queryFn: () => ipc.handler_list() });

export const useHandlerShow = (name: string | null) =>
  useQuery({
    queryKey: ["handler_show", name],
    queryFn: () => ipc.handler_show({ name: name! }),
    enabled: !!name,
  });

export const useHandlerScaffold = () => {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: ScaffoldArgs) => ipc.handler_scaffold(args),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["handler_list"] }),
  });
};

// similarly for delete + rename
```

Plus `useEffect` in HandlerList page to subscribe to `handlers:list` event and invalidate.

- [ ] **Step 4: Commit**

```
studio-shell: TS mirrors + hooks for handler ops

Plan 7 T10. HandlerSummary / HandlerDetail / SourceFileSummary /
ScaffoldArgs / ManifestStatus / ScaffoldTemplate mirrors.
ipc.handler_* wrappers for all 7 commands. TanStack hooks with
list-invalidation on mutation success.
```

---

## Task 11: Handlers list page

**Files:** Create `pages/HandlerList.tsx`; modify `App.tsx`, `Sidebar.tsx`; tests

- [ ] **Step 1: Test**

```tsx
// __tests__/handler-list.test.tsx
// Mock ipc.handler_list returns 3 handlers (valid/invalid/missing).
// Renders all 3 with correct status dot color (test text "valid"/"invalid"/"missing").
// Empty state: ipc returns [] → "No handlers" text + "Create your first handler" button.
```

- [ ] **Step 2: Implement**

List page with the layout from §6.2 of the design doc.

- [ ] **Step 3: Enable sidebar Handlers link**

`Sidebar.tsx`: enable the Handlers entry (`NavLink to="/handlers"`).

- [ ] **Step 4: Register route**

`App.tsx`: `<Route path="/handlers" element={<HandlerListPage />} />`.

- [ ] **Step 5: Commit**

```
studio-shell: /handlers list page + sidebar link enabled

Plan 7 T11. Renders HandlerSummary[] with manifest-status dots,
relative timestamps, row menu (Edit / Reveal / Rename / Delete).
Empty state has "Create your first handler" CTA.

Sidebar's previously-anchored Handlers link is now active.
```

---

## Task 12: Handler detail page

**Files:** Create `pages/HandlerDetail.tsx`; modify `App.tsx`; tests

- [ ] **Step 1: Test** — renders manifest + source files
- [ ] **Step 2: Implement** — layout from §6.3 of the design doc; reuses Plan 5's `ManifestReportView`
- [ ] **Step 3: Register route** — `/handlers/:name`
- [ ] **Step 4: Commit**

---

## Task 13: Scaffold modal

**Files:** Create `components/ScaffoldHandlerDialog.tsx`; modify HandlerList to wire trigger; tests

- [ ] **Step 1: Test** — name regex validation, template radio, submit calls `handler_scaffold`, navigates to `/handlers/<new>`
- [ ] **Step 2: Implement** — design §6.4
- [ ] **Step 3: Commit**

---

## Task 14: Delete typed-token + Rename dialogs

**Files:** Create `DeleteHandlerDialog.tsx`, `RenameHandlerDialog.tsx`; tests

- [ ] **Step 1: Tests** — delete requires typed name match; rename requires regex valid + destination check
- [ ] **Step 2: Implement** — design §6.5 + §6.6. Reuse the typed-token pattern from Plan 4's `CancelDialog`.
- [ ] **Step 3: Commit**

---

## Task 15: Settings Editor section (preferred_editor)

**Files:** Modify `Settings` struct, `SettingsForm.tsx`, TS mirror

- [ ] **Step 1: Backend** — `Settings.preferred_editor: Option<String>`; `OpenOpts.preferred_editor` + builder; `StudioCore.preferred_editor` field set in `open()` (analogous to Plan 6 `max_concurrent_runs`)
- [ ] **Step 2: TS mirror** — Settings interface gains `preferred_editor: string | null`
- [ ] **Step 3: SettingsForm** — new Section "Editor" with single field (text input + placeholder example). Validate at save: `shell_words::split` succeeds, else show error.
- [ ] **Step 4: Tests** — settings-form test asserts the field renders + saves
- [ ] **Step 5: Commit**

---

## Task 16: HUMAN_SMOKE Plan 7

**Files:** `apps/rowforge-studio/HUMAN_SMOKE.md`

Walkthrough:
- Sidebar Handlers link → `/handlers` shows list (manual scenario: pre-populate `<workspace>/handlers/` with 1-2 dirs, expect them to appear)
- Click row → detail page; manifest renders correctly
- Edit (opens VS Code at handler dir); Reveal (opens Finder)
- New handler → scaffold modal → pick template → submit → land on new detail page → handler.go exists
- Rename → handler list reflects new name → old name gone
- Delete → typed-token dialog → confirm → handler disappears from list
- Settings Editor section → set `preferred_editor` → Edit uses it
- Negative paths: scaffold with invalid name (rejected), delete a non-existent handler, rename to existing name

---

## Task 17: Spec docs (en + zh-Hant)

**Files:** Modify `docs/spec/studio/part-{2,5,7,8}*.md`

- part-2: Settings gains `preferred_editor`; footnote on lazy rename in handler_instances discussion
- part-5: §5.3 four new UiError variants
- part-7: §7.3 IA — `/handlers` and `/handlers/:name` move from "anchored" to active routes; §7.4 add scaffold/rename/delete flows
- part-8: §8.5 / §8.6 cross-refs updated to Plan 7 file paths

Mirror in zh-Hant for all four files.

---

## Task 18: Final verification + PR

```bash
cargo build && cargo test
cd apps/rowforge-studio
pnpm tsc -b && pnpm test && pnpm build
```

Expected counts:
- Rust: ~270 (Plan 6 ended at 260; ~10 new handler tests + 4 UiError + 3 resolver + scaffold/delete/rename = ~10-15 net)
- Vitest: ~70 (Plan 6 ended at 65; +1-2 per new component = +5)

Manual smoke per HUMAN_SMOKE Plan 7 section.

Open PR. Same pattern as Plan 6: PR carries impl + plan + design docs.

---

## Acceptance criteria

1. `cargo build` clean
2. `cargo test` workspace passes; ~270 tests
3. `pnpm tsc -b` + `pnpm test` (~70 tests) + `pnpm build` clean
4. Sidebar Handlers link enabled; `/handlers` route renders
5. `handler_list` returns all dirs under `<workspace>/handlers/` with correct ManifestStatus
6. `handler_show` includes parsed manifest + source files
7. `handler_open_editor` opens VS Code (or `$EDITOR`) at the handler dir
8. `handler_reveal` opens the OS file manager
9. `handler_scaffold` writes 3 different templates correctly (variable replacement verified)
10. `handler_delete` refuses paths outside `<workspace>/handlers/` (symlink defense)
11. `handler_rename` updates dir; `handler_instances.source_snapshot_dir` unchanged
12. Settings page has Editor section; `preferred_editor` persists across restart
13. Spec docs (en + zh-Hant) updated
14. **(human)** HUMAN_SMOKE Plan 7 walkthrough

---

## Self-review

**Spec coverage:** every section of the design doc maps to at least one task:
- §4.1 types → T1
- §4.2 list/show → T3
- §4.2 editor/reveal → T4
- §4.2 scaffold → T6 (templates in T5)
- §4.2 delete → T7
- §4.2 rename → T8
- §4.3 UiError → T2
- §4.4 Settings.preferred_editor → T15
- §4.5 templates → T5
- §5 Tauri commands → T9
- §6 UI pages + modals → T11-T14

**Placeholder scan:** templates (T5 step 1) reference `examples/handlers/golang-billing-channel` for handler.go content — implementer needs to actually write the Go boilerplate, not just `//TODO`. Each template's handler.go should compile and produce well-formed outcomes for at least 1 row.

**Type consistency:** `HandlerSummary` shape consistent across Rust → Tauri serde → TS mirror. `ScaffoldTemplate` snake_case (`go_stdio` / `go_batch` / `empty`) on both sides. `ManifestStatus` ditto.

**Order dependency:** T1 (types) → T2 (errors) → T3 (list/show, needs types + errors) → T4 (editor, needs UiError::EditorNotFound + Settings.preferred_editor — though T15 can be after) → T5 (templates) → T6 (scaffold, needs T5) → T7 (delete) → T8 (rename) → T9 (Tauri, needs T3-T8) → T10 (TS mirrors, needs T9 wire-ups) → T11-T14 (UI, needs T10) → T15 (Settings, independent of T11-T14) → T16 (docs) → T17 (spec) → T18 (verify).

T15 has soft dependency on T4 (editor resolver uses preferred_editor) but the resolver accepts `None` as a tier-1 input so T4 can land before T15 fills in the Settings field. Acceptable.
