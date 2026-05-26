//! Tauri commands wrapping the `StudioCore` surface.
//!
//! Every command returns `Result<T, UiError>`; the structured error is
//! serialized to JSON for the React layer to classify by `kind` (spec
//! §5.3 / §5.5).

use std::path::PathBuf;

use rowforge_studio_core::{
    AttemptDetail, AttemptId, BuildOutcome, CancelMode, ExecDeleteBulkResult, ExecDetail,
    ExecRollup, ExecSummary, ExecutionId, ExportOpts, ExportReport, FailedPageQuery, FailedRowPage,
    HandlerDetail, HandlerLogLine, HandlerSummary, ListFilter, ManifestReport, ManifestSource,
    OpenOpts, ProgressSnapshot, RowHistory, RunHandle, RunOpts, RunStartedHandle, RunStatus,
    ScaffoldArgs, Settings, StartExecArgs, StudioCore, UiError, Workspace,
};
use tauri::Emitter;
use tauri::State;

use crate::settings as settings_io;
use crate::state::AppState;

#[tauri::command]
pub fn workspace_open(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    path: Option<PathBuf>,
) -> Result<Workspace, UiError> {
    // Plan 6 T9: read Settings first so we can size SessionRegistry from
    // max_concurrent_runs. Loaded settings may be defaulted (file not
    // present yet — that's fine).
    let prev = settings_io::load(&app)?;
    let opts = match path {
        Some(p) => OpenOpts::new().with_workspace(p),
        None => OpenOpts::new(),
    };
    // Plan 6 T9: size SessionRegistry from Settings.max_concurrent_runs.
    // Plan 7 T15: seed StudioCore.preferred_editor from Settings.preferred_editor.
    // Plan 9 T5: seed capture_raw_stdout from Settings.handler_log_capture_raw_stdout.
    let opts = opts
        .with_max_concurrent_runs(prev.max_concurrent_runs)
        .with_preferred_editor(prev.preferred_editor.clone())
        .with_handler_log_capture_raw_stdout(prev.handler_log_capture_raw_stdout)
        .with_smoke_default_rows(prev.smoke_default_rows)
        .with_smoke_timeout_per_row_secs(prev.smoke_timeout_per_row_secs);
    let core = StudioCore::open(opts)?;
    let workspace = core.workspace().clone();

    // Persist the chosen path to settings so next boot autoloads.
    // Reuse the loaded `prev` — single load instead of two.
    let mut s = prev;
    s.workspace_root = Some(workspace.root.clone());
    settings_io::save(&app, &s)?;

    let sessions = core.sessions();
    *state.core.lock().unwrap_or_else(|p| p.into_inner()) = Some(std::sync::Arc::new(core));

    // Spawn the 1 Hz workspace rollup forwarder for this workspace session.
    // If a prior forwarder is alive (user switched workspaces), abort it
    // first so we don't leak forwarders emitting from stale registries.
    let app_clone = app.clone();
    let new_task = tauri::async_runtime::spawn(async move {
        crate::events::forward_active_runs(app_clone, sessions).await;
    });
    let mut task_slot = state
        .active_runs_task
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if let Some(prior) = task_slot.replace(new_task) {
        prior.abort();
    }

    Ok(workspace)
}

#[tauri::command]
pub fn exec_list(state: State<'_, AppState>) -> Result<Vec<ExecSummary>, UiError> {
    let core = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        guard.as_ref().ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?.clone()
    };
    core.list(ListFilter::default())
}

#[tauri::command]
pub fn workspace_settings_load(app: tauri::AppHandle) -> Result<Settings, UiError> {
    settings_io::load(&app)
}

#[tauri::command]
pub fn workspace_settings_save(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    settings: Settings,
) -> Result<(), UiError> {
    settings_io::save(&app, &settings)?;
    // Plan 7 T15: refresh preferred_editor in the live StudioCore so the
    // next handler_open_editor call uses the new value without requiring a
    // workspace re-open.
    // Plan 9 T5: refresh capture_raw_stdout so the next start_run picks it up.
    let core = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        guard.as_ref().cloned()
    };
    if let Some(core) = core {
        core.set_preferred_editor(settings.preferred_editor.clone());
        core.set_handler_log_capture_raw_stdout(settings.handler_log_capture_raw_stdout);
        core.set_smoke_defaults(
            settings.smoke_default_rows,
            settings.smoke_timeout_per_row_secs,
        );
    }
    Ok(())
}

/// Returns the currently-open workspace, if any. None means no workspace
/// is open yet (BootGate hasn't completed autoload or user hasn't picked).
#[tauri::command]
pub fn workspace_current(
    state: State<'_, AppState>,
) -> Result<Option<Workspace>, UiError> {
    let core = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        guard.as_ref().cloned()
    };
    Ok(core.map(|c| c.workspace().clone()))
}

#[tauri::command]
pub fn exec_show(
    state: State<'_, AppState>,
    id: ExecutionId,
) -> Result<ExecDetail, UiError> {
    let core = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        guard.as_ref().ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?.clone()
    };
    core.show(&id)
}

#[tauri::command]
pub fn attempt_show(
    state: State<'_, AppState>,
    execution_id: ExecutionId,
    attempt_id: AttemptId,
) -> Result<AttemptDetail, UiError> {
    let core = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        guard.as_ref().ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?.clone()
    };
    core.attempt(&execution_id, &attempt_id)
}

#[tauri::command]
pub fn exec_rollup(
    state: State<'_, AppState>,
    id: ExecutionId,
) -> Result<ExecRollup, UiError> {
    let core = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        guard.as_ref().ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?.clone()
    };
    core.rollup(&id)
}

#[tauri::command]
pub fn attempt_failed_page(
    state: State<'_, AppState>,
    query: FailedPageQuery,
) -> Result<FailedRowPage, UiError> {
    let core = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        guard.as_ref().ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?.clone()
    };
    core.failed_page(query)
}

#[tauri::command]
pub fn attempt_row_history(
    state: State<'_, AppState>,
    execution_id: ExecutionId,
    seq: u64,
) -> Result<RowHistory, UiError> {
    let core = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        guard.as_ref().ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?.clone()
    };
    core.row_history(&execution_id, seq)
}

#[tauri::command]
pub async fn run_start(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    execution_id: ExecutionId,
    handler_dir: PathBuf,
    row_limit: Option<u64>,
    workers: Option<u32>,
    dry_run: Option<bool>,
    skip_attempted: Option<bool>,
    only_row_ids: Option<Vec<u64>>,
) -> Result<RunStartedHandle, UiError> {
    // Clone the Arc out of the mutex so it is not held across .await points.
    // studio-core::start_run internally calls tokio::spawn (tick loop +
    // pipeline task); those spawns require an entered tokio runtime.
    // Making this command async ensures Tauri executes it on its tokio
    // runtime, so the inner spawn calls have a runtime context.
    let core = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        guard
            .as_ref()
            .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?
            .clone()
    };
    let (started, stream_rx) = {
        let mut opts = RunOpts::new(handler_dir);
        if let Some(n) = row_limit {
            opts = opts.with_row_limit(n);
        }
        if let Some(w) = workers {
            opts = opts.with_workers(w);
        }
        if let Some(d) = dry_run {
            opts = opts.with_dry_run(d);
        }
        if let Some(s) = skip_attempted {
            opts = opts.with_skip_attempted(s);
        }
        opts = opts.with_only_row_ids(only_row_ids);
        let started = core.start_run(&execution_id, opts)?;
        let stream = core
            .subscribe(&started.handle)
            .map_err(|e| UiError::Internal(e.to_string()))?;
        (started, stream.rx)
    };

    // Spawn the per-run event forwarder onto run:<handle>.
    let handle_for_task = started.handle.clone();
    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        crate::events::forward_run_events(app_clone, handle_for_task, stream_rx).await;
    });

    Ok(started)
}

#[tauri::command]
pub fn run_cancel(
    state: State<'_, AppState>,
    handle: RunHandle,
    mode: CancelMode,
) -> Result<(), UiError> {
    let core = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        guard.as_ref().ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?.clone()
    };
    core.cancel(&handle, mode)
}

#[tauri::command]
pub fn run_status(
    state: State<'_, AppState>,
    handle: RunHandle,
) -> Result<RunStatus, UiError> {
    let core = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        guard.as_ref().ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?.clone()
    };
    core.status(&handle)
}

#[tauri::command]
pub fn run_active(
    state: State<'_, AppState>,
) -> Result<Vec<RunHandle>, UiError> {
    let core = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        guard.as_ref().ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?.clone()
    };
    Ok(core.active_runs())
}

/// Return the live `RunHandle` for an attempt if one is currently
/// running, or `None` otherwise. AttemptDetail uses this on mount to
/// offer "Watch live" when the user navigates in without `?run=` in
/// the URL (e.g. coming from the executions list rather than the Run
/// button's auto-navigate).
#[tauri::command]
pub fn attempt_active_handle(
    state: State<'_, AppState>,
    attempt_id: AttemptId,
) -> Result<Option<RunHandle>, UiError> {
    let core = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        guard
            .as_ref()
            .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?
            .clone()
    };
    Ok(core.active_handle_for_attempt(attempt_id.as_str()))
}

/// Snapshot of an active run's counters. Used by the UI to bootstrap
/// state when subscribing to a run that's already in flight — Tauri
/// events don't queue, so events emitted before `listen()` attaches
/// are lost; this command fills them back in.
#[tauri::command]
pub fn run_snapshot(
    state: State<'_, AppState>,
    handle: RunHandle,
) -> Result<ProgressSnapshot, UiError> {
    let core = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        guard
            .as_ref()
            .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?
            .clone()
    };
    core.snapshot(&handle)
}

#[tauri::command]
pub fn exec_start(
    state: State<'_, AppState>,
    args: StartExecArgs,
) -> Result<ExecutionId, UiError> {
    let core = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        guard
            .as_ref()
            .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?
            .clone()
    };
    core.start_exec(args)
}

#[tauri::command]
pub async fn exec_export(
    state: State<'_, AppState>,
    id: ExecutionId,
    opts: ExportOpts,
) -> Result<ExportReport, UiError> {
    // Clone the Arc out of the mutex — no .await happens inside. We make this
    // command async so Tauri schedules it on a worker thread, since
    // export_execution does meaningful sync IO that would otherwise block
    // the IPC main thread for seconds-to-minutes on large execs.
    let core = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        guard
            .as_ref()
            .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?
            .clone()
    };
    core.export(&id, opts)
}

#[tauri::command]
pub fn manifest_validate(
    state: State<'_, AppState>,
    source: ManifestSource,
) -> Result<ManifestReport, UiError> {
    let core = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        guard
            .as_ref()
            .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?
            .clone()
    };
    core.validate_manifest(source)
}

// ===== Plan 7 handler authoring =====

#[tauri::command]
pub fn handler_list(state: State<'_, AppState>) -> Result<Vec<HandlerSummary>, UiError> {
    let core = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        guard
            .as_ref()
            .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?
            .clone()
    };
    core.handler_list()
}

#[tauri::command]
pub fn handler_show(
    state: State<'_, AppState>,
    name: String,
) -> Result<HandlerDetail, UiError> {
    let core = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        guard
            .as_ref()
            .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?
            .clone()
    };
    core.handler_show(&name)
}

#[tauri::command]
pub fn handler_open_editor(
    state: State<'_, AppState>,
    name: String,
) -> Result<(), UiError> {
    let core = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        guard
            .as_ref()
            .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?
            .clone()
    };
    core.handler_open_editor(&name)
}

#[tauri::command]
pub fn handler_reveal(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    name: String,
) -> Result<(), UiError> {
    // studio-core returns the path; we wrap with shell::open at the layer
    // boundary so studio-core stays OS-policy-free.
    let path = {
        let core = {
            let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
            guard
                .as_ref()
                .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?
                .clone()
        };
        core.handler_reveal_path(&name)?
    };
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_path(path.to_string_lossy().to_string(), None::<String>)
        .map_err(|e| UiError::Io(e.to_string()))?;
    Ok(())
}

#[tauri::command]
pub fn handler_scaffold(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    args: ScaffoldArgs,
) -> Result<String, UiError> {
    let name = {
        let core = {
            let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
            guard
                .as_ref()
                .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?
                .clone()
        };
        core.handler_scaffold(args)?
    };
    // Spec §8.5.2 — coarse refresh hint after mutation.
    let _ = app.emit("handlers:list", ());
    Ok(name)
}

#[tauri::command]
pub fn handler_delete(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    name: String,
) -> Result<(), UiError> {
    let core = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        guard
            .as_ref()
            .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?
            .clone()
    };
    core.handler_delete(&name)?;
    let _ = app.emit("handlers:list", ());
    Ok(())
}

#[tauri::command]
pub fn handler_rename(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    old: String,
    new: String,
) -> Result<(), UiError> {
    let core = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        guard
            .as_ref()
            .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?
            .clone()
    };
    core.handler_rename(&old, &new)?;
    let _ = app.emit("handlers:list", ());
    Ok(())
}

// ===== Plan 8 handler build =====

#[tauri::command]
pub async fn handler_build(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    name: String,
) -> Result<BuildOutcome, UiError> {
    // handler_build shells out to an external build tool (go build, cargo, …)
    // which can take seconds. We make the command async so Tauri dispatches it
    // on its worker-thread pool and the main IPC reactor stays free.
    // Clone the Arc out before calling — no .await inside, so no guard-across-await risk.
    let core = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        guard
            .as_ref()
            .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?
            .clone()
    };
    let result = core.handler_build(&name);

    // Emit list-refresh hint so HandlerSummary.last_modified (which folds
    // over top-level entries, including the new binary) gets picked up by the
    // UI after a successful (or failed-but-cached) build.
    let _ = app.emit("handlers:list", ());

    result
}

// ===== Plan 9 T6 — handler log commands =====

#[tauri::command]
pub fn handler_log_tail(
    state: State<'_, AppState>,
    exec_id: String,
    attempt_id: String,
    max_lines: Option<usize>,
) -> Result<Vec<HandlerLogLine>, UiError> {
    let core = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        guard
            .as_ref()
            .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?
            .clone()
    };
    core.handler_log_tail(&exec_id, &attempt_id, max_lines.unwrap_or(5000))
}

#[tauri::command]
pub async fn handler_log_subscribe(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    exec_id: String,
    attempt_id: String,
) -> Result<(), UiError> {
    let _ = exec_id; // not currently needed by subscribe; included for future symmetry
    let rx_result = {
        let core = {
            let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
            guard
                .as_ref()
                .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?
                .clone()
        };
        core.handler_log_subscribe(&attempt_id)
    };
    let mut rx = rx_result?;
    let event_name = format!("handler_log:{}", attempt_id);

    // Cancel any stale subscription for this attempt before starting a new one
    // to avoid orphaned pump tasks.
    if let Some((_, old)) = state.handler_log_cancels.remove(&attempt_id) {
        old.cancel();
    }

    // Track the new task so unsubscribe can stop it.
    let cancel_token = tokio_util::sync::CancellationToken::new();
    state
        .handler_log_cancels
        .insert(attempt_id.clone(), cancel_token.clone());

    let app_clone = app.clone();
    tokio::spawn(async move {
        use tauri::Emitter;
        use tokio::sync::broadcast::error::RecvError;
        let mut batch = Vec::<HandlerLogLine>::with_capacity(64);
        let mut dropped: u64 = 0;
        let mut interval =
            tokio::time::interval(std::time::Duration::from_millis(100));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = cancel_token.cancelled() => break,
                msg = rx.recv() => match msg {
                    Ok(line) => {
                        batch.push(line);
                        if batch.len() >= 64 {
                            let payload = serde_json::json!({
                                "lines": &batch,
                                "dropped": dropped,
                            });
                            let _ = app_clone.emit(&event_name, payload);
                            batch.clear();
                            dropped = 0;
                        }
                    }
                    Err(RecvError::Lagged(n)) => { dropped += n; }
                    Err(RecvError::Closed) => break,
                },
                _ = interval.tick() => {
                    if !batch.is_empty() || dropped > 0 {
                        let payload = serde_json::json!({
                            "lines": &batch,
                            "dropped": dropped,
                        });
                        let _ = app_clone.emit(&event_name, payload);
                        batch.clear();
                        dropped = 0;
                    }
                },
            }
        }
    });

    Ok(())
}

#[tauri::command]
pub fn handler_log_unsubscribe(
    state: State<'_, AppState>,
    attempt_id: String,
) -> Result<(), UiError> {
    if let Some((_, token)) = state.handler_log_cancels.remove(&attempt_id) {
        token.cancel();
    }
    Ok(())
}

// ===== Plan 11 — re-run failed rows =====

/// Return the seq values of rows that failed in a specific attempt.
/// Used by the Re-run failed flow to seed `only_row_ids` on the next
/// `run_start` call.
#[tauri::command]
pub fn attempt_failed_row_ids(
    state: State<'_, AppState>,
    exec_id: String,
    attempt_id: String,
) -> Result<Vec<u64>, UiError> {
    let core = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        guard
            .as_ref()
            .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?
            .clone()
    };
    core.attempt_failed_row_ids(&exec_id, &attempt_id)
}

// ===== Plan 12 — handler import + fork =====

#[tauri::command]
pub fn handler_import_from_folder(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    source_path: String,
    name: String,
) -> Result<(), UiError> {
    let core = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        guard
            .as_ref()
            .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?
            .clone()
    };
    let result = core.handler_import_from_folder(std::path::Path::new(&source_path), &name);
    if result.is_ok() {
        use tauri::Emitter;
        let _ = app.emit("handlers:list", ());
    }
    result
}

#[tauri::command]
pub fn handler_fork(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    source_name: String,
    new_name: String,
) -> Result<(), UiError> {
    let core = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        guard
            .as_ref()
            .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?
            .clone()
    };
    let result = core.handler_fork(&source_name, &new_name);
    if result.is_ok() {
        use tauri::Emitter;
        let _ = app.emit("handlers:list", ());
    }
    result
}

// ===== Plan 10 — execution delete commands =====

#[tauri::command]
pub fn execution_delete(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    exec_id: String,
) -> Result<(), UiError> {
    let core = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        guard
            .as_ref()
            .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?
            .clone()
    };
    let result = core.execution_delete(&exec_id);
    if result.is_ok() {
        let _ = app.emit("exec_list:refresh", ());
    }
    result
}

#[tauri::command]
pub fn execution_delete_bulk(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    exec_ids: Vec<String>,
) -> Result<ExecDeleteBulkResult, UiError> {
    let core = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        guard
            .as_ref()
            .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?
            .clone()
    };
    let result = core.execution_delete_bulk(&exec_ids);
    if !result.deleted.is_empty() {
        let _ = app.emit("exec_list:refresh", ());
    }
    Ok(result)
}

// ===== Plan 13 — handler smoke test =====

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

#[tauri::command]
pub fn handler_smoke_load_fixtures(
    state: State<'_, AppState>,
    path: String,
    limit: usize,
) -> Result<Vec<serde_json::Map<String, serde_json::Value>>, UiError> {
    let core = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        guard
            .as_ref()
            .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?
            .clone()
    };
    core.handler_smoke_load_fixtures(std::path::Path::new(&path), limit)
}

