//! rowforge Studio Tauri shell.
//!
//! See `docs/spec/studio/part-5-api.md` §5.1 for crate-boundary contract:
//! this layer is thin glue; all projection logic lives in
//! `rowforge-studio-core`.

mod state;
mod commands;
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
            commands::exec_list,
            commands::workspace_settings_load,
            commands::workspace_settings_save,
            commands::exec_show,
            commands::attempt_show,
            commands::exec_rollup,
            commands::attempt_failed_page,
            commands::attempt_row_history,
            commands::run_start,
            commands::run_cancel,
            commands::run_status,
            commands::run_active,
            commands::attempt_replay_start,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
