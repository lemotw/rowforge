# Plan 13 — Handler smoke test Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use `- [ ]` checkbox syntax.

**Goal:** Add a Smoke test surface to HandlerDetailPage that dispatches 1–100 rows to the handler binary inline (paste JSON / fixtures file / synthetic row) and shows outcomes without creating an exec.

**Architecture:** A new `StudioCore::handler_smoke_run` reuses Plan 8's build gate and `rowforge_core::worker::Worker` to run a forced-row-mode mini-pool. Outcomes are returned synchronously (request/response, no events). A new `StudioCore::handler_smoke_load_fixtures` reads jsonl/json/csv files. Active-run gating queries sqlite by joining `attempts → handler_instances.source_snapshot_dir`. UI adds a `<SmokeSection />` to HandlerDetailPage (keeping the existing section-based layout — no tabs introduced).

**Design spec:** `docs/superpowers/specs/2026-05-26-studio-plan-13-handler-smoke-test-design.md`

**Deviations from design:**
- Spec §5.1 says "Smoke test tab" — the existing HandlerDetailPage uses Sections, not tabs. We render as a Section to avoid introducing a tab system for one entry. Visually equivalent; same data.

---

## File map

| Path | Role | Action |
|------|------|--------|
| `crates/rowforge-studio-core/src/settings.rs` | Settings struct | Modify — add 2 fields |
| `crates/rowforge-studio-core/src/workspace.rs` | OpenOpts | Modify — plumb settings through |
| `crates/rowforge-studio-core/src/smoke.rs` | Smoke types + run logic | **Create** |
| `crates/rowforge-studio-core/src/lib.rs` | StudioCore public API | Modify — add 2 methods + mod decl + re-exports |
| `crates/rowforge-core/src/execution_store.rs` | sqlite gate | Modify — add `has_active_attempt_for_handler_dir` |
| `apps/rowforge-studio/src-tauri/src/commands.rs` | Tauri commands | Modify — add 2 commands |
| `apps/rowforge-studio/src-tauri/src/lib.rs` | command registration | Modify — register 2 commands |
| `apps/rowforge-studio/src-tauri/src/settings.rs` | settings serde mirror | Modify — pass new fields through |
| `apps/rowforge-studio/src-tauri/tests/ipc_contract.rs` | ipc registration tests | Modify — add 2 tests |
| `apps/rowforge-studio/src/ipc/types.ts` | TS types | Modify — add Smoke* types |
| `apps/rowforge-studio/src/ipc/client.ts` | invoke wrappers | Modify — add 2 wrappers |
| `apps/rowforge-studio/src/ipc/use-handlers.ts` | React Query hooks | Modify — add 2 hooks |
| `apps/rowforge-studio/src/components/SmokeSection.tsx` | Smoke UI | **Create** |
| `apps/rowforge-studio/src/components/__tests__/SmokeSection.test.tsx` | Component tests | **Create** |
| `apps/rowforge-studio/src/pages/HandlerDetailPage.tsx` | mount SmokeSection | Modify — render below LastBuildSection |
| `docs/spec/studio/part-5-api.md` | API spec | Modify — add §5.X smoke commands |
| `docs/spec/studio/part-5-api.zh-Hant.md` | API spec | Modify |
| `docs/spec/studio/part-7-ui.md` | UI spec | Modify — HandlerDetailPage smoke section |
| `docs/spec/studio/part-7-ui.zh-Hant.md` | UI spec | Modify |
| `docs/spec/studio/part-8-handler-authoring.md` | replace §8.4.3 deferred-surface placeholder | Modify |
| `docs/spec/studio/part-8-handler-authoring.zh-Hant.md` | mirror | Modify |
| `docs/HUMAN_SMOKE/plan-13-handler-smoke-test.md` | Smoke walkthrough | **Create** |

---

## Task 1: Settings — smoke_default_rows + smoke_timeout_per_row_secs

**Files:**
- Modify: `crates/rowforge-studio-core/src/settings.rs`
- Modify: `crates/rowforge-studio-core/src/workspace.rs`
- Modify: `apps/rowforge-studio/src-tauri/src/settings.rs`
- Test: inline `#[cfg(test)] mod tests` in `settings.rs`

- [ ] **Step 1: Write failing test for default values**

Add to `crates/rowforge-studio-core/src/settings.rs` `tests` mod:

```rust
#[test]
fn smoke_defaults() {
    let s = Settings::default();
    assert_eq!(s.smoke_default_rows, 5);
    assert_eq!(s.smoke_timeout_per_row_secs, 30);
}

#[test]
fn smoke_fields_tolerant_to_missing() {
    let json = br#"{"schema_version": 1}"#;
    let parsed = Settings::load_from(json.as_slice()).unwrap();
    assert_eq!(parsed.smoke_default_rows, 5);
    assert_eq!(parsed.smoke_timeout_per_row_secs, 30);
}

#[test]
fn smoke_fields_roundtrip() {
    let mut s = Settings::default();
    s.smoke_default_rows = 12;
    s.smoke_timeout_per_row_secs = 90;
    let mut buf = Vec::new();
    s.save_to(&mut buf).unwrap();
    let parsed = Settings::load_from(buf.as_slice()).unwrap();
    assert_eq!(parsed.smoke_default_rows, 12);
    assert_eq!(parsed.smoke_timeout_per_row_secs, 90);
}
```

- [ ] **Step 2: Verify tests fail**

```
cargo test -p rowforge-studio-core --lib settings::tests::smoke_defaults
```

Expected: compile error (`smoke_default_rows` not a field).

- [ ] **Step 3: Add fields to Settings**

In `crates/rowforge-studio-core/src/settings.rs`:

```rust
pub struct Settings {
    pub schema_version: u8,
    pub workspace_root: Option<PathBuf>,
    pub max_concurrent_runs: Option<u32>,
    pub telemetry_opt_in: bool,
    #[serde(default)]
    pub preferred_editor: Option<String>,
    #[serde(default)]
    pub handler_log_capture_raw_stdout: bool,
    /// Plan 13: default row count in the smoke test UI.
    /// Clamped to 1..=100 by handler_smoke_run.
    #[serde(default = "default_smoke_rows")]
    pub smoke_default_rows: usize,
    /// Plan 13: per-row timeout for smoke runs (seconds).
    /// 0 means no timeout.
    #[serde(default = "default_smoke_timeout")]
    pub smoke_timeout_per_row_secs: u64,
}

fn default_smoke_rows() -> usize { 5 }
fn default_smoke_timeout() -> u64 { 30 }
```

Update `Default::default()` to set both:

```rust
impl Default for Settings {
    fn default() -> Self {
        Settings {
            schema_version: CURRENT_SCHEMA_VERSION,
            workspace_root: None,
            max_concurrent_runs: None,
            telemetry_opt_in: false,
            preferred_editor: None,
            handler_log_capture_raw_stdout: false,
            smoke_default_rows: 5,
            smoke_timeout_per_row_secs: 30,
        }
    }
}
```

- [ ] **Step 4: Run tests; expect PASS**

```
cargo test -p rowforge-studio-core --lib settings::
```

Expected: 3 new tests pass; existing settings tests still pass.

- [ ] **Step 5: Plumb fields through OpenOpts**

In `crates/rowforge-studio-core/src/workspace.rs`, add the same pair to `OpenOpts`:

```rust
pub struct OpenOpts {
    pub workspace: Option<std::path::PathBuf>,
    pub preferred_editor: Option<String>,
    pub max_concurrent_runs: Option<u32>,
    pub handler_log_capture_raw_stdout: bool,
    /// Plan 13: clamped to 1..=100 at smoke-run time.
    pub smoke_default_rows: usize,
    /// Plan 13: 0 = no timeout.
    pub smoke_timeout_per_row_secs: u64,
}
```

Update any `OpenOpts::default()` (or struct literal in tests) accordingly. Run `cargo build -p rowforge-studio-core` and fix any callers that construct `OpenOpts` literally.

- [ ] **Step 6: Mirror in tauri settings shim**

In `apps/rowforge-studio/src-tauri/src/settings.rs`, find where `OpenOpts` is constructed from a loaded `Settings` (likely in `workspace_open`). Add:

```rust
OpenOpts {
    workspace: ...,
    preferred_editor: settings.preferred_editor.clone(),
    max_concurrent_runs: settings.max_concurrent_runs,
    handler_log_capture_raw_stdout: settings.handler_log_capture_raw_stdout,
    smoke_default_rows: settings.smoke_default_rows,
    smoke_timeout_per_row_secs: settings.smoke_timeout_per_row_secs,
}
```

Run `cargo build -p rowforge-studio` to confirm it compiles.

- [ ] **Step 7: Commit**

```bash
git add crates/rowforge-studio-core/src/settings.rs \
        crates/rowforge-studio-core/src/workspace.rs \
        apps/rowforge-studio/src-tauri/src/settings.rs
git commit -m "studio: Plan 13 T1 — Settings.smoke_default_rows + smoke_timeout_per_row_secs"
```

---

## Task 2: StudioCore stores smoke settings

**Files:**
- Modify: `crates/rowforge-studio-core/src/lib.rs`
- Test: `crates/rowforge-studio-core/tests/foundation.rs` (or wherever `StudioCore::open` smoke tests live; if no test, inline)

- [ ] **Step 1: Failing test that opens a workspace with custom smoke settings**

Add to `crates/rowforge-studio-core/tests/foundation.rs`:

```rust
#[test]
fn studio_core_stores_smoke_settings_from_open_opts() {
    let tmp = tempfile::tempdir().unwrap();
    let opts = rowforge_studio_core::OpenOpts {
        workspace: Some(tmp.path().to_path_buf()),
        preferred_editor: None,
        max_concurrent_runs: None,
        handler_log_capture_raw_stdout: false,
        smoke_default_rows: 7,
        smoke_timeout_per_row_secs: 60,
    };
    let core = rowforge_studio_core::StudioCore::open(opts).unwrap();
    assert_eq!(core.smoke_default_rows(), 7);
    assert_eq!(core.smoke_timeout_per_row_secs(), 60);
}
```

- [ ] **Step 2: Verify FAIL**

```
cargo test -p rowforge-studio-core --test foundation studio_core_stores_smoke_settings_from_open_opts
```

Expected: `smoke_default_rows` method not found.

- [ ] **Step 3: Add fields to StudioCore + accessors**

In `crates/rowforge-studio-core/src/lib.rs`, in the `StudioCore` struct definition:

```rust
pub struct StudioCore {
    workspace: Workspace,
    pub(crate) store: std::sync::Arc<std::sync::Mutex<rowforge_core::execution_store::ExecutionStore>>,
    exec_list_cache: Cache<ExecListKey, Vec<ExecSummary>>,
    pub(crate) sessions: std::sync::Arc<crate::session::SessionRegistry>,
    preferred_editor: Option<String>,
    capture_raw_stdout: bool,
    build_cache: std::sync::Mutex<std::collections::HashMap<String, rowforge_core::build::BuildOutcome>>,
    /// Plan 13: clamped to 1..=100 at smoke-run time.
    smoke_default_rows: usize,
    /// Plan 13: 0 = no timeout.
    smoke_timeout_per_row_secs: u64,
    /// Plan 13: serializes concurrent smoke runs across all handlers.
    /// One smoke at a time per workspace process.
    smoke_lock: std::sync::Arc<tokio::sync::Mutex<()>>,
}
```

In `StudioCore::open`, after building the existing struct:

```rust
Ok(Self {
    workspace,
    store,
    exec_list_cache: Cache::new(DEFAULT_TTL),
    sessions,
    preferred_editor: opts.preferred_editor,
    build_cache: std::sync::Mutex::new(std::collections::HashMap::new()),
    capture_raw_stdout: opts.handler_log_capture_raw_stdout,
    smoke_default_rows: opts.smoke_default_rows.clamp(1, 100),
    smoke_timeout_per_row_secs: opts.smoke_timeout_per_row_secs,
    smoke_lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
})
```

Add the accessors near the other accessor methods (e.g. after `capture_raw_stdout`):

```rust
/// Plan 13: default row count to show in the Smoke test UI (1..=100).
pub fn smoke_default_rows(&self) -> usize {
    self.smoke_default_rows
}

/// Plan 13: per-row timeout for smoke runs (seconds; 0 = no timeout).
pub fn smoke_timeout_per_row_secs(&self) -> u64 {
    self.smoke_timeout_per_row_secs
}

/// Plan 13: update smoke defaults in-place after a settings_save so the
/// next smoke run picks up the new values. Changes don't affect already-
/// running smoke runs (none possible; smoke is synchronous w.r.t. the IPC
/// caller, but the global smoke_lock serializes them anyway).
pub fn set_smoke_defaults(&mut self, rows: usize, timeout_secs: u64) {
    self.smoke_default_rows = rows.clamp(1, 100);
    self.smoke_timeout_per_row_secs = timeout_secs;
}
```

- [ ] **Step 4: Run test; expect PASS**

```
cargo test -p rowforge-studio-core --test foundation studio_core_stores_smoke_settings_from_open_opts
```

Expected: PASS.

- [ ] **Step 5: Plumb set_smoke_defaults into the Tauri settings_save path**

In `apps/rowforge-studio/src-tauri/src/commands.rs`, find `workspace_settings_save`. After the existing `set_preferred_editor` / `set_handler_log_capture_raw_stdout` calls, add:

```rust
core.set_smoke_defaults(
    settings.smoke_default_rows,
    settings.smoke_timeout_per_row_secs,
);
```

(Look up the exact mutator/locking pattern from how `set_preferred_editor` is called in the same function; mirror it.)

- [ ] **Step 6: Verify build**

```
cargo build -p rowforge-studio
```

Expected: builds clean.

- [ ] **Step 7: Commit**

```bash
git add crates/rowforge-studio-core/src/lib.rs \
        crates/rowforge-studio-core/tests/foundation.rs \
        apps/rowforge-studio/src-tauri/src/commands.rs
git commit -m "studio: Plan 13 T2 — StudioCore stores smoke settings + accessors"
```

---

## Task 3: Smoke types — SmokeRunRequest / SmokeOutcome / SmokeRunResult

**Files:**
- Create: `crates/rowforge-studio-core/src/smoke.rs`
- Modify: `crates/rowforge-studio-core/src/lib.rs` (add `pub mod smoke;` and re-exports)
- Test: inline `#[cfg(test)] mod tests` in `smoke.rs`

- [ ] **Step 1: Failing test for SmokeOutcome wire shape**

Create `crates/rowforge-studio-core/src/smoke.rs`:

```rust
//! Plan 13 — Handler smoke test types and runner.
//!
//! See `docs/superpowers/specs/2026-05-26-studio-plan-13-handler-smoke-test-design.md`.

use serde::{Deserialize, Serialize};

/// One row's outcome from a smoke run. Status mirrors the wire protocol's
/// `Inbound::Result` / `Inbound::Error` variants, plus a `"crash"` sentinel
/// when the handler exited mid-run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct SmokeOutcome {
    pub seq: u64,
    pub status: String, // "success" | "error" | "crash"
    pub code: Option<String>,
    pub message: Option<String>,
    pub dur_ms: u64,
    pub data: Option<serde_json::Value>,
}

/// Request payload for `StudioCore::handler_smoke_run`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SmokeRunRequest {
    pub handler_name: String,
    pub rows: Vec<serde_json::Map<String, serde_json::Value>>,
}

/// Result returned by `StudioCore::handler_smoke_run`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SmokeRunResult {
    pub outcomes: Vec<SmokeOutcome>,
    pub stderr_tail: String,
    pub exit_code: Option<i32>,
    pub elapsed_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke_outcome_serializes_with_camel_compatible_snake() {
        let o = SmokeOutcome {
            seq: 3,
            status: "success".into(),
            code: None,
            message: None,
            dur_ms: 42,
            data: Some(serde_json::json!({"sent": true})),
        };
        let v = serde_json::to_value(&o).unwrap();
        assert_eq!(v["seq"], serde_json::json!(3));
        assert_eq!(v["status"], serde_json::json!("success"));
        assert_eq!(v["dur_ms"], serde_json::json!(42));
        assert_eq!(v["data"]["sent"], serde_json::json!(true));
        // None fields render as null (not omitted) — keeps TS type stable.
        assert_eq!(v["code"], serde_json::Value::Null);
    }

    #[test]
    fn smoke_run_request_roundtrip() {
        let req = SmokeRunRequest {
            handler_name: "alpha".into(),
            rows: vec![
                serde_json::Map::from_iter([
                    ("id".to_string(), serde_json::json!("1")),
                ]),
            ],
        };
        let s = serde_json::to_string(&req).unwrap();
        let parsed: SmokeRunRequest = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed.handler_name, "alpha");
        assert_eq!(parsed.rows.len(), 1);
    }

    #[test]
    fn smoke_run_result_roundtrip() {
        let r = SmokeRunResult {
            outcomes: vec![],
            stderr_tail: "boot\n".into(),
            exit_code: Some(0),
            elapsed_ms: 100,
        };
        let s = serde_json::to_string(&r).unwrap();
        let parsed: SmokeRunResult = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed.exit_code, Some(0));
        assert_eq!(parsed.stderr_tail, "boot\n");
    }
}
```

- [ ] **Step 2: Wire module + re-exports**

In `crates/rowforge-studio-core/src/lib.rs`, add to the `pub mod` list (alphabetical with siblings):

```rust
pub mod smoke;
```

In the `pub use` block (where other types are re-exported):

```rust
pub use smoke::{SmokeOutcome, SmokeRunRequest, SmokeRunResult};
```

- [ ] **Step 3: Run tests**

```
cargo test -p rowforge-studio-core --lib smoke::tests
```

Expected: 3 PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/rowforge-studio-core/src/smoke.rs \
        crates/rowforge-studio-core/src/lib.rs
git commit -m "studio-core: Plan 13 T3 — Smoke* types"
```

---

## Task 4: Fixtures loader — handler_smoke_load_fixtures

**Files:**
- Modify: `crates/rowforge-studio-core/src/smoke.rs` (add loader function)
- Modify: `crates/rowforge-studio-core/src/lib.rs` (add `StudioCore::handler_smoke_load_fixtures`)
- Test: inline in `smoke.rs`

- [ ] **Step 1: Failing tests for the 4 source types + empty case**

Add to `crates/rowforge-studio-core/src/smoke.rs`:

```rust
use std::path::Path;

/// Load up to `limit` rows from a fixtures path. Supports:
///
/// - `.jsonl` / `.ndjson` — one JSON object per line; lines that fail to parse
///   are skipped with a tracing::warn
/// - `.json`              — top-level array of objects
/// - `.csv`               — header row → object per data row (string values)
/// - directory            — pick the first matching file by the precedence
///   above (jsonl > ndjson > json > csv); non-matching dirs error
///
/// Returns `Err(UiError::InvalidArg)` when no rows are found.
pub fn load_fixtures(
    path: &Path,
    limit: usize,
) -> Result<Vec<serde_json::Map<String, serde_json::Value>>, crate::UiError> {
    let metadata = std::fs::metadata(path).map_err(|e| {
        crate::UiError::InvalidArg(format!("fixtures path: {e}"))
    })?;
    let target = if metadata.is_dir() {
        pick_fixture_in_dir(path)
            .ok_or_else(|| crate::UiError::InvalidArg(
                "directory contains no .jsonl/.ndjson/.json/.csv file".into()
            ))?
    } else {
        path.to_path_buf()
    };
    let ext = target
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let rows = match ext.as_str() {
        "jsonl" | "ndjson" => load_jsonl(&target, limit)?,
        "json" => load_json_array(&target, limit)?,
        "csv" => load_csv(&target, limit)?,
        other => {
            return Err(crate::UiError::InvalidArg(format!(
                "unsupported fixtures extension: {other}"
            )));
        }
    };
    if rows.is_empty() {
        return Err(crate::UiError::InvalidArg(
            "no rows found in fixtures path".into(),
        ));
    }
    Ok(rows)
}

fn pick_fixture_in_dir(dir: &Path) -> Option<std::path::PathBuf> {
    const PRECEDENCE: &[&str] = &["jsonl", "ndjson", "json", "csv"];
    let entries = std::fs::read_dir(dir).ok()?;
    let mut found: std::collections::HashMap<&str, std::path::PathBuf> =
        std::collections::HashMap::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|n| n.starts_with('.'))
        {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        for cand in PRECEDENCE {
            if ext == *cand && !found.contains_key(cand) {
                found.insert(cand, path.clone());
            }
        }
    }
    for cand in PRECEDENCE {
        if let Some(p) = found.remove(cand) {
            return Some(p);
        }
    }
    None
}

fn load_jsonl(
    path: &Path,
    limit: usize,
) -> Result<Vec<serde_json::Map<String, serde_json::Value>>, crate::UiError> {
    use std::io::BufRead;
    let f = std::fs::File::open(path)
        .map_err(|e| crate::UiError::Io(format!("open {}: {e}", path.display())))?;
    let mut out = Vec::with_capacity(limit.min(64));
    for (lineno, line) in std::io::BufReader::new(f).lines().enumerate() {
        if out.len() >= limit {
            break;
        }
        let line = match line {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(line = lineno + 1, error = %e, "smoke jsonl read");
                continue;
            }
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<serde_json::Value>(trimmed) {
            Ok(serde_json::Value::Object(m)) => out.push(m),
            Ok(_) => tracing::warn!(line = lineno + 1, "smoke jsonl: not an object"),
            Err(e) => tracing::warn!(line = lineno + 1, error = %e, "smoke jsonl parse"),
        }
    }
    Ok(out)
}

fn load_json_array(
    path: &Path,
    limit: usize,
) -> Result<Vec<serde_json::Map<String, serde_json::Value>>, crate::UiError> {
    let bytes = std::fs::read(path)
        .map_err(|e| crate::UiError::Io(format!("read {}: {e}", path.display())))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| crate::UiError::InvalidArg(format!("json parse: {e}")))?;
    let arr = match value {
        serde_json::Value::Array(a) => a,
        _ => {
            return Err(crate::UiError::InvalidArg(
                "json file is not a top-level array".into(),
            ))
        }
    };
    let mut out = Vec::with_capacity(arr.len().min(limit));
    for item in arr {
        if out.len() >= limit {
            break;
        }
        if let serde_json::Value::Object(m) = item {
            out.push(m);
        } else {
            tracing::warn!("smoke json array: non-object element skipped");
        }
    }
    Ok(out)
}

fn load_csv(
    path: &Path,
    limit: usize,
) -> Result<Vec<serde_json::Map<String, serde_json::Value>>, crate::UiError> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_path(path)
        .map_err(|e| crate::UiError::InvalidArg(format!("csv open: {e}")))?;
    let headers: Vec<String> = rdr
        .headers()
        .map_err(|e| crate::UiError::InvalidArg(format!("csv headers: {e}")))?
        .iter()
        .map(|s| s.to_string())
        .collect();
    let mut out = Vec::with_capacity(limit.min(64));
    for result in rdr.records() {
        if out.len() >= limit {
            break;
        }
        let record = match result {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "smoke csv row");
                continue;
            }
        };
        let mut obj = serde_json::Map::with_capacity(headers.len());
        for (i, val) in record.iter().enumerate() {
            let key = headers.get(i).cloned().unwrap_or_else(|| format!("col{i}"));
            obj.insert(key, serde_json::Value::String(val.to_string()));
        }
        out.push(obj);
    }
    Ok(out)
}
```

Add tests below `smoke_run_result_roundtrip`:

```rust
#[test]
fn load_fixtures_jsonl_happy() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("fx.jsonl");
    std::fs::write(&p, b"{\"id\":\"1\"}\n{\"id\":\"2\"}\n").unwrap();
    let rows = load_fixtures(&p, 10).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].get("id").unwrap(), &serde_json::json!("1"));
}

#[test]
fn load_fixtures_jsonl_respects_limit() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("fx.jsonl");
    std::fs::write(&p, b"{\"id\":\"1\"}\n{\"id\":\"2\"}\n{\"id\":\"3\"}\n").unwrap();
    let rows = load_fixtures(&p, 2).unwrap();
    assert_eq!(rows.len(), 2);
}

#[test]
fn load_fixtures_json_array() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("fx.json");
    std::fs::write(&p, br#"[{"id":"1"},{"id":"2"}]"#).unwrap();
    let rows = load_fixtures(&p, 10).unwrap();
    assert_eq!(rows.len(), 2);
}

#[test]
fn load_fixtures_csv() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("fx.csv");
    std::fs::write(&p, b"id,email\n1,a@x.com\n2,b@x.com\n").unwrap();
    let rows = load_fixtures(&p, 10).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].get("id").unwrap(), &serde_json::json!("1"));
    assert_eq!(rows[0].get("email").unwrap(), &serde_json::json!("a@x.com"));
}

#[test]
fn load_fixtures_dir_picks_first_match() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.csv"), b"id\n1\n").unwrap();
    std::fs::write(dir.path().join("b.jsonl"), b"{\"id\":\"2\"}\n").unwrap();
    let rows = load_fixtures(dir.path(), 10).unwrap();
    // jsonl precedes csv
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("id").unwrap(), &serde_json::json!("2"));
}

#[test]
fn load_fixtures_empty_returns_invalid_arg() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("fx.jsonl");
    std::fs::write(&p, b"").unwrap();
    let err = load_fixtures(&p, 10).unwrap_err();
    assert!(matches!(err, crate::UiError::InvalidArg(_)));
}

#[test]
fn load_fixtures_unsupported_ext() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("fx.txt");
    std::fs::write(&p, b"hello").unwrap();
    let err = load_fixtures(&p, 10).unwrap_err();
    assert!(matches!(err, crate::UiError::InvalidArg(_)));
}
```

- [ ] **Step 2: Verify FAIL**

```
cargo test -p rowforge-studio-core --lib smoke::tests::load_fixtures
```

Expected: compile errors — `csv` crate not yet a dep, possibly `tempfile`.

- [ ] **Step 3: Add csv dependency**

Check `crates/rowforge-studio-core/Cargo.toml`. If `csv` isn't listed, add under `[dependencies]`:

```toml
csv = "1"
```

`tempfile` should already be a `[dev-dependencies]` entry from earlier plans; if not, add it.

Build to confirm:

```
cargo build -p rowforge-studio-core
```

- [ ] **Step 4: Run tests; expect PASS**

```
cargo test -p rowforge-studio-core --lib smoke::tests::load_fixtures
```

Expected: 7 PASS.

- [ ] **Step 5: Add `StudioCore::handler_smoke_load_fixtures` wrapper**

In `crates/rowforge-studio-core/src/lib.rs`, near the other `handler_*` methods, add:

```rust
/// Plan 13: read up to `limit` rows from a fixtures path. Path may be
/// anywhere on disk; it is not constrained to the workspace.
///
/// Errors:
/// - `InvalidArg` — path missing / unsupported extension / no rows found
/// - `Io`         — read failure on a recognized extension
pub fn handler_smoke_load_fixtures(
    &self,
    path: &std::path::Path,
    limit: usize,
) -> Result<Vec<serde_json::Map<String, serde_json::Value>>, UiError> {
    let clamped = limit.clamp(1, 100);
    crate::smoke::load_fixtures(path, clamped)
}
```

- [ ] **Step 6: Build + test**

```
cargo build -p rowforge-studio-core && cargo test -p rowforge-studio-core --lib smoke
```

Expected: builds + tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/rowforge-studio-core/src/smoke.rs \
        crates/rowforge-studio-core/src/lib.rs \
        crates/rowforge-studio-core/Cargo.toml
git commit -m "studio-core: Plan 13 T4 — handler_smoke_load_fixtures"
```

---

## Task 5: Active-run gate — has_active_attempt_for_handler_dir

**Files:**
- Modify: `crates/rowforge-core/src/execution_store.rs`
- Test: inline `#[cfg(test)] mod tests` in `execution_store.rs` (alongside existing tests)

- [ ] **Step 1: Failing test that seeds a running attempt and asserts the gate fires**

Add to `crates/rowforge-core/src/execution_store.rs` tests module. Look at existing tests like `has_active_attempt_returns_true_for_running` and mirror the seeding pattern. Skeleton:

```rust
#[test]
fn has_active_attempt_for_handler_dir_matches_only_target_handler() {
    let tmp = tempfile::tempdir().unwrap();
    let mut store = ExecutionStore::open(tmp.path()).unwrap();

    // Two handler instances sharing the same handler_id but different snapshot dirs.
    let hi_a = store
        .upsert_handler_instance(NewHandlerInstance {
            handler_id: "alpha".into(),
            manifest_hash: "h1".into(),
            source_snapshot_dir: tmp.path().join("handlers/alpha"),
            binary_hash: None,
        })
        .unwrap();
    let hi_b = store
        .upsert_handler_instance(NewHandlerInstance {
            handler_id: "beta".into(),
            manifest_hash: "h2".into(),
            source_snapshot_dir: tmp.path().join("handlers/beta"),
            binary_hash: None,
        })
        .unwrap();

    // One running attempt against alpha; none for beta.
    let exec_id = "e_test01";
    store
        .insert_execution(NewExecution {
            id: exec_id.into(),
            name: None,
            input_csv_id: "csv_unregistered".into(),
            input_csv_hash: "x".into(),
            input_row_count: 10,
            current_handler_instance_id: Some(hi_a.id.clone()),
            state: ExecutionState::Pending,
        })
        .unwrap();
    store
        .insert_attempt(NewAttempt {
            id: "r_test01".into(),
            execution_id: exec_id.into(),
            handler_instance_id: hi_a.id.clone(),
            parent_attempt_id: None,
            run_type_source: "full".into(),
            run_type_sample_size: None,
            run_type_simulation: "real".into(),
            state: "running".into(),
            success_count: 0,
            failed_count: 0,
            aborted_reason: None,
        })
        .unwrap();

    let alpha_dir = tmp.path().join("handlers/alpha");
    let beta_dir = tmp.path().join("handlers/beta");
    assert!(store
        .has_active_attempt_for_handler_dir(&alpha_dir)
        .unwrap());
    assert!(!store
        .has_active_attempt_for_handler_dir(&beta_dir)
        .unwrap());
}
```

(Field names / constructors come from the existing `NewExecution` / `NewAttempt` / `NewHandlerInstance` shapes already present in `execution_store.rs` — read the file for exact types before writing the test; the test SHOULD compile against the existing schema except for the missing method.)

- [ ] **Step 2: Verify FAIL**

```
cargo test -p rowforge-core --lib has_active_attempt_for_handler_dir_matches_only_target_handler
```

Expected: method not found.

- [ ] **Step 3: Implement the gate**

In `crates/rowforge-core/src/execution_store.rs`, add next to `has_active_attempt`:

```rust
/// Plan 13: cross-process active-run gate for "is this handler dir busy?"
///
/// Returns true when ANY attempt joined through `handler_instances` to
/// the given `handler_dir` is in a non-terminal state. The smoke runner
/// uses this to refuse a smoke when an exec attempt is already running
/// against the same handler binary.
///
/// `handler_dir` is compared as `source_snapshot_dir` text — the caller
/// must pass the exact canonical path used at handler-instance insert time
/// (i.e. `<workspace>/handlers/<name>`, no trailing slash).
pub fn has_active_attempt_for_handler_dir(
    &self,
    handler_dir: &Path,
) -> Result<bool> {
    let dir_str = handler_dir.to_string_lossy();
    let count: i64 = self.conn.query_row(
        "SELECT COUNT(*)
           FROM attempts a
           JOIN handler_instances hi ON a.handler_instance_id = hi.id
          WHERE hi.source_snapshot_dir = ?1
            AND a.state NOT IN ('completed', 'aborted')",
        params![dir_str.as_ref()],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}
```

- [ ] **Step 4: Run test; expect PASS**

```
cargo test -p rowforge-core --lib has_active_attempt_for_handler_dir
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/rowforge-core/src/execution_store.rs
git commit -m "rowforge-core: Plan 13 T5 — has_active_attempt_for_handler_dir gate"
```

---

## Task 6: handler_smoke_run — the actual runner

**Files:**
- Modify: `crates/rowforge-studio-core/src/smoke.rs`
- Modify: `crates/rowforge-studio-core/src/lib.rs`
- Test: `crates/rowforge-studio-core/tests/foundation.rs` (uses the existing test-handler fixture pattern)

- [ ] **Step 1: Add `UiError::HandlerBusy` variant**

In `crates/rowforge-studio-core/src/error.rs`, after `ExecutionInUse`:

```rust
/// Handler has an active run; must finish or cancel it before smoke testing.
#[error("handler '{name}' has an active run; cancel it first")]
HandlerBusy { name: String },
```

Add a serializer test in the same file's `tests` mod:

```rust
#[test]
fn handler_busy_carries_name() {
    let e = UiError::HandlerBusy { name: "alpha".into() };
    let v = serde_json::to_value(&e).unwrap();
    assert_eq!(v["kind"], json!("handler_busy"));
    assert_eq!(v["message"]["name"], json!("alpha"));
}
```

Run the test, confirm it fails to compile (new variant), then build — should pass once the variant is added because `#[non_exhaustive]` is already on the enum.

```
cargo test -p rowforge-studio-core --lib error::tests::handler_busy_carries_name
```

Expected: PASS.

- [ ] **Step 2: Failing test for the happy-path smoke run**

In `crates/rowforge-studio-core/tests/foundation.rs`, find the existing handler fixture helpers (Plan 7/8 tests already build a no-op echo handler in a temp dir for exec tests; reuse the helper). Add:

```rust
#[tokio::test(flavor = "multi_thread")]
async fn handler_smoke_run_happy_path_returns_outcomes_per_row() {
    use rowforge_studio_core::{OpenOpts, SmokeRunRequest, StudioCore};

    let tmp = tempfile::tempdir().unwrap();
    // Place a built test handler at <ws>/handlers/echo that emits Result for each Row.
    // Use the existing test helper from Plan 8 (look up its name in foundation.rs).
    write_test_echo_handler(&tmp.path().join("handlers").join("echo"));

    let core = StudioCore::open(OpenOpts {
        workspace: Some(tmp.path().to_path_buf()),
        preferred_editor: None,
        max_concurrent_runs: None,
        handler_log_capture_raw_stdout: false,
        smoke_default_rows: 5,
        smoke_timeout_per_row_secs: 5,
    })
    .unwrap();

    let rows = vec![
        serde_json::Map::from_iter([("id".into(), serde_json::json!("1"))]),
        serde_json::Map::from_iter([("id".into(), serde_json::json!("2"))]),
    ];
    let result = core
        .handler_smoke_run(SmokeRunRequest {
            handler_name: "echo".into(),
            rows,
        })
        .await
        .unwrap();

    assert_eq!(result.outcomes.len(), 2);
    assert_eq!(result.outcomes[0].seq, 1);
    assert_eq!(result.outcomes[0].status, "success");
    assert_eq!(result.outcomes[1].seq, 2);
}
```

(If `write_test_echo_handler` doesn't exist, look for similar helpers in `foundation.rs` — Plan 8 introduced a builder. If absent, copy the `go_stdio` template into the test handler dir and invoke `cargo run --bin go-something`-style; or write a tiny Python-based echo handler inline. The implementer can pick the simplest reusable helper.)

Add a second test for the row-count limit:

```rust
#[tokio::test(flavor = "multi_thread")]
async fn handler_smoke_run_rejects_more_than_100_rows() {
    use rowforge_studio_core::{OpenOpts, SmokeRunRequest, StudioCore, UiError};

    let tmp = tempfile::tempdir().unwrap();
    write_test_echo_handler(&tmp.path().join("handlers").join("echo"));
    let core = StudioCore::open(OpenOpts {
        workspace: Some(tmp.path().to_path_buf()),
        preferred_editor: None,
        max_concurrent_runs: None,
        handler_log_capture_raw_stdout: false,
        smoke_default_rows: 5,
        smoke_timeout_per_row_secs: 5,
    }).unwrap();
    let rows: Vec<_> = (0..101)
        .map(|i| serde_json::Map::from_iter([("id".into(), serde_json::json!(i))]))
        .collect();
    let err = core
        .handler_smoke_run(SmokeRunRequest { handler_name: "echo".into(), rows })
        .await
        .unwrap_err();
    assert!(matches!(err, UiError::InvalidArg(_)));
}

#[tokio::test(flavor = "multi_thread")]
async fn handler_smoke_run_rejects_empty_rows() {
    use rowforge_studio_core::{OpenOpts, SmokeRunRequest, StudioCore, UiError};

    let tmp = tempfile::tempdir().unwrap();
    write_test_echo_handler(&tmp.path().join("handlers").join("echo"));
    let core = StudioCore::open(OpenOpts {
        workspace: Some(tmp.path().to_path_buf()),
        preferred_editor: None,
        max_concurrent_runs: None,
        handler_log_capture_raw_stdout: false,
        smoke_default_rows: 5,
        smoke_timeout_per_row_secs: 5,
    }).unwrap();
    let err = core
        .handler_smoke_run(SmokeRunRequest {
            handler_name: "echo".into(),
            rows: vec![],
        })
        .await
        .unwrap_err();
    assert!(matches!(err, UiError::InvalidArg(_)));
}

#[tokio::test(flavor = "multi_thread")]
async fn handler_smoke_run_rejects_unknown_handler() {
    use rowforge_studio_core::{OpenOpts, SmokeRunRequest, StudioCore, UiError};

    let tmp = tempfile::tempdir().unwrap();
    let core = StudioCore::open(OpenOpts {
        workspace: Some(tmp.path().to_path_buf()),
        preferred_editor: None,
        max_concurrent_runs: None,
        handler_log_capture_raw_stdout: false,
        smoke_default_rows: 5,
        smoke_timeout_per_row_secs: 5,
    }).unwrap();
    let rows = vec![serde_json::Map::from_iter([("id".into(), serde_json::json!("1"))])];
    let err = core
        .handler_smoke_run(SmokeRunRequest { handler_name: "ghost".into(), rows })
        .await
        .unwrap_err();
    assert!(matches!(err, UiError::HandlerNotFound { .. }));
}

#[tokio::test(flavor = "multi_thread")]
async fn handler_smoke_run_rejects_invalid_name() {
    use rowforge_studio_core::{OpenOpts, SmokeRunRequest, StudioCore, UiError};

    let tmp = tempfile::tempdir().unwrap();
    let core = StudioCore::open(OpenOpts {
        workspace: Some(tmp.path().to_path_buf()),
        preferred_editor: None,
        max_concurrent_runs: None,
        handler_log_capture_raw_stdout: false,
        smoke_default_rows: 5,
        smoke_timeout_per_row_secs: 5,
    }).unwrap();
    let rows = vec![serde_json::Map::from_iter([("id".into(), serde_json::json!("1"))])];
    let err = core
        .handler_smoke_run(SmokeRunRequest { handler_name: "../etc".into(), rows })
        .await
        .unwrap_err();
    assert!(matches!(err, UiError::InvalidHandlerName { .. }));
}
```

- [ ] **Step 3: Verify FAIL**

```
cargo test -p rowforge-studio-core --test foundation handler_smoke_run
```

Expected: `handler_smoke_run` method not found.

- [ ] **Step 4: Implement `run_smoke` in smoke.rs**

Add to `crates/rowforge-studio-core/src/smoke.rs`:

```rust
use std::time::Duration;

use crate::handler::validate_name;
use crate::UiError;

/// Internal smoke runner. The `StudioCore::handler_smoke_run` wrapper owns
/// the lock + sqlite gate and forwards to this function with the resolved
/// handler dir.
pub(crate) async fn run_smoke(
    handler_name: &str,
    handler_dir: &std::path::Path,
    rows: Vec<serde_json::Map<String, serde_json::Value>>,
    timeout_per_row_secs: u64,
) -> Result<SmokeRunResult, UiError> {
    // Load manifest.
    let (manifest, _) = rowforge_core::manifest::Manifest::load_from_dir(handler_dir)
        .map_err(|e| UiError::Io(format!("manifest load: {e}")))?;

    // Plan 8 build gate. If needs_build, attempt to build; surface build
    // errors with the same UiError variants used by handler_build so the UI
    // can reuse LastBuildSection error rendering.
    if rowforge_core::build::needs_build(handler_dir, &manifest) {
        match rowforge_core::build::run_build(handler_dir, &manifest) {
            Ok(_) => {}
            Err(rowforge_core::build::BuildError::BuildFailed {
                exit_code, ..
            }) => {
                return Err(UiError::BuildFailed {
                    name: handler_name.to_string(),
                    exit_code,
                })
            }
            Err(rowforge_core::build::BuildError::ToolchainMissing { tool }) => {
                return Err(UiError::ToolchainMissing {
                    name: handler_name.to_string(),
                    tool,
                })
            }
            Err(rowforge_core::build::BuildError::NoBuildCommand) => {
                return Err(UiError::NoBuildCommand {
                    name: handler_name.to_string(),
                })
            }
            Err(rowforge_core::build::BuildError::Io(e)) => return Err(UiError::Io(e)),
        }
    }

    // Derive `columns` from the first row's keys (best-effort; handlers that
    // don't rely on Init.meta.columns won't notice an order mismatch here).
    let columns: Vec<String> = rows
        .first()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();

    let started = std::time::Instant::now();
    let mut worker = rowforge_core::worker::Worker::spawn(
        0,
        handler_dir,
        &manifest,
        "smoke",
        &Default::default(),
        &columns,
    )
    .await
    .map_err(|e| UiError::Io(format!("spawn worker: {e}")))?;

    // Drain stderr into a bounded ring (last 4 KiB).
    let stderr_handle = worker.take_stderr();
    let stderr_tail = std::sync::Arc::new(tokio::sync::Mutex::new(String::new()));
    let stderr_task = if let Some(mut h) = stderr_handle {
        let buf = stderr_tail.clone();
        Some(tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut chunk = [0u8; 1024];
            loop {
                let n = match h.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                let s = String::from_utf8_lossy(&chunk[..n]).to_string();
                let mut guard = buf.lock().await;
                guard.push_str(&s);
                if guard.len() > 4096 {
                    let cut = guard.len() - 4096;
                    let _ = guard.drain(0..cut);
                }
            }
        }))
    } else {
        None
    };

    let mut outcomes: Vec<SmokeOutcome> = Vec::with_capacity(rows.len());
    let row_timeout = if timeout_per_row_secs == 0 {
        Duration::from_secs(60 * 60) // effectively unlimited
    } else {
        Duration::from_secs(timeout_per_row_secs)
    };

    for (i, data) in rows.into_iter().enumerate() {
        let seq = (i as u64) + 1;
        let t0 = std::time::Instant::now();
        let row_msg = rowforge_core::protocol::Outbound::Row {
            seq,
            data,
            meta: rowforge_core::protocol::RowMeta {
                dry_run: false,
                row_index: seq,
            },
        };
        if let Err(e) = worker.send_row(&row_msg).await {
            outcomes.push(SmokeOutcome {
                seq,
                status: "crash".into(),
                code: Some("HANDLER_IO".into()),
                message: Some(format!("{e}")),
                dur_ms: 0,
                data: None,
            });
            break;
        }
        let recv_res = tokio::time::timeout(row_timeout, worker.recv()).await;
        match recv_res {
            Err(_) => {
                outcomes.push(SmokeOutcome {
                    seq,
                    status: "crash".into(),
                    code: Some("ROW_TIMEOUT".into()),
                    message: Some(format!("row exceeded {row_timeout:?}")),
                    dur_ms: t0.elapsed().as_millis() as u64,
                    data: None,
                });
                break;
            }
            Ok(Err(e)) => {
                outcomes.push(SmokeOutcome {
                    seq,
                    status: "crash".into(),
                    code: Some("HANDLER_IO".into()),
                    message: Some(format!("{e}")),
                    dur_ms: t0.elapsed().as_millis() as u64,
                    data: None,
                });
                break;
            }
            Ok(Ok(None)) => {
                outcomes.push(SmokeOutcome {
                    seq,
                    status: "crash".into(),
                    code: Some("HANDLER_EXIT".into()),
                    message: Some("handler closed stdout".into()),
                    dur_ms: t0.elapsed().as_millis() as u64,
                    data: None,
                });
                break;
            }
            Ok(Ok(Some(msg))) => match msg {
                rowforge_core::protocol::Inbound::Result { seq: rseq, data } => {
                    outcomes.push(SmokeOutcome {
                        seq: rseq,
                        status: "success".into(),
                        code: None,
                        message: None,
                        dur_ms: t0.elapsed().as_millis() as u64,
                        data: Some(serde_json::Value::Object(data)),
                    });
                }
                rowforge_core::protocol::Inbound::Error {
                    seq: rseq,
                    code,
                    message,
                    data,
                } => {
                    outcomes.push(SmokeOutcome {
                        seq: rseq,
                        status: "error".into(),
                        code: Some(code),
                        message: Some(message),
                        dur_ms: t0.elapsed().as_millis() as u64,
                        data: data.map(serde_json::Value::Object),
                    });
                }
                other => {
                    outcomes.push(SmokeOutcome {
                        seq,
                        status: "crash".into(),
                        code: Some("PROTOCOL".into()),
                        message: Some(format!("unexpected inbound: {other:?}")),
                        dur_ms: t0.elapsed().as_millis() as u64,
                        data: None,
                    });
                    break;
                }
            },
        }
    }

    let exit_code = match worker.shutdown(Duration::from_secs(2)).await {
        Ok(code) => code,
        Err(_) => None,
    };
    if let Some(t) = stderr_task {
        // 250ms grace for the stderr pump to drain after the handler exited.
        let _ = tokio::time::timeout(Duration::from_millis(250), t).await;
    }
    let stderr_string = stderr_tail.lock().await.clone();

    Ok(SmokeRunResult {
        outcomes,
        stderr_tail: stderr_string,
        exit_code,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

/// Used by `StudioCore::handler_smoke_run` for the `is_valid_id_component`
/// style guard. Mirrors `handler::validate_name`; re-exported here for clarity.
pub(crate) fn name_ok(name: &str) -> bool {
    validate_name(name)
}
```

- [ ] **Step 5: Wire `StudioCore::handler_smoke_run`**

In `crates/rowforge-studio-core/src/lib.rs`, add near the other handler methods:

```rust
/// Plan 13: run N≤100 rows through the handler binary and return outcomes
/// inline. Does NOT create an execution or persist outcomes. Forces row mode
/// (one row at a time) for protocol simplicity.
///
/// Errors:
/// - `InvalidHandlerName` — `request.handler_name` fails the name regex
/// - `InvalidArg`         — empty rows, > 100 rows
/// - `HandlerNotFound`    — `<workspace>/handlers/<name>` missing
/// - `HandlerBusy`        — an active exec attempt is using this handler
/// - `BuildFailed` / `ToolchainMissing` / `NoBuildCommand` — Plan 8 gate
/// - `Io`                 — manifest load / worker spawn / stdio failure
pub async fn handler_smoke_run(
    &self,
    request: SmokeRunRequest,
) -> Result<SmokeRunResult, UiError> {
    // Name + row validation up-front (cheap, no I/O).
    if !crate::smoke::name_ok(&request.handler_name) {
        return Err(UiError::InvalidHandlerName {
            name: request.handler_name.clone(),
        });
    }
    if request.rows.is_empty() {
        return Err(UiError::InvalidArg(
            "smoke needs at least 1 row".into(),
        ));
    }
    if request.rows.len() > 100 {
        return Err(UiError::InvalidArg(
            "smoke limit is 100 rows".into(),
        ));
    }

    let handler_dir = self
        .workspace
        .root
        .as_path()
        .join("handlers")
        .join(&request.handler_name);
    if !handler_dir.is_dir() {
        return Err(UiError::HandlerNotFound {
            name: request.handler_name.clone(),
        });
    }

    // Cross-process gate: refuse if any exec attempt is running this handler.
    {
        let guard = self.store.lock().unwrap_or_else(|p| p.into_inner());
        let busy = guard
            .has_active_attempt_for_handler_dir(&handler_dir)
            .map_err(|e| UiError::Io(format!("active-run gate: {e}")))?;
        if busy {
            return Err(UiError::HandlerBusy {
                name: request.handler_name.clone(),
            });
        }
    }

    // Process-local gate: serialize smoke runs.
    let _lock = self.smoke_lock.clone().lock_owned().await;
    let timeout_secs = self.smoke_timeout_per_row_secs;
    crate::smoke::run_smoke(
        &request.handler_name,
        &handler_dir,
        request.rows,
        timeout_secs,
    )
    .await
}
```

- [ ] **Step 6: Run tests; expect PASS**

```
cargo test -p rowforge-studio-core --test foundation handler_smoke_run
```

Expected: 5 PASS. If the echo-handler fixture doesn't exist, write a small inline Python handler (Plan 8 has a `python3-uppercase` example handler in `examples/handlers/` — borrow that pattern).

- [ ] **Step 7: Commit**

```bash
git add crates/rowforge-studio-core/src/smoke.rs \
        crates/rowforge-studio-core/src/lib.rs \
        crates/rowforge-studio-core/src/error.rs \
        crates/rowforge-studio-core/tests/foundation.rs
git commit -m "studio-core: Plan 13 T6 — handler_smoke_run via Worker"
```

---

## Task 7: Tauri commands — handler_smoke_run + handler_smoke_load_fixtures

**Files:**
- Modify: `apps/rowforge-studio/src-tauri/src/commands.rs`
- Modify: `apps/rowforge-studio/src-tauri/src/lib.rs`
- Modify: `apps/rowforge-studio/src-tauri/tests/ipc_contract.rs`

- [ ] **Step 1: Failing ipc_contract tests**

In `apps/rowforge-studio/src-tauri/tests/ipc_contract.rs`, find the existing pattern for command-registration tests (e.g. `plan12_handler_fork_command_registered`). Add:

```rust
#[test]
fn plan13_handler_smoke_run_command_registered() {
    let builder = tauri::Builder::default().invoke_handler(tauri::generate_handler![
        rowforge_studio::commands::handler_smoke_run,
    ]);
    let _ = builder; // compilation == registration
}

#[test]
fn plan13_handler_smoke_load_fixtures_command_registered() {
    let builder = tauri::Builder::default().invoke_handler(tauri::generate_handler![
        rowforge_studio::commands::handler_smoke_load_fixtures,
    ]);
    let _ = builder;
}
```

(Exact module path may differ — look at how the file imports other commands.)

- [ ] **Step 2: Verify FAIL**

```
cargo test -p rowforge-studio --test ipc_contract plan13
```

Expected: command symbols not found.

- [ ] **Step 3: Add Tauri commands**

In `apps/rowforge-studio/src-tauri/src/commands.rs`, append:

```rust
// ===== Plan 13 — handler smoke test =====

#[tauri::command]
pub async fn handler_smoke_run(
    state: State<'_, AppState>,
    request: rowforge_studio_core::SmokeRunRequest,
) -> Result<rowforge_studio_core::SmokeRunResult, UiError> {
    // We need the core's async method; clone the Arc out under the mutex,
    // then drop the mutex before awaiting.
    let core_clone = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        let _core = guard
            .as_ref()
            .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
        // StudioCore isn't Clone; we hold a brief reference inside the
        // mutex and dispatch the async call there. To avoid holding the
        // sync lock across .await, we move the call into spawn_blocking
        // with a oneshot — but `handler_smoke_run` is itself async. The
        // cleanest path is to keep StudioCore inside Arc and clone the Arc
        // here. If `AppState.core` is `Mutex<Option<StudioCore>>` today,
        // refactor to `Mutex<Option<Arc<StudioCore>>>`.
        //
        // Pragmatic shortcut for v1: call the async method while holding
        // the sync mutex by spawning a tokio task that grabs an Arc.
        //
        // The implementer should pick whichever fits the AppState shape
        // already in use. See handler_build for the sync analogue.
        // Below is the most defensive approach: clone into Arc.
        // ...
        // The actual implementation depends on AppState; document the
        // chosen approach inline.
        std::sync::Arc::new(())
    };
    // Placeholder — replaced in Step 4.
    let _ = core_clone;
    Err(UiError::Internal("smoke run wiring incomplete".into()))
}

#[tauri::command]
pub fn handler_smoke_load_fixtures(
    state: State<'_, AppState>,
    path: String,
    limit: usize,
) -> Result<Vec<serde_json::Map<String, serde_json::Value>>, UiError> {
    let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
    let core = guard
        .as_ref()
        .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
    core.handler_smoke_load_fixtures(std::path::Path::new(&path), limit)
}
```

- [ ] **Step 4: Choose the StudioCore async dispatch shape**

`StudioCore::handler_smoke_run` is `async fn` and holds an internal mutex. The Tauri command must not hold `state.core.lock()` (a `std::sync::Mutex`) across an `.await`. Refactor `AppState.core` to `Mutex<Option<Arc<StudioCore>>>` if it isn't already. Replace the `handler_smoke_run` body with:

```rust
#[tauri::command]
pub async fn handler_smoke_run(
    state: State<'_, AppState>,
    request: rowforge_studio_core::SmokeRunRequest,
) -> Result<rowforge_studio_core::SmokeRunResult, UiError> {
    let core = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        guard
            .as_ref()
            .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?
            .clone()
    };
    core.handler_smoke_run(request).await
}
```

Required prerequisite refactor (separate sub-step before this):
- Open `apps/rowforge-studio/src-tauri/src/state.rs`. If `core` is `Mutex<Option<StudioCore>>`, change it to `Mutex<Option<Arc<StudioCore>>>` and update every existing read-site to `.clone()` the Arc out instead of taking `&StudioCore`. Adjust `workspace_open` to wrap the constructed `StudioCore` in `Arc::new(...)` before insert.

Alternative (zero refactor): wrap the smoke run with `tokio::task::spawn_blocking` and a `tokio::runtime::Handle::current().block_on` shim. This is uglier but avoids the AppState change. Prefer the Arc refactor; document the choice in the commit message.

- [ ] **Step 5: Register the commands**

In `apps/rowforge-studio/src-tauri/src/lib.rs`, add to the `generate_handler![]` list:

```rust
commands::handler_smoke_run,           // Plan 13
commands::handler_smoke_load_fixtures, // Plan 13
```

- [ ] **Step 6: Run tests; expect PASS**

```
cargo test -p rowforge-studio --test ipc_contract plan13
cargo build -p rowforge-studio
```

Expected: ipc_contract tests PASS; full build clean.

- [ ] **Step 7: Commit**

```bash
git add apps/rowforge-studio/src-tauri/src/commands.rs \
        apps/rowforge-studio/src-tauri/src/lib.rs \
        apps/rowforge-studio/src-tauri/src/state.rs \
        apps/rowforge-studio/src-tauri/tests/ipc_contract.rs
git commit -m "studio-shell: Plan 13 T7 — smoke commands + Arc<StudioCore> refactor"
```

---

## Task 8: ipc client/types/hooks

**Files:**
- Modify: `apps/rowforge-studio/src/ipc/types.ts`
- Modify: `apps/rowforge-studio/src/ipc/client.ts`
- Modify: `apps/rowforge-studio/src/ipc/use-handlers.ts`

- [ ] **Step 1: Add TS types**

In `apps/rowforge-studio/src/ipc/types.ts`, near the existing handler types:

```ts
// ===== Plan 13 — handler smoke test =====

export type SmokeOutcomeStatus = "success" | "error" | "crash";

export interface SmokeOutcome {
  seq: number;
  status: SmokeOutcomeStatus;
  code: string | null;
  message: string | null;
  dur_ms: number;
  data: unknown | null;
}

export interface SmokeRunRequest {
  handler_name: string;
  rows: Record<string, unknown>[];
}

export interface SmokeRunResult {
  outcomes: SmokeOutcome[];
  stderr_tail: string;
  exit_code: number | null;
  elapsed_ms: number;
}
```

If `UiError` has a TS union enumerating `kind` values, add `"handler_busy"` to it (search for the existing `kind: "handler_not_found"` literal and follow that pattern).

- [ ] **Step 2: Add invoke wrappers**

In `apps/rowforge-studio/src/ipc/client.ts`, in the `ipc` object literal:

```ts
import type {
  // ...existing imports...
  SmokeRunRequest,
  SmokeRunResult,
} from "./types";
```

```ts
  // ===== Plan 13 handler smoke test =====

  handler_smoke_run: (args: { request: SmokeRunRequest }) =>
    invoke<SmokeRunResult>("handler_smoke_run", args),
  handler_smoke_load_fixtures: (args: { path: string; limit: number }) =>
    invoke<Record<string, unknown>[]>("handler_smoke_load_fixtures", args),
```

- [ ] **Step 3: Add hooks**

In `apps/rowforge-studio/src/ipc/use-handlers.ts`, alongside `useHandlerFork`:

```ts
import { useMutation } from "@tanstack/react-query";
import { ipc } from "./client";
import type { SmokeRunRequest } from "./types";

// ... existing hooks ...

export const useHandlerSmokeRun = () =>
  useMutation({
    mutationFn: (request: SmokeRunRequest) =>
      ipc.handler_smoke_run({ request }),
  });

export const useHandlerSmokeLoadFixtures = () =>
  useMutation({
    mutationFn: (args: { path: string; limit: number }) =>
      ipc.handler_smoke_load_fixtures(args),
  });
```

- [ ] **Step 4: Verify build + type-check**

```
cd apps/rowforge-studio
pnpm tsc -b
```

Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add apps/rowforge-studio/src/ipc/types.ts \
        apps/rowforge-studio/src/ipc/client.ts \
        apps/rowforge-studio/src/ipc/use-handlers.ts
git commit -m "studio-ui: Plan 13 T8 — smoke ipc client + types + hooks"
```

---

## Task 9: SmokeSection UI component

**Files:**
- Create: `apps/rowforge-studio/src/components/SmokeSection.tsx`

- [ ] **Step 1: Write the component**

```tsx
import { useEffect, useMemo, useState } from "react";
import { open as dialogOpen } from "@tauri-apps/plugin-dialog";
import { Button } from "@/components/ui/button";
import {
  useHandlerSmokeRun,
  useHandlerSmokeLoadFixtures,
} from "@/ipc/use-handlers";
import { uiErrorMessage, type SmokeOutcome } from "@/ipc/types";

type Source = "paste" | "fixtures" | "synthetic";

interface Props {
  handlerName: string;
  defaultRows: number;
}

export function SmokeSection({ handlerName, defaultRows }: Props) {
  const smoke = useHandlerSmokeRun();
  const loadFixtures = useHandlerSmokeLoadFixtures();

  const [source, setSource] = useState<Source>("paste");
  const [pasted, setPasted] = useState("");
  const [fixturePath, setFixturePath] = useState<string | null>(null);
  const [loadedRows, setLoadedRows] = useState<Record<string, unknown>[] | null>(null);
  const [rowCount, setRowCount] = useState(defaultRows);

  // Parse pasted JSON lines.
  const parsedPaste = useMemo(() => {
    if (source !== "paste") return { rows: [], error: null as string | null };
    const lines = pasted.split("\n").map((l) => l.trim()).filter(Boolean);
    const rows: Record<string, unknown>[] = [];
    for (let i = 0; i < lines.length; i++) {
      try {
        const v = JSON.parse(lines[i]);
        if (typeof v !== "object" || v === null || Array.isArray(v)) {
          return { rows: [], error: `line ${i + 1}: not a JSON object` };
        }
        rows.push(v as Record<string, unknown>);
      } catch (e) {
        return {
          rows: [],
          error: `line ${i + 1}: ${(e as Error).message}`,
        };
      }
    }
    return { rows, error: null };
  }, [source, pasted]);

  // Reset loaded rows when source changes.
  useEffect(() => {
    if (source !== "fixtures") {
      setLoadedRows(null);
      setFixturePath(null);
    }
  }, [source]);

  const availableRows: Record<string, unknown>[] = useMemo(() => {
    if (source === "paste") return parsedPaste.rows;
    if (source === "fixtures") return loadedRows ?? [];
    return [{ row: 1 }];
  }, [source, parsedPaste.rows, loadedRows]);

  const effectiveRows = availableRows.slice(0, Math.min(rowCount, 100));
  const canRun =
    effectiveRows.length > 0 &&
    parsedPaste.error == null &&
    !smoke.isPending &&
    !loadFixtures.isPending;

  const pickFixture = async () => {
    const path = await dialogOpen({ directory: false, multiple: false });
    if (typeof path !== "string") return;
    setFixturePath(path);
    loadFixtures.mutate(
      { path, limit: 100 },
      {
        onSuccess: (rows) => setLoadedRows(rows),
      },
    );
  };

  const runSmoke = () => {
    smoke.mutate({
      handler_name: handlerName,
      rows: effectiveRows,
    });
  };

  return (
    <div className="space-y-3">
      <h2 className="text-sm font-medium uppercase text-muted-foreground">
        Smoke test
      </h2>

      <div className="rounded border border-zinc-700 p-4 space-y-4">
        <div className="flex gap-4 text-sm">
          {(["paste", "fixtures", "synthetic"] as const).map((s) => (
            <label key={s} className="flex items-center gap-2 cursor-pointer">
              <input
                type="radio"
                name="smoke-source"
                checked={source === s}
                onChange={() => setSource(s)}
              />
              <span>
                {s === "paste"
                  ? "Paste JSON"
                  : s === "fixtures"
                    ? "Fixtures…"
                    : "One synthetic row"}
              </span>
            </label>
          ))}
        </div>

        {source === "paste" && (
          <div className="space-y-1">
            <textarea
              value={pasted}
              onChange={(e) => setPasted(e.target.value)}
              placeholder={`{"id":"1","email":"a@example.com"}\n{"id":"2","email":"b@example.com"}`}
              className="w-full h-32 rounded border border-zinc-700 bg-zinc-900 p-2 font-mono text-xs"
            />
            {parsedPaste.error ? (
              <div className="text-xs text-red-300">{parsedPaste.error}</div>
            ) : (
              <div className="text-xs text-muted-foreground">
                {parsedPaste.rows.length} row
                {parsedPaste.rows.length === 1 ? "" : "s"} parsed
              </div>
            )}
          </div>
        )}

        {source === "fixtures" && (
          <div className="space-y-2">
            <Button onClick={pickFixture} variant="outline" size="sm">
              {fixturePath ? "Change…" : "Pick file…"}
            </Button>
            {fixturePath && (
              <code
                className="block break-all rounded border border-zinc-700 bg-zinc-900 px-2 py-1 text-xs text-muted-foreground"
                title={fixturePath}
              >
                {fixturePath}
              </code>
            )}
            {loadFixtures.isPending && (
              <div className="text-xs text-muted-foreground">Loading…</div>
            )}
            {loadFixtures.isError && (
              <div className="text-xs text-red-300">
                {uiErrorMessage(loadFixtures.error)}
              </div>
            )}
            {loadedRows && (
              <div className="text-xs text-muted-foreground">
                {loadedRows.length} row{loadedRows.length === 1 ? "" : "s"}{" "}
                loaded — keys:{" "}
                {Object.keys(loadedRows[0] ?? {}).slice(0, 4).join(", ") ||
                  "(empty)"}
              </div>
            )}
          </div>
        )}

        {source === "synthetic" && (
          <div className="text-xs text-muted-foreground">
            Dispatches a single row{" "}
            <code className="font-mono">{"{ \"row\": 1 }"}</code> — useful for
            verifying the binary starts at all.
          </div>
        )}

        <div className="flex items-center gap-3">
          <label className="text-sm">Rows to run:</label>
          <input
            type="number"
            min={1}
            max={100}
            value={rowCount}
            onChange={(e) =>
              setRowCount(
                Math.max(1, Math.min(100, parseInt(e.target.value, 10) || 1)),
              )
            }
            className="w-20 rounded border border-zinc-700 bg-zinc-900 px-2 py-1 text-sm"
          />
          <span className="text-xs text-muted-foreground">(max 100)</span>
          <div className="flex-1" />
          <Button onClick={runSmoke} disabled={!canRun}>
            {smoke.isPending ? "Running…" : "Run smoke test"}
          </Button>
        </div>

        {smoke.isError && (
          <div className="rounded border border-red-500/40 bg-red-500/10 p-2 text-sm text-red-200">
            {uiErrorMessage(smoke.error)}
          </div>
        )}

        {smoke.data && <SmokeResults result={smoke.data} />}
      </div>
    </div>
  );
}

function SmokeResults({
  result,
}: {
  result: import("@/ipc/types").SmokeRunResult;
}) {
  const counts = result.outcomes.reduce(
    (acc, o) => {
      acc[o.status] = (acc[o.status] ?? 0) + 1;
      return acc;
    },
    {} as Record<string, number>,
  );

  return (
    <div className="space-y-3">
      <div className="text-xs text-muted-foreground">
        Outcomes ({result.outcomes.length})
        {counts.success ? ` · ✓ ${counts.success} success` : ""}
        {counts.error ? ` · ✗ ${counts.error} error` : ""}
        {counts.crash ? ` · ⚠ ${counts.crash} crash` : ""}
        {" · "}
        {result.elapsed_ms} ms
        {result.exit_code != null && ` · exit ${result.exit_code}`}
      </div>
      <div className="overflow-x-auto rounded border border-zinc-700">
        <table className="w-full text-xs">
          <thead className="bg-zinc-900">
            <tr>
              <Th>seq</Th>
              <Th>status</Th>
              <Th>message</Th>
              <Th>dur_ms</Th>
              <Th>data</Th>
            </tr>
          </thead>
          <tbody>
            {result.outcomes.map((o) => (
              <OutcomeRow key={o.seq} o={o} />
            ))}
          </tbody>
        </table>
      </div>
      {result.stderr_tail && (
        <details>
          <summary className="text-xs text-muted-foreground cursor-pointer">
            stderr tail ({result.stderr_tail.length} B)
          </summary>
          <pre className="mt-2 max-h-64 overflow-auto rounded border border-zinc-700 bg-zinc-900 p-2 text-xs whitespace-pre-wrap">
            {result.stderr_tail}
          </pre>
        </details>
      )}
    </div>
  );
}

function OutcomeRow({ o }: { o: SmokeOutcome }) {
  const statusColor =
    o.status === "success"
      ? "text-green-300"
      : o.status === "error"
        ? "text-yellow-300"
        : "text-red-300";
  const dataPreview = o.data
    ? JSON.stringify(o.data).slice(0, 80)
    : "—";
  return (
    <tr className="border-t border-zinc-800">
      <Td>{o.seq}</Td>
      <Td>
        <span className={statusColor}>{o.status}</span>
      </Td>
      <Td>{o.message ?? (o.code ? `${o.code}` : "—")}</Td>
      <Td>{o.dur_ms}</Td>
      <Td>
        <code
          className="font-mono break-all"
          title={o.data ? JSON.stringify(o.data) : undefined}
        >
          {dataPreview}
        </code>
      </Td>
    </tr>
  );
}

function Th({ children }: { children: React.ReactNode }) {
  return <th className="px-2 py-1 text-left font-medium">{children}</th>;
}

function Td({ children }: { children: React.ReactNode }) {
  return <td className="px-2 py-1 align-top">{children}</td>;
}
```

- [ ] **Step 2: Build + type-check**

```
cd apps/rowforge-studio
pnpm tsc -b
```

Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add apps/rowforge-studio/src/components/SmokeSection.tsx
git commit -m "studio-ui: Plan 13 T9 — SmokeSection component"
```

---

## Task 10: Mount SmokeSection on HandlerDetailPage

**Files:**
- Modify: `apps/rowforge-studio/src/pages/HandlerDetailPage.tsx`
- Modify: `apps/rowforge-studio/src/ipc/queries.ts` (or wherever `useSettings` lives) — if no `useSettings` exists, just use a constant default of 5

- [ ] **Step 1: Import + render the section**

In `apps/rowforge-studio/src/pages/HandlerDetailPage.tsx`, add the import:

```tsx
import { SmokeSection } from "@/components/SmokeSection";
```

In the JSX returned from `HandlerDetailPage`, insert below `<LastBuildSection … />`:

```tsx
<LastBuildSection last_build={data.last_build} pending={build.isPending} />
<SmokeSection handlerName={data.summary.name} defaultRows={5} />
<SourceFilesSection detail={data} />
```

(Keep `defaultRows={5}` as a constant for v1; reading from Settings is a polish follow-on. Inline a TODO comment is unnecessary — settings can later be threaded via `useWorkspaceSettings`.)

- [ ] **Step 2: Build + type-check**

```
cd apps/rowforge-studio
pnpm tsc -b
```

Expected: clean.

- [ ] **Step 3: Manual smoke (developer sanity, not the test framework)**

Run `pnpm tauri dev` from `apps/rowforge-studio/`. Open a workspace with at least one handler. On HandlerDetailPage:
- Verify a "Smoke test" section appears below "Last build".
- Paste `{"id":"1"}` and click Run; expect the outcomes table to render.
- Switch to "Fixtures" mode and pick a `.jsonl` file; expect "N rows loaded" + key preview.

(This is a sanity check before writing vitest tests, not a final acceptance check.)

- [ ] **Step 4: Commit**

```bash
git add apps/rowforge-studio/src/pages/HandlerDetailPage.tsx
git commit -m "studio-ui: Plan 13 T10 — mount SmokeSection on HandlerDetailPage"
```

---

## Task 11: Vitest tests for SmokeSection

**Files:**
- Create: `apps/rowforge-studio/src/components/__tests__/SmokeSection.test.tsx`

- [ ] **Step 1: Write tests**

```tsx
import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { SmokeSection } from "@/components/SmokeSection";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import { open as dialogOpen } from "@tauri-apps/plugin-dialog";

const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;
const dialogOpenMock = dialogOpen as unknown as ReturnType<typeof vi.fn>;

function wrap(ui: React.ReactElement) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(<QueryClientProvider client={qc}>{ui}</QueryClientProvider>);
}

beforeEach(() => {
  invokeMock.mockReset();
  dialogOpenMock.mockReset();
});

describe("SmokeSection", () => {
  it("renders paste mode by default with run disabled", () => {
    wrap(<SmokeSection handlerName="alpha" defaultRows={5} />);
    expect(screen.getByRole("button", { name: /run smoke test/i })).toBeDisabled();
  });

  it("parses pasted JSON lines and enables run", () => {
    wrap(<SmokeSection handlerName="alpha" defaultRows={5} />);
    const textarea = screen.getByPlaceholderText(/email/i) as HTMLTextAreaElement;
    fireEvent.change(textarea, {
      target: { value: '{"id":"1"}\n{"id":"2"}' },
    });
    expect(screen.getByText(/2 rows parsed/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /run smoke test/i })).toBeEnabled();
  });

  it("flags invalid JSON line", () => {
    wrap(<SmokeSection handlerName="alpha" defaultRows={5} />);
    const textarea = screen.getByPlaceholderText(/email/i) as HTMLTextAreaElement;
    fireEvent.change(textarea, { target: { value: "{not json}" } });
    expect(screen.getByText(/line 1:/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /run smoke test/i })).toBeDisabled();
  });

  it("calls handler_smoke_run with the parsed rows", async () => {
    invokeMock.mockResolvedValueOnce({
      outcomes: [
        { seq: 1, status: "success", code: null, message: null, dur_ms: 5, data: { ok: true } },
      ],
      stderr_tail: "",
      exit_code: 0,
      elapsed_ms: 10,
    });
    wrap(<SmokeSection handlerName="alpha" defaultRows={5} />);
    const textarea = screen.getByPlaceholderText(/email/i) as HTMLTextAreaElement;
    fireEvent.change(textarea, { target: { value: '{"id":"1"}' } });
    fireEvent.click(screen.getByRole("button", { name: /run smoke test/i }));
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("handler_smoke_run", {
        request: {
          handler_name: "alpha",
          rows: [{ id: "1" }],
        },
      });
    });
    await waitFor(() => {
      expect(screen.getByText(/✓ 1 success/)).toBeInTheDocument();
    });
  });

  it("fixtures mode: picking a file calls handler_smoke_load_fixtures", async () => {
    dialogOpenMock.mockResolvedValueOnce("/tmp/fx.jsonl");
    invokeMock.mockResolvedValueOnce([{ id: "1" }, { id: "2" }]);
    wrap(<SmokeSection handlerName="alpha" defaultRows={5} />);
    fireEvent.click(screen.getByLabelText(/fixtures/i));
    fireEvent.click(screen.getByRole("button", { name: /pick file/i }));
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("handler_smoke_load_fixtures", {
        path: "/tmp/fx.jsonl",
        limit: 100,
      });
    });
    await waitFor(() => {
      expect(screen.getByText(/2 rows loaded/)).toBeInTheDocument();
    });
  });

  it("synthetic mode shows the row preview", () => {
    wrap(<SmokeSection handlerName="alpha" defaultRows={5} />);
    fireEvent.click(screen.getByLabelText(/one synthetic row/i));
    expect(screen.getByText(/single row/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /run smoke test/i })).toBeEnabled();
  });

  it("clamps row count input to 1..100", () => {
    wrap(<SmokeSection handlerName="alpha" defaultRows={5} />);
    const input = screen.getByDisplayValue("5") as HTMLInputElement;
    fireEvent.change(input, { target: { value: "200" } });
    expect(input.value).toBe("100");
    fireEvent.change(input, { target: { value: "0" } });
    expect(input.value).toBe("1");
  });

  it("renders handler_busy error from ipc", async () => {
    invokeMock.mockRejectedValueOnce({
      kind: "handler_busy",
      message: { name: "alpha" },
    });
    wrap(<SmokeSection handlerName="alpha" defaultRows={5} />);
    const textarea = screen.getByPlaceholderText(/email/i) as HTMLTextAreaElement;
    fireEvent.change(textarea, { target: { value: '{"id":"1"}' } });
    fireEvent.click(screen.getByRole("button", { name: /run smoke test/i }));
    await waitFor(() => {
      // uiErrorMessage formats "handler_busy" — exact wording depends on the
      // mapper; assert by kind keyword instead.
      expect(screen.getByText(/handler|busy/i)).toBeInTheDocument();
    });
  });
});
```

- [ ] **Step 2: Run tests**

```
cd apps/rowforge-studio
pnpm test SmokeSection
```

Expected: 8 PASS. If `uiErrorMessage` doesn't format `handler_busy`, add a case to the existing mapper (look at how other variants are mapped in `apps/rowforge-studio/src/ipc/types.ts`).

- [ ] **Step 3: Run full vitest suite**

```
pnpm test
```

Expected: 166 → 174 PASS (8 new).

- [ ] **Step 4: Commit**

```bash
git add apps/rowforge-studio/src/components/__tests__/SmokeSection.test.tsx \
        apps/rowforge-studio/src/ipc/types.ts
git commit -m "studio-ui: Plan 13 T11 — SmokeSection vitest coverage"
```

---

## Task 12: Spec docs + HUMAN_SMOKE

**Files:**
- Modify: `docs/spec/studio/part-5-api.md`
- Modify: `docs/spec/studio/part-5-api.zh-Hant.md`
- Modify: `docs/spec/studio/part-7-ui.md`
- Modify: `docs/spec/studio/part-7-ui.zh-Hant.md`
- Modify: `docs/spec/studio/part-8-handler-authoring.md`
- Modify: `docs/spec/studio/part-8-handler-authoring.zh-Hant.md`
- Create: `docs/HUMAN_SMOKE/plan-13-handler-smoke-test.md`

- [ ] **Step 1: part-5-api.md — document the two new commands**

Add a new subsection under §5 (find the next free §5.X based on what's already there; recent plans added §5.10+). Document:

```md
### §5.X handler_smoke_run

**Args (camelCase from JS, snake_case in Rust):**
- `request: SmokeRunRequest`

**SmokeRunRequest:**
```ts
{ handler_name: string; rows: Record<string, unknown>[] }
```

**Returns:** `SmokeRunResult`
```ts
{
  outcomes: SmokeOutcome[];
  stderr_tail: string;
  exit_code: number | null;
  elapsed_ms: number;
}
```

**SmokeOutcome:**
```ts
{
  seq: number;
  status: "success" | "error" | "crash";
  code: string | null;
  message: string | null;
  dur_ms: number;
  data: unknown | null;
}
```

**Errors:** `invalid_handler_name`, `invalid_arg` (empty rows / >100), `handler_not_found`, `handler_busy`, `build_failed`, `toolchain_missing`, `no_build_command`, `io`.

### §5.X+1 handler_smoke_load_fixtures

**Args:** `path: string`, `limit: number`

**Returns:** `Record<string, unknown>[]`

Reads jsonl / ndjson / json (array) / csv files, or picks the first match in a directory by precedence jsonl > ndjson > json > csv. `limit` is clamped 1..=100.

**Errors:** `invalid_arg` (no rows / unsupported ext / dir-with-no-match), `io`.
```

Then add a new `handler_busy` row to the UiError table earlier in part-5.

- [ ] **Step 2: Mirror in part-5-api.zh-Hant.md**

Translate the same content. Keep the TypeScript blocks verbatim.

- [ ] **Step 3: part-7-ui.md — describe the SmokeSection**

Find the HandlerDetailPage section. Append:

```md
#### Smoke test section

Below "Last build", a new `<SmokeSection />` lets the user dispatch 1–100 rows
to the handler binary without creating an exec. Sources:

- **Paste JSON** — one JSON object per line; live "N rows parsed"
- **Fixtures…** — pick a `.jsonl` / `.ndjson` / `.json` (top-level array) /
  `.csv` file or a directory containing one (precedence: jsonl > ndjson >
  json > csv)
- **One synthetic row** — sends `{ "row": 1 }`

Forced single-worker row mode. Outcomes render inline in a 5-column table
(seq / status / message / dur_ms / data). `stderr` is collapsible (last 4 KiB).
```

- [ ] **Step 4: Mirror in part-7-ui.zh-Hant.md**

- [ ] **Step 5: part-8-handler-authoring.md — replace §8.4.3 placeholder**

Find §8.4.3 (Plan 8 spec marked it as deferred). Replace contents with:

```md
### §8.4.3 Smoke test

Studio surfaces a Smoke test section on each handler's detail page. The user
can paste JSON lines, pick a fixtures file, or dispatch one synthetic row,
and observe outcomes inline without creating an execution.

- Bounded to 100 rows total per smoke run
- Forced row mode (batch handlers still receive rows one at a time)
- Reuses Plan 8's build gate — rebuilds if `needs_build` is true
- Refuses when an exec attempt is already running against this handler
  (cross-process sqlite gate)

API: see part-5 §`handler_smoke_run` and §`handler_smoke_load_fixtures`.
```

- [ ] **Step 6: Mirror in part-8-handler-authoring.zh-Hant.md**

- [ ] **Step 7: HUMAN_SMOKE walkthrough**

Create `docs/HUMAN_SMOKE/plan-13-handler-smoke-test.md`:

```md
# Plan 13 HUMAN_SMOKE — Handler smoke test

> Use this checklist after merging Plan 13 to verify the surface end-to-end.

## Setup
- [ ] Fresh workspace at `/tmp/plan13-smoke-ws/`
- [ ] Scaffold a `go_stdio` handler named `echo` with primary_field `id`
- [ ] Open it in editor, replace the row handler body with one that echoes
      `{"echoed": <id>}` and returns success

## Paste mode happy path
- [ ] Build via "Build" button — verify Last build shows success
- [ ] In Smoke section: paste two lines:
      `{"id":"1"}`
      `{"id":"2"}`
- [ ] Header shows "2 rows parsed"
- [ ] Click Run smoke test
- [ ] Outcomes table shows seq 1 + seq 2, both `success`, data column shows
      `{"echoed":"1"}` / `{"echoed":"2"}`
- [ ] elapsed_ms < 2000

## Paste mode invalid JSON
- [ ] Replace line 2 with `not json`
- [ ] Header flips to red "line 2: <parse error>"
- [ ] Run button is disabled

## Synthetic mode
- [ ] Click "One synthetic row" radio
- [ ] Description text shows the synthetic row preview
- [ ] Run button is enabled
- [ ] Click Run; outcomes table has 1 row (seq 1, success)

## Fixtures mode — jsonl
- [ ] Create `/tmp/smoke-fx.jsonl` containing:
      `{"id":"a"}`
      `{"id":"b"}`
      `{"id":"c"}`
- [ ] Click "Fixtures…" radio, click "Pick file…", choose the jsonl
- [ ] Path shows in a code block; "3 rows loaded — keys: id"
- [ ] Set Rows to run = 2
- [ ] Run; outcomes table has 2 rows (seq 1, seq 2)

## Fixtures mode — csv
- [ ] Create `/tmp/smoke-fx.csv` containing:
      `id,email`
      `1,a@x.com`
      `2,b@x.com`
- [ ] Switch fixture file to the csv
- [ ] "2 rows loaded — keys: id, email"
- [ ] Run; outcomes succeed

## Fixtures mode — directory
- [ ] Create dir `/tmp/smoke-fx-dir/` containing both the jsonl and csv above
- [ ] Pick the dir
- [ ] Loaded rows come from the jsonl (precedence)

## Fixtures mode — empty
- [ ] Create empty `/tmp/empty.jsonl`
- [ ] Pick it
- [ ] Red error: "no rows found in fixtures path"

## Row count cap
- [ ] Type "200" in Rows to run; verify it clamps to 100
- [ ] Type "0"; verify it clamps to 1

## Build failure surface
- [ ] Break the handler source (e.g. add `syntax error` line)
- [ ] Click Run (with any valid row source)
- [ ] Error block shows BuildFailed message; outcomes table empty

## Active-run gate
- [ ] In one terminal: start a normal exec with this handler that takes >10s
      (use a large input)
- [ ] While it runs, open Studio, navigate to the handler, try to smoke
- [ ] Expect HandlerBusy error: "handler '<name>' has an active run"

## Stderr tail
- [ ] Inside the handler, before responding, `fmt.Fprintln(os.Stderr, "boot")`
- [ ] Run smoke; expand the "stderr tail" details
- [ ] See the boot line(s); confirmed last-4KiB trimmed by writing a >5KiB
      stderr loop and verifying only the tail survives
```

- [ ] **Step 8: Commit**

```bash
git add docs/spec/studio/part-5-api.md \
        docs/spec/studio/part-5-api.zh-Hant.md \
        docs/spec/studio/part-7-ui.md \
        docs/spec/studio/part-7-ui.zh-Hant.md \
        docs/spec/studio/part-8-handler-authoring.md \
        docs/spec/studio/part-8-handler-authoring.zh-Hant.md \
        docs/HUMAN_SMOKE/plan-13-handler-smoke-test.md
git commit -m "docs: Plan 13 — API + UI + handler-authoring spec sync + HUMAN_SMOKE"
```

---

## Final acceptance gates

After all tasks pass review:

- [ ] `cargo build && cargo test` clean (target ~418 PASS, +10 from 408)
- [ ] `pnpm tsc -b && pnpm test && pnpm build` clean (target ~174 PASS, +8 from 166)
- [ ] HandlerDetailPage shows "Smoke test" section below "Last build"
- [ ] Paste 3 JSON lines, click Run → outcomes table renders 3 rows with correct status mapping
- [ ] Fixtures picker accepts `.jsonl`, `.json` (array), `.csv`, and directories
- [ ] Picking a file with no rows shows "no rows found in fixtures path"
- [ ] Row count input clamped to 1..=100
- [ ] Active-run gate: if an exec attempt is running using this handler, Run is rejected with HandlerBusy
- [ ] Build failure surfaces the build failure error (BuildFailed UiError)
- [ ] stderr tail collapsible block shows handler's stderr output (last 4 KiB)
- [ ] HUMAN_SMOKE Plan 13 walkthrough added
- [ ] Spec docs (part-5 + part-7 + part-8 en + zh-Hant) updated
