use clap::{Parser, Subcommand};

use rowforge_cli::pack_cmd;
mod exec_cmd;
mod handler_build_cmd;
mod run_cmd;

#[derive(Parser)]
#[command(name = "rowforge", version)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run a handler against a CSV (headless mode).
    Run(run_cmd::RunArgs),
    /// Cross-compile rowforge + bundle handlers into a zip.
    Pack(pack_cmd::PackArgs),
    /// Manage executions (execution-centric model).
    Exec(exec_cmd::ExecArgs),
    /// Operate on handlers (build, validate, etc.).
    Handler {
        #[command(subcommand)]
        action: HandlerCmd,
    },
}

#[derive(Subcommand)]
enum HandlerCmd {
    /// Build one or all handlers under <workspace>/handlers/.
    Build {
        /// Handler name (omit to build all).
        name: Option<String>,
        /// Force rebuild even when not stale.
        #[arg(long)]
        force: bool,
    },
}

#[tokio::main]
async fn main() {
    // Default to INFO level so users see basic per-attempt progress without
    // needing to set RUST_LOG. Power users can still override (e.g.
    // `RUST_LOG=rowforge_core=debug` for per-row tracing).
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .init();
    let cli = Cli::parse();
    let exit = match cli.cmd {
        Cmd::Run(args) => match run_cmd::run(args).await {
            Ok(code) => code,
            Err(e) => {
                eprintln!("[rowforge] error: {:#}", e);
                map_error_to_exit_code(&e)
            }
        },
        Cmd::Pack(args) => match pack_cmd::run(args).await {
            Ok(code) => code,
            Err(e) => {
                eprintln!("[rowforge] error: {:#}", e);
                3
            }
        },
        Cmd::Exec(args) => match exec_cmd::run(args).await {
            Ok(code) => code,
            Err(e) => {
                eprintln!("[rowforge] error: {:#}", e);
                3
            }
        },
        Cmd::Handler { action } => {
            let workspace = match rowforge_workspace() {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("[rowforge] error: {:#}", e);
                    std::process::exit(3);
                }
            };
            match action {
                HandlerCmd::Build { name, force } => {
                    match handler_build_cmd::run(&workspace, name, force) {
                        Ok(code) => code,
                        Err(e) => {
                            eprintln!("[rowforge] error: {:#}", e);
                            3
                        }
                    }
                }
            }
        }
    };
    std::process::exit(exit);
}

fn rowforge_workspace() -> anyhow::Result<std::path::PathBuf> {
    if let Ok(env) = std::env::var("ROWFORGE_HOME") {
        return Ok(std::path::PathBuf::from(env));
    }
    rowforge_core::workspace::default_workspace_root()
        .ok_or_else(|| anyhow::anyhow!("no home dir"))
}

fn map_error_to_exit_code(err: &anyhow::Error) -> i32 {
    let s = format!("{:#}", err).to_lowercase();
    if s.contains("startup timeout") || s.contains("all workers") {
        2 // run abort
    } else {
        3 // arg/config error
    }
}
