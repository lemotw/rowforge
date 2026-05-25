//! Tauri commands wrapping the `StudioCore` surface.
//!
//! Every command returns `Result<T, UiError>`; the structured error is
//! serialized to JSON for the React layer to classify by `kind` (spec
//! §5.3 / §5.5).

use std::path::PathBuf;

use rowforge_studio_core::{
    AttemptDetail, AttemptId, CancelMode, ExecDetail, ExecRollup, ExecSummary, ExecutionId,
    ExportOpts, ExportReport, FailedPageQuery, FailedRowPage, ListFilter, ManifestReport,
    ManifestSource, OpenOpts, ProgressSnapshot, RowHistory, RunHandle, RunOpts,
    RunStartedHandle, RunStatus, Settings, StartExecArgs, StudioCore, UiError, Workspace,
};
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
    let opts = opts.with_max_concurrent_runs(prev.max_concurrent_runs);
    let core = StudioCore::open(opts)?;
    let workspace = core.workspace().clone();

    // Persist the chosen path to settings so next boot autoloads.
    // Reuse the loaded `prev` — single load instead of two.
    let mut s = prev;
    s.workspace_root = Some(workspace.root.clone());
    settings_io::save(&app, &s)?;

    let sessions = core.sessions();
    *state.core.lock().unwrap_or_else(|p| p.into_inner()) = Some(core);

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
    let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
    let core = guard
        .as_ref()
        .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
    core.list(ListFilter::default())
}

#[tauri::command]
pub fn workspace_settings_load(app: tauri::AppHandle) -> Result<Settings, UiError> {
    settings_io::load(&app)
}

#[tauri::command]
pub fn workspace_settings_save(
    app: tauri::AppHandle,
    settings: Settings,
) -> Result<(), UiError> {
    settings_io::save(&app, &settings)
}

/// Returns the currently-open workspace, if any. None means no workspace
/// is open yet (BootGate hasn't completed autoload or user hasn't picked).
#[tauri::command]
pub fn workspace_current(
    state: State<'_, AppState>,
) -> Result<Option<Workspace>, UiError> {
    let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
    Ok(guard.as_ref().map(|c| c.workspace().clone()))
}

#[tauri::command]
pub fn exec_show(
    state: State<'_, AppState>,
    id: ExecutionId,
) -> Result<ExecDetail, UiError> {
    let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
    let core = guard.as_ref().ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
    core.show(&id)
}

#[tauri::command]
pub fn attempt_show(
    state: State<'_, AppState>,
    execution_id: ExecutionId,
    attempt_id: AttemptId,
) -> Result<AttemptDetail, UiError> {
    let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
    let core = guard.as_ref().ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
    core.attempt(&execution_id, &attempt_id)
}

#[tauri::command]
pub fn exec_rollup(
    state: State<'_, AppState>,
    id: ExecutionId,
) -> Result<ExecRollup, UiError> {
    let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
    let core = guard.as_ref().ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
    core.rollup(&id)
}

#[tauri::command]
pub fn attempt_failed_page(
    state: State<'_, AppState>,
    query: FailedPageQuery,
) -> Result<FailedRowPage, UiError> {
    let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
    let core = guard.as_ref().ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
    core.failed_page(query)
}

#[tauri::command]
pub fn attempt_row_history(
    state: State<'_, AppState>,
    execution_id: ExecutionId,
    seq: u64,
) -> Result<RowHistory, UiError> {
    let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
    let core = guard.as_ref().ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
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
) -> Result<RunStartedHandle, UiError> {
    // Scope the MutexGuard so it is dropped before any .await point.
    // studio-core::start_run internally calls tokio::spawn (tick loop +
    // pipeline task); those spawns require an entered tokio runtime.
    // Making this command async ensures Tauri executes it on its tokio
    // runtime, so the inner spawn calls have a runtime context.
    let (started, stream_rx) = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        let core = guard
            .as_ref()
            .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
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
        let started = core.start_run(&execution_id, opts)?;
        let stream = core
            .subscribe(&started.handle)
            .map_err(|e| UiError::Internal(e.to_string()))?;
        (started, stream.rx)
    }; // guard dropped here, before any .await

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
    let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
    let core = guard.as_ref().ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
    core.cancel(&handle, mode)
}

#[tauri::command]
pub fn run_status(
    state: State<'_, AppState>,
    handle: RunHandle,
) -> Result<RunStatus, UiError> {
    let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
    let core = guard.as_ref().ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
    core.status(&handle)
}

#[tauri::command]
pub fn run_active(
    state: State<'_, AppState>,
) -> Result<Vec<RunHandle>, UiError> {
    let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
    let core = guard.as_ref().ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
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
    let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
    let core = guard
        .as_ref()
        .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
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
    let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
    let core = guard
        .as_ref()
        .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
    core.snapshot(&handle)
}

#[tauri::command]
pub fn exec_start(
    state: State<'_, AppState>,
    args: StartExecArgs,
) -> Result<ExecutionId, UiError> {
    let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
    let core = guard
        .as_ref()
        .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
    core.start_exec(args)
}

#[tauri::command]
pub async fn exec_export(
    state: State<'_, AppState>,
    id: ExecutionId,
    opts: ExportOpts,
) -> Result<ExportReport, UiError> {
    // Scope the guard tightly — no .await happens inside. We make this
    // command async so Tauri schedules it on a worker thread, since
    // export_execution does meaningful sync IO that would otherwise block
    // the IPC main thread for seconds-to-minutes on large execs.
    let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
    let core = guard
        .as_ref()
        .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
    core.export(&id, opts)
}

#[tauri::command]
pub fn manifest_validate(
    state: State<'_, AppState>,
    source: ManifestSource,
) -> Result<ManifestReport, UiError> {
    let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
    let core = guard
        .as_ref()
        .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
    core.validate_manifest(source)
}

