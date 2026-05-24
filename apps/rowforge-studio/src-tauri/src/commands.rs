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
use tauri::{Emitter as _, State};

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

    let sessions = core.sessions();
    *state.core.lock().unwrap_or_else(|p| p.into_inner()) = Some(core);

    // Spawn the 1 Hz workspace rollup forwarder for this workspace session.
    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        crate::events::forward_active_runs(app_clone, sessions).await;
    });

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
) -> Result<RunHandle, UiError> {
    // Scope the MutexGuard so it is dropped before any .await point.
    // studio-core::start_run internally calls tokio::spawn (tick loop +
    // pipeline task); those spawns require an entered tokio runtime.
    // Making this command async ensures Tauri executes it on its tokio
    // runtime, so the inner spawn calls have a runtime context.
    let (handle, stream_rx) = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        let core = guard
            .as_ref()
            .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
        let opts = RunOpts::new(handler_dir);
        let handle = core.start_run(&execution_id, opts)?;
        let stream = core
            .subscribe(&handle)
            .map_err(|e| UiError::Internal(e.to_string()))?;
        (handle, stream.rx)
    }; // guard dropped here, before any .await

    // Spawn the per-run event forwarder onto run:<handle>.
    let handle_for_task = handle.clone();
    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        crate::events::forward_run_events(app_clone, handle_for_task, stream_rx).await;
    });

    Ok(handle)
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
/// forwarder bridges this onto `run:<handle>` events the same way as
/// live runs, so the React side subscribes symmetrically.
#[tauri::command]
pub fn attempt_replay_start(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    execution_id: ExecutionId,
    attempt_id: AttemptId,
    speed: f32,
) -> Result<RunHandle, UiError> {
    use rowforge_studio_core::{AttemptStream as _, ReplayAttemptStream};

    let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
    let core = guard.as_ref().ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;

    // Resolve attempt_dir from workspace root + exec_id + attempt_id.
    let attempt_dir = core.workspace().root
        .join("executions")
        .join(execution_id.as_str())
        .join("attempts")
        .join(attempt_id.as_str());

    let stream = ReplayAttemptStream::from_attempt(&attempt_dir, speed)
        .map_err(|e| UiError::Io(e.to_string()))?;

    // Allocate a fresh handle and forward the replay stream to Tauri events.
    let handle = RunHandle::new();
    let app_clone = app.clone();
    let handle_for_task = handle.clone();
    let channel = format!("run:{}", handle.as_str());

    tauri::async_runtime::spawn(async move {
        use futures::StreamExt as _;
        let mut events = Box::new(stream).events();
        while let Some(event) = events.next().await {
            let _ = app_clone.emit(&channel, &event);
        }
        // Replay stream ends after Done is emitted — nothing more to do.
        let _ = handle_for_task; // keep handle alive in closure
    });

    Ok(handle)
}
