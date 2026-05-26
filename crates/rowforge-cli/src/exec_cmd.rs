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
use rowforge_core::build::{needs_build, run_build, BuildError};
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
    /// Delete an execution and all its attempt data (sqlite + dir).
    Delete {
        /// Execution id (mutually exclusive with --all-completed).
        exec_id: Option<String>,
        /// Delete every execution that has no active run.
        #[arg(long, conflicts_with = "exec_id")]
        all_completed: bool,
    },
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
        ExecCmd::Export(a) => do_export(&store, a),
        // Delete is intercepted in main.rs before reaching this function.
        ExecCmd::Delete { .. } => unreachable!("Delete is handled in main before exec_cmd::run"),
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

    // Auto-build gate: build (or rebuild) the handler binary when stale.
    if needs_build(&handler_canon, &manifest) {
        eprintln!("[rowforge] building {} ...", manifest.name);
        match run_build(&handler_canon, &manifest) {
            Ok(outcome) => {
                let dur = outcome
                    .finished_at
                    .signed_duration_since(outcome.started_at)
                    .num_milliseconds();
                eprintln!("[rowforge] build ok ({} ms)", dur);
            }
            Err(BuildError::BuildFailed { exit_code, outcome, .. }) => {
                eprintln!("[rowforge] build failed (exit {}):", exit_code);
                if !outcome.stdout.is_empty() {
                    eprint!("{}", outcome.stdout);
                }
                if !outcome.stderr.is_empty() {
                    eprint!("{}", outcome.stderr);
                }
                std::process::exit(2);
            }
            Err(BuildError::ToolchainMissing { tool }) => {
                eprintln!("[rowforge] build tool '{}' not found in PATH", tool);
                std::process::exit(2);
            }
            Err(BuildError::NoBuildCommand) => {
                unreachable!("needs_build returned true but manifest has no build command");
            }
            Err(BuildError::Io(e)) => {
                eprintln!("[rowforge] build io error: {}", e);
                std::process::exit(2);
            }
        }
    }

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
        on_progress: Some(std::sync::Arc::new({
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
        on_handler_log: None,
        cancel: None,
        input_format: None,
        fsync_outcomes: a.fsync_outcomes,
        capture_raw_stdout: false,
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
// Main export entry point (thin CLI wrapper)
// ---------------------------------------------------------------------------

fn do_export(store: &ExecutionStore, a: ExportArgs) -> Result<i32> {
    use rowforge_core::export::{ExportFormat as F, ExportOpts, export_execution};
    let format = match a.format {
        ExportFormat::Csv => F::Csv,
        ExportFormat::Jsonl => F::Jsonl,
        ExportFormat::Both => F::Both,
    };
    let mut opts = ExportOpts::new(format).with_require_complete(a.strict);
    if let Some(dir) = a.output_dir {
        opts = opts.with_output_dir(dir);
    }
    match export_execution(store, &a.exec_id, &opts) {
        Ok(report) => {
            println!(
                "exported {} success / {} failed rows to {}",
                report.success_count,
                report.failed_count,
                report.output_dir.display()
            );
            for w in &report.warnings {
                eprintln!("warning [{}]: {}", w.code, w.message);
            }
            Ok(0)
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("export_incomplete:") {
                eprintln!("error: execution not fully processed");
                Ok(3)
            } else {
                Err(e.into())
            }
        }
    }
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
    rowforge_core::workspace::default_workspace_root()
        .ok_or_else(|| anyhow!("no home dir"))
}
