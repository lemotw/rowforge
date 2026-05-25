//! `rowforge exec delete` — hard-delete one or all completed executions.
//!
//! Usage:
//!   exec delete <exec_id>
//!   exec delete --all-completed
//!
//! Exit code = number of per-item failures, capped at 125.

use std::path::Path;
use anyhow::Context;

pub fn run(workspace: &Path, exec_id: Option<String>, all_completed: bool) -> anyhow::Result<i32> {
    let core = open_studio_core(workspace)?;

    let targets: Vec<String> = if all_completed {
        let list = core.list(rowforge_studio_core::ListFilter::default())
            .map_err(|e| anyhow::anyhow!("exec list failed: {}", e))?;
        list.into_iter().map(|s| s.id.as_str().to_string()).collect()
    } else {
        vec![exec_id.context("provide an exec_id or --all-completed")?]
    };

    if targets.is_empty() {
        eprintln!("[rowforge] no executions to delete");
        return Ok(0);
    }

    let result = core.execution_delete_bulk(&targets);
    for id in &result.deleted {
        eprintln!("[{}] deleted", id);
    }
    for f in &result.failed {
        eprintln!("[{}] skipped: {}", f.exec_id, f.reason);
    }
    let code = (result.failed.len() as i32).min(125);
    Ok(code)
}

fn open_studio_core(workspace: &Path) -> anyhow::Result<rowforge_studio_core::StudioCore> {
    rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(workspace.to_path_buf()),
    )
    .map_err(|e| anyhow::anyhow!("open studio core: {}", e))
}
