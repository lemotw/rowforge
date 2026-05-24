//! Workspace location helpers shared by CLI and Studio.
//!
//! A "workspace" (also called `home` in the CLI's older terminology) is a
//! directory containing `executions.db` and the per-execution
//! subdirectories under `executions/`.
//!
//! Spec: `docs/spec/studio/part-1-overview.md` §1.5 (workspace ownership),
//! `docs/spec/studio/part-4-data.md` §4.1 (artifact list).

use std::path::PathBuf;

/// Where to find the executions store on this machine when no override is
/// given.
///
/// Returns the same path the CLI's `rowforge exec` commands have always
/// used: `$HOME/.rowforge`. Returns `None` only if the OS cannot resolve
/// a home directory (very rare; sandboxed installs).
pub fn default_workspace_root() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".rowforge"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_workspace_root_under_home() {
        let root = default_workspace_root().expect("home dir available");
        assert!(root.ends_with(".rowforge"), "got {:?}", root);
    }
}
