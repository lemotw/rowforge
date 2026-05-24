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

#[derive(Default)]
pub struct AppState {
    pub core: Mutex<Option<StudioCore>>,
}
