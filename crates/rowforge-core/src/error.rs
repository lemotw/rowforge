use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CoreError {
    #[error("handler startup timeout after {timeout_ms}ms")]
    StartupTimeout { timeout_ms: u64 },
    #[error("handler exited unexpectedly with code {code:?}")]
    HandlerExit { code: Option<i32> },
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("protocol: {0}")]
    Protocol(String),
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("store: {0}")]
    Store(String),
    #[error("schema version {found} is newer than this binary knows about (max {max_known})")]
    SchemaTooNew { found: u8, max_known: u8 },
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
