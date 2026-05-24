//! Tauri commands wrapping the `StudioCore` surface.
//!
//! Every command returns `Result<T, UiError>`; the structured error is
//! serialized to JSON for the React layer to classify by `kind` (spec
//! §5.3 / §5.5).

use std::path::PathBuf;

use rowforge_studio_core::{
    ExecSummary, ListFilter, OpenOpts, Settings, StudioCore, UiError, Workspace,
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

    *state.core.lock().unwrap() = Some(core);
    Ok(workspace)
}

#[tauri::command]
pub fn exec_list(state: State<'_, AppState>) -> Result<Vec<ExecSummary>, UiError> {
    let guard = state.core.lock().unwrap();
    let core = guard
        .as_ref()
        .ok_or_else(|| UiError::WorkspaceUnavailable("no workspace open".into()))?;
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
