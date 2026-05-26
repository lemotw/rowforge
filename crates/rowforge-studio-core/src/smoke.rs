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

impl SmokeRunRequest {
    /// Construct with required fields.
    pub fn new(
        handler_name: impl Into<String>,
        rows: Vec<serde_json::Map<String, serde_json::Value>>,
    ) -> Self {
        Self {
            handler_name: handler_name.into(),
            rows,
        }
    }
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

use std::path::Path;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Plan 13 T6 — run_smoke + name_ok
// ---------------------------------------------------------------------------

/// Internal smoke runner. The `StudioCore::handler_smoke_run` wrapper owns
/// the lock + sqlite gate and forwards to this function with the resolved
/// handler dir.
pub(crate) async fn run_smoke(
    handler_name: &str,
    handler_dir: &Path,
    rows: Vec<serde_json::Map<String, serde_json::Value>>,
    timeout_per_row_secs: u64,
) -> Result<SmokeRunResult, crate::UiError> {
    let (manifest, _) = rowforge_core::manifest::Manifest::load_from_dir(handler_dir)
        .map_err(|e| crate::UiError::Io(format!("manifest load: {e}")))?;

    if rowforge_core::build::needs_build(handler_dir, &manifest) {
        match rowforge_core::build::run_build(handler_dir, &manifest) {
            Ok(_) => {}
            Err(rowforge_core::build::BuildError::BuildFailed { exit_code, .. }) => {
                return Err(crate::UiError::BuildFailed {
                    name: handler_name.to_string(),
                    exit_code,
                })
            }
            Err(rowforge_core::build::BuildError::ToolchainMissing { tool }) => {
                return Err(crate::UiError::ToolchainMissing {
                    name: handler_name.to_string(),
                    tool,
                })
            }
            Err(rowforge_core::build::BuildError::NoBuildCommand) => {
                return Err(crate::UiError::NoBuildCommand {
                    name: handler_name.to_string(),
                })
            }
            Err(rowforge_core::build::BuildError::Io(e)) => return Err(crate::UiError::Io(e)),
        }
    }

    // Derive `columns` from the first row's keys (best-effort).
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
    .map_err(|e| crate::UiError::Io(format!("spawn worker: {e}")))?;

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
        Duration::from_secs(60 * 60)
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

/// Returns true iff `name` is a valid handler name (`[a-z0-9][a-z0-9-]*`).
pub(crate) fn name_ok(name: &str) -> bool {
    crate::handler::validate_name(name)
}

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
}
