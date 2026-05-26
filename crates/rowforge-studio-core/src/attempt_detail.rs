//! AttemptDetail projection — per spec part-2 §2.2.4.
//!
//! Plan 3 returns a static snapshot regardless of whether the attempt
//! is still running. The Live tab (Plan 4) will replace this for
//! in-progress attempts via SessionRegistry. The Studio UI surfaces
//! a "May be stale; refresh manually" banner when state is non-terminal
//! (driven by the `is_terminal` field below).

use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::{AttemptCountsStub, AttemptId, ExecutionId};

#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct AttemptDetail {
    pub id: AttemptId,
    pub execution_id: ExecutionId,
    pub state: String,
    pub run_type: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub stats: AttemptCountsStub,
    pub by_error_code: BTreeMap<String, u64>,
    pub handler_instance: HandlerInstanceView,
    pub paths: AttemptPaths,
    pub is_terminal: bool,
    /// Plan 14: when state is `aborted`, this carries the reason
    /// (`Some("hard_cancel")` for force-killed, `None` for soft cancel).
    pub cancelled_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct HandlerInstanceView {
    pub id: Option<String>,
    pub handler_id: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct AttemptPaths {
    pub meta_json: PathBuf,
    pub outcomes_jsonl: PathBuf,
    pub handler_stderr_log: PathBuf,
}
