use anyhow::Context;
use clap::Args;
use rowforge_core::csv_io::FieldMap;
use rowforge_core::rerun::{looks_like_failed_csv, prepare_rerun_input};
use rowforge_core::run::{execute, RunRequest};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Args, Debug)]
pub struct RunArgs {
    /// Handler directory (containing rowforge.yaml)
    #[arg(long)]
    pub handler: PathBuf,

    /// Input CSV path
    #[arg(long)]
    pub input: PathBuf,

    /// Output directory (will be created)
    #[arg(long)]
    pub output_dir: PathBuf,

    /// Worker pool size
    #[arg(long, default_value_t = 1)]
    pub workers: u32,

    /// Only process first --dry-run-sample rows; sets meta.dry_run = true
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,

    /// Sample size for dry run
    #[arg(long, default_value_t = 10)]
    pub dry_run_sample: usize,

    /// Override field mapping: schema_field=csv_column. Repeatable.
    #[arg(long = "field-map", value_parser = parse_kv)]
    pub field_map: Vec<(String, String)>,

    /// Override handler config: key=json_value. Repeatable.
    #[arg(long = "config", value_parser = parse_kv)]
    pub config: Vec<(String, String)>,

    /// Suppress progress; print summary only
    #[arg(long, default_value_t = false)]
    pub quiet: bool,

    /// Print events as JSON-Lines on stdout
    #[arg(long, default_value_t = false)]
    pub json_events: bool,

    /// When input is a previous failed.csv, include WORKER_CRASH rows
    #[arg(long, default_value_t = false)]
    pub include_crash_rows: bool,

    /// Mark this run as a re-run of a parent run id (recorded in meta.json)
    #[arg(long)]
    pub parent_run_id: Option<String>,
}

fn parse_kv(s: &str) -> Result<(String, String), String> {
    let (k, v) = s
        .split_once('=')
        .ok_or_else(|| format!("expected K=V, got '{}'", s))?;
    Ok((k.to_string(), v.to_string()))
}

pub async fn run(args: RunArgs) -> anyhow::Result<i32> {
    // Detect re-run input: if input is a failed.csv, filter it.
    let _rerun_temp; // keeps tempfile alive until end of function
    let effective_input = if looks_like_failed_csv(&args.input)? {
        let tmp = prepare_rerun_input(&args.input, args.include_crash_rows)
            .context("preparing rerun input")?;
        let path = tmp.path().to_path_buf();
        _rerun_temp = Some(tmp);
        eprintln!(
            "[rowforge] input detected as failed.csv; filtered for rerun (include_crash_rows={})",
            args.include_crash_rows
        );
        path
    } else {
        _rerun_temp = None;
        args.input.clone()
    };

    let mut field_map = FieldMap::new();
    for (k, v) in args.field_map {
        field_map.insert(k, v);
    }

    let mut config = BTreeMap::new();
    for (k, v) in args.config {
        // try parse as JSON; fallback to string
        let val: serde_json::Value =
            serde_json::from_str(&v).unwrap_or_else(|_| serde_json::Value::String(v.clone()));
        config.insert(k, val);
    }

    let json_events = args.json_events;
    let quiet = args.quiet;
    let cb: Option<rowforge_core::run::ProgressCallback> = if quiet {
        None
    } else if json_events {
        Some(Box::new(|ev| {
            let s = match ev {
                rowforge_core::run::RunProgressEvent::Started { total_rows } => {
                    serde_json::json!({"type":"started","total_rows":total_rows})
                }
                rowforge_core::run::RunProgressEvent::RowDone { seq, success } => {
                    serde_json::json!({"type":"row_done","seq":seq,"success":success})
                }
                rowforge_core::run::RunProgressEvent::Completed { success, failed } => {
                    serde_json::json!({"type":"completed","success":success,"failed":failed})
                }
            };
            println!("{}", s);
        }))
    } else {
        Some(Box::new(|ev| {
            if let rowforge_core::run::RunProgressEvent::Completed { success, failed } = ev {
                eprintln!("[rowforge] progress: {} OK / {} failed", success, failed);
            }
        }))
    };

    let req = RunRequest {
        run_id: chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string(),
        parent_run_id: args.parent_run_id,
        handler_dir: args.handler.clone(),
        input_csv: effective_input,
        output_dir: args.output_dir.clone(),
        workers: args.workers,
        dry_run: args.dry_run,
        dry_run_sample: args.dry_run_sample,
        row_limit: None,
        skip_seqs: std::collections::HashSet::new(),
        field_map,
        config_overrides: config,
        shutdown_grace: Duration::from_secs(5),
        on_progress: cb,
        cancel: None,
        input_format: None,
        fsync_outcomes: false,
    };
    let report = execute(req).await?;

    // Exit codes per spec §5.5:
    //   2 = run aborted (no handler ever produced a row; STARTUP_FAILED)
    //   1 = at least one row failed at handler level
    //   0 = everything succeeded
    if report.aborted {
        Ok(2)
    } else if report.failed_count > 0 {
        Ok(1)
    } else {
        Ok(0)
    }
}
