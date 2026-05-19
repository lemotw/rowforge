use thiserror::Error;

#[derive(Debug, Error)]
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
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
