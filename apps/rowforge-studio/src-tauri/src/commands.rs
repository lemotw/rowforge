//! Tauri commands — stubs until later tasks fill them in.
use serde::Serialize;

#[derive(Serialize)]
pub struct StubWorkspace;

#[tauri::command]
pub fn workspace_open() -> Result<StubWorkspace, String> {
    Err("not implemented".into())
}
#[tauri::command]
pub fn exec_list() -> Result<Vec<()>, String> {
    Err("not implemented".into())
}
#[tauri::command]
pub fn workspace_settings_load() -> Result<(), String> {
    Err("not implemented".into())
}
#[tauri::command]
pub fn workspace_settings_save() -> Result<(), String> {
    Err("not implemented".into())
}
