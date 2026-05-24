//! App state: the lazily-opened StudioCore.
//!
//! `core` is None until the user picks a workspace via Workspace Picker
//! (Plan 2) or the boot autoload finds settings.workspace_root.
//!
//! We use `std::sync::Mutex` instead of `tokio::sync::RwLock` because
//! `rusqlite::Connection` (held inside `ExecutionStore`) is `!Send`.

use rowforge_studio_core::StudioCore;
use std::sync::Mutex;

#[derive(Default)]
pub struct AppState {
    pub core: Mutex<Option<StudioCore>>,
}
