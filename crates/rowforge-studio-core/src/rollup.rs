//! ExecRollup projection — cold; part-2 §2.2.5 + cancelled_last addition.

use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct ExecRollup {
    pub resolved: u64,
    pub failed_last: u64,
    pub crashed_last: u64,
    pub cancelled_last: u64,
    pub too_large: u64,
    pub never_attempted: u64,
    pub by_error_code: BTreeMap<String, u64>,
}
