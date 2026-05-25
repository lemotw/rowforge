//! Tauri commands wrapping the `StudioCore` surface.
//!
//! Every command returns `Result<T, UiError>`; the structured error is
//! serialized to JSON for the React layer to classify by `kind` (spec
//! §5.3 / §5.5).

use std::path::PathBuf;

use rowforge_studio_core::{
    AttemptDetail, AttemptId, BuildOutcome, CancelMode, ExecDetail, ExecRollup, ExecSummary,
    ExecutionId, ExportOpts, ExportReport, FailedPageQuery, FailedRowPage, HandlerDetail,
    HandlerSummary, ListFilter, ManifestReport, ManifestSource, OpenOpts, ProgressSnapshot,
    RowHistory, RunHandle, RunOpts, RunStartedHandle, RunStatus, ScaffoldArgs, Settings,
    StartExecArgs, StudioCore, UiError, Workspace,
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
    let opts = opts
        .with_max_concurrent_runs(prev.max_concurrent_runs)
        .with_preferred_editor(prev.preferred_editor.clone());
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
    state: State<'_, AppState>,
    settings: Settings,
) -> Result<(), UiError> {
    settings_io::save(&app, &settings)?;
    // Plan 7 T15: refresh preferred_editor in the live StudioCore so the
    // next handler_open_editor call uses the new value without requiring a
    // workspace re-open.
    let mut guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(core) = guard.as_mut() {
        core.set_preferred_editor(settings.preferred_editor.clone());
    }
    Ok(())
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

// ===== Plan 7 handler authoring =====

#[tauri::command]
pub fn handler_list(state: State<'_, AppState>) -> Result<Vec<HandlerSummary>, UiError> {
    let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
    let core = guard
        .as_ref()
        .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
    core.handler_list()
}

#[tauri::command]
pub fn handler_show(
    state: State<'_, AppState>,
    name: String,
) -> Result<HandlerDetail, UiError> {
    let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
    let core = guard
        .as_ref()
        .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
    core.handler_show(&name)
}

#[tauri::command]
pub fn handler_open_editor(
    state: State<'_, AppState>,
    name: String,
) -> Result<(), UiError> {
    let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
    let core = guard
        .as_ref()
        .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
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
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        let core = guard
            .as_ref()
            .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
        core.handler_reveal_path(&name)?
    };
    use tauri_plugin_shell::ShellExt;
    app.shell()
        .open(path.to_string_lossy().to_string(), None)
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
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        let core = guard
            .as_ref()
            .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
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
    {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        let core = guard
            .as_ref()
            .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
        core.handler_delete(&name)?;
    }
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
    {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        let core = guard
            .as_ref()
            .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
        core.handler_rename(&old, &new)?;
    }
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
    //
    // AppState.core is Mutex<Option<StudioCore>> (not Arc-wrapped), so we
    // cannot move it into spawn_blocking. Instead we hold the mutex only for
    // the synchronous call — no .await inside, so there is no risk of holding
    // a guard across an await point.
    let result = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        let core = guard
            .as_ref()
            .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
        core.handler_build(&name)
    };

    // Emit list-refresh hint so HandlerSummary.last_modified (which folds
    // over top-level entries, including the new binary) gets picked up by the
    // UI after a successful (or failed-but-cached) build.
    let _ = app.emit("handlers:list", ());

    result
}

