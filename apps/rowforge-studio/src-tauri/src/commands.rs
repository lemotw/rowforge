//! Tauri commands wrapping the `StudioCore` surface.
//!
//! Every command returns `Result<T, UiError>`; the structured error is
//! serialized to JSON for the React layer to classify by `kind` (spec
//! §5.3 / §5.5).

use std::path::PathBuf;

use rowforge_studio_core::{
    AttemptDetail, AttemptId, CancelMode, ExecDetail, ExecRollup, ExecSummary, ExecutionId,
    FailedPageQuery, FailedRowPage, ListFilter, OpenOpts, RowHistory, RunHandle, RunOpts,
    RunStatus, Settings, StudioCore, UiError, Workspace,
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
    let opts = match path {
        Some(p) => OpenOpts::new().with_workspace(p),
        None => OpenOpts::new(),
    };
    let core = StudioCore::open(opts)?;
    let workspace = core.workspace().clone();

    // Persist the chosen path to settings so next boot autoloads.
    let mut s = settings_io::load(&app)?;
    s.workspace_root = Some(workspace.root.clone());
    settings_io::save(&app, &s)?;

    *state.core.lock().unwrap_or_else(|p| p.into_inner()) = Some(core);
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
pub fn run_start(
    state: State<'_, AppState>,
    execution_id: ExecutionId,
    handler_dir: PathBuf,
) -> Result<RunHandle, UiError> {
    let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
    let core = guard.as_ref().ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
    let opts = RunOpts::new(handler_dir);
    core.start_run(&execution_id, opts)
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

/// Replay command — returns a fresh RunHandle whose events stream from
/// a ReplayAttemptStream instead of a live pipeline. The Tauri event
/// forwarder (T13) bridges this onto `run:<handle>` events the same
/// way as live runs.
#[tauri::command]
pub fn attempt_replay_start(
    _state: State<'_, AppState>,
    _execution_id: ExecutionId,
    _attempt_id: AttemptId,
    _speed: f32,
) -> Result<RunHandle, UiError> {
    // Plan 4 T23 (React replay UI) calls this; the actual bridge from
    // the ReplayAttemptStream into the run:<handle> event channel is
    // wired in T13's event bridge. For T12 we stub the command so the
    // invoke_handler registration compiles. T13 fills in the body.
    Err(UiError::Internal("replay bridge wired in T13".into()))
}
