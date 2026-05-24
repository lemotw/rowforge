# Studio Plan 01 — Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up `rowforge-studio-core` crate with workspace open + execution-list projections, plus the supporting lift from `rowforge-cli` into `rowforge-core`.

**Architecture:** New Rust crate `crates/rowforge-studio-core` consuming the existing `rowforge-core::execution_store`. CLI's hard-coded `~/.rowforge` discovery moves into `rowforge_core::workspace` so both CLI and Studio share it. No UI, no Tauri, no async runtime usage beyond what `rowforge-core` already needs.

**Tech Stack:** Rust 2021, `thiserror`, `serde`, `chrono`, `dirs`, `tempfile` (test), `rusqlite` (transitive via core).

**Spec references:** Part 1 §1.3 architecture; Part 2 §2.2.1 `Workspace`, §2.2.2 `ExecSummary`; Part 5 §5.1 crate boundary, §5.2 `open`/`list`, §5.3 `UiError`; Part 4 §4.3 hot/warm caching (warm tier wiring lands in Plan 3 — Plan 1 ships uncached).

---

## File structure

### New
- `crates/rowforge-studio-core/Cargo.toml`
- `crates/rowforge-studio-core/src/lib.rs` — public `StudioCore` impl, re-exports
- `crates/rowforge-studio-core/src/error.rs` — `UiError` v1 subset
- `crates/rowforge-studio-core/src/workspace.rs` — `Workspace`, `OpenOpts`
- `crates/rowforge-studio-core/src/exec_view.rs` — `ExecSummary`, `ListFilter`, `list_executions` projection
- `crates/rowforge-studio-core/tests/foundation.rs` — integration tests
- `crates/rowforge-core/src/workspace.rs` — `default_workspace_root`, `WorkspaceLocation`

### Modified
- `Cargo.toml` (workspace root) — add member, add `dirs` to `workspace.dependencies`
- `crates/rowforge-core/Cargo.toml` — add `dirs` dep
- `crates/rowforge-core/src/lib.rs` — `pub mod workspace;`
- `crates/rowforge-cli/src/exec_cmd.rs:1012-1018` — replace inline `rowforge_home()` with the lifted function

### Out of scope for Plan 1
- `attempts_count`, `last_attempt_state`, `last_attempt_counts` on `ExecSummary` — Plan 3 fills these in.
- Mtime-probe caching — Plan 3.
- `Workspace::schema_version` mismatch handling — Plan 3.
- Tauri integration — Plan 2.

These fields exist in the spec but are stubbed (`None` / `0`) in Plan 1 to keep this slice small and testable in isolation.

---

## Task 1: Add `rowforge-studio-core` to the workspace

**Files:**
- Modify: `Cargo.toml`
- Create: `crates/rowforge-studio-core/Cargo.toml`
- Create: `crates/rowforge-studio-core/src/lib.rs`

- [ ] **Step 1.1: Add the crate to the workspace members and shared deps**

Open `Cargo.toml` at the repo root. The `[workspace]` block currently lists three members. Update to:

```toml
[workspace]
resolver = "2"
members = [
    "crates/rowforge-core",
    "crates/rowforge-cli",
    "crates/rowforge-studio-core",
    "crates/test-handler",
]
```

In the same file, under `[workspace.dependencies]`, add `dirs` (if not already present at the same version — it is not, per earlier inspection):

```toml
dirs = "5"
```

Note: `dirs` may already appear; the existing `dirs = "5"` listed in workspace deps is what we want, so verify and only add if missing. The grep before this task showed it present; in that case skip this addition.

- [ ] **Step 1.2: Create the crate's Cargo.toml**

```bash
mkdir -p crates/rowforge-studio-core/src
mkdir -p crates/rowforge-studio-core/tests
```

Create `crates/rowforge-studio-core/Cargo.toml`:

```toml
[package]
name = "rowforge-studio-core"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
publish = false

[dependencies]
rowforge-core = { path = "../rowforge-core" }
thiserror = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
chrono = { workspace = true }
dirs = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
anyhow = { workspace = true }
```

- [ ] **Step 1.3: Create a skeleton `lib.rs` so the crate compiles**

Create `crates/rowforge-studio-core/src/lib.rs`:

```rust
//! rowforge-studio-core — GUI-only extension of rowforge-core.
//!
//! See `docs/spec/studio/part-1-overview.md` for principles and
//! `docs/spec/studio/part-5-api.md` for the public surface.

pub mod error;
pub mod exec_view;
pub mod workspace;

pub use error::UiError;
pub use exec_view::{ExecSummary, ListFilter};
pub use workspace::{OpenOpts, Workspace};

/// Top-level handle returned by `StudioCore::open`.
///
/// Plan 1 ships only `open` and `list`. Later plans add `show`, `attempt`,
/// `start_run`, `cancel`, `subscribe`, `start_exec`, `export`, plus the
/// handler-authoring surface (Part 8).
pub struct StudioCore {
    workspace: Workspace,
    store: rowforge_core::execution_store::ExecutionStore,
}

impl StudioCore {
    /// Stub — implementations land in Task 6 / Task 8.
    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }
}
```

- [ ] **Step 1.4: Create three empty module files so the skeleton compiles**

```bash
touch crates/rowforge-studio-core/src/error.rs
touch crates/rowforge-studio-core/src/exec_view.rs
touch crates/rowforge-studio-core/src/workspace.rs
```

Each must contain at least a placeholder so `pub use` in `lib.rs` resolves. Open each and add:

`crates/rowforge-studio-core/src/error.rs`:
```rust
//! Stub — filled in Task 5.
#[derive(Debug)]
pub struct UiError;
```

`crates/rowforge-studio-core/src/workspace.rs`:
```rust
//! Stub — filled in Task 4.
use std::path::PathBuf;

#[derive(Debug, Clone, Default)]
pub struct OpenOpts {
    pub workspace: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct Workspace {
    pub root: PathBuf,
    pub schema_version: u8,
}
```

`crates/rowforge-studio-core/src/exec_view.rs`:
```rust
//! Stub — filled in Task 7.
#[derive(Debug, Clone, Default)]
pub struct ListFilter;

#[derive(Debug, Clone)]
pub struct ExecSummary;
```

- [ ] **Step 1.5: Verify the workspace still compiles**

Run: `cargo build -p rowforge-studio-core`
Expected: PASS (warnings about unused fields are fine for now).

- [ ] **Step 1.6: Commit**

```bash
git add Cargo.toml crates/rowforge-studio-core
git commit -m "studio-core: scaffold crate skeleton

Adds an empty rowforge-studio-core crate to the workspace with stub
modules. Later tasks fill in workspace open, exec list projections,
and error types."
```

---

## Task 2: Lift workspace discovery into `rowforge-core::workspace`

The CLI defines `rowforge_home()` privately in `exec_cmd.rs` returning
`~/.rowforge`. Per Part 5 §5.1 the lift goes into core so Studio shares
it.

**Files:**
- Create: `crates/rowforge-core/src/workspace.rs`
- Modify: `crates/rowforge-core/src/lib.rs:1-20`
- Modify: `crates/rowforge-core/Cargo.toml`

- [ ] **Step 2.1: Add `dirs` dep to core**

In `crates/rowforge-core/Cargo.toml`, under `[dependencies]`, add (or confirm exists):

```toml
dirs = { workspace = true }
```

- [ ] **Step 2.2: Write the failing test**

Create `crates/rowforge-core/src/workspace.rs`:

```rust
//! Workspace location helpers shared by CLI and Studio.
//!
//! A "workspace" (also called `home` in the CLI's older terminology) is a
//! directory containing `executions.db` and the per-execution
//! subdirectories under `executions/`.
//!
//! Spec: `docs/spec/studio/part-1-overview.md` §1.5 (workspace ownership),
//! `docs/spec/studio/part-4-data.md` §4.1 (artifact list).

use std::path::PathBuf;

/// Where to find the executions store on this machine when no override is
/// given.
///
/// Returns the same path the CLI's `rowforge exec` commands have always
/// used: `$HOME/.rowforge`. Returns `None` only if the OS cannot resolve
/// a home directory (very rare; sandboxed installs).
pub fn default_workspace_root() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".rowforge"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_workspace_root_under_home() {
        let root = default_workspace_root().expect("home dir available");
        assert!(root.ends_with(".rowforge"), "got {:?}", root);
    }
}
```

- [ ] **Step 2.3: Run the test to verify it fails (module not registered)**

Run: `cargo test -p rowforge-core --lib workspace::tests::default_workspace_root_under_home`
Expected: FAIL with "unresolved import" or similar — module not declared yet.

- [ ] **Step 2.4: Register the module**

Edit `crates/rowforge-core/src/lib.rs`. Insert `pub mod workspace;` in alphabetical position:

```rust
//! rowforge core: handler orchestration, CSV I/O, run lifecycle.
pub mod accumulator;
pub mod cancel;
pub mod csv_io;
pub mod input_stream;
pub mod reader;
pub mod error;
pub mod execution_store;
pub mod jsonl_writer;
pub mod manifest;
pub mod meta;
pub mod pool;
pub mod pool_streaming;
pub mod protocol;
pub mod rerun;
pub mod row_resolution;
pub mod run;
pub mod runtime;
pub mod worker;
pub mod worker_loop;
pub mod workspace;
```

- [ ] **Step 2.5: Run the test to verify it passes**

Run: `cargo test -p rowforge-core --lib workspace::tests::default_workspace_root_under_home`
Expected: PASS.

- [ ] **Step 2.6: Migrate CLI's `rowforge_home()` to call the lifted function**

In `crates/rowforge-cli/src/exec_cmd.rs` around line 1012, replace:

```rust
fn rowforge_home() -> Result<PathBuf> {
    let h = dirs::home_dir().ok_or_else(|| anyhow!("no home dir"))?;
    Ok(h.join(".rowforge"))
}
```

with:

```rust
fn rowforge_home() -> Result<PathBuf> {
    rowforge_core::workspace::default_workspace_root()
        .ok_or_else(|| anyhow!("no home dir"))
}
```

Do not rename `rowforge_home()` itself — the CLI's call sites stay
untouched.

- [ ] **Step 2.7: Verify CLI still builds and its tests still pass**

Run: `cargo build -p rowforge-cli`
Expected: PASS.

Run: `cargo test -p rowforge-cli`
Expected: PASS (the same set of tests as before).

- [ ] **Step 2.8: Commit**

```bash
git add crates/rowforge-core/src/workspace.rs crates/rowforge-core/src/lib.rs crates/rowforge-core/Cargo.toml crates/rowforge-cli/src/exec_cmd.rs
git commit -m "core: lift default_workspace_root from cli

CLI's inlined ~/.rowforge resolver moves into
rowforge_core::workspace so studio-core can share it
(spec part-5 §5.1)."
```

---

## Task 3: Define the `UiError` v1 subset

Plan 1 only surfaces three variants. Later plans add `RunAborted`,
`RunBusy`, `HandlerBusy`, etc.

**Files:**
- Modify: `crates/rowforge-studio-core/src/error.rs`
- Test: covered by Task 6's integration test.

- [ ] **Step 3.1: Replace the stub with the v1 surface**

Open `crates/rowforge-studio-core/src/error.rs` and replace its contents:

```rust
//! UI-facing error type.
//!
//! Surface is intentionally narrow in Plan 1 (open + list paths only).
//! Later plans extend with `RunAborted`, `RunBusy`, `HandlerBusy`, etc.
//! Spec: `docs/spec/studio/part-5-api.md` §5.3.

use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum UiError {
    /// Workspace cannot be located (no `$HOME` and no explicit override) or
    /// the SQLite store could not be opened.
    #[error("workspace unavailable: {0}")]
    WorkspaceUnavailable(String),

    /// I/O failure reading or scanning workspace artefacts.
    #[error("io error: {0}")]
    Io(String),

    /// Unclassifiable internal failure. Future plans should classify
    /// instead of reaching for this.
    #[error("internal: {0}")]
    Internal(String),
}

impl From<std::io::Error> for UiError {
    fn from(e: std::io::Error) -> Self {
        UiError::Io(e.to_string())
    }
}

impl From<rowforge_core::error::CoreError> for UiError {
    fn from(e: rowforge_core::error::CoreError) -> Self {
        // CoreError lacks variant-level discrimination today; treat as
        // workspace-unavailable when surfaced from store open paths,
        // internal otherwise. Plan 3 revisits when we classify more
        // narrowly per call site.
        UiError::Internal(e.to_string())
    }
}
```

- [ ] **Step 3.2: Verify the crate still builds**

Run: `cargo build -p rowforge-studio-core`
Expected: PASS.

- [ ] **Step 3.3: Commit**

```bash
git add crates/rowforge-studio-core/src/error.rs
git commit -m "studio-core: define UiError v1 subset

Three variants for the open + list code paths. Later plans add the
remaining 7 (RunAborted, RunBusy, ManifestInvalid, etc.) when their
call sites land."
```

---

## Task 4: Implement the `Workspace` projection

**Files:**
- Modify: `crates/rowforge-studio-core/src/workspace.rs`
- Test: covered in Task 6's integration test (no unit test fixture
  available without an on-disk workspace).

- [ ] **Step 4.1: Replace the stub with the real projection**

Open `crates/rowforge-studio-core/src/workspace.rs` and replace:

```rust
//! Workspace projection and open options.
//!
//! Spec: `docs/spec/studio/part-2-model.md` §2.2.1.

use serde::Serialize;
use std::path::PathBuf;

/// Options for `StudioCore::open`. None ⇒ use the platform default
/// (`rowforge_core::workspace::default_workspace_root()`).
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct OpenOpts {
    pub workspace: Option<PathBuf>,
}

impl OpenOpts {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_workspace(mut self, p: PathBuf) -> Self {
        self.workspace = Some(p);
        self
    }
}

/// A handle to the on-disk workspace identity. The `schema_version` is
/// captured at open time and never refreshed during a session.
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct Workspace {
    pub root: PathBuf,
    /// SQLite `schema_version` recorded at the moment we opened the
    /// store. Plan 3 starts enforcing a hard pin here; Plan 1 just
    /// records the value.
    pub schema_version: u8,
}
```

- [ ] **Step 4.2: Verify the crate still builds**

Run: `cargo build -p rowforge-studio-core`
Expected: PASS.

- [ ] **Step 4.3: Commit**

```bash
git add crates/rowforge-studio-core/src/workspace.rs
git commit -m "studio-core: project Workspace + OpenOpts

Concrete types for the open path. schema_version is recorded but not
yet enforced (Plan 3)."
```

---

## Task 5: Expose a `schema_version` accessor on `ExecutionStore`

`ExecutionStore::SCHEMA_VERSION` is a private constant. Plan 1 needs to
read it to populate `Workspace.schema_version`. Add a small public
accessor.

**Files:**
- Modify: `crates/rowforge-core/src/execution_store.rs`

- [ ] **Step 5.1: Write the failing test**

Append to the existing `#[cfg(test)] mod tests { ... }` block in
`crates/rowforge-core/src/execution_store.rs`:

```rust
#[test]
fn schema_version_is_exposed() {
    let tmp = tempfile::tempdir().unwrap();
    let store = ExecutionStore::open(tmp.path()).unwrap();
    assert!(store.schema_version() >= 1);
}
```

- [ ] **Step 5.2: Run to verify it fails**

Run: `cargo test -p rowforge-core --lib execution_store::tests::schema_version_is_exposed`
Expected: FAIL with "no method named `schema_version`".

- [ ] **Step 5.3: Add the accessor**

Inside `impl ExecutionStore` block (after the existing `pub fn open`), add:

```rust
/// The SQLite `schema_version` recorded after `open_with_migrations`
/// completes. Studio uses this to enforce a hard version pin
/// (spec part-4 §4.6).
pub fn schema_version(&self) -> u8 {
    SCHEMA_VERSION as u8
}
```

- [ ] **Step 5.4: Verify the test passes**

Run: `cargo test -p rowforge-core --lib execution_store::tests::schema_version_is_exposed`
Expected: PASS.

- [ ] **Step 5.5: Commit**

```bash
git add crates/rowforge-core/src/execution_store.rs
git commit -m "core: expose ExecutionStore::schema_version

Public accessor for the SQLite schema version. studio-core needs it
to populate Workspace projection (spec part-2 §2.2.1)."
```

---

## Task 6: Implement `StudioCore::open` end-to-end (test first)

The first integration test: open a workspace pointed at an
empty-but-initialized SQLite store and verify the Workspace fields.

**Files:**
- Create: `crates/rowforge-studio-core/tests/foundation.rs`
- Modify: `crates/rowforge-studio-core/src/lib.rs` — add `open` impl

- [ ] **Step 6.1: Write the failing integration test**

Create `crates/rowforge-studio-core/tests/foundation.rs`:

```rust
//! Plan 1 integration coverage.
//!
//! Each test bootstraps a temp workspace, runs CLI-equivalent setup via
//! rowforge_core::execution_store, then exercises the studio-core
//! surface. No CLI binary is invoked.

use rowforge_core::execution_store::{ExecutionStore, NewExecution};
use rowforge_studio_core::{OpenOpts, StudioCore};
use std::path::PathBuf;

/// Helper: produces a temp workspace dir with an initialized SQLite
/// store and zero executions.
fn empty_workspace() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    // Trigger schema bootstrap by opening once.
    let _store = ExecutionStore::open(tmp.path()).unwrap();
    tmp
}

#[test]
fn open_records_workspace_root_and_schema_version() {
    let tmp = empty_workspace();
    let core = StudioCore::open(
        OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    )
    .expect("open");
    assert_eq!(core.workspace().root, PathBuf::from(tmp.path()));
    assert!(core.workspace().schema_version >= 1);
}

#[test]
fn open_with_nonexistent_workspace_path_creates_it() {
    // ExecutionStore::open is permissive and creates the dir if needed.
    // Studio inherits this behaviour in Plan 1; Plan 3 will tighten
    // (read-only mode + explicit "this is a new workspace" UX).
    let tmp = tempfile::tempdir().unwrap();
    let fresh = tmp.path().join("brand-new");
    let core = StudioCore::open(
        OpenOpts::new().with_workspace(fresh.clone()),
    )
    .expect("open creates dir");
    assert_eq!(core.workspace().root, fresh);
    assert!(fresh.join("executions.db").exists());
}
```

- [ ] **Step 6.2: Run the tests to verify they fail**

Run: `cargo test -p rowforge-studio-core --test foundation`
Expected: FAIL with "no function or associated item named `open`".

- [ ] **Step 6.3: Implement `StudioCore::open`**

Open `crates/rowforge-studio-core/src/lib.rs`. Replace the existing
`impl StudioCore` block with:

```rust
impl StudioCore {
    /// Open a workspace. If `opts.workspace` is None, falls back to
    /// `rowforge_core::workspace::default_workspace_root()`.
    pub fn open(opts: OpenOpts) -> Result<Self, UiError> {
        let root = match opts.workspace {
            Some(p) => p,
            None => rowforge_core::workspace::default_workspace_root()
                .ok_or_else(|| {
                    UiError::WorkspaceUnavailable(
                        "no home directory available".into(),
                    )
                })?,
        };
        let store = rowforge_core::execution_store::ExecutionStore::open(&root)
            .map_err(|e| UiError::WorkspaceUnavailable(e.to_string()))?;
        let workspace = Workspace {
            root,
            schema_version: store.schema_version(),
        };
        Ok(Self { workspace, store })
    }

    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }
}
```

The `store` field is now used by `open`; remove any `#[allow(dead_code)]`
hints if compilation warns. The crate's other modules
(`exec_view`) will read from `store` in Task 8.

- [ ] **Step 6.4: Verify the tests pass**

Run: `cargo test -p rowforge-studio-core --test foundation`
Expected: PASS — both tests.

- [ ] **Step 6.5: Commit**

```bash
git add crates/rowforge-studio-core/src
git add crates/rowforge-studio-core/tests
git commit -m "studio-core: implement StudioCore::open

Opens (or creates) a workspace SQLite store and records schema
version. Test fixture: empty workspace + custom path."
```

---

## Task 7: Implement the `ExecSummary` projection

Reads `Execution` rows from the store and maps to `ExecSummary`. Plan 1
fills `attempts_count = 0`, `last_attempt_state = None`,
`last_attempt_counts = None`. Plan 3 backfills these.

**Files:**
- Modify: `crates/rowforge-studio-core/src/exec_view.rs`

- [ ] **Step 7.1: Replace the stub with the projection types and mapper**

Open `crates/rowforge-studio-core/src/exec_view.rs` and replace:

```rust
//! ExecSummary projection from the on-disk store.
//!
//! Spec: `docs/spec/studio/part-2-model.md` §2.2.2.
//!
//! Plan 1 scope: name + created_at + input_rows are populated; the
//! attempt-derived fields (count, last state, last counts) are stubbed
//! and filled in Plan 3 once the attempts join + meta.json read are
//! implemented.

use chrono::{DateTime, Utc};
use rowforge_core::execution_store::Execution;
use serde::Serialize;

/// Filter passed to `list`. Reserved for future use; Plan 1 has no
/// filter knobs.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct ListFilter;

/// Light-weight projection for the exec list page.
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct ExecSummary {
    pub id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub input_rows: Option<u64>,

    // Stubs filled in Plan 3.
    pub attempts_count: u32,
    pub last_attempt_state: Option<String>,
    pub last_attempt_counts: Option<AttemptCountsStub>,
}

/// Placeholder for Plan 3's full `AttemptCounts`. Kept as its own type
/// so the public field above does not change shape later.
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct AttemptCountsStub {
    pub success: u64,
    pub failed: u64,
    pub crashed: u64,
}

/// Plan 1 conversion: ignore attempts entirely.
impl From<&Execution> for ExecSummary {
    fn from(e: &Execution) -> Self {
        ExecSummary {
            id: e.id.clone(),
            name: e.name.clone().unwrap_or_default(),
            created_at: e.created_at,
            input_rows: Some(e.input_row_count),
            attempts_count: 0,
            last_attempt_state: None,
            last_attempt_counts: None,
        }
    }
}
```

- [ ] **Step 7.2: Verify the crate still builds**

Run: `cargo build -p rowforge-studio-core`
Expected: PASS.

- [ ] **Step 7.3: Commit**

```bash
git add crates/rowforge-studio-core/src/exec_view.rs
git commit -m "studio-core: project ExecSummary (Plan 1 subset)

id + name + created_at + input_rows populated from Execution rows;
the attempt-derived fields are stubs to be filled in Plan 3."
```

---

## Task 8: Implement `StudioCore::list`

**Files:**
- Modify: `crates/rowforge-studio-core/src/lib.rs`
- Modify: `crates/rowforge-studio-core/tests/foundation.rs`

- [ ] **Step 8.1: Write the failing test**

Append to `crates/rowforge-studio-core/tests/foundation.rs`:

```rust
use rowforge_studio_core::ListFilter;

#[test]
fn list_empty_workspace_returns_empty_vec() {
    let tmp = empty_workspace();
    let core = StudioCore::open(
        OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    )
    .unwrap();
    let rows = core.list(ListFilter::default()).expect("list");
    assert!(rows.is_empty(), "got {:?}", rows);
}

#[test]
fn list_reflects_executions_created_via_core() {
    let tmp = empty_workspace();
    // Write a tiny CSV the core store can snapshot. The store computes
    // input_row_count and input_csv_hash itself from the file.
    let csv = tmp.path().join("input.csv");
    std::fs::write(&csv, "billid\nb01\nb02\n").unwrap();

    // Create an execution row directly through the core store, bypassing
    // the CLI command machinery. Scope it so the connection drops before
    // we open a second one via StudioCore.
    {
        let mut store = ExecutionStore::open(tmp.path()).unwrap();
        store
            .create_execution(NewExecution {
                name: Some("smoke".into()),
                input_csv_id: "smoke-csv".into(),
                input_csv_path: csv,
                current_handler_instance_id: None,
            })
            .unwrap();
    }

    let core = StudioCore::open(
        OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    )
    .unwrap();
    let rows = core.list(ListFilter::default()).expect("list");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "smoke");
    assert_eq!(rows[0].input_rows, Some(2));
    assert_eq!(rows[0].attempts_count, 0, "Plan 1 stubs this");
}
```

`NewExecution` fields are exactly the four shown
(`name`, `input_csv_id`, `input_csv_path`, `current_handler_instance_id`);
verified against `crates/rowforge-core/src/execution_store.rs:85`. The
store derives `input_csv_hash` and `input_row_count` from the file at
`create_execution` time.

- [ ] **Step 8.2: Run to verify the new tests fail**

Run: `cargo test -p rowforge-studio-core --test foundation list_`
Expected: FAIL with "no method named `list`".

- [ ] **Step 8.3: Implement `list`**

In `crates/rowforge-studio-core/src/lib.rs`, extend the `impl StudioCore` block:

```rust
impl StudioCore {
    // ... open + workspace as before ...

    /// List all executions in this workspace, newest first.
    ///
    /// Plan 1 emits one DB call per invocation (no caching). Plan 3
    /// adds the warm-tier mtime probe per spec part-4 §4.3.
    pub fn list(&self, _filter: ListFilter) -> Result<Vec<ExecSummary>, UiError> {
        let executions = self
            .store
            .list_executions()
            .map_err(|e| UiError::Internal(e.to_string()))?;
        Ok(executions.iter().map(ExecSummary::from).collect())
    }
}
```

- [ ] **Step 8.4: Verify both new tests pass**

Run: `cargo test -p rowforge-studio-core --test foundation`
Expected: PASS — all four tests (two from Task 6, two from Task 8).

- [ ] **Step 8.5: Commit**

```bash
git add crates/rowforge-studio-core
git commit -m "studio-core: implement list()

Returns ExecSummary projections from the on-disk store, newest
first. No caching in Plan 1; warm-tier mtime probe arrives in
Plan 3 (spec part-4 §4.3)."
```

---

## Task 9: Smoke-check the workspace as a unit

Top-to-bottom run on the full repo to ensure no regressions on CLI or
core.

- [ ] **Step 9.1: Build the workspace**

Run: `cargo build`
Expected: PASS for all four crates.

- [ ] **Step 9.2: Run all tests**

Run: `cargo test`
Expected: PASS. Existing CLI / core tests + the new four foundation
tests in `rowforge-studio-core`.

- [ ] **Step 9.3: Confirm CLI is functionally unchanged**

If you have a `~/.rowforge` workspace from prior CLI use:

Run: `cargo run -p rowforge-cli -- exec list`
Expected: same output as before Plan 1 (CLI behaviour preserved).

If no workspace exists, this will print an empty list or create the
home dir — same as before the lift.

- [ ] **Step 9.4: Commit nothing (sanity check only)**

No file changes. Move on to Plan 2.

---

## Plan 1 acceptance

You can declare Plan 1 done when:

1. `cargo test -p rowforge-studio-core --test foundation` passes 4/4.
2. `cargo test` passes globally (no regressions in CLI or core).
3. The new public API matches what Plan 2 will consume:
   - `StudioCore::open(OpenOpts) -> Result<Self, UiError>`
   - `StudioCore::workspace() -> &Workspace`
   - `StudioCore::list(ListFilter) -> Result<Vec<ExecSummary>, UiError>`
4. The lift is shared: `rowforge_core::workspace::default_workspace_root`
   exists and the CLI uses it.

## What lands in Plan 2 next

- Tauri 2 scaffold under `apps/rowforge-studio/src-tauri`.
- Vite + React + Tailwind + shadcn under `apps/rowforge-studio/`.
- Tauri commands `workspace_open`, `exec_list` wrapping `StudioCore`.
- W-1 Workspace Home rendering. Workspace Picker boot screen.
- IPC contract test: a smoke test that the JSON shape produced by
  serializing `ExecSummary` matches what the React side expects.

## Open questions Plan 1 deliberately punts

1. **Workspace-not-yet-initialized UX.** Plan 1 silently creates the
   dir (matches CLI behaviour). Plan 3 should tighten with a real
   "this workspace is empty — first-run wizard?" surface.
2. **Schema-version mismatch.** Currently records the live value; does
   not refuse to open a newer schema. Plan 3 adds the hard pin per
   spec part-4 §4.6.
3. **`CoreError → UiError` mapping is currently `Internal`.** Plan 3
   classifies more narrowly at each call site.
