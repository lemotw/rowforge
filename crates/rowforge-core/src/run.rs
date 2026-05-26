use crate::reader::FieldMap;
use crate::input_stream::{open_input, InputFormat};
use crate::manifest::Manifest;
use crate::meta::{manifest_hash, write_meta, HandlerMeta, RunMeta, Stats};
use crate::pool_streaming::{compute_run_stats, run_pool_streaming, StreamingPoolConfig};
use chrono::Utc;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

// -----------------------------------------------------------------------------
// Outcome error code constants.
//
// Centralized here so pool / writer / rerun all reference the same string.
// Any addition or rename to this list is a wire/observability change — bump
// docs accordingly.
// -----------------------------------------------------------------------------

/// Worker process died (or its stdout closed) with a row in flight.
pub const ERR_WORKER_CRASH: &str = "WORKER_CRASH";

/// Worker crashed in batch mode and the handler is NOT marked idempotent —
/// rows in the in-flight batch cannot be safely re-dispatched. See spec
/// §6.5 D4 (idempotent semantics).
pub const ERR_WORKER_CRASH_UNSAFE: &str = "WORKER_CRASH_UNSAFE";

/// Row was queued but the run was cancelled before dispatch. Synthesized
/// for any row not yet sent to a worker. T6 wires this up; defined here so
/// the constant ships with T4.
pub const ERR_CANCELLED: &str = "CANCELLED";

/// A single row's serialized JSON exceeded `ROW_HARD_CAP_BYTES` (4 MiB) —
/// the pool's batch accumulator rejects it without ever placing it in a
/// batch. See `runtime::ROW_HARD_CAP_BYTES`.
pub const ERR_ROW_TOO_LARGE: &str = "ROW_TOO_LARGE";

/// Handler's `batch_result` envelope was malformed or had a wrong number
/// of entries. T5 sets this on each row in the offending batch.
pub const ERR_BATCH_PROTOCOL_ERROR: &str = "BATCH_PROTOCOL_ERROR";

/// Handler returned an unexpected message variant for a row (e.g., `Ready`
/// or `BatchResult` in row mode). Defined here alongside other ERR_* constants
/// for centralized observability.
pub const ERR_PROTOCOL_ERROR: &str = "ROW_PROTOCOL_ERROR";

#[derive(Debug, Clone)]
pub enum RunProgressEvent {
    Started { total_rows: u64 },
    RowDone { seq: u64, success: bool },
    Completed { success: u64, failed: u64 },
}

pub type ProgressCallback = std::sync::Arc<dyn Fn(RunProgressEvent) + Send + Sync>;

pub struct RunRequest {
    pub run_id: String,
    pub parent_run_id: Option<String>,
    pub handler_dir: PathBuf,
    pub input_csv: PathBuf,
    pub output_dir: PathBuf,
    pub workers: u32,
    /// When true, every dispatched row carries `meta.dry_run = true`. Handler
    /// reads it via `ctx.DryRun`. Does NOT limit how many rows are dispatched.
    pub dry_run: bool,
    /// Legacy: bundled "dry-run + limit" knob. New callers should set
    /// `row_limit` instead. Kept so existing `rowforge run --dry-run
    /// --dry-run-sample N` keeps its old behavior.
    pub dry_run_sample: usize,
    /// If set, dispatch at most this many CSV rows. Independent of `dry_run`.
    /// Used by `rowforge exec run --sample N` to test-run a real subset.
    /// When None, the full input is dispatched.
    pub row_limit: Option<usize>,
    /// Skip these CSV row indices (seq) entirely. Applied BEFORE row_limit:
    /// if user passes skip_seqs = {0,1,2} and row_limit = 2 on a 10-row CSV,
    /// dispatch is rows {3,4} (the first 2 unskipped rows). Used by `exec run`
    /// to honor RowResolution monotonicity (spec I5).
    pub skip_seqs: std::collections::HashSet<u64>,
    pub field_map: FieldMap,
    pub config_overrides: BTreeMap<String, serde_json::Value>,
    pub shutdown_grace: Duration,
    pub on_progress: Option<ProgressCallback>,
    /// Optional live-broadcast callback for handler log lines.
    /// When set, every captured stderr/stdout line is forwarded to this callback
    /// in addition to being written to `handler_log.log`. Used by Studio's
    /// SessionRegistry to stream log lines to the Logs tab in real time.
    /// The CLI leaves this `None`.
    pub on_handler_log: Option<crate::pool_streaming::HandlerLogCallback>,
    /// Optional cancellation. If Some and the token fires, the run aborts
    /// cleanly: pool stops dispatching, in-flight workers receive shutdown,
    /// remaining queued rows are NOT processed (do not become STARTUP_FAILED;
    /// they're simply absent from the outcomes). The resulting `RunReport`
    /// will have `aborted = true`.
    pub cancel: Option<crate::cancel::CancellationToken>,
    /// Explicit input format override. If None, auto-detected from the
    /// `input_csv` file extension (.csv → Csv, .jsonl/.ndjson → Jsonl).
    /// Added in P8 for streaming dispatch support.
    pub input_format: Option<InputFormat>,
    /// When true, call sync_data after every outcomes.jsonl append for
    /// increased durability at the cost of throughput (P10 wire-up).
    pub fsync_outcomes: bool,
    /// When true, valid outcome JSON stdout lines are also written to
    /// `handler_log.log` (in addition to `outcomes.jsonl`). Controlled by
    /// `Settings.handler_log_capture_raw_stdout`; default false.
    /// Useful for diagnosing protocol issues — normal operation should leave
    /// this off to avoid duplicating outcome data in the log.
    pub capture_raw_stdout: bool,
    /// When Some, dispatch only the rows whose `seq` (0-based CSV row index)
    /// is in this list. All other rows are skipped silently. Row indices not
    /// present in the input are also silently ignored.
    ///
    /// Precedence: `only_row_ids` takes priority over `skip_seqs` —
    /// if a seq is in `only_row_ids`, it is dispatched even if it would
    /// have been skipped by `skip_seqs` (re-run intent overrides resume intent).
    ///
    /// `None` (default): existing behavior, dispatch all rows (modulo skip_seqs).
    /// `Some(vec![])`: dispatch nothing (vacuous noop).
    pub only_row_ids: Option<Vec<u64>>,
}

pub struct RunReport {
    pub success_count: u64,
    pub failed_count: u64,
    pub by_error_code: BTreeMap<String, u64>,
    pub run_dir: PathBuf,
    /// True when the Run did not process every queued row to completion.
    /// Cases: workers failed to start (STARTUP_FAILED synthesis) OR the
    /// caller cancelled via `RunRequest.cancel`. CLI maps this to exit
    /// code 2 ("run aborted") per spec §5.5; bare row failures map to exit 1.
    pub aborted: bool,
    /// Why the run was aborted, if `aborted` is true. Propagated from
    /// `StreamingPoolReport.abort_reason` (e.g. "stalled at ...", "worker
    /// errors: ...", "cancelled by operator", "all workers failed to
    /// start: ..."). None when `aborted` is false.
    pub abort_reason: Option<String>,
}

pub async fn execute(req: RunRequest) -> anyhow::Result<RunReport> {
    let started = Utc::now();

    // 1. Load manifest + compute hash.
    let (manifest, manifest_path) = Manifest::load_from_dir(&req.handler_dir)?;
    let manifest_bytes = std::fs::read(&manifest_path)?;
    let mfst_hash = manifest_hash(&manifest_bytes);

    // 2. Create output dir.
    std::fs::create_dir_all(&req.output_dir)?;

    // Guard: --output-dir must not live inside --handler. Otherwise
    // snapshot_handler walks the handler tree and recurses into the output
    // dir we just wrote, infinitely growing the path until the OS rejects it
    // with ENAMETOOLONG.
    let handler_canon = std::fs::canonicalize(&req.handler_dir)
        .map_err(|e| anyhow::anyhow!("canonicalize handler dir: {}", e))?;
    let output_canon = std::fs::canonicalize(&req.output_dir)
        .map_err(|e| anyhow::anyhow!("canonicalize output dir: {}", e))?;
    if output_canon == handler_canon || output_canon.starts_with(&handler_canon) {
        return Err(anyhow::anyhow!(
            "output_dir ({}) must not be inside handler dir ({}) — \
             handler-snapshot would recursively copy itself. Pick an output_dir \
             outside the handler folder.",
            output_canon.display(),
            handler_canon.display()
        ));
    }

    // 3. Detect format + open input stream (also performs required-input check).
    let format = InputFormat::detect(&req.input_csv, req.input_format)
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    // Snapshot input file with the appropriate extension.
    let input_ext = match format {
        InputFormat::Csv => "csv",
        InputFormat::Jsonl => "jsonl",
    };
    let input_snapshot = req.output_dir.join(format!("input.{}", input_ext));
    std::fs::copy(&req.input_csv, &input_snapshot)?;

    // Snapshot handler dir.
    snapshot_handler(&req.handler_dir, &req.output_dir.join("handler-snapshot"))?;

    // Count total input rows (for meta.json). We need this before streaming
    // starts, so we do a quick pass counting lines (minus header for CSV).
    let input_row_count: u64 = count_input_rows(&req.input_csv, format)
        .map_err(|e| anyhow::anyhow!("count input rows: {}", e))?;

    // Open the InputStream (performs required-input check).
    let input = open_input(&req.input_csv, format, &manifest.required_input)
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    // 4. Effective config = manifest defaults overlaid by CLI overrides.
    let mut effective_config: BTreeMap<String, serde_json::Value> = manifest
        .config
        .iter()
        .map(|(k, v)| (k.clone(), v.default.clone()))
        .collect();
    for (k, v) in &req.config_overrides {
        effective_config.insert(k.clone(), v.clone());
    }

    let runtime = manifest.runtime.clone().unwrap_or_default();

    // Effective row_limit: honour legacy dry_run + dry_run_sample knob.
    let effective_limit: Option<usize> = match req.row_limit {
        Some(n) => Some(n),
        None if req.dry_run => Some(req.dry_run_sample),
        None => None,
    };

    if let Some(cb) = req.on_progress.as_ref() {
        // When only_row_ids is set, the dispatchable row count is the filter
        // length, not the full input row count. For Plan 11 flows (failed-row
        // re-run), every seq in the filter is known to exist in the input
        // (sourced from the previous attempt's outcomes), so len(filter) ==
        // actual dispatched count.
        //
        // Note: if a caller passes only_row_ids with seqs that don't exist
        // in the input (manual API misuse), the actual dispatched count will
        // be lower. len(filter) is then an upper bound. Plan 11 flows never
        // hit this case.
        let total_rows = if let Some(filter) = &req.only_row_ids {
            filter.len() as u64
        } else {
            input_row_count
        };
        cb(RunProgressEvent::Started { total_rows });
    }

    // 5. Run the streaming pool.
    let jsonl_path = req.output_dir.join("outcomes.jsonl");

    // Bridge: rowforge-core's pool only knows about `Fn(seq, success)`. Map
    // it onto the caller's full ProgressCallback so per-row `RowDone` events
    // fire as the pool durably appends to outcomes.jsonl.
    let on_row_done: Option<Arc<dyn Fn(u64, bool) + Send + Sync>> = req
        .on_progress
        .as_ref()
        .map(|cb| {
            let cb = cb.clone();
            Arc::new(move |seq: u64, success: bool| {
                cb(RunProgressEvent::RowDone { seq, success });
            }) as Arc<dyn Fn(u64, bool) + Send + Sync>
        });

    let pool_cfg = StreamingPoolConfig {
        handler_dir: req.handler_dir.clone(),
        manifest: Arc::new(manifest.clone()),
        workers: req.workers,
        run_id: req.run_id.clone(),
        config: effective_config.clone(),
        shutdown_grace: req.shutdown_grace,
        cancel: req.cancel.clone(),
        runtime: runtime.clone(),
        jsonl_path: jsonl_path.clone(),
        fsync_outcomes: req.fsync_outcomes,
        stall_timeout: None,
        stall_poll_interval: None,
        on_row_done,
        on_handler_log: req.on_handler_log.clone(),
        capture_raw_stdout: req.capture_raw_stdout,
    };

    let pool_report = run_pool_streaming(
        input,
        req.skip_seqs.clone(),
        req.only_row_ids.clone(),
        effective_limit,
        req.field_map.clone(),
        req.dry_run,
        pool_cfg,
    )
    .await
    .map_err(|e| anyhow::anyhow!("{}", e))?;

    // 6. Compute stats from outcomes.jsonl.
    //
    // Per-row consumers (exec export, rerun) read outcomes.jsonl directly.
    // RunReport only carries aggregate counts; they come from a single pass
    // over the file here.
    let run_stats = compute_run_stats(&jsonl_path)
        .map_err(|e| anyhow::anyhow!("compute_run_stats: {}", e))?;

    let aborted = pool_report.aborted;
    let abort_reason = pool_report.abort_reason.clone();
    if aborted {
        tracing::warn!(
            reason = abort_reason.as_deref().unwrap_or("(unspecified)"),
            "run aborted"
        );
    }

    if let Some(cb) = req.on_progress.as_ref() {
        cb(RunProgressEvent::Completed {
            success: run_stats.success,
            failed: run_stats.failed,
        });
    }

    // 7. Write meta.json.
    //
    // NOTE: success.csv and failed.csv are no longer written here (v3.3).
    // Per-attempt output is only outcomes.jsonl. Use `exec export` (P10) to
    // produce sorted CSV on demand.
    let stats = Stats {
        success: run_stats.success,
        failed: run_stats.failed,
        by_error_code: run_stats.by_error_code.clone(),
        avg_dur_ms: 0, // streaming pipeline doesn't track per-row timing yet
    };
    let report = RunReport {
        success_count: stats.success,
        failed_count: stats.failed,
        by_error_code: stats.by_error_code.clone(),
        run_dir: req.output_dir.clone(),
        aborted,
        abort_reason: abort_reason.clone(),
    };
    let meta = RunMeta {
        run_id: req.run_id,
        parent_run_id: req.parent_run_id,
        started_at: started,
        ended_at: Utc::now(),
        input_path: req.input_csv.display().to_string(),
        input_row_count,
        handler: HandlerMeta {
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            manifest_hash: mfst_hash,
        },
        config: effective_config,
        stats,
        dry_run: req.dry_run,
    };
    write_meta(&req.output_dir.join("meta.json"), &meta)?;

    Ok(report)
}

// ---------------------------------------------------------------------------
// count_input_rows
// ---------------------------------------------------------------------------

/// Count the number of data rows in the input file (excluding the CSV header).
/// Used to populate `meta.json.input_row_count`.
fn count_input_rows(path: &Path, format: InputFormat) -> Result<u64, crate::error::CoreError> {
    let content = std::fs::read(path)?;
    let lines: Vec<&[u8]> = content.split(|&b| b == b'\n').collect();

    Ok(match format {
        InputFormat::Csv => {
            // Subtract 1 for the header row; ignore trailing empty line.
            let total = lines
                .iter()
                .filter(|l| !l.trim_ascii().is_empty())
                .count();
            total.saturating_sub(1) as u64
        }
        InputFormat::Jsonl => lines
            .iter()
            .filter(|l| !l.trim_ascii().is_empty())
            .count() as u64,
    })
}

// ---------------------------------------------------------------------------
// snapshot_handler / copy_dir_recursive
// ---------------------------------------------------------------------------

fn snapshot_handler(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    copy_dir_recursive(
        src,
        dest,
        &[
            "target",
            "node_modules",
            "__pycache__",
            "dist",
            ".venv",
            ".git",
        ],
    )
}

fn copy_dir_recursive(src: &Path, dest: &Path, ignore: &[&str]) -> std::io::Result<()> {
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if ignore.iter().any(|i| **i == *name_str) {
            continue;
        }
        let to = dest.join(&name);
        let ty = entry.file_type()?;
        if ty.is_dir() {
            std::fs::create_dir_all(&to)?;
            copy_dir_recursive(&entry.path(), &to, ignore)?;
        } else if ty.is_file() {
            std::fs::copy(entry.path(), to)?;
        }
        // skip symlinks for v1
    }
    Ok(())
}
