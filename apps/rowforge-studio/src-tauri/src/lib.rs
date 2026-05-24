//! rowforge Studio Tauri shell.
//!
//! See `docs/spec/studio/part-5-api.md` §5.1 for crate-boundary contract:
//! this layer is thin glue; all projection logic lives in
//! `rowforge-studio-core`.

mod state;
mod commands;
mod events;
mod settings;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(state::AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::workspace_open,
            commands::workspace_current,
            commands::workspace_settings_load,
            commands::workspace_settings_save,
            commands::exec_list,
            commands::exec_show,
            commands::exec_rollup,
            commands::exec_start,              // T9
            commands::exec_export,             // T9
            commands::manifest_validate,       // T9
            commands::attempt_show,
            commands::attempt_failed_page,
            commands::attempt_row_history,
            commands::run_start,
            commands::run_cancel,
            commands::run_status,
            commands::run_active,
            commands::run_snapshot,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
