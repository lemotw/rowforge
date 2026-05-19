// P11: run_pool, accumulate_batches, PoolConfig, BatchJob, run_worker_loop,
// dispatch_row, and dispatch_batch were deleted (superseded by
// run_pool_streaming + accumulator_task + worker_loop::run_worker_loop).
//
// KEPT: RowJob (used by reader_task), RowOutcome, BatchOutcome (used by
// jsonl_writer, pool_streaming, exec export, and all test modules).

use serde::{Deserialize, Serialize};

/// One row of input ready to be dispatched to a worker.
pub struct RowJob {
    pub seq: u64,
    pub data: serde_json::Map<String, serde_json::Value>,
    pub meta: crate::protocol::RowMeta,
}

/// Outcome for one row, stored in `outcomes.jsonl`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RowOutcome {
    Success {
        seq: u64,
        data: serde_json::Map<String, serde_json::Value>,
        dur_ms: u64,
    },
    Error {
        seq: u64,
        code: String,
        message: String,
        dur_ms: u64,
        /// Optional handler-supplied domain payload (mirrors
        /// `Inbound::Error::data` / `BatchEntry::Error::data`). This is the
        /// handler's domain-specific context; `exec export` (P10) discovers
        /// which keys are present by scanning `outcomes.jsonl`. Synthesized
        /// error outcomes (CANCELLED, STARTUP_FAILED, WORKER_CRASH, etc.)
        /// carry `None` here.
        data: Option<serde_json::Map<String, serde_json::Value>>,
    },
    Crash {
        seq: u64,
        worker_id: u32,
        crash_at_seq: u64,
    },
}

/// Per-line format for `outcomes.jsonl`.
///
/// One `BatchOutcome` is appended atomically to `outcomes.jsonl` after a
/// worker completes (or the accumulator synthesizes) a batch.  The `seqs`
/// field preserves the batch's logical ordering (strictly ascending within a
/// batch, though batches themselves arrive in completion order and may
/// interleave).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BatchOutcome {
    /// `seqs[0]` — the smallest seq in this batch.  Denormalised for fast
    /// scanning without deserialising the full `seqs` array.
    pub first_seq: u64,
    /// All seq numbers in this batch, strictly ascending.
    pub seqs: Vec<u64>,
    /// Per-row outcome, positionally aligned with `seqs`.
    pub outcomes: Vec<RowOutcome>,
}

impl BatchOutcome {
    /// Construct a `BatchOutcome` from a `Vec<RowOutcome>`, deriving `seqs` and
    /// `first_seq` from the outcomes themselves. Panics if `outcomes` is empty.
    ///
    /// This constructor enforces the invariant `first_seq == seqs[0]` at the
    /// type level — callers cannot accidentally pass a mismatched `first_seq`.
    pub fn from_outcomes(outcomes: Vec<RowOutcome>) -> Self {
        let seqs: Vec<u64> = outcomes
            .iter()
            .map(|o| match o {
                RowOutcome::Success { seq, .. }
                | RowOutcome::Error { seq, .. }
                | RowOutcome::Crash { seq, .. } => *seq,
            })
            .collect();
        let first_seq = seqs[0];
        Self { first_seq, seqs, outcomes }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_success(seq: u64) -> RowOutcome {
        RowOutcome::Success {
            seq,
            data: serde_json::Map::from_iter([("k".to_string(), serde_json::json!("v"))]),
            dur_ms: 10,
        }
    }

    fn make_error(seq: u64) -> RowOutcome {
        RowOutcome::Error {
            seq,
            code: "MY_ERR".to_string(),
            message: "something went wrong".to_string(),
            dur_ms: 5,
            data: Some(serde_json::Map::from_iter([(
                "detail".to_string(),
                serde_json::json!("extra"),
            )])),
        }
    }

    fn make_crash(seq: u64) -> RowOutcome {
        RowOutcome::Crash {
            seq,
            worker_id: 2,
            crash_at_seq: seq,
        }
    }

    #[test]
    fn batch_outcome_roundtrip_success_only() {
        let bo = BatchOutcome {
            first_seq: 0,
            seqs: vec![0, 1, 2],
            outcomes: vec![make_success(0), make_success(1), make_success(2)],
        };
        let json = serde_json::to_string(&bo).expect("serialize");
        let parsed: BatchOutcome = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(bo, parsed);
        // Verify the serde tag is present in the serialized form
        assert!(json.contains("\"type\":\"success\""));
    }

    #[test]
    fn batch_outcome_roundtrip_mixed() {
        let bo = BatchOutcome {
            first_seq: 10,
            seqs: vec![10, 11, 12],
            outcomes: vec![make_success(10), make_error(11), make_crash(12)],
        };
        let json = serde_json::to_string(&bo).expect("serialize");
        let parsed: BatchOutcome = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(bo, parsed);
        assert!(json.contains("\"type\":\"success\""));
        assert!(json.contains("\"type\":\"error\""));
        assert!(json.contains("\"type\":\"crash\""));
    }

    #[test]
    fn batch_outcome_jsonl_line() {
        let bo = BatchOutcome {
            first_seq: 0,
            seqs: vec![0],
            outcomes: vec![make_success(0)],
        };
        let json = serde_json::to_string(&bo).expect("serialize");
        // Must be a single line — no embedded newlines
        assert!(
            !json.contains('\n'),
            "serialized BatchOutcome must not contain newlines; got: {json:?}"
        );
    }

    #[test]
    fn from_outcomes_derives_first_seq_and_seqs() {
        let outcomes = vec![make_success(5), make_error(6), make_crash(7)];
        let bo = BatchOutcome::from_outcomes(outcomes);
        assert_eq!(bo.first_seq, 5);
        assert_eq!(bo.seqs, vec![5, 6, 7]);
        assert_eq!(bo.outcomes.len(), 3);
    }
}
