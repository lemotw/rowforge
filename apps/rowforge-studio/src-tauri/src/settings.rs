//! Settings file path resolution + IO using Tauri's app_data_dir.
//!
//! Path: `<app_data_dir>/rowforge-studio/settings.json` per spec §5.6.

use std::fs;
use std::path::PathBuf;

use rowforge_studio_core::{Settings, UiError};
use tauri::{Manager, Runtime};

fn settings_path<R: Runtime>(app: &tauri::AppHandle<R>) -> Result<PathBuf, UiError> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| UiError::Io(format!("app_data_dir: {e}")))?;
    let ws_dir = dir.join("rowforge-studio");
    fs::create_dir_all(&ws_dir).map_err(|e| UiError::Io(e.to_string()))?;
    Ok(ws_dir.join("settings.json"))
}

pub fn load<R: Runtime>(app: &tauri::AppHandle<R>) -> Result<Settings, UiError> {
    let p = settings_path(app)?;
    if !p.exists() {
        return Ok(Settings::default());
    }
    let f = fs::File::open(&p).map_err(|e| UiError::Io(e.to_string()))?;
    Settings::load_from(f)
}

pub fn save<R: Runtime>(
    app: &tauri::AppHandle<R>,
    settings: &Settings,
) -> Result<(), UiError> {
    let p = settings_path(app)?;
    let f = fs::File::create(&p).map_err(|e| UiError::Io(e.to_string()))?;
    settings.save_to(f)
}
