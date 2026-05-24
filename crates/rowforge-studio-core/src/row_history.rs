//! RowHistory projection — on-demand fold across attempts for one seq.
//! Spec part-2 §2.2.7.

use serde::Serialize;

use crate::failed::RowOutcomeKind;
use crate::AttemptId;

#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct RowHistory {
    pub seq: u64,
    /// Per-attempt entries for this seq. Each tuple is
    /// `(attempt_id, kind, error_code)`. Success outcomes do not produce
    /// entries — `resolved_at` indicates them.
    pub rows: Vec<(AttemptId, RowOutcomeKind, Option<String>)>,
    /// First attempt that produced Success for this seq, if any.
    pub resolved_at: Option<AttemptId>,
}
