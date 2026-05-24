//! FailedRowPage projection — paged scan of outcomes.jsonl. Part-2 §2.2.6.
//!
//! ## outcomes.jsonl line shape
//!
//! Each line is a `BatchOutcome` JSON object:
//!   `{"first_seq":N,"seqs":[...],"outcomes":[...]}`
//!
//! Each element in `outcomes` carries a `"type"` tag:
//!   - `"success"`: `{type, seq, data, dur_ms}`
//!   - `"error"`:   `{type, seq, code, message, dur_ms, data?}` ← failed
//!   - `"crash"`:   `{type, seq, worker_id, crash_at_seq}`      ← failed
//!
//! There is no `too_large` type and no `row_index` / `raw` field in the
//! real format. `RowOutcomeKind::TooLarge` is retained in the public API
//! so the UI is forward-compatible, but the parser never produces it from
//! the current file format.

use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::{AttemptId, ExecutionId};

#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct FailedPageQuery {
    pub execution_id: ExecutionId,
    pub attempt_id: AttemptId,
    pub offset: u64,
    pub limit: u32,
    pub error_code_filter: Option<String>,
}

impl FailedPageQuery {
    pub fn new(
        execution_id: ExecutionId,
        attempt_id: AttemptId,
        offset: u64,
        limit: u32,
        error_code_filter: Option<String>,
    ) -> Self {
        Self {
            execution_id,
            attempt_id,
            offset,
            limit,
            error_code_filter,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct FailedRowPage {
    pub rows: Vec<FailedRow>,
    pub next_offset: Option<u64>,
    pub total_known: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct FailedRow {
    /// The per-row sequence number from the outcomes file.
    pub seq: u64,
    /// Error code — `None` for `Crash` (which has no code).
    pub error_code: Option<String>,
    /// Human-readable message — `None` for `Crash`.
    pub message: Option<String>,
    /// Outcome kind: Error or Crash (TooLarge reserved for future).
    pub kind: RowOutcomeKind,
    /// Handler-supplied domain payload for Error rows; `null` for Crash.
    #[serde(rename = "raw_record")]
    pub data: serde_json::Value,
    /// Duration in milliseconds. Crash rows carry 0 (no timing data).
    pub dur_ms: u64,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum RowOutcomeKind {
    Error,
    Crash,
    TooLarge,
}

// ---------------------------------------------------------------------------
// Internal: flat representation of one RowOutcome deserialized from the
// `outcomes` array of a BatchOutcome line.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RawRowOutcome {
    #[serde(rename = "type")]
    kind: String,
    seq: u64,
    // Error-variant fields
    code: Option<String>,
    message: Option<String>,
    dur_ms: Option<u64>,
    data: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct RawBatchOutcome {
    outcomes: Vec<RawRowOutcome>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Read a page of failed rows from `outcomes.jsonl`.
///
/// Iterates all BatchOutcome lines linearly, collecting `Error` and `Crash`
/// entries. `offset` / `limit` apply to the failed-row stream only (success
/// rows do not advance the offset counter). `error_code_filter` keeps only
/// rows whose `code` matches exactly.
///
/// `next_offset` is `Some(n)` when the page was cut at the limit; `None`
/// when the file was exhausted. `total_known` is always `None` in v1 (no
/// sidecar index).
///
/// Malformed lines are silently skipped (per Plan 3 open question).
pub fn read_failed_page(
    outcomes_jsonl: &Path,
    query: &FailedPageQuery,
) -> Result<FailedRowPage, std::io::Error> {
    use std::io::{BufRead, BufReader};

    let f = std::fs::File::open(outcomes_jsonl)?;
    let reader = BufReader::new(f);

    let limit = (query.limit as usize).min(500);
    let mut rows: Vec<FailedRow> = Vec::with_capacity(limit);
    let mut failed_seen: u64 = 0;

    let mut next_offset: Option<u64> = None;

    'outer: for line_res in reader.lines() {
        let line = line_res?;
        let batch: RawBatchOutcome = match serde_json::from_str(&line) {
            Ok(b) => b,
            Err(_) => continue, // skip malformed lines silently
        };

        for ro in batch.outcomes {
            let kind = match ro.kind.as_str() {
                "error" => RowOutcomeKind::Error,
                "crash" => RowOutcomeKind::Crash,
                _ => continue, // skip success + unknown
            };

            // Apply error_code_filter (only meaningful for Error rows).
            if let Some(ref filter) = query.error_code_filter {
                let code = ro.code.as_deref().unwrap_or("");
                if code != filter {
                    continue;
                }
            }

            // Advance past offset.
            if failed_seen < query.offset {
                failed_seen += 1;
                continue;
            }

            // Page already full: this outcome is the (limit+1)-th matching
            // item — a genuine next row exists. Record its logical index and
            // stop scanning.
            if rows.len() >= limit {
                next_offset = Some(failed_seen);
                break 'outer;
            }

            rows.push(FailedRow {
                seq: ro.seq,
                kind,
                error_code: ro.code.clone(),
                message: ro.message.clone(),
                data: ro.data.unwrap_or(serde_json::Value::Null),
                dur_ms: ro.dur_ms.unwrap_or(0),
            });
            failed_seen += 1;
        }
    }

    // next_offset is Some only when a (limit+1)-th matching row was observed.
    // If the file was exhausted with rows.len() == limit, next_offset stays None.
    Ok(FailedRowPage {
        rows,
        next_offset,
        total_known: None,
    })
}
