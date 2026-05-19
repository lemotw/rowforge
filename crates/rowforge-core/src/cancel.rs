//! Cancellation re-export.
//!
//! Run callers (Studio, future HTTP API) construct a `CancellationToken`,
//! pass it into `RunRequest`, and call `.cancel()` to abort an in-flight run.
//! The pool dispatch loop races each row send against the token.

pub use tokio_util::sync::CancellationToken;
