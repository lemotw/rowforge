//! Export logic shared between rowforge-cli and rowforge-studio-core.
//!
//! See spec docs/spec/cli/part-2-model.md for resolution semantics.

use crate::error::CoreError;
use crate::execution_store::{AttemptState, ExecutionStore};
use crate::row_resolution::{ResolutionCounts, RowResolution};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, CoreError>;

// ---------------------------------------------------------------------------
// Public types (scaffolded in Task 1)
// ---------------------------------------------------------------------------

#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Csv,
    Jsonl,
    Both,
}

#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExportOpts {
    /// None = auto-pick `<exec_dir>/exports/<UTC-timestamp>/`.
    pub output_dir: Option<PathBuf>,
    pub format: ExportFormat,
    /// If true and any rows are `NeverAttempted` (or any attempt is aborted),
    /// return an incomplete-export error before any file is written.
    pub require_complete: bool,
}

impl ExportOpts {
    pub fn new(format: ExportFormat) -> Self {
        ExportOpts {
            output_dir: None,
            format,
            require_complete: false,
        }
    }

    pub fn with_output_dir(mut self, dir: PathBuf) -> Self {
        self.output_dir = Some(dir);
        self
    }

    pub fn with_require_complete(mut self, require_complete: bool) -> Self {
        self.require_complete = require_complete;
        self
    }
}

#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportReport {
    pub output_dir: PathBuf,
    pub written_files: Vec<PathBuf>,
    pub success_count: u64,
    pub failed_count: u64,
    pub warnings: Vec<ExportWarning>,
}

#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportWarning {
    pub code: String,
    pub message: String,
}

// ---------------------------------------------------------------------------
// Completeness summary (used in resolution.json)
// ---------------------------------------------------------------------------

#[derive(serde::Serialize)]
struct Completeness {
    fully_processed: bool,
    completion_percent: f64,
    completed_attempts: u32,
    aborted_attempts: u32,
    aborted_attempt_ids: Vec<String>,
    aborted_reasons: Vec<String>,
}

impl Completeness {
    fn compute(res: &RowResolution, aborted: &[(String, Option<String>)]) -> Self {
        let total = res.input_row_count;
        let resolved = res.counts.resolved
            + res.counts.failed_last
            + res.counts.crashed_last
            + res.counts.cancelled_last
            + res.counts.too_large;
        let completion_percent = if total > 0 {
            (resolved as f64 / total as f64) * 100.0
        } else {
            100.0
        };
        let aborted_count = aborted.len() as u32;
        let all_attempts_count = res.merged_from_attempts.len() as u32;
        let completed_count = all_attempts_count.saturating_sub(aborted_count);
        Completeness {
            fully_processed: res.counts.never_attempted == 0 && aborted_count == 0,
            completion_percent,
            completed_attempts: completed_count,
            aborted_attempts: aborted_count,
            aborted_attempt_ids: aborted.iter().map(|(id, _)| id.clone()).collect(),
            aborted_reasons: aborted
                .iter()
                .map(|(_, r)| r.clone().unwrap_or_default())
                .collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// Collect aborted attempts
// ---------------------------------------------------------------------------

fn collect_aborted_attempts(
    store: &ExecutionStore,
    exec_id: &str,
) -> Result<Vec<(String, Option<String>)>> {
    let attempts = store.list_attempts_for_execution(exec_id)?;
    Ok(attempts
        .into_iter()
        .filter(|a| a.state == AttemptState::Aborted)
        .map(|a| (a.id.clone(), a.aborted_reason.clone()))
        .collect())
}

// ---------------------------------------------------------------------------
// Export warnings (§14.5)
// ---------------------------------------------------------------------------

fn emit_export_warnings(
    res: &RowResolution,
    aborted: &[(String, Option<String>)],
) -> Vec<ExportWarning> {
    let mut warnings = Vec::new();
    if res.counts.never_attempted > 0 {
        tracing::warn!(
            never_attempted = res.counts.never_attempted,
            input_row_count = res.input_row_count,
            "export contains {} rows that were never attempted; \
             execution may be incomplete. Run more attempts to cover them.",
            res.counts.never_attempted
        );
        warnings.push(ExportWarning {
            code: "NEVER_ATTEMPTED".to_string(),
            message: format!(
                "{} rows were never attempted; execution may be incomplete",
                res.counts.never_attempted
            ),
        });
    }
    if !aborted.is_empty() {
        tracing::warn!(
            aborted_attempts = aborted.len(),
            "export includes data from {} aborted attempt(s); \
             check resolution.json for per-row resolution counts.",
            aborted.len()
        );
        warnings.push(ExportWarning {
            code: "ABORTED_ATTEMPTS".to_string(),
            message: format!(
                "export includes data from {} aborted attempt(s)",
                aborted.len()
            ),
        });
    }
    warnings
}

// ---------------------------------------------------------------------------
// CSV column discovery helpers (§14.3, D12)
// ---------------------------------------------------------------------------

/// Collect the union of all handler data keys from canonical_success records,
/// excluding "seqid". Returns a BTreeSet so they are alphabetically sorted.
fn discover_success_keys(res: &RowResolution) -> std::collections::BTreeSet<String> {
    let mut keys = std::collections::BTreeSet::new();
    for (_, p) in &res.per_seq {
        if let Some(s) = &p.canonical_success {
            for h in &s.headers {
                if h != "seqid" {
                    keys.insert(h.clone());
                }
            }
        }
    }
    keys
}

/// Collect the union of all handler data keys from latest_failure records,
/// excluding "seqid", "errcode", "errmessage".
fn discover_failure_data_keys(res: &RowResolution) -> std::collections::BTreeSet<String> {
    let mut keys = std::collections::BTreeSet::new();
    for (_, p) in &res.per_seq {
        if let Some(f) = &p.latest_failure {
            for h in &f.headers {
                if h != "seqid" && h != "errcode" && h != "errmessage" {
                    keys.insert(h.clone());
                }
            }
        }
    }
    keys
}

// ---------------------------------------------------------------------------
// CSV writers
// ---------------------------------------------------------------------------

fn write_success_csv(path: &Path, res: &RowResolution) -> anyhow::Result<()> {
    use anyhow::Context;

    let keys = discover_success_keys(res);
    // Column order: seqid first, then alphabetical handler keys.
    let cols: Vec<String> = std::iter::once("seqid".to_string())
        .chain(keys.into_iter())
        .collect();

    let mut w = csv::Writer::from_path(path)
        .with_context(|| format!("create {}", path.display()))?;
    w.write_record(&cols).context("write success header")?;

    for (seq, p) in &res.per_seq {
        if let Some(s) = &p.canonical_success {
            let mut row: Vec<String> = Vec::with_capacity(cols.len());
            row.push(seq.to_string());
            for col in cols.iter().skip(1) {
                let val = s
                    .headers
                    .iter()
                    .position(|h| h == col)
                    .and_then(|i| s.raw.get(i))
                    .map(|v| v.to_string())
                    .unwrap_or_default();
                row.push(val);
            }
            w.write_record(&row).context("write success row")?;
        }
    }
    w.flush()?;
    Ok(())
}

fn write_failed_csv(path: &Path, res: &RowResolution) -> anyhow::Result<()> {
    use anyhow::Context;
    use crate::row_resolution::ResolutionState;

    let data_keys = discover_failure_data_keys(res);
    // Column order: seqid, errcode, errmessage, then alphabetical data keys.
    let cols: Vec<String> = ["seqid", "errcode", "errmessage"]
        .iter()
        .map(|s| s.to_string())
        .chain(data_keys.into_iter())
        .collect();

    let mut w = csv::Writer::from_path(path)
        .with_context(|| format!("create {}", path.display()))?;
    w.write_record(&cols).context("write failed header")?;

    for (seq, p) in &res.per_seq {
        match p.state {
            ResolutionState::Resolved => continue,
            ResolutionState::NeverAttempted => {
                // Synthesize a row: seqid, NEVER_ATTEMPTED, message, empty data cols.
                let mut row: Vec<String> = Vec::with_capacity(cols.len());
                row.push(seq.to_string());
                row.push("NEVER_ATTEMPTED".to_string());
                row.push(
                    "row never reached a worker (was not sampled or never dispatched)"
                        .to_string(),
                );
                for _ in cols.iter().skip(3) {
                    row.push(String::new());
                }
                w.write_record(&row)
                    .context("write synthetic NEVER_ATTEMPTED")?;
            }
            _ => {
                if let Some(fr) = &p.latest_failure {
                    let mut row: Vec<String> = Vec::with_capacity(cols.len());
                    row.push(seq.to_string());
                    // errcode
                    row.push(
                        fr.headers
                            .iter()
                            .position(|h| h == "errcode")
                            .and_then(|i| fr.raw.get(i))
                            .map(|v| v.to_string())
                            .unwrap_or_default(),
                    );
                    // errmessage
                    row.push(
                        fr.headers
                            .iter()
                            .position(|h| h == "errmessage")
                            .and_then(|i| fr.raw.get(i))
                            .map(|v| v.to_string())
                            .unwrap_or_default(),
                    );
                    // data keys
                    for col in cols.iter().skip(3) {
                        let val = fr
                            .headers
                            .iter()
                            .position(|h| h == col)
                            .and_then(|i| fr.raw.get(i))
                            .map(|v| v.to_string())
                            .unwrap_or_default();
                        row.push(val);
                    }
                    w.write_record(&row).context("write failed row")?;
                }
            }
        }
    }
    w.flush()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// JSONL export helpers (§14.4, D1, D2, D3)
// ---------------------------------------------------------------------------

/// Write a JSON object with a fixed key ordering using a Vec<(String, Value)>
/// serialized manually so that insertion order is preserved regardless of
/// serde_json's Map implementation (which may not have `preserve_order`).
fn write_json_object(
    writer: &mut impl std::io::Write,
    fields: Vec<(&str, serde_json::Value)>,
) -> anyhow::Result<()> {
    let mut parts = Vec::with_capacity(fields.len());
    for (k, v) in fields {
        let key_json = serde_json::to_string(k)?;
        let val_json = serde_json::to_string(&v)?;
        parts.push(format!("{}:{}", key_json, val_json));
    }
    writeln!(writer, "{{{}}}", parts.join(","))?;
    Ok(())
}

fn write_success_jsonl(path: &Path, res: &RowResolution) -> anyhow::Result<()> {
    use anyhow::Context;

    let keys: Vec<String> = discover_success_keys(res).into_iter().collect();

    let mut file = std::fs::File::create(path)
        .with_context(|| format!("create {}", path.display()))?;

    for (seq, p) in &res.per_seq {
        if let Some(s) = &p.canonical_success {
            let mut fields: Vec<(&str, serde_json::Value)> = Vec::new();
            fields.push(("seqid", serde_json::Value::Number((*seq).into())));
            for key in &keys {
                let val = s
                    .headers
                    .iter()
                    .position(|h| h == key)
                    .and_then(|i| s.raw.get(i))
                    .map(|v| serde_json::Value::String(v.to_string()))
                    .unwrap_or(serde_json::Value::Null); // D1: null for missing
                fields.push((key.as_str(), val));
            }
            write_json_object(&mut file, fields)
                .with_context(|| format!("write success.jsonl row seq={seq}"))?;
        }
    }
    Ok(())
}

fn write_failed_jsonl(path: &Path, res: &RowResolution) -> anyhow::Result<()> {
    use anyhow::Context;
    use crate::row_resolution::ResolutionState;

    let data_keys: Vec<String> = discover_failure_data_keys(res).into_iter().collect();

    let mut file = std::fs::File::create(path)
        .with_context(|| format!("create {}", path.display()))?;

    for (seq, p) in &res.per_seq {
        match p.state {
            ResolutionState::Resolved => continue,
            ResolutionState::NeverAttempted => {
                // D3 order: seqid, errcode, errmessage, ...data keys (all null)
                let mut fields: Vec<(&str, serde_json::Value)> = Vec::new();
                fields.push(("seqid", serde_json::Value::Number((*seq).into())));
                fields.push((
                    "errcode",
                    serde_json::Value::String("NEVER_ATTEMPTED".to_string()),
                ));
                fields.push((
                    "errmessage",
                    serde_json::Value::String(
                        "row never reached a worker (was not sampled or never dispatched)"
                            .to_string(),
                    ),
                ));
                for key in &data_keys {
                    fields.push((key.as_str(), serde_json::Value::Null));
                }
                write_json_object(&mut file, fields)
                    .with_context(|| format!("write failed.jsonl NEVER_ATTEMPTED seq={seq}"))?;
            }
            _ => {
                if let Some(fr) = &p.latest_failure {
                    // D3 order: seqid, errcode, errmessage, ...data keys
                    let errcode = fr
                        .headers
                        .iter()
                        .position(|h| h == "errcode")
                        .and_then(|i| fr.raw.get(i))
                        .map(|v| serde_json::Value::String(v.to_string()))
                        .unwrap_or(serde_json::Value::Null);
                    let errmessage = fr
                        .headers
                        .iter()
                        .position(|h| h == "errmessage")
                        .and_then(|i| fr.raw.get(i))
                        .map(|v| serde_json::Value::String(v.to_string()))
                        .unwrap_or(serde_json::Value::Null);

                    let mut fields: Vec<(&str, serde_json::Value)> = Vec::new();
                    fields.push(("seqid", serde_json::Value::Number((*seq).into())));
                    fields.push(("errcode", errcode));
                    fields.push(("errmessage", errmessage));
                    for key in &data_keys {
                        let val = fr
                            .headers
                            .iter()
                            .position(|h| h == key)
                            .and_then(|i| fr.raw.get(i))
                            .map(|v| serde_json::Value::String(v.to_string()))
                            .unwrap_or(serde_json::Value::Null);
                        fields.push((key.as_str(), val));
                    }
                    write_json_object(&mut file, fields)
                        .with_context(|| format!("write failed.jsonl row seq={seq}"))?;
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// resolution.json with completeness (§14.6)
// ---------------------------------------------------------------------------

fn write_resolution_json_with_completeness(
    path: &Path,
    res: &RowResolution,
    comp: &Completeness,
) -> anyhow::Result<()> {
    #[derive(serde::Serialize)]
    struct Summary<'a> {
        execution_id: &'a str,
        input_row_count: u64,
        counts: &'a ResolutionCounts,
        completeness: &'a Completeness,
        merged_from_attempts: &'a [String],
        by_error_code: &'a BTreeMap<String, u64>,
        skipped_running: &'a [String],
    }

    let summary = Summary {
        execution_id: &res.execution_id,
        input_row_count: res.input_row_count,
        counts: &res.counts,
        completeness: comp,
        merged_from_attempts: &res.merged_from_attempts,
        by_error_code: &res.by_error_code,
        skipped_running: &res.skipped_running,
    };
    let body = serde_json::to_string_pretty(&summary)?;
    std::fs::write(path, body)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Top-level export entry point
// ---------------------------------------------------------------------------

/// Top-level export entry. Studio + CLI both call this.
pub fn export_execution(
    store: &ExecutionStore,
    exec_id: &str,
    opts: &ExportOpts,
) -> Result<ExportReport> {
    let exec = store
        .get_execution(exec_id)?
        .ok_or_else(|| CoreError::Store(format!("execution not found: {exec_id}")))?;

    let res = crate::row_resolution::compute_resolution(store, &exec.id)?;

    let aborted = collect_aborted_attempts(store, &exec.id)?;
    let completeness = Completeness::compute(&res, &aborted);

    // Completeness check (strict mode).
    if opts.require_complete && !completeness.fully_processed {
        let unresolved = res.counts.never_attempted;
        return Err(CoreError::Store(format!("export_incomplete:{unresolved}")));
    }

    // Resolve output directory.
    let output_dir = match opts.output_dir.clone() {
        Some(d) => d,
        None => exec
            .dir
            .join("exports")
            .join(chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string()),
    };
    std::fs::create_dir_all(&output_dir)
        .map_err(CoreError::Io)?;

    // Emit warnings.
    let warnings = emit_export_warnings(&res, &aborted);

    let mut written_files: Vec<PathBuf> = Vec::new();

    // Write data files.
    match opts.format {
        ExportFormat::Csv => {
            let p = output_dir.join("success.csv");
            write_success_csv(&p, &res).map_err(CoreError::Other)?;
            written_files.push(p);
            let p = output_dir.join("failed.csv");
            write_failed_csv(&p, &res).map_err(CoreError::Other)?;
            written_files.push(p);
        }
        ExportFormat::Jsonl => {
            let p = output_dir.join("success.jsonl");
            write_success_jsonl(&p, &res).map_err(CoreError::Other)?;
            written_files.push(p);
            let p = output_dir.join("failed.jsonl");
            write_failed_jsonl(&p, &res).map_err(CoreError::Other)?;
            written_files.push(p);
        }
        ExportFormat::Both => {
            let p = output_dir.join("success.csv");
            write_success_csv(&p, &res).map_err(CoreError::Other)?;
            written_files.push(p);
            let p = output_dir.join("failed.csv");
            write_failed_csv(&p, &res).map_err(CoreError::Other)?;
            written_files.push(p);
            let p = output_dir.join("success.jsonl");
            write_success_jsonl(&p, &res).map_err(CoreError::Other)?;
            written_files.push(p);
            let p = output_dir.join("failed.jsonl");
            write_failed_jsonl(&p, &res).map_err(CoreError::Other)?;
            written_files.push(p);
        }
    }

    // Always write resolution.json.
    let resolution_path = output_dir.join("resolution.json");
    write_resolution_json_with_completeness(&resolution_path, &res, &completeness)
        .map_err(CoreError::Other)?;
    written_files.push(resolution_path);

    let success_count = res.counts.resolved;
    let failed_count = res.counts.failed_last
        + res.counts.crashed_last
        + res.counts.cancelled_last
        + res.counts.too_large
        + res.counts.never_attempted;

    Ok(ExportReport {
        output_dir,
        written_files,
        success_count,
        failed_count,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn export_format_serializes_snake_case() {
        assert_eq!(serde_json::to_value(ExportFormat::Csv).unwrap(), json!("csv"));
        assert_eq!(serde_json::to_value(ExportFormat::Jsonl).unwrap(), json!("jsonl"));
        assert_eq!(serde_json::to_value(ExportFormat::Both).unwrap(), json!("both"));
    }

    #[test]
    fn export_opts_round_trip() {
        let opts = ExportOpts {
            output_dir: Some(PathBuf::from("/tmp/x")),
            format: ExportFormat::Both,
            require_complete: true,
        };
        let s = serde_json::to_string(&opts).unwrap();
        let back: ExportOpts = serde_json::from_str(&s).unwrap();
        assert_eq!(opts, back);
    }
}
