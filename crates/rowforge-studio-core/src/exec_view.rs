//! ExecSummary projection from the on-disk store.
//!
//! Spec: `docs/spec/studio/part-2-model.md` §2.2.2.
//!
//! Plan 1 scope: name + created_at + input_rows are populated; the
//! attempt-derived fields (count, last state, last counts) are stubbed
//! and filled in Plan 3 once the attempts join + meta.json read are
//! implemented.

use chrono::{DateTime, Utc};
use rowforge_core::execution_store::Execution;
use serde::{Deserialize, Serialize};

/// Filter passed to `list`. Reserved for future use; Plan 1 has no
/// filter knobs.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct ListFilter;

/// Light-weight projection for the exec list page.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ExecSummary {
    pub id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub input_rows: Option<u64>,

    // Stubs filled in Plan 3.
    pub attempts_count: u32,
    pub last_attempt_state: Option<String>,
    pub last_attempt_counts: Option<AttemptCountsStub>,
}

/// Placeholder for Plan 3's full `AttemptCounts`. Kept as its own type
/// so the public field above does not change shape later.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AttemptCountsStub {
    pub success: u64,
    pub failed: u64,
    pub crashed: u64,
}

/// Plan 1 conversion: ignore attempts entirely.
impl From<&Execution> for ExecSummary {
    fn from(e: &Execution) -> Self {
        ExecSummary {
            id: e.id.clone(),
            name: e.name.clone().unwrap_or_default(),
            created_at: e.created_at,
            input_rows: Some(e.input_row_count),
            attempts_count: 0,
            last_attempt_state: None,
            last_attempt_counts: None,
        }
    }
}
