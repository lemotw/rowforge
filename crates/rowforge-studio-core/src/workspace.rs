//! Stub — filled in Task 4.
use std::path::PathBuf;

#[derive(Debug, Clone, Default)]
pub struct OpenOpts {
    pub workspace: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct Workspace {
    pub root: PathBuf,
    pub schema_version: u8,
}
