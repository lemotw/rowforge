# Studio Plan 03 — Exec History Browser Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** End-to-end exec history browser. Click an exec row → see attempts, rollup, bindings → click an attempt → see failed rows with raw_record + per-error histogram → click a row → row-history across attempts. Plus all Plan 1/Plan 2 carry-forwards: type renames, mutex poison fix, caching, schema pin, attempts-field backfill.

**Architecture:** Same three-layer stack as Plan 2 (`rowforge-core` ← `rowforge-studio-core` ← `apps/rowforge-studio`). Plan 3 extends the projection set and adds the React routes for exec/attempt details. No new external deps beyond what's already in `pnpm-lock.yaml`.

**Tech Stack:** Same as Plan 2. Adds: `react-router-dom` v6 path params, TanStack Query for the new cold-loading rollup/failed-page hooks.

**Spec references:** Part 1 §1.4 (in-v1 scope); Part 2 §2.2.3–2.2.7 (projections); Part 3 §3.3 (state machine — read-only here); Part 4 §4.3 (cache tiers), §4.6 (schema versioning); Part 5 §5.1 (counts-only lift), §5.2 (full API), §5.3 (spec-aligned `UiError`), §5.5 (Tauri commands); Part 7 §7.3 (IA), §7.4 Flow C, §7.5 (color map), §7.6.6 (failed-row table), §7.13 W-3 / W-4 / W-6.

---

## Decisions resolved during brainstorm

| Decision | Choice | Why |
|---|---|---|
| Plan 3 split | Single big plan (backend + UI) | Smaller PR cycles weren't worth the bookkeeping; Plan 2 size was tractable |
| Caching depth | Full spec compliance — mtime probe + 30 s TTL + refresh hooks | Spec is explicit; warm tier mandatory because of external-CLI-mutation trust erosion |
| Running attempt detail UX | Static snapshot + "May be stale" banner; manual refresh button | Lets users see *something* for in-progress attempts; live updates land in Plan 4 |
| Header workspace click | Modal with Switch / Reveal / Reload | Adds the missing entry point Plan 2 carry-forward asked for |
| `ExecutionId` / `AttemptId` newtype scope | Studio-core only; CLI stays on bare `String`; boundary converts | Avoids touching the entire CLI; spec only requires it for Studio's API |

---

## File structure

### New — `rowforge-core` extensions
- `crates/rowforge-core/src/row_resolution.rs` — add `pub fn compute_resolution_counts_only(store, exec_id) -> Result<ResolutionCounts, CoreError>` (spec §5.1 lift)

### New — `rowforge-studio-core` projections + modules
- `crates/rowforge-studio-core/src/ids.rs` — `ExecutionId` / `AttemptId` newtypes
- `crates/rowforge-studio-core/src/cache.rs` — warm-tier cache: mtime probe + TTL (`Cached<T>`, `mtime_probe`, `Invalidator`)
- `crates/rowforge-studio-core/src/exec_detail.rs` — `ExecDetail`, `AttemptSummary`, `HandlerBindingView`, `FieldMapping`, `InputFormat`
- `crates/rowforge-studio-core/src/attempt_detail.rs` — `AttemptDetail`, `AttemptStats`, `AttemptPaths`, `HandlerInstanceView`, `RunType`, `AttemptState`
- `crates/rowforge-studio-core/src/rollup.rs` — `ExecRollup`
- `crates/rowforge-studio-core/src/failed.rs` — `FailedPageQuery`, `FailedRowPage`, `FailedRow`, `RowOutcomeKind`
- `crates/rowforge-studio-core/src/row_history.rs` — `RowHistory`

### New — Tauri commands
- `apps/rowforge-studio/src-tauri/src/commands.rs` — 5 new commands: `exec_show`, `attempt_show`, `exec_rollup`, `attempt_failed_page`, `attempt_row_history`

### New — React pages + components
- `apps/rowforge-studio/src/pages/ExecDetail.tsx` (W-3)
- `apps/rowforge-studio/src/pages/AttemptDetail.tsx` (W-4 terminal subset)
- `apps/rowforge-studio/src/components/FailedRowsTable.tsx` (W-6)
- `apps/rowforge-studio/src/components/RowHistoryDrawer.tsx`
- `apps/rowforge-studio/src/components/RollupCard.tsx`
- `apps/rowforge-studio/src/components/ErrorsByCodeList.tsx`
- `apps/rowforge-studio/src/components/Breadcrumb.tsx`
- `apps/rowforge-studio/src/components/WorkspaceMenu.tsx` — header click modal
- `apps/rowforge-studio/src/components/ui/dialog.tsx` — shadcn primitive (added on demand)
- `apps/rowforge-studio/src/components/ui/tabs.tsx` — shadcn primitive
- `apps/rowforge-studio/src/components/ui/sheet.tsx` — shadcn primitive (for RowHistory drawer)

### Modified — Plan 2 carry-forward fixes (Task 1)
- `apps/rowforge-studio/src-tauri/src/commands.rs` — replace `lock().unwrap()` with `lock().unwrap_or_else(|p| p.into_inner())`
- `apps/rowforge-studio/src-tauri/src/state.rs` — fix `!Send` comment → `!Sync (and !Send)`
- `apps/rowforge-studio/src/pages/ExecList.tsx` — drop local `useWorkspaceRoot`, use canonical `useSettings`
- `apps/rowforge-studio/src-tauri/tests/ipc_contract.rs` — assert `UiError.message` field name explicitly

### Modified — `UiError` spec §5.3 alignment (Task 3)
- `crates/rowforge-studio-core/src/error.rs` — rename `WorkspaceUnavailable` → `WorkspaceLocked { by: String }`, add `NotFound { kind, id }`, `InvalidArg`, `UnknownHandle`
- `apps/rowforge-studio/src/ipc/types.ts` — sync TS mirror

### Out of scope for Plan 03
- Live `AttemptDetail` updates (Plan 4 — SessionRegistry + ProgressAggregator)
- `start_run` / `cancel` / `subscribe` (Plan 4)
- `start_exec` / `export` (Plan 5)
- `Settings` UI page (Plan 5)
- Handler authoring (Plans 6–8)
- v2 sidecar `outcomes.idx` (any future plan)
- Replay of finished attempts as event stream (Plan 7+)

---

## Task 1: Carry-forward fixes from Plan 2

Single small commit that resolves the four minor issues the Plan 2 final review noted.

**Files:**
- Modify: `apps/rowforge-studio/src-tauri/src/state.rs`
- Modify: `apps/rowforge-studio/src-tauri/src/commands.rs`
- Modify: `apps/rowforge-studio/src/pages/ExecList.tsx`
- Modify: `apps/rowforge-studio/src-tauri/tests/ipc_contract.rs`

- [ ] **Step 1.1: Fix `state.rs` `!Send` comment**

Replace the doc comment block at the top of `state.rs`:

```rust
//! App state: the lazily-opened StudioCore.
//!
//! `core` is None until the user picks a workspace via Workspace Picker
//! or the boot autoload finds settings.workspace_root.
//!
//! Lock choice: `std::sync::Mutex` (not `tokio::sync::RwLock`) because
//! `ExecutionStore` holds a `rusqlite::Connection` which is `!Sync` (and
//! `!Send` per SQLite's threading model). RwLock requires `T: Send + Sync`
//! to expose concurrent reads, which is unsound here. Mutex serializes
//! all access correctly.
```

- [ ] **Step 1.2: Fix `commands.rs` mutex poison handling**

Replace both occurrences of `state.core.lock().unwrap()`:

```rust
let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
```

In `workspace_open` and `exec_list`.

- [ ] **Step 1.3: Consolidate ExecListPage workspace query**

Open `apps/rowforge-studio/src/pages/ExecList.tsx`. Replace the local `useWorkspaceRoot` hook with `useSettings`:

```tsx
import { useSettings, useExecList } from "@/ipc/queries";
// remove the local useWorkspaceRoot definition + the useQuery + ipc imports it needs

export function ExecListPage() {
  const settings = useSettings();
  const list = useExecList(true);

  const workspace = settings.data?.workspace_root
    ? { root: settings.data.workspace_root, schema_version: settings.data.schema_version }
    : null;
  // ... rest unchanged
}
```

This dedupes the query cache and uses the canonical settings shape.

- [ ] **Step 1.4: Lock `UiError.message` field name in contract test**

Open `apps/rowforge-studio/src-tauri/tests/ipc_contract.rs`. In `ui_error_workspace_unavailable_shape`, replace the loose assertion:

```rust
// was:
// Don't assert the inner string key — we're discovering it.
// new:
assert_eq!(
    v.get("message").and_then(|m| m.as_str()),
    Some("no home"),
    "UiError content field must be 'message' (adjacent tagging): {v:?}"
);
```

Same change in `ui_error_internal_shape` for `"boom"`.

- [ ] **Step 1.5: Run tests**

```bash
cd /Users/lemo/code/lemo/repo/rowforge
cargo test -p rowforge-studio --test ipc_contract
cd apps/rowforge-studio && pnpm test
```

Expected: contract test 4/4 pass; Vitest 2/2 pass.

- [ ] **Step 1.6: Commit**

```bash
cd /Users/lemo/code/lemo/repo/rowforge
git add apps/rowforge-studio/src-tauri/src/state.rs apps/rowforge-studio/src-tauri/src/commands.rs apps/rowforge-studio/src/pages/ExecList.tsx apps/rowforge-studio/src-tauri/tests/ipc_contract.rs
git commit -m "studio-shell: Plan 2 carry-forward fixes

- state.rs: correct !Send -> !Sync comment (real lock-choice rationale)
- commands.rs: lock().unwrap_or_else(into_inner) for mutex poison
- ExecList: drop local useWorkspaceRoot, use canonical useSettings
- ipc_contract: assert UiError.message field name explicitly"
```

---

## Task 2: `ExecutionId` / `AttemptId` newtypes

**Files:**
- Create: `crates/rowforge-studio-core/src/ids.rs`
- Modify: `crates/rowforge-studio-core/src/lib.rs`
- Modify: `crates/rowforge-studio-core/src/exec_view.rs`

- [ ] **Step 2.1: Write the failing test**

Create `crates/rowforge-studio-core/src/ids.rs`:

```rust
//! Strong newtypes for execution and attempt identifiers.
//!
//! Studio uses these to prevent crossed args at call sites
//! (`StudioCore::attempt(exec, attempt)` is hard to swap). CLI continues
//! to use bare `String` IDs; conversion happens at the Tauri command
//! boundary.
//!
//! Spec: `docs/spec/studio/part-2-model.md` §2.1 entity inventory
//! (`ExecutionId`, `AttemptId` are conceptual types in the spec; this
//! module gives them concrete Rust shape).

use serde::{Deserialize, Serialize};

macro_rules! id_newtype {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(s: impl Into<String>) -> Self {
                Self(s.into())
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_owned())
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id_newtype!(ExecutionId);
id_newtype!(AttemptId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_serialize_transparently_as_string() {
        let e = ExecutionId::new("e1");
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v, serde_json::Value::String("e1".into()));
    }

    #[test]
    fn ids_deserialize_from_string() {
        let e: ExecutionId = serde_json::from_str(r#""e1""#).unwrap();
        assert_eq!(e.as_str(), "e1");
    }

    #[test]
    fn execution_and_attempt_are_distinct_types() {
        // This is a compile-time test; if the next line compiles without
        // an explicit conversion it means the type system isn't catching
        // crossed args.
        let e = ExecutionId::new("e1");
        let a = AttemptId::new("a1");
        assert_ne!(e.as_str(), a.as_str());
        // The following would not compile; uncomment to verify:
        // fn takes_exec(_: ExecutionId) {}
        // takes_exec(a);
    }
}
```

- [ ] **Step 2.2: Run — fail**

```bash
cargo test -p rowforge-studio-core --lib ids::tests
```

Expected: FAIL — unresolved module.

- [ ] **Step 2.3: Register the module**

In `crates/rowforge-studio-core/src/lib.rs`, add `pub mod ids;` and `pub use ids::{ExecutionId, AttemptId};`.

- [ ] **Step 2.4: Update `ExecSummary.id` to `ExecutionId`**

In `crates/rowforge-studio-core/src/exec_view.rs`:

```rust
use crate::ids::ExecutionId;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ExecSummary {
    pub id: ExecutionId,             // was: String
    pub name: String,
    // ... rest unchanged
}

impl From<&Execution> for ExecSummary {
    fn from(e: &Execution) -> Self {
        ExecSummary {
            id: ExecutionId::new(e.id.clone()),
            // ... rest unchanged
        }
    }
}
```

Since `ExecutionId` is `#[serde(transparent)]`, the JSON shape stays `"id": "<string>"` — no TS mirror change required for `types.ts`.

- [ ] **Step 2.5: Run all tests**

```bash
cargo test -p rowforge-studio-core
```

Expected: all 7 tests (Plan 2 foundation + settings) + 3 new ids tests = **10 tests pass**.

- [ ] **Step 2.6: Commit**

```bash
git add crates/rowforge-studio-core
git commit -m "studio-core: ExecutionId / AttemptId newtypes

Strong newtypes prevent crossed args at StudioCore::attempt(exec, attempt)
sites. serde(transparent) keeps the JSON wire format as a plain string,
so TS mirror is unchanged. CLI continues with bare String."
```

---

## Task 3: `UiError` spec §5.3 alignment

**Files:**
- Modify: `crates/rowforge-studio-core/src/error.rs`
- Modify: `apps/rowforge-studio/src-tauri/src/commands.rs`
- Modify: `apps/rowforge-studio/src/ipc/types.ts`
- Modify: `apps/rowforge-studio/src-tauri/tests/ipc_contract.rs`

- [ ] **Step 3.1: Rewrite `error.rs` with the spec §5.3 variants**

```rust
//! UI-facing error type.
//!
//! Surface aligned with spec `docs/spec/studio/part-5-api.md` §5.3.
//! Plan 3 lands the spec-named variants; the previous `WorkspaceUnavailable`
//! becomes `WorkspaceLocked { by }`. Plan 4 adds `RunAborted`, `RunBusy`,
//! `UnknownHandle`; Plan 6 adds `HandlerBusy`, `EditorNotFound`, etc.

use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error, Serialize)]
#[serde(tag = "kind", content = "message", rename_all = "snake_case")]
#[non_exhaustive]
pub enum UiError {
    /// Workspace cannot be opened: no `$HOME`, incompatible schema, or
    /// SQLite failure.
    #[error("workspace locked or incompatible: {0}")]
    WorkspaceLocked(String),

    /// Entity not found. `kind` describes what (`"execution"`, `"attempt"`).
    #[error("{0}")]
    NotFound(String),

    /// Caller-supplied argument is invalid.
    #[error("invalid argument: {0}")]
    InvalidArg(String),

    /// I/O failure reading workspace artefacts.
    #[error("io error: {0}")]
    Io(String),

    /// Internal failure. Future plans should classify instead.
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
        UiError::Internal(e.to_string())
    }
}
```

Note: kept the `content = "message"` adjacent-tagging fix from Plan 2 Task 11.

Renamed: `WorkspaceUnavailable` → `WorkspaceLocked`. The single string param replaces the spec's `WorkspaceLocked { by: String }`; we use the simpler tuple form because adjacent tagging works for newtype variants. The JSON shape becomes `{"kind":"workspace_locked","message":"<by reason>"}`.

Added: `NotFound`, `InvalidArg` for the read APIs (Tasks 8–12).

Held back for later plans: `HandlerBuildFailed`, `RunAborted`, `UnknownHandle`, `ManifestInvalid`, `RunBusy`, `HandlerBusy`, `ToolchainMissing` — added when their call sites land (Plans 4, 6, 7).

- [ ] **Step 3.2: Update `commands.rs` call sites**

In `apps/rowforge-studio/src-tauri/src/commands.rs`, replace every `UiError::WorkspaceUnavailable(...)` with `UiError::WorkspaceLocked(...)`. There should be 2 occurrences (`workspace_open` map_err, `exec_list` ok_or_else).

- [ ] **Step 3.3: Update TS mirror**

In `apps/rowforge-studio/src/ipc/types.ts`:

```ts
export type UiErrorKind =
  | "workspace_locked"
  | "not_found"
  | "invalid_arg"
  | "io"
  | "internal";

// UiError shape unchanged (kind + message); only the kind values
// rename. Frontend code that branched on "workspace_unavailable"
// must update to "workspace_locked".
```

Grep for `"workspace_unavailable"` in `apps/rowforge-studio/src/` and replace every occurrence with `"workspace_locked"`. (Realistically there are none today; BootGate/WorkspacePicker pass errors through `uiErrorMessage` which doesn't branch by kind.)

- [ ] **Step 3.4: Update contract test**

In `apps/rowforge-studio/src-tauri/tests/ipc_contract.rs`:

- Rename test `ui_error_workspace_unavailable_shape` → `ui_error_workspace_locked_shape`.
- Update the assertion: `assert_eq!(kind, "workspace_locked");`
- Update the construction: `UiError::WorkspaceLocked("no home".into())`.
- Add a new test for `NotFound`:

```rust
#[test]
fn ui_error_not_found_shape() {
    let err = UiError::NotFound("execution e1 not found".into());
    let v = serde_json::to_value(&err).unwrap();
    assert_eq!(v.get("kind").and_then(|k| k.as_str()).unwrap(), "not_found");
    assert_eq!(v.get("message").and_then(|m| m.as_str()).unwrap(), "execution e1 not found");
}
```

- [ ] **Step 3.5: Run tests**

```bash
cargo test -p rowforge-studio --test ipc_contract
cargo build -p rowforge-studio
```

Expected: 5 contract tests pass; build clean.

- [ ] **Step 3.6: Commit**

```bash
git add crates/rowforge-studio-core/src/error.rs apps/rowforge-studio/src-tauri/src/commands.rs apps/rowforge-studio/src/ipc/types.ts apps/rowforge-studio/src-tauri/tests/ipc_contract.rs
git commit -m "studio-core: align UiError with spec §5.3

Renames WorkspaceUnavailable -> WorkspaceLocked, adds NotFound and
InvalidArg variants for Plan 3 read APIs. Adjacent tagging
(content = \"message\") preserved from Plan 2 Task 11."
```

---

## Task 4: Schema-version hard pin

Per spec part-4 §4.6: Studio refuses to open a workspace with a newer schema. `Workspace.schema_version` was just recorded in Plan 1; Plan 3 enforces.

**Files:**
- Modify: `crates/rowforge-studio-core/src/lib.rs` (in `StudioCore::open`)
- Modify: `crates/rowforge-studio-core/tests/foundation.rs`

- [ ] **Step 4.1: Write the failing test**

Add to `crates/rowforge-studio-core/tests/foundation.rs`:

```rust
use rowforge_studio_core::UiError;

#[test]
fn open_refuses_newer_schema_version() {
    // Simulate a future schema by directly writing schema_version > current
    // into the SQLite pragma after a normal open.
    let tmp = tempfile::tempdir().unwrap();
    {
        let _store = ExecutionStore::open(tmp.path()).unwrap();
    }

    // Bump the user_version PRAGMA to a future schema. ExecutionStore's
    // `migrate()` reads this and refuses if higher than its known max;
    // StudioCore::open should surface that as WorkspaceLocked.
    let conn = rusqlite::Connection::open(tmp.path().join("executions.db")).unwrap();
    conn.pragma_update(None, "user_version", 99i64).unwrap();
    drop(conn);

    let result = StudioCore::open(
        OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    );
    let err = result.expect_err("should refuse newer schema");
    match err {
        UiError::WorkspaceLocked(msg) => {
            assert!(msg.contains("schema") || msg.contains("version"),
                    "expected schema/version in message, got: {msg}");
        }
        other => panic!("expected WorkspaceLocked, got: {other:?}"),
    }
}
```

Note: this test depends on `rowforge-core::execution_store` enforcing schema bounds at open time. If `ExecutionStore::open` does not currently refuse a future schema, we need to either:

- Add the refusal to `rowforge-core::execution_store::open` (preferred — both CLI and Studio benefit).
- Or do the check in `StudioCore::open` after open (Studio-only).

Check `crates/rowforge-core/src/execution_store.rs` around line 196 (the `open` function) and around line 211 (the `migrate` function). If `migrate` already errors on a future schema, the test passes after Step 4.2 wiring. If not, add the check to `migrate` in a sub-step here:

- [ ] **Step 4.2: (conditional) Add schema-bound check to `rowforge-core::execution_store::migrate`**

Only do this if the existing `migrate` doesn't refuse newer schemas. Inspect first.

If needed, add after the existing `user_version` read in `migrate`:

```rust
if current_version > SCHEMA_VERSION {
    return Err(CoreError::SchemaTooNew {
        found: current_version as u8,
        max_known: SCHEMA_VERSION as u8,
    });
}
```

Add the variant to `CoreError`:

```rust
// crates/rowforge-core/src/error.rs
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CoreError {
    // ... existing
    #[error("schema version {found} is newer than this binary knows about (max {max_known})")]
    SchemaTooNew { found: u8, max_known: u8 },
}
```

If this is added, CLI tests should still pass (no existing test creates a future schema).

- [ ] **Step 4.3: Surface as `WorkspaceLocked` in `StudioCore::open`**

In `crates/rowforge-studio-core/src/lib.rs`, in the `open` method, the existing line:

```rust
let store = rowforge_core::execution_store::ExecutionStore::open(&root)
    .map_err(|e| UiError::WorkspaceLocked(e.to_string()))?;
```

is already `WorkspaceLocked` after Task 3. The new `SchemaTooNew` from core flows through this map and becomes `WorkspaceLocked("schema version N is newer ...")`. Match the test's substring expectation.

- [ ] **Step 4.4: Run tests**

```bash
cargo test -p rowforge-studio-core --test foundation
cargo test -p rowforge-core
```

Expected: foundation 5 tests pass (4 old + 1 new); core tests pass with whatever changes you made.

- [ ] **Step 4.5: Commit**

```bash
git add crates/rowforge-studio-core crates/rowforge-core
git commit -m "core+studio: refuse newer SQLite schema_version

Enforces spec part-4 §4.6: a workspace written by a newer rowforge
cannot be opened. Surfaces as UiError::WorkspaceLocked in studio-core
with a clear message."
```

---

## Task 5: Warm-tier cache infrastructure

Per spec part-4 §4.3: warm tier caches with mtime probe + 30 s TTL +
explicit refresh triggers (window focus, user-initiated, end-of-run).
Plan 3 ships the probe + TTL; the focus / end-of-run triggers wire
through TanStack Query on the React side (Plan 4 adds end-of-run).

**Files:**
- Create: `crates/rowforge-studio-core/src/cache.rs`
- Modify: `crates/rowforge-studio-core/src/lib.rs`

- [ ] **Step 5.1: Write the cache module + tests**

Create `crates/rowforge-studio-core/src/cache.rs`:

```rust
//! Warm-tier cache for projections backed by on-disk artefacts.
//!
//! Pattern: each cached projection records (a) the mtime of its source
//! file at fetch time and (b) the wall-clock at fetch time. A subsequent
//! request re-stats the source; cache is valid iff the stat mtime equals
//! the recorded mtime AND we are within `TTL` of the fetch time.
//!
//! Spec: `docs/spec/studio/part-4-data.md` §4.3.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

pub const DEFAULT_TTL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct CacheEntry<T> {
    pub value: T,
    pub source_mtime: SystemTime,
    pub fetched_at: Instant,
}

#[derive(Debug)]
pub struct Cache<K: std::hash::Hash + Eq, T> {
    entries: Mutex<std::collections::HashMap<K, CacheEntry<T>>>,
    ttl: Duration,
}

impl<K: std::hash::Hash + Eq + Clone, T: Clone> Cache<K, T> {
    pub fn new(ttl: Duration) -> Self {
        Self {
            entries: Mutex::new(Default::default()),
            ttl,
        }
    }

    /// Return cached value iff the source mtime + TTL pass.
    pub fn get_if_fresh(&self, key: &K, source: &Path) -> Option<T> {
        let entry = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        let cached = entry.get(key)?;
        if cached.fetched_at.elapsed() > self.ttl {
            return None;
        }
        let live_mtime = std::fs::metadata(source).ok()?.modified().ok()?;
        if live_mtime != cached.source_mtime {
            return None;
        }
        Some(cached.value.clone())
    }

    pub fn put(&self, key: K, value: T, source: &Path) {
        if let Ok(meta) = std::fs::metadata(source) {
            if let Ok(mtime) = meta.modified() {
                let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
                entries.insert(
                    key,
                    CacheEntry {
                        value,
                        source_mtime: mtime,
                        fetched_at: Instant::now(),
                    },
                );
            }
        }
    }

    pub fn invalidate(&self, key: &K) {
        self.entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(key);
    }

    pub fn invalidate_all(&self) {
        self.entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
    }
}

/// Helper: workspace-level cache key for the exec list (singleton).
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct ExecListKey;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::thread::sleep;

    #[test]
    fn cache_hit_within_ttl_and_unchanged_mtime() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("src.db");
        fs::write(&p, b"v1").unwrap();

        let c: Cache<String, i32> = Cache::new(Duration::from_secs(30));
        c.put("k".into(), 42, &p);

        let got = c.get_if_fresh(&"k".into(), &p);
        assert_eq!(got, Some(42));
    }

    #[test]
    fn cache_miss_when_mtime_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("src.db");
        fs::write(&p, b"v1").unwrap();

        let c: Cache<String, i32> = Cache::new(Duration::from_secs(30));
        c.put("k".into(), 42, &p);

        // Mtime changes on next write.
        sleep(Duration::from_millis(10));
        fs::write(&p, b"v2").unwrap();

        assert_eq!(c.get_if_fresh(&"k".into(), &p), None);
    }

    #[test]
    fn cache_miss_when_ttl_expires() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("src.db");
        fs::write(&p, b"v1").unwrap();

        let c: Cache<String, i32> = Cache::new(Duration::from_millis(20));
        c.put("k".into(), 42, &p);

        sleep(Duration::from_millis(40));
        assert_eq!(c.get_if_fresh(&"k".into(), &p), None);
    }

    #[test]
    fn invalidate_drops_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("src.db");
        fs::write(&p, b"v1").unwrap();

        let c: Cache<String, i32> = Cache::new(DEFAULT_TTL);
        c.put("k".into(), 42, &p);
        c.invalidate(&"k".into());
        assert_eq!(c.get_if_fresh(&"k".into(), &p), None);
    }
}
```

- [ ] **Step 5.2: Run — fail (module not registered)**

```bash
cargo test -p rowforge-studio-core --lib cache::tests
```

Expected: FAIL — unresolved module.

- [ ] **Step 5.3: Register the module**

In `lib.rs`, add `pub mod cache;` (alphabetical). No re-export — cache is internal infrastructure used by StudioCore methods, not by callers.

- [ ] **Step 5.4: Run — pass**

```bash
cargo test -p rowforge-studio-core --lib cache::tests
```

Expected: 4 tests pass.

- [ ] **Step 5.5: Wire `Cache` into `StudioCore` for the exec list**

In `crates/rowforge-studio-core/src/lib.rs`, extend `StudioCore`:

```rust
use crate::cache::{Cache, DEFAULT_TTL, ExecListKey};

pub struct StudioCore {
    workspace: Workspace,
    store: rowforge_core::execution_store::ExecutionStore,
    exec_list_cache: Cache<ExecListKey, Vec<ExecSummary>>,
}

impl StudioCore {
    pub fn open(opts: OpenOpts) -> Result<Self, UiError> {
        // ... existing logic
        Ok(Self {
            workspace,
            store,
            exec_list_cache: Cache::new(DEFAULT_TTL),
        })
    }

    pub fn list(&self, _filter: ListFilter) -> Result<Vec<ExecSummary>, UiError> {
        let db_path = self.workspace.root.join("executions.db");
        if let Some(cached) = self.exec_list_cache.get_if_fresh(&ExecListKey, &db_path) {
            return Ok(cached);
        }
        let executions = self
            .store
            .list_executions()
            .map_err(|e| UiError::Internal(e.to_string()))?;
        let summaries: Vec<ExecSummary> = executions.iter().map(ExecSummary::from).collect();
        self.exec_list_cache.put(ExecListKey, summaries.clone(), &db_path);
        Ok(summaries)
    }
}
```

`exec_list_cache` shape is reused for `exec_show` and `attempt_show` in Tasks 8/9 (with appropriate key types and source paths).

- [ ] **Step 5.6: Foundation tests still pass**

```bash
cargo test -p rowforge-studio-core
```

Expected: 14 tests pass (previous 10 + 4 cache).

- [ ] **Step 5.7: Commit**

```bash
git add crates/rowforge-studio-core
git commit -m "studio-core: warm-tier cache + mtime probe (spec part-4 §4.3)

Cache<K, T> records source mtime + fetch time on put. get_if_fresh
re-stats and validates TTL. Wired into StudioCore::list as the
first consumer; show/attempt follow in Tasks 8-9."
```

---

## Task 6: Backfill `ExecSummary` attempt fields

Plan 1 stubbed `attempts_count: 0`, `last_attempt_state: None`,
`last_attempt_counts: None`. Plan 3 fills them.

**Files:**
- Modify: `crates/rowforge-studio-core/src/exec_view.rs`
- Modify: `crates/rowforge-studio-core/tests/foundation.rs`

- [ ] **Step 6.1: Write the failing test**

In `crates/rowforge-studio-core/tests/foundation.rs`, replace the existing `list_reflects_executions_created_via_core` assertions with strengthened ones (or add a new test):

```rust
#[test]
fn list_reflects_attempts_count_and_last_state() {
    let tmp = empty_workspace();
    let csv = tmp.path().join("input.csv");
    std::fs::write(&csv, "billid\nb01\nb02\n").unwrap();

    let exec_id = {
        let mut store = ExecutionStore::open(tmp.path()).unwrap();
        let created = store
            .create_execution(NewExecution {
                name: Some("smoke".into()),
                input_csv_id: "smoke-csv".into(),
                input_csv_path: csv,
                current_handler_instance_id: None,
            })
            .unwrap();
        // Create a finished attempt manually so attempts_count = 1.
        // The exact API depends on ExecutionStore; copy the pattern from
        // existing core tests (e.g. `attempt_lifecycle_bumps_exec_to_iterating`).
        // Pseudocode:
        // let attempt = store.start_attempt(NewAttempt { ... }).unwrap();
        // store.finish_attempt(FinishAttempt { ... }).unwrap();
        created.id
    };

    let core = StudioCore::open(
        OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    ).unwrap();
    let rows = core.list(ListFilter::default()).unwrap();
    let row = rows.iter().find(|r| r.id.as_str() == exec_id).unwrap();
    assert_eq!(row.attempts_count, 1, "attempts_count should reflect created attempts");
    assert!(row.last_attempt_state.is_some(), "last_attempt_state populated");
}
```

Verify the `NewAttempt` / `FinishAttempt` / `AttemptState` APIs in `crates/rowforge-core/src/execution_store.rs` and adapt the test code to compile.

- [ ] **Step 6.2: Run — fail (assertions don't hold yet)**

Test should run but fail at the `attempts_count == 1` assertion (Plan 1 stub returns 0).

- [ ] **Step 6.3: Backfill in `ExecSummary::from(&Execution)` — DOES NOT WORK**

The naive `From<&Execution>` impl doesn't have access to attempts; we need to switch to a constructor that takes the store. Change `ExecSummary::from` to a `fn from_execution(e: &Execution, store: &ExecutionStore) -> Self`:

```rust
use rowforge_core::execution_store::{Execution, ExecutionStore};

impl ExecSummary {
    pub fn from_execution(
        e: &Execution,
        store: &ExecutionStore,
    ) -> Result<Self, rowforge_core::error::CoreError> {
        let attempts = store.list_attempts_for_execution(&e.id)?;
        let last = attempts.last();

        let last_attempt_counts = last.map(|att| {
            // meta.json read for counts; if file is missing or partial,
            // synthesize zeros. attempt path is in the store's layout.
            let meta_path = store.attempt_dir(&e.id, &att.id).join("meta.json");
            read_meta_counts(&meta_path).unwrap_or_default()
        });

        Ok(ExecSummary {
            id: ExecutionId::new(e.id.clone()),
            name: e.name.clone().unwrap_or_default(),
            created_at: e.created_at,
            input_rows: Some(e.input_row_count),
            attempts_count: attempts.len() as u32,
            last_attempt_state: last.map(|a| a.state.as_str().to_string()),
            last_attempt_counts,
        })
    }
}

fn read_meta_counts(path: &std::path::Path) -> Option<AttemptCountsStub> {
    let bytes = std::fs::read(path).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let stats = v.get("stats")?;
    Some(AttemptCountsStub {
        success: stats.get("success")?.as_u64()?,
        failed: stats.get("failed")?.as_u64()?,
        crashed: stats.get("crashed")?.as_u64()?,
    })
}
```

Required: `store.attempt_dir(exec_id, attempt_id) -> PathBuf`. Check existing `ExecutionStore` (there's an `attempts_dir_for` or similar; grep). If not public, add as part of this task (small lift).

- [ ] **Step 6.4: Update `StudioCore::list` to use the new constructor**

```rust
pub fn list(&self, _filter: ListFilter) -> Result<Vec<ExecSummary>, UiError> {
    let db_path = self.workspace.root.join("executions.db");
    if let Some(cached) = self.exec_list_cache.get_if_fresh(&ExecListKey, &db_path) {
        return Ok(cached);
    }
    let executions = self
        .store
        .list_executions()
        .map_err(|e| UiError::Internal(e.to_string()))?;
    let summaries: Vec<ExecSummary> = executions
        .iter()
        .map(|e| ExecSummary::from_execution(e, &self.store))
        .collect::<Result<_, _>>()
        .map_err(|e| UiError::Internal(e.to_string()))?;
    self.exec_list_cache.put(ExecListKey, summaries.clone(), &db_path);
    Ok(summaries)
}
```

- [ ] **Step 6.5: Run — pass**

```bash
cargo test -p rowforge-studio-core
```

Expected: all tests pass; new attempt-count test passes.

- [ ] **Step 6.6: Commit**

```bash
git add crates/rowforge-studio-core
git commit -m "studio-core: backfill ExecSummary attempt fields

Replaces ExecSummary::from(&Execution) (returned stubs) with
from_execution(e, store) that joins attempts and reads the latest
meta.json for counts. Eliminates Plan 1 stubs."
```

---

## Task 7: `compute_resolution` counts-only lift

Per spec §5.1, lift a counts-only entry point so Studio's rollup view
doesn't materialize the canonical-success map.

**Files:**
- Modify: `crates/rowforge-core/src/row_resolution.rs`
- (optionally) Modify: `crates/rowforge-cli/src/exec_cmd.rs` — if the CLI's existing call sites can also benefit

- [ ] **Step 7.1: Write the failing test**

In `crates/rowforge-core/src/row_resolution.rs`'s `#[cfg(test)] mod tests`:

```rust
#[test]
fn counts_only_matches_full_computation() {
    // Set up a workspace with a couple of executions + outcomes; assert
    // that compute_resolution_counts_only returns the same counts as
    // compute_resolution().counts.
    let (store, exec_id) = make_fixture_with_outcomes();
    let full = compute_resolution(&store, &exec_id).unwrap();
    let counts = compute_resolution_counts_only(&store, &exec_id).unwrap();
    assert_eq!(counts.resolved, full.counts.resolved);
    assert_eq!(counts.failed_last, full.counts.failed_last);
    // ... etc
}
```

Use whatever fixture helpers exist in `row_resolution.rs` tests (there are already tests for `compute_resolution` to crib from).

- [ ] **Step 7.2: Run — fail (function doesn't exist)**

```bash
cargo test -p rowforge-core --lib row_resolution::tests::counts_only_matches_full_computation
```

Expected: compile error "no function named `compute_resolution_counts_only`".

- [ ] **Step 7.3: Implement counts-only**

Below the existing `compute_resolution`:

```rust
/// Counts-only equivalent of `compute_resolution`. Streams every
/// attempt's outcomes once and folds into `ResolutionCounts` without
/// allocating the canonical-success map. Used by Studio for rollup
/// projections that only display aggregate numbers.
///
/// Spec: `docs/spec/studio/part-5-api.md` §5.1 lift list.
pub fn compute_resolution_counts_only(
    store: &ExecutionStore,
    exec_id: &str,
) -> Result<ResolutionCounts, CoreError> {
    // Implementation: copy the inner loop of compute_resolution but
    // accumulate only into ResolutionCounts (skip the per-row HashMap
    // and the by_error_code if cheap). Reference the existing code.
    todo!("implement by extracting the counts arm of compute_resolution")
}
```

Then implement by inspecting `compute_resolution`'s body (around lines 100-400 of `row_resolution.rs`) and extracting the counts-only path.

- [ ] **Step 7.4: Run — pass**

```bash
cargo test -p rowforge-core --lib row_resolution::tests
```

Expected: existing tests pass + new counts-only test passes.

- [ ] **Step 7.5: Commit**

```bash
git add crates/rowforge-core
git commit -m "core: lift compute_resolution_counts_only

Studio's ExecRollup projection (Plan 3 Task 10) needs cross-attempt
fold without the canonical-success HashMap. Same logic as
compute_resolution, just counts."
```

---

## Task 8: `StudioCore::show` + `ExecDetail` projection

**Files:**
- Create: `crates/rowforge-studio-core/src/exec_detail.rs`
- Modify: `crates/rowforge-studio-core/src/lib.rs`
- Modify: `crates/rowforge-studio-core/tests/foundation.rs`

- [ ] **Step 8.1: Write the failing test**

```rust
#[test]
fn show_returns_exec_detail_for_existing_exec() {
    let tmp = empty_workspace();
    let csv = tmp.path().join("input.csv");
    std::fs::write(&csv, "billid\nb01\nb02\n").unwrap();
    let exec_id = {
        let mut store = ExecutionStore::open(tmp.path()).unwrap();
        store.create_execution(NewExecution {
            name: Some("smoke".into()),
            input_csv_id: "smoke-csv".into(),
            input_csv_path: csv,
            current_handler_instance_id: None,
        }).unwrap().id
    };

    let core = StudioCore::open(OpenOpts::new().with_workspace(tmp.path().to_path_buf())).unwrap();
    let detail = core.show(&ExecutionId::new(exec_id.clone())).unwrap();
    assert_eq!(detail.summary.id.as_str(), exec_id);
    assert_eq!(detail.summary.name, "smoke");
    assert_eq!(detail.attempts.len(), 0);
}

#[test]
fn show_returns_not_found_for_unknown_exec() {
    let tmp = empty_workspace();
    let core = StudioCore::open(OpenOpts::new().with_workspace(tmp.path().to_path_buf())).unwrap();
    let err = core.show(&ExecutionId::new("missing")).expect_err("should not exist");
    matches!(err, UiError::NotFound(_));
}
```

- [ ] **Step 8.2: Run — fail (no method)**

```bash
cargo test -p rowforge-studio-core --test foundation show_
```

- [ ] **Step 8.3: Implement `ExecDetail` projection**

Create `crates/rowforge-studio-core/src/exec_detail.rs`:

```rust
//! ExecDetail projection — entity page (Part 2 §2.2.3, Part 7 W-3).

use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::{ExecSummary, ExecutionId, AttemptId};

#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct ExecDetail {
    pub summary: ExecSummary,
    pub input_path_snapshot: PathBuf,
    pub input_format: InputFormat,
    pub handler_binding: HandlerBindingView,
    pub attempts: Vec<AttemptSummary>,
    pub field_mapping: Option<FieldMapping>,
    pub config_overrides: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputFormat {
    Csv,
    Jsonl,
    Ndjson,
}

#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct HandlerBindingView {
    pub handler_id: Option<String>,
    pub handler_instance_id: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct AttemptSummary {
    pub id: AttemptId,
    pub state: String, // raw AttemptState string for now
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub finished_at: Option<chrono::DateTime<chrono::Utc>>,
    pub run_type: String,
    pub stats: Option<crate::AttemptCountsStub>,
}

#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct FieldMapping {
    pub fields: BTreeMap<String, String>,
}
```

- [ ] **Step 8.4: Implement `StudioCore::show`**

In `lib.rs`:

```rust
use crate::exec_detail::{
    AttemptSummary, ExecDetail, HandlerBindingView, InputFormat,
};

impl StudioCore {
    pub fn show(&self, id: &ExecutionId) -> Result<ExecDetail, UiError> {
        let exec = self
            .store
            .get_execution(id.as_str())
            .map_err(|e| UiError::Internal(e.to_string()))?
            .ok_or_else(|| UiError::NotFound(format!("execution {} not found", id)))?;

        let summary = ExecSummary::from_execution(&exec, &self.store)
            .map_err(|e| UiError::Internal(e.to_string()))?;

        let attempts_raw = self
            .store
            .list_attempts_for_execution(id.as_str())
            .map_err(|e| UiError::Internal(e.to_string()))?;

        let attempts: Vec<AttemptSummary> = attempts_raw
            .into_iter()
            .map(|a| AttemptSummary {
                id: AttemptId::new(a.id),
                state: a.state.as_str().to_string(),
                started_at: a.started_at,
                finished_at: a.finished_at,
                run_type: a.run_type.as_str().to_string(),
                stats: None, // populated in Task 9's attempt() detail
            })
            .collect();

        Ok(ExecDetail {
            summary,
            input_path_snapshot: exec.dir.join("input.csv"),
            input_format: InputFormat::Csv, // hard-coded until manifest-time format detection
            handler_binding: HandlerBindingView {
                handler_id: None,
                handler_instance_id: exec.current_handler_instance_id.clone(),
                version: None,
            },
            attempts,
            field_mapping: None,
            config_overrides: Default::default(),
        })
    }
}
```

- [ ] **Step 8.5: Register module + run tests**

In `lib.rs` add `pub mod exec_detail;` + `pub use exec_detail::{ExecDetail, AttemptSummary, HandlerBindingView, InputFormat};`.

```bash
cargo test -p rowforge-studio-core --test foundation
```

Expected: 2 new tests pass.

- [ ] **Step 8.6: Commit**

```bash
git add crates/rowforge-studio-core
git commit -m "studio-core: ExecDetail projection + StudioCore::show

Per spec part-2 §2.2.3. Joins exec + attempts; reuses ExecSummary
via from_execution. NotFound surface for missing execution_id."
```

---

## Task 9: `StudioCore::attempt` + `AttemptDetail` projection

**Files:**
- Create: `crates/rowforge-studio-core/src/attempt_detail.rs`
- Modify: `crates/rowforge-studio-core/src/lib.rs`
- Modify: `crates/rowforge-studio-core/tests/foundation.rs`

- [ ] **Step 9.1: Write the failing test**

```rust
#[test]
fn attempt_returns_detail_for_terminal_attempt() {
    let (tmp, exec_id, attempt_id) = make_workspace_with_finished_attempt();
    let core = StudioCore::open(OpenOpts::new().with_workspace(tmp.path().to_path_buf())).unwrap();
    let det = core.attempt(&ExecutionId::new(exec_id), &AttemptId::new(attempt_id)).unwrap();
    assert!(matches!(det.state.as_str(), "done" | "aborted" | "crashed"));
    assert!(det.finished_at.is_some());
}

#[test]
fn attempt_returns_not_found_for_unknown_attempt() {
    let tmp = empty_workspace();
    let core = StudioCore::open(OpenOpts::new().with_workspace(tmp.path().to_path_buf())).unwrap();
    let err = core.attempt(&ExecutionId::new("missing"), &AttemptId::new("a1")).expect_err("");
    matches!(err, UiError::NotFound(_));
}
```

Helper `make_workspace_with_finished_attempt` constructs a fixture with one
finished attempt. Use existing core test fixtures as reference.

- [ ] **Step 9.2: Run — fail (no method)**

- [ ] **Step 9.3: Implement `AttemptDetail` projection**

Create `crates/rowforge-studio-core/src/attempt_detail.rs`:

```rust
//! AttemptDetail projection — per spec part-2 §2.2.4.
//!
//! In Plan 3 this returns a static snapshot regardless of whether the
//! attempt is still running. The Live tab (Plan 4) will replace this for
//! in-progress attempts via SessionRegistry. The Studio UI surfaces a
//! "May be stale; refresh manually" banner when state is non-terminal.

use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::{AttemptCountsStub, AttemptId, ExecutionId};

#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct AttemptDetail {
    pub id: AttemptId,
    pub execution_id: ExecutionId,
    pub state: String,                          // AttemptState raw
    pub run_type: String,                       // RunType raw
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub stats: AttemptCountsStub,
    pub by_error_code: BTreeMap<String, u64>,   // 32-entry cap with OTHER overflow
    pub handler_instance: HandlerInstanceView,
    pub paths: AttemptPaths,
    pub is_terminal: bool,                      // false ⇒ UI shows stale banner
}

#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct HandlerInstanceView {
    pub id: Option<String>,
    pub handler_id: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct AttemptPaths {
    pub meta_json: PathBuf,
    pub outcomes_jsonl: PathBuf,
    pub handler_stderr_log: PathBuf,
}
```

- [ ] **Step 9.4: Implement `StudioCore::attempt`**

```rust
use crate::attempt_detail::{AttemptDetail, AttemptPaths, HandlerInstanceView};

impl StudioCore {
    pub fn attempt(
        &self,
        e: &ExecutionId,
        r: &AttemptId,
    ) -> Result<AttemptDetail, UiError> {
        let exec = self
            .store
            .get_execution(e.as_str())
            .map_err(|err| UiError::Internal(err.to_string()))?
            .ok_or_else(|| UiError::NotFound(format!("execution {} not found", e)))?;

        let attempts = self
            .store
            .list_attempts_for_execution(e.as_str())
            .map_err(|err| UiError::Internal(err.to_string()))?;

        let attempt = attempts
            .into_iter()
            .find(|a| a.id == r.as_str())
            .ok_or_else(|| UiError::NotFound(format!("attempt {} not found", r)))?;

        let attempt_dir = exec.dir.join("attempts").join(&attempt.id);
        let meta_path = attempt_dir.join("meta.json");
        let (stats, by_error_code) = read_meta_full(&meta_path).unwrap_or_default();

        let state_str = attempt.state.as_str().to_string();
        let is_terminal = matches!(state_str.as_str(), "done" | "aborted" | "crashed");

        Ok(AttemptDetail {
            id: AttemptId::new(attempt.id),
            execution_id: e.clone(),
            state: state_str,
            run_type: attempt.run_type.as_str().to_string(),
            started_at: attempt.started_at,
            finished_at: attempt.finished_at,
            stats,
            by_error_code,
            handler_instance: HandlerInstanceView {
                id: exec.current_handler_instance_id.clone(),
                handler_id: None,
                version: None,
            },
            paths: AttemptPaths {
                meta_json: meta_path,
                outcomes_jsonl: attempt_dir.join("outcomes.jsonl"),
                handler_stderr_log: attempt_dir.join("handler.stderr.log"),
            },
            is_terminal,
        })
    }
}

fn read_meta_full(path: &std::path::Path)
    -> Option<(AttemptCountsStub, BTreeMap<String, u64>)>
{
    let bytes = std::fs::read(path).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let stats = v.get("stats")?;
    let counts = AttemptCountsStub {
        success: stats.get("success")?.as_u64().unwrap_or(0),
        failed: stats.get("failed")?.as_u64().unwrap_or(0),
        crashed: stats.get("crashed")?.as_u64().unwrap_or(0),
    };
    let by_code = v.get("by_error_code")
        .and_then(|m| m.as_object())
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_u64()?)))
                .collect()
        })
        .unwrap_or_default();
    Some((counts, by_code))
}
```

- [ ] **Step 9.5: Register + run**

```bash
cargo test -p rowforge-studio-core
```

Expected: tests pass.

- [ ] **Step 9.6: Commit**

```bash
git add crates/rowforge-studio-core
git commit -m "studio-core: AttemptDetail projection + StudioCore::attempt

Per spec part-2 §2.2.4. Reads meta.json for stats + by_error_code.
is_terminal flag drives UI's stale-snapshot banner for non-terminal
attempts (Plan 4 will replace for live)."
```

---

## Task 10: `StudioCore::rollup` + `ExecRollup` projection

**Files:**
- Create: `crates/rowforge-studio-core/src/rollup.rs`
- Modify: `crates/rowforge-studio-core/src/lib.rs`
- Modify: `crates/rowforge-studio-core/tests/foundation.rs`

- [ ] **Step 10.1: Write the failing test**

```rust
#[test]
fn rollup_returns_resolution_counts() {
    let (tmp, exec_id, _) = make_workspace_with_finished_attempt();
    let core = StudioCore::open(OpenOpts::new().with_workspace(tmp.path().to_path_buf())).unwrap();
    let r = core.rollup(&ExecutionId::new(exec_id)).unwrap();
    // Specific assertions depend on the fixture, but at minimum the
    // function should return a struct with all fields zero-or-positive.
    assert!(r.resolved >= 0);
}
```

- [ ] **Step 10.2: Implement `ExecRollup` + `rollup`**

Create `crates/rowforge-studio-core/src/rollup.rs`:

```rust
//! ExecRollup projection — cold-loaded; part-2 §2.2.5.

use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct ExecRollup {
    pub resolved: u64,
    pub failed_last: u64,
    pub crashed_last: u64,
    pub too_large: u64,
    pub never_attempted: u64,
    pub by_error_code: BTreeMap<String, u64>,
}
```

In `lib.rs`:

```rust
impl StudioCore {
    pub fn rollup(&self, id: &ExecutionId) -> Result<ExecRollup, UiError> {
        // Validate existence first to return clean NotFound.
        let _ = self.store
            .get_execution(id.as_str())
            .map_err(|e| UiError::Internal(e.to_string()))?
            .ok_or_else(|| UiError::NotFound(format!("execution {} not found", id)))?;

        let counts = rowforge_core::row_resolution::compute_resolution_counts_only(
            &self.store,
            id.as_str(),
        )
        .map_err(|e| UiError::Internal(e.to_string()))?;

        Ok(ExecRollup {
            resolved: counts.resolved,
            failed_last: counts.failed_last,
            crashed_last: counts.crashed_last,
            too_large: counts.too_large,
            never_attempted: counts.never_attempted,
            by_error_code: counts.by_error_code,
        })
    }
}
```

Verify `ResolutionCounts` field names in `rowforge-core::row_resolution`; adjust if needed.

- [ ] **Step 10.3: Run + commit**

```bash
cargo test -p rowforge-studio-core
git add crates/rowforge-studio-core
git commit -m "studio-core: ExecRollup projection + StudioCore::rollup

Per spec part-2 §2.2.5. Cold path — every call re-folds outcomes via
compute_resolution_counts_only. No caching in v1 because the fold is
expected to take seconds and is gated by user action."
```

---

## Task 11: `StudioCore::failed_page` + `FailedRowPage`

**Files:**
- Create: `crates/rowforge-studio-core/src/failed.rs`
- Modify: `crates/rowforge-studio-core/src/lib.rs`
- Modify: `crates/rowforge-studio-core/tests/foundation.rs`

- [ ] **Step 11.1: Write the failing test**

```rust
#[test]
fn failed_page_returns_failed_rows_from_outcomes() {
    let (tmp, exec_id, attempt_id) = make_workspace_with_failed_outcomes();
    let core = StudioCore::open(OpenOpts::new().with_workspace(tmp.path().to_path_buf())).unwrap();
    let page = core.failed_page(FailedPageQuery {
        execution_id: ExecutionId::new(exec_id),
        attempt_id: AttemptId::new(attempt_id),
        offset: 0,
        limit: 100,
        error_code_filter: None,
    }).unwrap();
    assert!(!page.rows.is_empty());
    assert!(page.total_known.is_none(), "Plan 3 has no index, total unknown");
}
```

Fixture helper `make_workspace_with_failed_outcomes` writes a `outcomes.jsonl`
with a mix of success/error/crash entries.

- [ ] **Step 11.2: Define types + implement `failed_page`**

Create `crates/rowforge-studio-core/src/failed.rs`:

```rust
//! FailedRowPage projection — paged scan of outcomes.jsonl. Part-2 §2.2.6.

use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::{AttemptId, ExecutionId};

#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct FailedPageQuery {
    pub execution_id: ExecutionId,
    pub attempt_id: AttemptId,
    pub offset: u64,
    pub limit: u32,
    pub error_code_filter: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct FailedRowPage {
    pub rows: Vec<FailedRow>,
    pub next_offset: Option<u64>,
    pub total_known: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct FailedRow {
    pub seq: u64,
    pub row_index: u64,
    pub kind: RowOutcomeKind,
    pub error_code: Option<String>,
    pub message: Option<String>,
    pub raw_record: serde_json::Value,
    pub dur_ms: u32,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RowOutcomeKind {
    Error,
    Crash,
    TooLarge,
}

pub fn read_failed_page(
    outcomes_jsonl: &Path,
    query: &FailedPageQuery,
) -> Result<FailedRowPage, std::io::Error> {
    use std::io::{BufRead, BufReader};
    let f = std::fs::File::open(outcomes_jsonl)?;
    let reader = BufReader::new(f);

    let limit = query.limit.min(500) as usize;
    let mut rows = Vec::with_capacity(limit);
    let mut failed_seen: u64 = 0;
    let mut last_seen_pos: u64 = 0;

    for (i, line_res) in reader.lines().enumerate() {
        let line = line_res?;
        // Parse JSON; skip success rows; skip until offset; collect up to limit.
        let v: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let kind_enum = match kind {
            "error" => RowOutcomeKind::Error,
            "crash" => RowOutcomeKind::Crash,
            "too_large" => RowOutcomeKind::TooLarge,
            _ => continue, // skip success and unknown
        };

        if let Some(ref filter) = query.error_code_filter {
            let code = v.get("code").and_then(|c| c.as_str()).unwrap_or("");
            if code != filter {
                continue;
            }
        }

        if failed_seen < query.offset {
            failed_seen += 1;
            last_seen_pos = i as u64;
            continue;
        }
        if rows.len() >= limit {
            return Ok(FailedRowPage {
                rows,
                next_offset: Some(failed_seen),
                total_known: None,
            });
        }

        rows.push(FailedRow {
            seq: v.get("seq").and_then(|s| s.as_u64()).unwrap_or(0),
            row_index: v.get("row_index").and_then(|s| s.as_u64()).unwrap_or(0),
            kind: kind_enum,
            error_code: v.get("code").and_then(|c| c.as_str()).map(String::from),
            message: v.get("message").and_then(|m| m.as_str()).map(String::from),
            raw_record: v.get("raw").cloned().unwrap_or(serde_json::Value::Null),
            dur_ms: v.get("dur_ms").and_then(|d| d.as_u64()).unwrap_or(0) as u32,
        });
        failed_seen += 1;
        last_seen_pos = i as u64;
    }

    let _ = last_seen_pos; // currently unused; reserved for v2 sidecar index
    Ok(FailedRowPage {
        rows,
        next_offset: None,
        total_known: None,
    })
}
```

In `lib.rs`:

```rust
impl StudioCore {
    pub fn failed_page(&self, q: FailedPageQuery) -> Result<FailedRowPage, UiError> {
        // Validate exec + attempt exist; otherwise NotFound.
        let exec = self.store.get_execution(q.execution_id.as_str())
            .map_err(|e| UiError::Internal(e.to_string()))?
            .ok_or_else(|| UiError::NotFound(format!("execution {} not found", q.execution_id)))?;

        let outcomes = exec.dir.join("attempts").join(q.attempt_id.as_str()).join("outcomes.jsonl");
        if !outcomes.exists() {
            return Err(UiError::NotFound(format!("attempt {} has no outcomes.jsonl", q.attempt_id)));
        }
        crate::failed::read_failed_page(&outcomes, &q)
            .map_err(|e| UiError::Io(e.to_string()))
    }
}
```

- [ ] **Step 11.3: Run + commit**

```bash
cargo test -p rowforge-studio-core
git add crates/rowforge-studio-core
git commit -m "studio-core: FailedRowPage projection + StudioCore::failed_page

Per spec part-2 §2.2.6. Linear scan from offset; cursor pagination;
total_known is None (v2 sidecar index lands in a future plan)."
```

---

## Task 12: `StudioCore::row_history` + `RowHistory`

**Files:**
- Create: `crates/rowforge-studio-core/src/row_history.rs`
- Modify: `crates/rowforge-studio-core/src/lib.rs`
- Modify: `crates/rowforge-studio-core/tests/foundation.rs`

- [ ] **Step 12.1: Write the failing test**

```rust
#[test]
fn row_history_collects_per_attempt_outcomes_for_seq() {
    let (tmp, exec_id) = make_workspace_with_multi_attempt_outcomes();
    let core = StudioCore::open(OpenOpts::new().with_workspace(tmp.path().to_path_buf())).unwrap();
    let hist = core.row_history(&ExecutionId::new(exec_id), 1).unwrap();
    assert_eq!(hist.seq, 1);
    assert!(!hist.rows.is_empty(), "should have at least one (attempt, outcome) pair");
}
```

- [ ] **Step 12.2: Define types + implement `row_history`**

Create `crates/rowforge-studio-core/src/row_history.rs`:

```rust
//! RowHistory projection — on-demand fold across attempts for one seq.
//! Part-2 §2.2.7.

use serde::Serialize;

use crate::AttemptId;
use crate::failed::RowOutcomeKind;

#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct RowHistory {
    pub seq: u64,
    pub rows: Vec<(AttemptId, RowOutcomeKind, Option<String>)>,  // (attempt, kind, error_code)
    pub resolved_at: Option<AttemptId>,
}
```

In `lib.rs`:

```rust
impl StudioCore {
    pub fn row_history(&self, e: &ExecutionId, seq: u64) -> Result<RowHistory, UiError> {
        let exec = self.store.get_execution(e.as_str())
            .map_err(|err| UiError::Internal(err.to_string()))?
            .ok_or_else(|| UiError::NotFound(format!("execution {} not found", e)))?;

        let attempts = self.store.list_attempts_for_execution(e.as_str())
            .map_err(|err| UiError::Internal(err.to_string()))?;

        let mut rows = Vec::new();
        let mut resolved_at: Option<AttemptId> = None;

        for attempt in attempts {
            let outcomes = exec.dir.join("attempts").join(&attempt.id).join("outcomes.jsonl");
            if !outcomes.exists() {
                continue;
            }
            // Scan outcomes.jsonl for matching seq.
            use std::io::{BufRead, BufReader};
            let f = std::fs::File::open(&outcomes).map_err(UiError::from)?;
            for line in BufReader::new(f).lines() {
                let line = line.map_err(UiError::from)?;
                let v: serde_json::Value = match serde_json::from_str(&line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if v.get("seq").and_then(|s| s.as_u64()) != Some(seq) {
                    continue;
                }
                let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
                let kind_enum = match kind {
                    "success" => {
                        if resolved_at.is_none() {
                            resolved_at = Some(AttemptId::new(attempt.id.clone()));
                        }
                        // Don't push success rows into history; the resolved_at
                        // pointer is the canonical signal.
                        break;
                    }
                    "error" => Some(RowOutcomeKind::Error),
                    "crash" => Some(RowOutcomeKind::Crash),
                    "too_large" => Some(RowOutcomeKind::TooLarge),
                    _ => None,
                };
                if let Some(k) = kind_enum {
                    let code = v.get("code").and_then(|c| c.as_str()).map(String::from);
                    rows.push((AttemptId::new(attempt.id.clone()), k, code));
                }
                break; // one row per (attempt, seq)
            }
        }

        Ok(RowHistory { seq, rows, resolved_at })
    }
}
```

- [ ] **Step 12.3: Run + commit**

```bash
cargo test -p rowforge-studio-core
git add crates/rowforge-studio-core
git commit -m "studio-core: RowHistory projection + StudioCore::row_history

Per spec part-2 §2.2.7. On-demand fold across attempts for one seq.
Linear per-attempt scan; constant per attempt with future sidecar
index. resolved_at points at the first success."
```

---

## Task 13: Tauri commands for the 5 new APIs

**Files:**
- Modify: `apps/rowforge-studio/src-tauri/src/commands.rs`
- Modify: `apps/rowforge-studio/src-tauri/src/lib.rs`

- [ ] **Step 13.1: Add the commands**

In `commands.rs`:

```rust
use rowforge_studio_core::{
    AttemptDetail, AttemptId, ExecDetail, ExecRollup, ExecutionId,
    FailedPageQuery, FailedRowPage, RowHistory,
};

#[tauri::command]
pub fn exec_show(
    state: State<'_, AppState>,
    id: ExecutionId,
) -> Result<ExecDetail, UiError> {
    let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
    let core = guard.as_ref().ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
    core.show(&id)
}

#[tauri::command]
pub fn attempt_show(
    state: State<'_, AppState>,
    execution_id: ExecutionId,
    attempt_id: AttemptId,
) -> Result<AttemptDetail, UiError> {
    let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
    let core = guard.as_ref().ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
    core.attempt(&execution_id, &attempt_id)
}

#[tauri::command]
pub fn exec_rollup(
    state: State<'_, AppState>,
    id: ExecutionId,
) -> Result<ExecRollup, UiError> {
    let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
    let core = guard.as_ref().ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
    core.rollup(&id)
}

#[tauri::command]
pub fn attempt_failed_page(
    state: State<'_, AppState>,
    query: FailedPageQuery,
) -> Result<FailedRowPage, UiError> {
    let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
    let core = guard.as_ref().ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
    core.failed_page(query)
}

#[tauri::command]
pub fn attempt_row_history(
    state: State<'_, AppState>,
    execution_id: ExecutionId,
    seq: u64,
) -> Result<RowHistory, UiError> {
    let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
    let core = guard.as_ref().ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
    core.row_history(&execution_id, seq)
}
```

- [ ] **Step 13.2: Register in `lib.rs`**

```rust
.invoke_handler(tauri::generate_handler![
    commands::workspace_open,
    commands::exec_list,
    commands::workspace_settings_load,
    commands::workspace_settings_save,
    commands::exec_show,
    commands::attempt_show,
    commands::exec_rollup,
    commands::attempt_failed_page,
    commands::attempt_row_history,
])
```

- [ ] **Step 13.3: Update TS mirrors**

Append to `apps/rowforge-studio/src/ipc/types.ts`:

```ts
export type ExecutionId = string;
export type AttemptId = string;

export interface ExecDetail {
  summary: ExecSummary;
  input_path_snapshot: string;
  input_format: "csv" | "jsonl" | "ndjson";
  handler_binding: { handler_id: string | null; handler_instance_id: string | null; version: string | null };
  attempts: AttemptSummary[];
  field_mapping: { fields: Record<string, string> } | null;
  config_overrides: Record<string, unknown>;
}

export interface AttemptSummary {
  id: AttemptId;
  state: string;
  started_at: string;
  finished_at: string | null;
  run_type: string;
  stats: AttemptCountsStub | null;
}

export interface AttemptDetail {
  id: AttemptId;
  execution_id: ExecutionId;
  state: string;
  run_type: string;
  started_at: string;
  finished_at: string | null;
  stats: AttemptCountsStub;
  by_error_code: Record<string, number>;
  handler_instance: { id: string | null; handler_id: string | null; version: string | null };
  paths: { meta_json: string; outcomes_jsonl: string; handler_stderr_log: string };
  is_terminal: boolean;
}

export interface ExecRollup {
  resolved: number;
  failed_last: number;
  crashed_last: number;
  too_large: number;
  never_attempted: number;
  by_error_code: Record<string, number>;
}

export type RowOutcomeKind = "error" | "crash" | "too_large";

export interface FailedPageQuery {
  execution_id: ExecutionId;
  attempt_id: AttemptId;
  offset: number;
  limit: number;
  error_code_filter: string | null;
}

export interface FailedRowPage {
  rows: FailedRow[];
  next_offset: number | null;
  total_known: number | null;
}

export interface FailedRow {
  seq: number;
  row_index: number;
  kind: RowOutcomeKind;
  error_code: string | null;
  message: string | null;
  raw_record: unknown;
  dur_ms: number;
}

export interface RowHistory {
  seq: number;
  rows: Array<[AttemptId, RowOutcomeKind, string | null]>;
  resolved_at: AttemptId | null;
}
```

Append to `apps/rowforge-studio/src/ipc/client.ts`:

```ts
import type {
  AttemptDetail, AttemptId, ExecDetail, ExecRollup, ExecutionId,
  FailedPageQuery, FailedRowPage, RowHistory,
} from "./types";

export const ipc = {
  // ... existing wrappers
  exec_show: (args: { id: ExecutionId }) => invoke<ExecDetail>("exec_show", args),
  attempt_show: (args: { executionId: ExecutionId; attemptId: AttemptId }) =>
    invoke<AttemptDetail>("attempt_show", args),
  exec_rollup: (args: { id: ExecutionId }) => invoke<ExecRollup>("exec_rollup", args),
  attempt_failed_page: (args: { query: FailedPageQuery }) =>
    invoke<FailedRowPage>("attempt_failed_page", args),
  attempt_row_history: (args: { executionId: ExecutionId; seq: number }) =>
    invoke<RowHistory>("attempt_row_history", args),
};
```

Append to `apps/rowforge-studio/src/ipc/queries.ts`:

```ts
import type { AttemptId, ExecutionId, FailedPageQuery } from "./types";

export const useExecDetail = (id: ExecutionId | null) =>
  useQuery({
    queryKey: ["exec_show", id],
    queryFn: () => ipc.exec_show({ id: id! }),
    enabled: !!id,
  });

export const useAttemptDetail = (e: ExecutionId | null, r: AttemptId | null) =>
  useQuery({
    queryKey: ["attempt_show", e, r],
    queryFn: () => ipc.attempt_show({ executionId: e!, attemptId: r! }),
    enabled: !!e && !!r,
  });

export const useExecRollup = (id: ExecutionId | null, enabled: boolean) =>
  useQuery({
    queryKey: ["exec_rollup", id],
    queryFn: () => ipc.exec_rollup({ id: id! }),
    enabled: enabled && !!id,
    staleTime: 60_000, // cold; allow longer staleness
  });

export const useFailedPage = (query: FailedPageQuery | null) =>
  useQuery({
    queryKey: ["attempt_failed_page", query?.execution_id, query?.attempt_id, query?.offset, query?.error_code_filter],
    queryFn: () => ipc.attempt_failed_page({ query: query! }),
    enabled: !!query,
  });

export const useRowHistory = (e: ExecutionId | null, seq: number | null) =>
  useQuery({
    queryKey: ["attempt_row_history", e, seq],
    queryFn: () => ipc.attempt_row_history({ executionId: e!, seq: seq! }),
    enabled: !!e && seq !== null,
  });
```

- [ ] **Step 13.4: Run build + contract test**

```bash
cargo build -p rowforge-studio
cargo test -p rowforge-studio --test ipc_contract
cd apps/rowforge-studio && pnpm tsc -b
```

Expected: builds clean, tests pass.

- [ ] **Step 13.5: Commit**

```bash
cd /Users/lemo/code/lemo/repo/rowforge
git add apps/rowforge-studio
git commit -m "studio-shell: 5 new Tauri commands for exec/attempt detail

exec_show, attempt_show, exec_rollup, attempt_failed_page,
attempt_row_history. TS mirrors + TanStack Query hooks added.
All wrap the studio-core API; no projection logic in commands.rs."
```

---

## Task 14: Routing + breadcrumb + workspace menu modal

**Files:**
- Modify: `apps/rowforge-studio/src/App.tsx`
- Create: `apps/rowforge-studio/src/components/Breadcrumb.tsx`
- Create: `apps/rowforge-studio/src/components/WorkspaceMenu.tsx`
- Create: `apps/rowforge-studio/src/components/ui/dialog.tsx` (shadcn primitive)
- Modify: `apps/rowforge-studio/src/layout/Header.tsx`

- [ ] **Step 14.1: Routes**

Replace `apps/rowforge-studio/src/App.tsx`:

```tsx
import { Route, Routes, Navigate } from "react-router-dom";
import { BootGate } from "./pages/BootGate";
import { ExecListPage } from "./pages/ExecList";
import { ExecDetailPage } from "./pages/ExecDetail";
import { AttemptDetailPage } from "./pages/AttemptDetail";

export default function App() {
  return (
    <Routes>
      <Route path="/" element={<BootGate />} />
      <Route path="/exec/:id" element={<ExecDetailPage />} />
      <Route path="/exec/:id/attempt/:aid" element={<AttemptDetailPage />} />
      <Route path="*" element={<Navigate to="/" replace />} />
    </Routes>
  );
}
```

Update `BootGate` to navigate to `/` (it stays the gate but ExecListPage is now its successor on phase=ready). Move ExecListPage to render *as the success state of BootGate at path "/"*.

Actually simpler: `BootGate` continues to render `ExecListPage` directly on success — no route navigation needed. The routes for `/exec/:id` and `/exec/:id/attempt/:aid` skip the BootGate path entirely. But then how does the workspace open for those routes?

Decision: in Plan 3, route guards check `useSettings()` and if no workspace_root, redirect to `/`. Add to ExecDetailPage / AttemptDetailPage:

```tsx
const settings = useSettings();
if (!settings.data?.workspace_root) {
  return <Navigate to="/" replace />;
}
```

And BootGate's call to `workspace_open` runs once; if user navigates directly to `/exec/X`, the workspace must already be open. If a hard refresh happens, the redirect to `/` re-triggers BootGate + autoload.

- [ ] **Step 14.2: shadcn Dialog primitive**

Create `apps/rowforge-studio/src/components/ui/dialog.tsx`. Use the canonical shadcn dialog (built on `@radix-ui/react-dialog`). Install the radix package:

```bash
cd apps/rowforge-studio
pnpm add @radix-ui/react-dialog
```

Then the dialog component (canonical shadcn shape — copy from shadcn docs).

- [ ] **Step 14.3: WorkspaceMenu modal**

Create `apps/rowforge-studio/src/components/WorkspaceMenu.tsx`:

```tsx
import { useState } from "react";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { useQueryClient } from "@tanstack/react-query";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { open as shellOpen } from "@tauri-apps/plugin-shell";
import { useOpenWorkspace } from "@/ipc/queries";

export function WorkspaceMenu({
  workspaceRoot,
  open,
  onOpenChange,
}: {
  workspaceRoot: string;
  open: boolean;
  onOpenChange: (b: boolean) => void;
}) {
  const qc = useQueryClient();
  const openMut = useOpenWorkspace();

  const reveal = () => {
    shellOpen(workspaceRoot);
    onOpenChange(false);
  };

  const switchWs = async () => {
    const picked = await openDialog({ directory: true, multiple: false });
    if (typeof picked !== "string") return;
    openMut.mutate(picked, { onSuccess: () => { onOpenChange(false); window.location.hash = "/"; } });
  };

  const reload = () => {
    qc.invalidateQueries();
    onOpenChange(false);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Workspace</DialogTitle>
        </DialogHeader>
        <div className="my-2 font-mono text-sm text-muted-foreground">{workspaceRoot}</div>
        <div className="flex flex-col gap-2">
          <Button variant="outline" onClick={reveal}>Reveal in Finder</Button>
          <Button variant="outline" onClick={reload}>Reload data</Button>
          <Button variant="outline" onClick={switchWs}>Switch workspace…</Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
```

- [ ] **Step 14.4: Breadcrumb**

Create `apps/rowforge-studio/src/components/Breadcrumb.tsx`:

```tsx
import { Link } from "react-router-dom";
import { ChevronRight } from "lucide-react";

export interface Crumb { label: string; to?: string; mono?: boolean }

export function Breadcrumb({ crumbs }: { crumbs: Crumb[] }) {
  return (
    <nav className="flex items-center gap-1 text-sm text-muted-foreground">
      {crumbs.map((c, i) => (
        <span key={i} className="flex items-center gap-1">
          {i > 0 && <ChevronRight className="h-3 w-3" />}
          {c.to ? (
            <Link to={c.to} className={c.mono ? "font-mono hover:text-foreground" : "hover:text-foreground"}>{c.label}</Link>
          ) : (
            <span className={c.mono ? "font-mono text-foreground" : "text-foreground"}>{c.label}</span>
          )}
        </span>
      ))}
    </nav>
  );
}
```

- [ ] **Step 14.5: Header gets click + menu**

Replace `apps/rowforge-studio/src/layout/Header.tsx`:

```tsx
import { useState } from "react";
import type { Workspace } from "@/ipc/types";
import { Breadcrumb, type Crumb } from "@/components/Breadcrumb";
import { WorkspaceMenu } from "@/components/WorkspaceMenu";

export function Header({
  workspace,
  crumbs,
}: {
  workspace: Workspace | null;
  crumbs?: Crumb[];
}) {
  const [menuOpen, setMenuOpen] = useState(false);
  return (
    <header className="flex h-12 items-center gap-4 border-b border-border px-4 text-sm">
      <button
        className="font-mono text-muted-foreground underline decoration-dashed underline-offset-4 hover:text-foreground disabled:no-underline"
        onClick={() => setMenuOpen(true)}
        disabled={!workspace}
      >
        {workspace?.root ?? "—"}
      </button>
      {workspace && (
        <span className="text-xs text-muted-foreground/70">schema v{workspace.schema_version}</span>
      )}
      {crumbs && crumbs.length > 0 && (
        <div className="ml-4 border-l border-border pl-4">
          <Breadcrumb crumbs={crumbs} />
        </div>
      )}
      {workspace && (
        <WorkspaceMenu workspaceRoot={workspace.root} open={menuOpen} onOpenChange={setMenuOpen} />
      )}
    </header>
  );
}
```

- [ ] **Step 14.6: Verify build**

```bash
cd apps/rowforge-studio
pnpm tsc -b
pnpm build
```

- [ ] **Step 14.7: Commit**

```bash
cd /Users/lemo/code/lemo/repo/rowforge
git add apps/rowforge-studio
git commit -m "studio-shell: routes, breadcrumb, workspace menu modal

Adds /exec/:id and /exec/:id/attempt/:aid routes plus a Header
breadcrumb + click-to-switch-workspace modal (Plan 2 carry-forward).
Dialog primitive added on demand."
```

---

## Task 15: ExecDetail page (W-3)

**Files:**
- Create: `apps/rowforge-studio/src/pages/ExecDetail.tsx`
- Create: `apps/rowforge-studio/src/components/ui/tabs.tsx` (shadcn primitive)

- [ ] **Step 15.1: Tabs primitive**

```bash
cd apps/rowforge-studio
pnpm add @radix-ui/react-tabs
```

Create `apps/rowforge-studio/src/components/ui/tabs.tsx` per canonical shadcn shape.

- [ ] **Step 15.2: ExecDetail page**

Create `apps/rowforge-studio/src/pages/ExecDetail.tsx`:

```tsx
import { Link, Navigate, useParams } from "react-router-dom";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Table, Thead, Tr, Th, Td } from "@/components/ui/table";
import { Skeleton } from "@/components/ui/skeleton";
import { AppShell } from "@/layout/AppShell";
import { useExecDetail, useSettings } from "@/ipc/queries";
import { RollupCard } from "@/components/RollupCard";
import { uiErrorMessage } from "@/ipc/types";

export function ExecDetailPage() {
  const { id } = useParams<{ id: string }>();
  const settings = useSettings();
  const detail = useExecDetail(id ?? null);

  if (!settings.data?.workspace_root) return <Navigate to="/" replace />;

  const workspace = { root: settings.data.workspace_root, schema_version: settings.data.schema_version };
  const crumbs = [
    { label: "Executions", to: "/" },
    { label: detail.data?.summary.name || id || "...", mono: true },
  ];

  return (
    <AppShell workspace={workspace} crumbs={crumbs}>
      <div className="p-6">
        {detail.isLoading && <Skeleton className="h-32 w-full" />}
        {detail.isError && (
          <div className="text-red-300">{uiErrorMessage(detail.error)}</div>
        )}
        {detail.data && (
          <>
            <header className="mb-6">
              <h1 className="text-xl font-medium">{detail.data.summary.name || "(unnamed)"}</h1>
              <div className="mt-1 font-mono text-xs text-muted-foreground">
                id: {detail.data.summary.id} · input: {detail.data.input_path_snapshot} ({detail.data.summary.input_rows ?? "?"} rows)
              </div>
            </header>

            <Tabs defaultValue="attempts">
              <TabsList>
                <TabsTrigger value="attempts">Attempts ({detail.data.attempts.length})</TabsTrigger>
                <TabsTrigger value="rollup">Rollup</TabsTrigger>
                <TabsTrigger value="bindings">Bindings</TabsTrigger>
              </TabsList>

              <TabsContent value="attempts">
                {detail.data.attempts.length === 0 ? (
                  <div className="rounded-lg border border-dashed p-10 text-center text-muted-foreground">
                    This execution has never been run.
                  </div>
                ) : (
                  <Table>
                    <Thead>
                      <Tr><Th>#</Th><Th>State</Th><Th>Started</Th><Th>Run type</Th><Th></Th></Tr>
                    </Thead>
                    <tbody>
                      {detail.data.attempts.map((a, i) => (
                        <Tr key={a.id}>
                          <Td>{i + 1}</Td>
                          <Td><StateChip state={a.state} /></Td>
                          <Td className="font-mono">{new Date(a.started_at).toISOString().replace("T"," ").slice(0,16)}</Td>
                          <Td>{a.run_type}</Td>
                          <Td><Link to={`/exec/${id}/attempt/${a.id}`} className="text-primary hover:underline">open ⏵</Link></Td>
                        </Tr>
                      ))}
                    </tbody>
                  </Table>
                )}
              </TabsContent>

              <TabsContent value="rollup">
                <RollupCard executionId={id!} />
              </TabsContent>

              <TabsContent value="bindings">
                <pre className="rounded-lg border border-border bg-neutral-900 p-4 text-xs">
{JSON.stringify({
  handler_binding: detail.data.handler_binding,
  field_mapping: detail.data.field_mapping,
  config_overrides: detail.data.config_overrides,
}, null, 2)}
                </pre>
              </TabsContent>
            </Tabs>
          </>
        )}
      </div>
    </AppShell>
  );
}

function StateChip({ state }: { state: string }) {
  const tone =
    state === "done" ? "text-emerald-400" :
    state === "aborted" ? "text-neutral-400" :
    state === "crashed" ? "text-red-400" :
    state === "running" ? "text-emerald-300" :
    "text-blue-300";
  return <span className={tone}>● {state}</span>;
}
```

- [ ] **Step 15.3: RollupCard placeholder (full impl in Task 19)**

Create `apps/rowforge-studio/src/components/RollupCard.tsx`:

```tsx
import { useExecRollup } from "@/ipc/queries";
import { Skeleton } from "@/components/ui/skeleton";
import { uiErrorMessage } from "@/ipc/types";
import { useState } from "react";
import { Button } from "@/components/ui/button";

export function RollupCard({ executionId }: { executionId: string }) {
  const [enabled, setEnabled] = useState(false);
  const q = useExecRollup(executionId, enabled);
  if (!enabled) {
    return (
      <div className="rounded-lg border border-border p-6">
        <p className="mb-3 text-sm text-muted-foreground">
          Rollup folds outcomes from every attempt; this can take a few seconds.
        </p>
        <Button onClick={() => setEnabled(true)}>Compute rollup</Button>
      </div>
    );
  }
  if (q.isLoading) return <Skeleton className="h-32 w-full" />;
  if (q.isError) return <div className="text-red-300">{uiErrorMessage(q.error)}</div>;
  if (!q.data) return null;
  const r = q.data;
  return (
    <div className="grid grid-cols-5 gap-3">
      <Stat label="resolved" value={r.resolved} tone="text-emerald-400" />
      <Stat label="failed_last" value={r.failed_last} tone="text-red-400" />
      <Stat label="crashed_last" value={r.crashed_last} tone="text-red-500" />
      <Stat label="too_large" value={r.too_large} tone="text-amber-400" />
      <Stat label="never_attempted" value={r.never_attempted} tone="text-neutral-400" />
    </div>
  );
}

function Stat({ label, value, tone }: { label: string; value: number; tone: string }) {
  return (
    <div className="rounded-lg border border-border p-4">
      <div className={`text-2xl font-medium tabular-nums ${tone}`}>{value}</div>
      <div className="mt-1 text-xs text-muted-foreground">{label}</div>
    </div>
  );
}
```

- [ ] **Step 15.4: Build + commit**

```bash
cd apps/rowforge-studio
pnpm tsc -b && pnpm build
cd /Users/lemo/code/lemo/repo/rowforge
git add apps/rowforge-studio
git commit -m "studio-shell: ExecDetail page (W-3)

Tabs: Attempts (list with state chips + open links) / Rollup
(opt-in compute via RollupCard) / Bindings (read-only JSON for
now). Routes /exec/:id."
```

---

## Task 16: AttemptDetail page (W-4 terminal subset)

**Files:**
- Create: `apps/rowforge-studio/src/pages/AttemptDetail.tsx`
- Create: `apps/rowforge-studio/src/components/ErrorsByCodeList.tsx`

- [ ] **Step 16.1: ErrorsByCodeList**

```tsx
import { Table, Thead, Tr, Th, Td } from "@/components/ui/table";

export function ErrorsByCodeList({ data }: { data: Record<string, number> }) {
  const entries = Object.entries(data).sort((a, b) => b[1] - a[1]);
  if (entries.length === 0) {
    return <div className="text-muted-foreground">No errors recorded.</div>;
  }
  return (
    <Table>
      <Thead>
        <Tr><Th>Code</Th><Th className="text-right">Count</Th></Tr>
      </Thead>
      <tbody>
        {entries.map(([code, count]) => (
          <Tr key={code}>
            <Td className="font-mono">{code}</Td>
            <Td className="text-right tabular-nums">{count}</Td>
          </Tr>
        ))}
      </tbody>
    </Table>
  );
}
```

- [ ] **Step 16.2: AttemptDetail page**

```tsx
import { Link, Navigate, useParams } from "react-router-dom";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { open as shellOpen } from "@tauri-apps/plugin-shell";
import { AppShell } from "@/layout/AppShell";
import { useAttemptDetail, useSettings } from "@/ipc/queries";
import { uiErrorMessage } from "@/ipc/types";
import { ErrorsByCodeList } from "@/components/ErrorsByCodeList";
import { FailedRowsTable } from "@/components/FailedRowsTable";

export function AttemptDetailPage() {
  const { id, aid } = useParams<{ id: string; aid: string }>();
  const settings = useSettings();
  const detail = useAttemptDetail(id ?? null, aid ?? null);

  if (!settings.data?.workspace_root) return <Navigate to="/" replace />;
  const workspace = { root: settings.data.workspace_root, schema_version: settings.data.schema_version };
  const crumbs = [
    { label: "Executions", to: "/" },
    { label: id ?? "...", to: `/exec/${id}`, mono: true },
    { label: `Attempt ${aid}`, mono: true },
  ];

  return (
    <AppShell workspace={workspace} crumbs={crumbs}>
      <div className="p-6">
        {detail.isLoading && <Skeleton className="h-32 w-full" />}
        {detail.isError && <div className="text-red-300">{uiErrorMessage(detail.error)}</div>}
        {detail.data && (
          <>
            <header className="mb-4">
              <h1 className="text-xl font-medium">Attempt {detail.data.id}</h1>
              <div className="mt-1 text-sm text-muted-foreground">
                state: {detail.data.state} · run type: {detail.data.run_type} · started {new Date(detail.data.started_at).toISOString().slice(0, 19)}
              </div>
            </header>

            {!detail.data.is_terminal && (
              <div className="mb-4 rounded border border-amber-500/40 bg-amber-500/10 p-3 text-sm text-amber-200">
                ⚠ This attempt may still be running. Snapshot may be stale.{" "}
                <button onClick={() => detail.refetch()} className="underline">Refresh manually</button> · live progress arrives in Plan 4.
              </div>
            )}

            <Tabs defaultValue="summary">
              <TabsList>
                <TabsTrigger value="summary">Summary</TabsTrigger>
                <TabsTrigger value="failed">Failed rows</TabsTrigger>
                <TabsTrigger value="errors">Errors by code</TabsTrigger>
                <TabsTrigger value="artifacts">Artifacts</TabsTrigger>
              </TabsList>

              <TabsContent value="summary">
                <div className="grid grid-cols-3 gap-3">
                  <Stat label="success" value={detail.data.stats.success} tone="text-emerald-400" />
                  <Stat label="failed" value={detail.data.stats.failed} tone="text-red-400" />
                  <Stat label="crashed" value={detail.data.stats.crashed} tone="text-red-500" />
                </div>
              </TabsContent>

              <TabsContent value="failed">
                <FailedRowsTable executionId={id!} attemptId={aid!} pathsOutcomes={detail.data.paths.outcomes_jsonl} />
              </TabsContent>

              <TabsContent value="errors">
                <ErrorsByCodeList data={detail.data.by_error_code} />
              </TabsContent>

              <TabsContent value="artifacts">
                <ul className="space-y-2 text-sm">
                  {Object.entries(detail.data.paths).map(([k, v]) => (
                    <li key={k} className="flex items-center gap-2">
                      <span className="font-mono text-muted-foreground">{k}:</span>
                      <span className="font-mono">{v}</span>
                      <Button size="sm" variant="ghost" onClick={() => shellOpen(v)}>Reveal</Button>
                    </li>
                  ))}
                </ul>
              </TabsContent>
            </Tabs>
          </>
        )}
      </div>
    </AppShell>
  );
}

function Stat({ label, value, tone }: { label: string; value: number; tone: string }) {
  return (
    <div className="rounded-lg border border-border p-4">
      <div className={`text-2xl tabular-nums ${tone}`}>{value}</div>
      <div className="mt-1 text-xs text-muted-foreground">{label}</div>
    </div>
  );
}
```

- [ ] **Step 16.3: Build + commit**

```bash
cd apps/rowforge-studio && pnpm tsc -b && pnpm build
cd /Users/lemo/code/lemo/repo/rowforge
git add apps/rowforge-studio
git commit -m "studio-shell: AttemptDetail page (W-4 terminal subset)

Tabs Summary/Failed rows/Errors by code/Artifacts. Static snapshot
with stale-banner + manual refresh when state is non-terminal
(per brainstorm decision). Live tab arrives in Plan 4."
```

---

## Task 17: FailedRowsTable component (W-6)

**Files:**
- Create: `apps/rowforge-studio/src/components/FailedRowsTable.tsx`

- [ ] **Step 17.1: Implement**

```tsx
import { useState } from "react";
import { open as shellOpen } from "@tauri-apps/plugin-shell";
import { Button } from "@/components/ui/button";
import { Table, Thead, Tr, Th, Td } from "@/components/ui/table";
import { useFailedPage } from "@/ipc/queries";
import { uiErrorMessage } from "@/ipc/types";
import type { FailedRow } from "@/ipc/types";
import { RowHistoryDrawer } from "./RowHistoryDrawer";

const PAGE_LIMIT = 100;

export function FailedRowsTable({
  executionId,
  attemptId,
  pathsOutcomes,
}: { executionId: string; attemptId: string; pathsOutcomes: string }) {
  const [offset, setOffset] = useState(0);
  const [historySeq, setHistorySeq] = useState<number | null>(null);
  const [expanded, setExpanded] = useState<Set<number>>(new Set());

  const q = useFailedPage({
    execution_id: executionId,
    attempt_id: attemptId,
    offset,
    limit: PAGE_LIMIT,
    error_code_filter: null,
  });

  const toggle = (seq: number) =>
    setExpanded((s) => {
      const n = new Set(s);
      n.has(seq) ? n.delete(seq) : n.add(seq);
      return n;
    });

  return (
    <div>
      <div className="mb-2 flex justify-end gap-2">
        <Button size="sm" variant="ghost" onClick={() => shellOpen(pathsOutcomes)}>
          Reveal outcomes.jsonl
        </Button>
      </div>

      {q.isError && <div className="text-red-300">{uiErrorMessage(q.error)}</div>}

      <Table>
        <Thead>
          <Tr>
            <Th>seq</Th>
            <Th>row</Th>
            <Th>kind</Th>
            <Th>error_code</Th>
            <Th>message</Th>
            <Th className="text-right">dur_ms</Th>
            <Th></Th>
          </Tr>
        </Thead>
        <tbody>
          {q.data?.rows.map((r: FailedRow) => (
            <FailedRowItem
              key={r.seq}
              row={r}
              expanded={expanded.has(r.seq)}
              onToggle={() => toggle(r.seq)}
              onHistory={() => setHistorySeq(r.seq)}
            />
          ))}
        </tbody>
      </Table>

      <div className="mt-3 flex items-center justify-between">
        <span className="text-sm text-muted-foreground">
          Showing {offset + 1}–{offset + (q.data?.rows.length ?? 0)} of unknown
        </span>
        {q.data?.next_offset !== null && q.data?.next_offset !== undefined && (
          <Button size="sm" variant="outline" onClick={() => setOffset(q.data!.next_offset!)}>
            Load more
          </Button>
        )}
      </div>

      <RowHistoryDrawer
        executionId={executionId}
        seq={historySeq}
        onClose={() => setHistorySeq(null)}
      />
    </div>
  );
}

function FailedRowItem({
  row, expanded, onToggle, onHistory,
}: { row: FailedRow; expanded: boolean; onToggle: () => void; onHistory: () => void }) {
  return (
    <>
      <Tr>
        <Td className="font-mono">{row.seq}</Td>
        <Td className="font-mono">{row.row_index}</Td>
        <Td><KindChip kind={row.kind} /></Td>
        <Td><span className="font-mono text-xs">{row.error_code ?? "—"}</span></Td>
        <Td className="max-w-md truncate" title={row.message ?? ""}>{row.message}</Td>
        <Td className="text-right tabular-nums">{row.dur_ms}</Td>
        <Td>
          <button onClick={onToggle} className="text-xs text-primary hover:underline">{expanded ? "hide" : "raw"}</button>
          <button onClick={onHistory} className="ml-2 text-xs text-primary hover:underline">history</button>
        </Td>
      </Tr>
      {expanded && (
        <Tr>
          <Td colSpan={7} className="bg-neutral-900/50 p-4">
            <pre className="overflow-auto text-xs">{JSON.stringify(row.raw_record, null, 2)}</pre>
          </Td>
        </Tr>
      )}
    </>
  );
}

function KindChip({ kind }: { kind: string }) {
  const tone =
    kind === "error" ? "text-red-400" :
    kind === "crash" ? "text-red-500" :
    "text-amber-400";
  return <span className={tone}>● {kind}</span>;
}
```

- [ ] **Step 17.2: Build + commit**

```bash
cd apps/rowforge-studio && pnpm tsc -b
cd /Users/lemo/code/lemo/repo/rowforge
git add apps/rowforge-studio
git commit -m "studio-shell: FailedRowsTable (W-6)

Cursor-style pagination (no page numbers per spec part-4 §4.4).
Expand row to see raw_record JSON. Per-row history button opens
the drawer (Task 18). Reveal outcomes.jsonl via Tauri shell."
```

---

## Task 18: RowHistoryDrawer

**Files:**
- Create: `apps/rowforge-studio/src/components/RowHistoryDrawer.tsx`
- Create: `apps/rowforge-studio/src/components/ui/sheet.tsx` (shadcn primitive)

- [ ] **Step 18.1: Sheet primitive**

```bash
cd apps/rowforge-studio
# Sheet is built on dialog; we may not need a separate radix package.
```

Use `@radix-ui/react-dialog` (already added Task 14) for the sheet shape. Create `apps/rowforge-studio/src/components/ui/sheet.tsx` per shadcn's canonical sheet code (slide-in from right).

- [ ] **Step 18.2: Drawer**

```tsx
import { Sheet, SheetContent, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { Skeleton } from "@/components/ui/skeleton";
import { useRowHistory } from "@/ipc/queries";
import { uiErrorMessage } from "@/ipc/types";

export function RowHistoryDrawer({
  executionId, seq, onClose,
}: { executionId: string; seq: number | null; onClose: () => void }) {
  const q = useRowHistory(executionId, seq);

  return (
    <Sheet open={seq !== null} onOpenChange={(o) => { if (!o) onClose(); }}>
      <SheetContent>
        <SheetHeader>
          <SheetTitle>Row history · seq {seq}</SheetTitle>
        </SheetHeader>
        {q.isLoading && <Skeleton className="h-32 w-full" />}
        {q.isError && <div className="text-red-300">{uiErrorMessage(q.error)}</div>}
        {q.data && (
          <div className="mt-4 space-y-2 text-sm">
            {q.data.resolved_at && (
              <div className="text-emerald-400">
                ✓ resolved at attempt {q.data.resolved_at}
              </div>
            )}
            {q.data.rows.length === 0 ? (
              <div className="text-muted-foreground">No prior attempts for this row.</div>
            ) : (
              <ul className="space-y-1 font-mono text-xs">
                {q.data.rows.map(([att, kind, code], i) => (
                  <li key={i}>
                    attempt {att}: <span className="text-red-400">{kind}</span>
                    {code && <> · {code}</>}
                  </li>
                ))}
              </ul>
            )}
          </div>
        )}
      </SheetContent>
    </Sheet>
  );
}
```

- [ ] **Step 18.3: Build + commit**

```bash
cd apps/rowforge-studio && pnpm tsc -b
cd /Users/lemo/code/lemo/repo/rowforge
git add apps/rowforge-studio
git commit -m "studio-shell: RowHistoryDrawer

Side sheet showing per-attempt outcomes for a single seq. Opens
from FailedRowsTable's per-row history button. Calls
useRowHistory which fires only when seq is non-null."
```

---

## Task 19: Polish RollupCard rendering

Make the RollupCard show `by_error_code` too (placeholder in Task 15
only showed the 5 counters).

- [ ] **Step 19.1: Extend RollupCard**

In `apps/rowforge-studio/src/components/RollupCard.tsx`, after the 5-stat grid, append:

```tsx
{Object.keys(r.by_error_code).length > 0 && (
  <div className="mt-6">
    <h3 className="mb-2 text-sm font-medium text-muted-foreground">By error code</h3>
    <ErrorsByCodeList data={r.by_error_code} />
  </div>
)}
```

Import `ErrorsByCodeList`.

- [ ] **Step 19.2: Build + commit**

```bash
git add apps/rowforge-studio/src/components/RollupCard.tsx
git commit -m "studio-shell: RollupCard renders by_error_code"
```

---

## Task 20: Backend integration tests

**Files:**
- Modify: `crates/rowforge-studio-core/tests/foundation.rs`

The tests written incrementally during Tasks 6–12 may have used inline fixtures.
This task consolidates the helper functions and adds any missing edge-case
tests:

- `show_for_exec_with_no_attempts` — ExecDetail with empty attempts list
- `attempt_for_in_progress_attempt_marks_is_terminal_false`
- `failed_page_pagination_advances_offset_correctly`
- `row_history_resolved_at_points_at_first_success`
- `cache_invalidation_after_external_mtime_change`

- [ ] **Step 20.1: Add the missing tests**

- [ ] **Step 20.2: Run + commit**

```bash
cargo test -p rowforge-studio-core
git add crates/rowforge-studio-core
git commit -m "studio-core: round out integration tests for Plan 3 APIs"
```

---

## Task 21: React smoke tests (Vitest)

**Files:**
- Create: `apps/rowforge-studio/src/__tests__/exec-detail.test.tsx`
- Create: `apps/rowforge-studio/src/__tests__/attempt-detail.test.tsx`
- Create: `apps/rowforge-studio/src/__tests__/failed-rows.test.tsx`

Each follows the switch-on-cmd-name mock pattern from Plan 2 Task 12, asserting
that:

- ExecDetail renders attempt rows and Rollup compute button
- AttemptDetail shows stats grid + stale banner only when `is_terminal: false`
- FailedRowsTable renders rows + expansion toggle + history button click works

- [ ] **Step 21.1: Write all three test files**

- [ ] **Step 21.2: Run + commit**

```bash
cd apps/rowforge-studio && pnpm test
git add apps/rowforge-studio/src/__tests__
git commit -m "studio-shell: Vitest smoke for new Plan 3 pages"
```

---

## Task 22: Final smoke + HUMAN_SMOKE.md update

- [ ] **Step 22.1: Workspace smoke**

```bash
cargo build
cargo test     # expect ≥ 168 (Plan 2 baseline) + ~15 (Plan 3 new) = ~183
cd apps/rowforge-studio
pnpm tsc -b
pnpm build
pnpm test      # expect 5 (Plan 2 had 2; Plan 3 adds 3 page tests)
```

- [ ] **Step 22.2: Update `apps/rowforge-studio/HUMAN_SMOKE.md`**

Append a new section after the existing content:

```markdown
## Plan 03 additions

After picking a workspace with executions:

- Click an exec row → routes to `/exec/<id>` → Attempts tab shows attempt list.
- Click an attempt row → routes to `/exec/<id>/attempt/<aid>` → Summary tab.
- Click `Rollup` tab → Compute button → see resolved/failed_last/crashed_last/too_large/never_attempted counters + by_error_code table.
- Click `Failed rows` tab → table appears; click a row's `raw` to expand JSON; click `history` to open drawer.
- Click `Artifacts` tab → file paths with Reveal buttons that open Finder.
- Click the header workspace path → modal with Reveal / Reload / Switch workspace.
- For a running attempt (state != done/aborted/crashed), an amber banner appears: "Snapshot may be stale" with manual Refresh button. Live updates arrive in Plan 4.

### Schema-version pin

- Quit app. Manually bump SQLite `user_version` in `<workspace>/executions.db` to 99 via `sqlite3 ... "PRAGMA user_version = 99;"`.
- Relaunch: app refuses to open with `WorkspaceLocked` error mentioning the schema version.
```

- [ ] **Step 22.3: Commit**

```bash
cd /Users/lemo/code/lemo/repo/rowforge
git add apps/rowforge-studio/HUMAN_SMOKE.md
git commit -m "studio-shell: HUMAN_SMOKE.md Plan 3 additions"
```

---

## Plan 03 acceptance

1. `cargo test` workspace-wide — ≥ 183 tests pass.
2. `pnpm tsc -b` — clean.
3. `pnpm build` — dist/ produced.
4. `pnpm test` — 5 Vitest tests pass.
5. New Tauri commands registered: `exec_show`, `attempt_show`, `exec_rollup`, `attempt_failed_page`, `attempt_row_history`.
6. `UiError` matches spec §5.3 named variants (WorkspaceLocked / NotFound / InvalidArg / Io / Internal).
7. `ExecutionId` / `AttemptId` newtypes in use.
8. `Workspace.schema_version` enforced on open (refuses newer).
9. `ExecSummary.attempts_count` / `last_attempt_state` / `last_attempt_counts` populated (no longer stubs).
10. Warm-tier cache active for exec_list (mtime probe + 30 s TTL).
11. **(human)** `pnpm tauri dev` walkthrough completes all of HUMAN_SMOKE.md §"Plan 03 additions".

## Carry-forward to Plan 04

- `SessionRegistry` + `ProgressAggregator` for live runs
- `start_run` / `cancel` / `subscribe` Tauri commands + events
- `RunHandle` + `RunStream` types
- Live AttemptDetail (replaces snapshot fallback)
- Active runs pill in header
- 4 Hz Tick / 20 Hz OutcomeSample coalescing
- `runs:active` Tauri event

## Open questions Plan 03 punts

1. **End-of-run cache invalidation hook** — spec §4.3 says completed run should trigger refresh. Plan 4 has the hook (when Aborted/Done fires). Plan 3 only has mtime-probe + TTL.
2. **`outcomes.jsonl` partial-line handling** — current `failed::read_failed_page` skips JSON parse errors silently. Should they bubble as PipelineWarning? Spec §4.6 says "unknown error codes pass through as strings" so silent skip of malformed lines is conservative; flag for Plan 4 to revisit.
3. **InputFormat detection** — Plan 3 hard-codes `InputFormat::Csv`. Real detection from manifest's `input` field needs the manifest read path; Plan 4 or later.
4. **`HandlerBindingView.handler_id` / `.version`** — Plan 3 leaves these `None`. Filling requires reading the handler instance from SQLite + manifest. Plan 6 (handler authoring) will need this anyway.
5. **Sheet primitive imported from @radix-ui/react-dialog** — shadcn's canonical Sheet uses a separate package (`@radix-ui/react-dialog` is OK but sheet conventionally has its own slot system). Tightening optional.
