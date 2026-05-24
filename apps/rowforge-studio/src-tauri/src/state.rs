//! App state: the lazily-opened StudioCore.
//!
//! `core` is None until the user picks a workspace via Workspace Picker
//! or the boot autoload finds settings.workspace_root.
//!
//! Lock choice: `std::sync::Mutex` (not `tokio::sync::RwLock`) because
//! `ExecutionStore` holds a `rusqlite::Connection` which is `!Sync` (and
//! `!Send` per SQLite's threading model). RwLock requires `T: Send + Sync`
//! to expose concurrent reads, which is unsound here. Mutex serializes
//! all access correctly.

use rowforge_studio_core::StudioCore;
use std::sync::Mutex;
use tauri::async_runtime::JoinHandle;

#[derive(Default)]
pub struct AppState {
    pub core: Mutex<Option<StudioCore>>,
    /// Handle to the per-workspace `runs:active` forwarder task spawned
    /// by `workspace_open`. Stored so re-opening a workspace (switching)
    /// can abort the prior forwarder before starting a new one, instead
    /// of leaving stale forwarders alive emitting from old registries.
    pub active_runs_task: Mutex<Option<JoinHandle<()>>>,
}
