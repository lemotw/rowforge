//! `rowforge exec ...` — Execution management.
//!
//! MVP surface:
//!   exec start --csv <path> [--name N] [--csv-id ID]
//!   exec list
//!   exec show <id>
//!   exec set-state <id> <state> [--reason R]
//!
//! Backed by `rowforge_core::execution_store::ExecutionStore`. No attempt
//! wiring yet — that lands in a follow-up.

use anyhow::{anyhow, Context, Result};
use clap::{Args, Subcommand, ValueEnum};
use rowforge_core::execution_store::{
    Execution, ExecutionState, ExecutionStore, FinishAttempt, NewAttempt, NewExecution,
    NewHandlerInstance, RunType, Simulation, Source,
};
use rowforge_core::manifest::Manifest;
use rowforge_core::run::{execute, RunRequest};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Args)]
pub struct ExecArgs {
    #[command(subcommand)]
    pub cmd: ExecCmd,
}

#[derive(Subcommand)]
pub enum ExecCmd {
    /// Create a new execution from an input CSV.
    Start(StartArgs),
    /// List all executions, newest first.
    List,
    /// Show a single execution in detail.
    Show(ShowArgs),
    /// Manually transition execution state.
    SetState(SetStateArgs),
    /// Run an attempt against an existing execution.
    Run(RunAttemptArgs),
    /// List attempts for an execution.
    Attempts(AttemptsArgs),
    /// Show one attempt's full detail (paths, stats, run_type).
    Attempt(AttemptShowArgs),
    /// Merge all attempt outputs into a single success.csv / failed.csv
    /// per the RowResolution rules (spec § Row Resolution).
    Export(ExportArgs),
}

#[derive(Args)]
pub struct RunAttemptArgs {
    /// Execution id (e_...).
    pub exec_id: String,
    /// Handler directory (contains rowforge.yaml).
    #[arg(long)]
    pub handler: PathBuf,
    /// Dispatch at most N rows (test run). Real side effects unless --dry-run.
    #[arg(long)]
    pub sample: Option<usize>,
    /// Set `ctx.DryRun = true` on every dispatched row. Does not limit row count.
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,
    /// Worker pool size.
    #[arg(long, default_value_t = 1)]
    pub workers: u32,
    /// Override handler config: key=json_value. Repeatable.
    #[arg(long = "config", value_parser = parse_kv)]
    pub config: Vec<(String, String)>,
    /// Override field mapping: schema_field=csv_column. Repeatable.
    #[arg(long = "field-map", value_parser = parse_kv)]
    pub field_map: Vec<(String, String)>,
    /// Reset semantic: re-dispatch EVERY row including ones from prior
    /// attempts (Resolved, FailedLast, etc.). The default is one-shot —
    /// each row gets dispatched at most once across all attempts. Use
    /// --force when you've changed input semantics or want to re-test
    /// everything from scratch.
    #[arg(long, default_value_t = false)]
    pub force: bool,
    /// Re-dispatch rows whose prior outcome was NOT Resolved (FailedLast,
    /// CrashedLast, CancelledLast, TooLarge). Resolved (successful) rows
    /// stay skipped. Use when you've fixed the handler and want to retry
    /// failures without redoing successes. Mutually exclusive with --force.
    #[arg(long = "retry-failed", default_value_t = false, conflicts_with = "force")]
    pub retry_failed: bool,
    /// Call sync_data after every outcomes.jsonl append (durability at cost of
    /// throughput). Off by default; enable in CI or when disk reliability is a
    /// concern.
    #[arg(long, default_value_t = false)]
    pub fsync_outcomes: bool,
}

#[derive(Args)]
pub struct AttemptsArgs {
    pub exec_id: String,
}

#[derive(Args)]
pub struct AttemptShowArgs {
    pub attempt_id: String,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum ExportFormat {
    Csv,
    Jsonl,
    Both,
}

#[derive(Args)]
pub struct ExportArgs {
    pub exec_id: String,
    /// Output dir. Defaults to <exec_dir>/exports/<UTC-timestamp>/.
    #[arg(long)]
    pub output_dir: Option<PathBuf>,
    /// Output format: csv (default), jsonl, or both.
    #[arg(long, value_enum, default_value = "csv")]
    pub format: ExportFormat,
    /// Fail with exit code 3 if execution is not fully processed
    /// (never_attempted > 0 or any aborted attempt).
    #[arg(long, default_value_t = false)]
    pub strict: bool,
}

fn parse_kv(s: &str) -> std::result::Result<(String, String), String> {
    let (k, v) = s
        .split_once('=')
        .ok_or_else(|| format!("expected K=V, got '{}'", s))?;
    Ok((k.to_string(), v.to_string()))
}

#[derive(Args)]
pub struct StartArgs {
    /// Path to the input CSV. Snapshotted into the execution folder.
    #[arg(long)]
    pub csv: PathBuf,
    /// Human-friendly label.
    #[arg(long)]
    pub name: Option<String>,
    /// Logical CSV id for cross-referencing (free-form for now).
    #[arg(long, default_value = "csv_unregistered")]
    pub csv_id: String,
    /// Pin a handler instance id at creation time.
    #[arg(long)]
    pub handler_instance_id: Option<String>,
}

#[derive(Args)]
pub struct ShowArgs {
    pub id: String,
}

#[derive(Args)]
pub struct SetStateArgs {
    pub id: String,
    #[arg(value_enum)]
    pub state: CliState,
    /// Required when transitioning to Abandoned.
    #[arg(long)]
    pub reason: Option<String>,
}

#[derive(Copy, Clone, ValueEnum)]
pub enum CliState {
    Open,
    Iterating,
    Settled,
    Closed,
    Abandoned,
}

impl From<CliState> for ExecutionState {
    fn from(c: CliState) -> Self {
        match c {
            CliState::Open => ExecutionState::Open,
            CliState::Iterating => ExecutionState::Iterating,
            CliState::Settled => ExecutionState::Settled,
            CliState::Closed => ExecutionState::Closed,
            CliState::Abandoned => ExecutionState::Abandoned,
        }
    }
}

pub async fn run(args: ExecArgs) -> Result<i32> {
    let home = rowforge_home()?;
    let mut store = ExecutionStore::open(&home)
        .with_context(|| format!("open execution store at {}", home.display()))?;

    match args.cmd {
        ExecCmd::Start(a) => {
            let exec = store.create_execution(NewExecution {
                name: a.name,
                input_csv_id: a.csv_id,
                input_csv_path: a.csv,
                current_handler_instance_id: a.handler_instance_id,
            })?;
            println!("created {}", exec.id);
            println!("  dir:        {}", exec.dir.display());
            println!("  rows:       {}", exec.input_row_count);
            println!("  csv_hash:   {}", exec.input_csv_hash);
            println!("  state:      {:?}", exec.state);
            Ok(0)
        }
        ExecCmd::List => {
            let rows = store.list_executions()?;
            if rows.is_empty() {
                println!("(no executions)");
                return Ok(0);
            }
            println!(
                "{:<32} {:<11} {:>7} {:<25} {}",
                "ID", "STATE", "ROWS", "CREATED", "NAME"
            );
            for e in rows {
                println!(
                    "{:<32} {:<11} {:>7} {:<25} {}",
                    e.id,
                    format!("{:?}", e.state).to_lowercase(),
                    e.input_row_count,
                    e.created_at.to_rfc3339(),
                    e.name.as_deref().unwrap_or("-")
                );
            }
            Ok(0)
        }
        ExecCmd::Show(a) => {
            let exec = store
                .get_execution(&a.id)?
                .ok_or_else(|| anyhow!("execution not found: {}", a.id))?;
            print_exec(&exec);
            Ok(0)
        }
        ExecCmd::SetState(a) => {
            let state: ExecutionState = a.state.into();
            if matches!(state, ExecutionState::Abandoned) && a.reason.is_none() {
                return Err(anyhow!("--reason is required when setting state to abandoned"));
            }
            let exec = store.set_execution_state(&a.id, state, a.reason)?;
            print_exec(&exec);
            Ok(0)
        }
        ExecCmd::Run(a) => run_attempt(&mut store, a).await,
        ExecCmd::Attempts(a) => {
            let list = store.list_attempts_for_execution(&a.exec_id)?;
            if list.is_empty() {
                println!("(no attempts)");
                return Ok(0);
            }
            println!(
                "{:<32} {:<10} {:>7} {:>7} {:<12} {:<35} {}",
                "ATTEMPT ID", "STATE", "OK", "FAILED", "SOURCE", "STARTED", "DIR"
            );
            for at in list {
                let src = match at.run_type.source {
                    Source::Full => "full".to_string(),
                    Source::Sampled { size } => format!("sampled({size})"),
                };
                println!(
                    "{:<32} {:<10} {:>7} {:>7} {:<12} {:<35} {}",
                    at.id,
                    format!("{:?}", at.state).to_lowercase(),
                    at.success_count,
                    at.failed_count,
                    src,
                    at.started_at.to_rfc3339(),
                    at.dir.display(),
                );
            }
            Ok(0)
        }
        ExecCmd::Attempt(a) => show_attempt(&store, &a.attempt_id),
        ExecCmd::Export(a) => export_resolution(&store, a),
    }
}

fn show_attempt(store: &ExecutionStore, id: &str) -> Result<i32> {
    let at = store
        .get_attempt(id)?
        .ok_or_else(|| anyhow!("attempt not found: {}", id))?;
    println!("id:                  {}", at.id);
    println!("execution_id:        {}", at.execution_id);
    println!("handler_instance_id: {}", at.handler_instance_id);
    println!("state:               {:?}", at.state);
    println!("source:              {:?}", at.run_type.source);
    println!("simulation:          {:?}", at.run_type.simulation);
    println!("success_count:       {}", at.success_count);
    println!("failed_count:        {}", at.failed_count);
    println!("started_at:          {}", at.started_at.to_rfc3339());
    if let Some(t) = at.ended_at {
        println!("ended_at:            {}", t.to_rfc3339());
    }
    if let Some(r) = &at.aborted_reason {
        println!("aborted_reason:      {}", r);
    }
    println!("dir:                 {}", at.dir.display());
    println!("  success.csv:       {}", at.dir.join("success.csv").display());
    println!("  failed.csv:        {}", at.dir.join("failed.csv").display());
    println!("  meta.json:         {}", at.dir.join("meta.json").display());
    Ok(0)
}

async fn run_attempt(store: &mut ExecutionStore, a: RunAttemptArgs) -> Result<i32> {
    let exec = store
        .get_execution(&a.exec_id)?
        .ok_or_else(|| anyhow!("execution not found: {}", a.exec_id))?;

    // Retry policy (v3.3+):
    //   default        → skip everything attempted (one-shot, safe)
    //   --retry-failed → dispatch ONLY failures (skip everything except
    //                    FailedLast/CrashedLast/CancelledLast/TooLarge —
    //                    Resolved AND NeverAttempted are both skipped)
    //   --force        → skip nothing (full reset)
    let skip_seqs = if a.force {
        std::collections::HashSet::new()
    } else {
        let res = rowforge_core::row_resolution::compute_resolution(store, &exec.id)?;
        if a.retry_failed {
            // Dispatch ONLY failures: skip = all_seqs - failed_seqs.
            let failures = res.failed_seqs();
            let all_seqs: std::collections::HashSet<u64> =
                (0..exec.input_row_count).collect();
            let s: std::collections::HashSet<u64> =
                all_seqs.difference(&failures).copied().collect();
            eprintln!(
                "[rowforge] --retry-failed: dispatching {} failed row(s); \
                 skipping {} (Resolved + NeverAttempted)",
                failures.len(),
                s.len(),
            );
            s
        } else {
            let s = res.attempted_seqs();
            if !s.is_empty() {
                eprintln!(
                    "[rowforge] skipping {} already-attempted row(s); use \
                     --retry-failed to retry only failures, or --force to \
                     re-dispatch all",
                    s.len(),
                );
            }
            s
        }
    };

    let (manifest, manifest_path) =
        Manifest::load_from_dir(&a.handler).with_context(|| "load handler manifest")?;
    let manifest_bytes = std::fs::read(&manifest_path)?;
    let manifest_hash = format!("sha256:{:x}", Sha256::digest(&manifest_bytes));
    let handler_canon = std::fs::canonicalize(&a.handler)
        .with_context(|| format!("canonicalize {}", a.handler.display()))?;

    let hi = store.register_handler_instance(NewHandlerInstance {
        handler_id: manifest.name.clone(),
        manifest_hash,
        source_snapshot_dir: handler_canon.clone(),
        binary_hash: None,
    })?;

    let source = match a.sample {
        Some(n) => Source::Sampled { size: n as u32 },
        None => Source::Full,
    };
    let simulation = if a.dry_run {
        Simulation::Dry
    } else {
        Simulation::Real
    };
    let attempt = store.create_attempt(NewAttempt {
        execution_id: exec.id.clone(),
        handler_instance_id: hi.id.clone(),
        parent_attempt_id: None,
        run_type: RunType {
            source: source.clone(),
            simulation,
        },
    })?;

    tracing::info!(
        attempt_id = %attempt.id,
        handler_instance = %hi.id,
        ?source,
        ?simulation,
        workers = a.workers,
        skip_seqs = skip_seqs.len(),
        "attempt starting"
    );

    let mut field_map = rowforge_core::csv_io::FieldMap::new();
    for (k, v) in a.field_map {
        field_map.insert(k, v);
    }
    let mut config = BTreeMap::new();
    for (k, v) in a.config {
        let val: serde_json::Value =
            serde_json::from_str(&v).unwrap_or_else(|_| serde_json::Value::String(v.clone()));
        config.insert(k, val);
    }

    // Snapshot filename preserves source extension (input.csv | input.jsonl |
    // input.ndjson). Probe in priority order for the actual on-disk file.
    let input_snapshot = ["input.jsonl", "input.ndjson", "input.csv"]
        .iter()
        .map(|n| exec.dir.join(n))
        .find(|p| p.is_file())
        .ok_or_else(|| anyhow!("no input snapshot found in {}", exec.dir.display()))?;

    let req = RunRequest {
        run_id: attempt.id.clone(),
        parent_run_id: None,
        handler_dir: handler_canon,
        input_csv: input_snapshot,
        output_dir: attempt.dir.clone(),
        workers: a.workers,
        dry_run: a.dry_run,
        dry_run_sample: 0,
        row_limit: a.sample,
        skip_seqs,
        field_map,
        config_overrides: config,
        shutdown_grace: Duration::from_secs(5),
        on_progress: Some(Box::new({
            use std::sync::atomic::{AtomicU64, Ordering};
            use std::sync::Arc;
            let total = Arc::new(AtomicU64::new(0));
            let done = Arc::new(AtomicU64::new(0));
            let ok = Arc::new(AtomicU64::new(0));
            move |ev| match ev {
                rowforge_core::run::RunProgressEvent::Started { total_rows } => {
                    total.store(total_rows, Ordering::Relaxed);
                    tracing::info!(total_rows, "dispatching rows");
                }
                rowforge_core::run::RunProgressEvent::RowDone { seq: _, success } => {
                    let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                    if success {
                        ok.fetch_add(1, Ordering::Relaxed);
                    }
                    // Heartbeat every 100 rows.
                    if d % 100 == 0 {
                        let t = total.load(Ordering::Relaxed);
                        let o = ok.load(Ordering::Relaxed);
                        tracing::info!(
                            done = d,
                            total = t,
                            ok = o,
                            failed = d - o,
                            "progress"
                        );
                    }
                }
                rowforge_core::run::RunProgressEvent::Completed { success, failed } => {
                    tracing::info!(success, failed, "dispatch completed");
                }
            }
        })),
        cancel: None,
        input_format: None,
        fsync_outcomes: a.fsync_outcomes,
    };

    let result = execute(req).await;
    match result {
        Ok(report) => {
            store.finish_attempt(
                &attempt.id,
                FinishAttempt {
                    success_count: report.success_count,
                    failed_count: report.failed_count,
                    aborted: report.aborted,
                    aborted_reason: report.abort_reason.clone(),
                },
            )?;
            println!("[rowforge] attempt {} finished", attempt.id);
            println!("  dir:      {}", attempt.dir.display());
            println!("  success:  {}", report.success_count);
            println!("  failed:   {}", report.failed_count);
            println!("  aborted:  {}", report.aborted);
            if let Some(ref r) = report.abort_reason {
                println!("  reason:   {}", r);
            }
            if report.aborted {
                Ok(2)
            } else if report.failed_count > 0 {
                Ok(1)
            } else {
                Ok(0)
            }
        }
        Err(e) => {
            let _ = store.finish_attempt(
                &attempt.id,
                FinishAttempt {
                    success_count: 0,
                    failed_count: 0,
                    aborted: true,
                    aborted_reason: Some(format!("{e:#}")),
                },
            );
            Err(e)
        }
    }
}

// ---------------------------------------------------------------------------
// Completeness summary
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
    fn compute(
        res: &rowforge_core::row_resolution::RowResolution,
        aborted: &[(String, Option<String>)],
    ) -> Self {
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
    use rowforge_core::execution_store::AttemptState;
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
    res: &rowforge_core::row_resolution::RowResolution,
    aborted: &[(String, Option<String>)],
) {
    if res.counts.never_attempted > 0 {
        tracing::warn!(
            never_attempted = res.counts.never_attempted,
            input_row_count = res.input_row_count,
            "export contains {} rows that were never attempted; \
             execution may be incomplete. Run more attempts to cover them.",
            res.counts.never_attempted
        );
    }
    if !aborted.is_empty() {
        tracing::warn!(
            aborted_attempts = aborted.len(),
            "export includes data from {} aborted attempt(s); \
             check resolution.json for per-row resolution counts.",
            aborted.len()
        );
    }
}

// ---------------------------------------------------------------------------
// Main export entry point
// ---------------------------------------------------------------------------

fn export_resolution(store: &ExecutionStore, a: ExportArgs) -> Result<i32> {
    let exec = store
        .get_execution(&a.exec_id)?
        .ok_or_else(|| anyhow!("execution not found: {}", a.exec_id))?;
    let res = rowforge_core::row_resolution::compute_resolution(store, &exec.id)?;

    // Completeness check.
    let aborted = collect_aborted_attempts(store, &exec.id)?;
    let completeness = Completeness::compute(&res, &aborted);

    if a.strict && !completeness.fully_processed {
        let total = res.input_row_count;
        let resolved_for_pct = res.counts.resolved
            + res.counts.failed_last
            + res.counts.crashed_last
            + res.counts.cancelled_last
            + res.counts.too_large;
        eprintln!("[rowforge] error: execution not fully processed");
        eprintln!(
            "  resolved: {}/{} ({:.1}%)",
            resolved_for_pct, total, completeness.completion_percent
        );
        eprintln!("  never_attempted: {}", res.counts.never_attempted);
        eprintln!("  aborted_attempts: {}", aborted.len());
        eprintln!("  Run more attempts before exporting, or drop --strict.");
        return Ok(3);
    }

    let out_dir = a.output_dir.unwrap_or_else(|| {
        exec.dir
            .join("exports")
            .join(chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string())
    });
    std::fs::create_dir_all(&out_dir)
        .with_context(|| format!("create {}", out_dir.display()))?;

    // Emit warnings (only when NOT in strict — strict already exited above).
    emit_export_warnings(&res, &aborted);

    match a.format {
        ExportFormat::Csv => {
            write_success_csv(&out_dir.join("success.csv"), &res)?;
            write_failed_csv(&out_dir.join("failed.csv"), &res)?;
        }
        ExportFormat::Jsonl => {
            write_success_jsonl(&out_dir.join("success.jsonl"), &res)?;
            write_failed_jsonl(&out_dir.join("failed.jsonl"), &res)?;
        }
        ExportFormat::Both => {
            write_success_csv(&out_dir.join("success.csv"), &res)?;
            write_failed_csv(&out_dir.join("failed.csv"), &res)?;
            write_success_jsonl(&out_dir.join("success.jsonl"), &res)?;
            write_failed_jsonl(&out_dir.join("failed.jsonl"), &res)?;
        }
    }

    write_resolution_json_with_completeness(
        &out_dir.join("resolution.json"),
        &res,
        &completeness,
    )?;

    println!("exported to {}", out_dir.display());
    println!("  resolved:        {}", res.counts.resolved);
    println!("  failed_last:     {}", res.counts.failed_last);
    println!("  crashed_last:    {}", res.counts.crashed_last);
    println!("  cancelled_last:  {}", res.counts.cancelled_last);
    println!("  too_large:       {}", res.counts.too_large);
    println!("  never_attempted: {}", res.counts.never_attempted);
    Ok(0)
}

// ---------------------------------------------------------------------------
// CSV column discovery helpers (§14.3, D12)
// ---------------------------------------------------------------------------

/// Collect the union of all handler data keys from canonical_success records,
/// excluding "seqid". Returns a BTreeSet so they are alphabetically sorted.
fn discover_success_keys(
    res: &rowforge_core::row_resolution::RowResolution,
) -> std::collections::BTreeSet<String> {
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
fn discover_failure_data_keys(
    res: &rowforge_core::row_resolution::RowResolution,
) -> std::collections::BTreeSet<String> {
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

fn write_success_csv(
    path: &std::path::Path,
    res: &rowforge_core::row_resolution::RowResolution,
) -> Result<()> {
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

fn write_failed_csv(
    path: &std::path::Path,
    res: &rowforge_core::row_resolution::RowResolution,
) -> Result<()> {
    use rowforge_core::row_resolution::ResolutionState;

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
) -> Result<()> {
    let mut parts = Vec::with_capacity(fields.len());
    for (k, v) in fields {
        let key_json = serde_json::to_string(k)?;
        let val_json = serde_json::to_string(&v)?;
        parts.push(format!("{}:{}", key_json, val_json));
    }
    writeln!(writer, "{{{}}}", parts.join(","))?;
    Ok(())
}

fn write_success_jsonl(
    path: &std::path::Path,
    res: &rowforge_core::row_resolution::RowResolution,
) -> Result<()> {
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

fn write_failed_jsonl(
    path: &std::path::Path,
    res: &rowforge_core::row_resolution::RowResolution,
) -> Result<()> {
    use rowforge_core::row_resolution::ResolutionState;

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
    path: &std::path::Path,
    res: &rowforge_core::row_resolution::RowResolution,
    comp: &Completeness,
) -> Result<()> {
    #[derive(serde::Serialize)]
    struct Summary<'a> {
        execution_id: &'a str,
        input_row_count: u64,
        counts: &'a rowforge_core::row_resolution::ResolutionCounts,
        completeness: &'a Completeness,
        merged_from_attempts: &'a [String],
        by_error_code: &'a std::collections::BTreeMap<String, u64>,
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

fn print_exec(e: &Execution) {
    println!("id:                          {}", e.id);
    println!("name:                        {}", e.name.as_deref().unwrap_or("-"));
    println!("state:                       {:?}", e.state);
    println!("dir:                         {}", e.dir.display());
    println!("input_csv_id:                {}", e.input_csv_id);
    println!("input_csv_hash:              {}", e.input_csv_hash);
    println!("input_row_count:             {}", e.input_row_count);
    println!(
        "current_handler_instance_id: {}",
        e.current_handler_instance_id.as_deref().unwrap_or("-")
    );
    println!("created_at:                  {}", e.created_at.to_rfc3339());
    if let Some(t) = e.settled_at {
        println!("settled_at:                  {}", t.to_rfc3339());
    }
    if let Some(t) = e.closed_at {
        println!("closed_at:                   {}", t.to_rfc3339());
    }
    if let Some(t) = e.abandoned_at {
        println!("abandoned_at:                {}", t.to_rfc3339());
    }
    if let Some(r) = &e.abandoned_reason {
        println!("abandoned_reason:            {}", r);
    }
}

fn rowforge_home() -> Result<PathBuf> {
    if let Ok(env) = std::env::var("ROWFORGE_HOME") {
        return Ok(PathBuf::from(env));
    }
    let h = dirs::home_dir().ok_or_else(|| anyhow!("no home dir"))?;
    Ok(h.join(".rowforge"))
}
