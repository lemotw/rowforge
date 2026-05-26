//! Streaming Reader task for the dispatch pipeline (plan §4, v3.3).
//!
//! `reader_task` pulls rows from any [`InputStream`] implementation, applies
//! skip/limit/field-rename filtering, and forwards [`RowJob`]s to the bounded
//! `row_tx` channel using a cancel-aware send (§8.4, C6 invariant).

use std::collections::{BTreeMap, HashSet};

use crate::cancel::CancellationToken;
use crate::error::CoreError;
use crate::input_stream::InputStream;
use crate::pool::RowJob;
use crate::protocol::RowMeta;

// ---------------------------------------------------------------------------
// FieldMap
// ---------------------------------------------------------------------------

/// Field mapping: schema_field_name → csv_column_name.
///
/// At Row build time we walk the CSV columns and rename any column that is
/// the **value** in this map to the corresponding **key**. Columns absent
/// from the map keep their CSV name. This gives handler authors a way to
/// say "my handler expects `email` but the CSV column is `user_email`"
/// without modifying the CSV.
pub type FieldMap = BTreeMap<String, String>;

// ---------------------------------------------------------------------------
// ROW_CHANNEL_CAP
// ---------------------------------------------------------------------------

/// Capacity of the bounded `row_tx` / `row_rx` channel between the Reader
/// task and the Accumulator task (plan §11).
pub const ROW_CHANNEL_CAP: usize = 64;

// ---------------------------------------------------------------------------
// SendStop
// ---------------------------------------------------------------------------

/// Reason why the Reader's cancel-aware send stopped early.
///
/// Both variants cause the reader loop to `break` and return `Ok(())` — cancel
/// is not an error path for the reader.
#[derive(Debug)]
pub enum SendStop {
    /// The [`CancellationToken`] fired while the sender was waiting for channel
    /// space (channel-full + downstream stall scenario, §8.4 C6).
    Cancelled,
    /// The receiver half of the channel was dropped (downstream task exited).
    ReceiverDropped,
}

// ---------------------------------------------------------------------------
// ReaderConfig
// ---------------------------------------------------------------------------

/// Configuration passed to [`reader_task`].
pub struct ReaderConfig {
    /// Sequence numbers to skip entirely (seq counter still advances; skipped
    /// rows are never sent to the accumulator).
    pub skip_seqs: HashSet<u64>,
    /// When Some, dispatch only rows whose `seq` is in this set.
    /// Precedence: `only_row_ids` takes priority over `skip_seqs` —
    /// if a seq is in `only_row_ids`, it is dispatched even if it would
    /// have been skipped by `skip_seqs` (re-run intent overrides resume intent).
    /// Non-existent seqs in the set are silently ignored.
    /// `None`: existing behavior (dispatch all rows modulo skip_seqs).
    /// `Some(empty)`: dispatch nothing (vacuous noop).
    pub only_row_ids: Option<HashSet<u64>>,
    /// Stop after emitting this many rows (None = no limit).
    pub row_limit: Option<usize>,
    /// Top-level key renames applied before building the [`RowJob`].
    /// Map entry `(old, new)` renames key `old` to `new` in the row data.
    pub field_map: FieldMap,
    /// Whether this run is a dry run (forwarded verbatim into [`RowMeta`]).
    pub dry_run: bool,
}

// ---------------------------------------------------------------------------
// rename_top_level (local helper)
// ---------------------------------------------------------------------------

/// Rename top-level keys in `data` according to `field_map`.
///
/// For each `(old, new)` entry: if `data` contains `old`, remove it and
/// re-insert under `new`. Single-step rename only — chained renames
/// (e.g. `a → b` then `b → c` in the same map) are not supported and may
/// produce unexpected results depending on BTreeMap iteration order.
fn rename_top_level(
    data: &mut serde_json::Map<String, serde_json::Value>,
    field_map: &FieldMap,
) {
    for (old, new) in field_map {
        if let Some(v) = data.remove(old) {
            data.insert(new.clone(), v);
        }
    }
}

// ---------------------------------------------------------------------------
// reader_task
// ---------------------------------------------------------------------------

/// Stream rows from `input` to `row_tx`, applying filtering and field renames.
///
/// # Cancel behaviour (§8.4 C6)
///
/// `mpsc::Sender::send().await` does **not** wake when a `CancellationToken`
/// fires.  When the channel is full and the downstream accumulator is stalled,
/// a plain `send().await` would hang forever even after `cancel.cancel()` is
/// called.  This task therefore wraps every send in a `tokio::select!` that
/// races the send against the cancellation future, breaking out of the loop
/// (and returning `Ok`) when cancel fires first.
///
/// # Loop invariant
///
/// `seq` originates from the [`InputStream`] implementation (I1: monotonically
/// increasing from 0). `emitted` counts only rows that were actually sent.
pub async fn reader_task(
    input: Box<dyn InputStream>,
    config: ReaderConfig,
    row_tx: tokio::sync::mpsc::Sender<RowJob>,
    cancel: Option<CancellationToken>,
) -> Result<(), CoreError> {
    let ReaderConfig {
        skip_seqs,
        only_row_ids,
        row_limit,
        field_map,
        dry_run,
    } = config;

    let mut emitted: usize = 0;
    let mut parse_errors_skipped: u64 = 0;

    for src_result in input.yield_rows() {
        // Cheap cancel check before doing any work on this row.
        if cancel.as_ref().map_or(false, |c| c.is_cancelled()) {
            break;
        }

        // Per-row parse error: log + skip, do NOT abort the whole pipeline.
        // The bad row stays out of outcomes.jsonl → compute_resolution sees
        // it as NeverAttempted (re-tried on next attempt; the user must fix
        // the input to clear it). Applies to both CSV (mismatched columns,
        // bad quoting) and JSONL (malformed JSON).
        let mut row_src = match src_result {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "input row parse error; skipping");
                parse_errors_skipped += 1;
                continue;
            }
        };
        let seq = row_src.seq;

        // only_row_ids filter takes priority over skip_seqs: when Some, only
        // dispatch rows whose seq is in the set; skip_seqs is ignored for
        // matching rows (re-run intent overrides resume intent).
        if let Some(ref only_set) = only_row_ids {
            if !only_set.contains(&seq) {
                continue;
            }
            // seq is in only_row_ids → dispatch unconditionally (bypass skip_seqs)
        } else if skip_seqs.contains(&seq) {
            continue;
        }

        if let Some(max) = row_limit {
            if emitted >= max {
                break;
            }
        }

        // Apply top-level key renames.
        rename_top_level(&mut row_src.data, &field_map);

        let job = RowJob {
            seq,
            data: row_src.data,
            meta: RowMeta {
                dry_run,
                row_index: seq,
            },
        };

        // Cancel-aware send: avoid hanging forever when the channel is full
        // and the downstream task is stalled (§8.4 C6).
        let send_result: Result<(), SendStop> = match &cancel {
            Some(c) => {
                tokio::select! {
                    biased;
                    _ = c.cancelled() => Err(SendStop::Cancelled),
                    r = row_tx.send(job) => r.map_err(|_| SendStop::ReceiverDropped),
                }
            }
            None => row_tx.send(job).await.map_err(|_| SendStop::ReceiverDropped),
        };

        if send_result.is_err() {
            break;
        }

        emitted += 1;
    }

    if parse_errors_skipped > 0 {
        tracing::warn!(
            skipped = parse_errors_skipped,
            emitted,
            "reader finished: {} input row(s) skipped due to parse errors; \
             they will appear as NeverAttempted in resolution. Fix the input \
             to clear them in a subsequent attempt.",
            parse_errors_skipped
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input_stream::{CsvInputStream, JsonlInputStream};
    use std::collections::BTreeMap;
    use std::io::Write;
    use tempfile::NamedTempFile;
    use tokio::sync::mpsc;

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn temp_csv(content: &str) -> NamedTempFile {
        let mut f = tempfile::Builder::new()
            .suffix(".csv")
            .tempfile()
            .unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f.flush().unwrap();
        f
    }

    fn temp_jsonl(content: &str) -> NamedTempFile {
        let mut f = tempfile::Builder::new()
            .suffix(".jsonl")
            .tempfile()
            .unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f.flush().unwrap();
        f
    }

    fn default_config() -> ReaderConfig {
        ReaderConfig {
            skip_seqs: HashSet::new(),
            only_row_ids: None,
            row_limit: None,
            field_map: BTreeMap::new(),
            dry_run: false,
        }
    }

    // -----------------------------------------------------------------------
    // 1. reader_basic_csv_yields_all_rows
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn reader_basic_csv_yields_all_rows() {
        let f = temp_csv("id,val\n0,a\n1,b\n2,c\n3,d\n4,e\n");
        let input: Box<dyn InputStream> =
            Box::new(CsvInputStream::open(f.path(), &[]).unwrap());

        let (tx, mut rx) = mpsc::channel::<RowJob>(16);
        let result = reader_task(input, default_config(), tx, None).await;
        assert!(result.is_ok(), "reader_task failed: {:?}", result);

        let mut jobs = Vec::new();
        while let Ok(j) = rx.try_recv() {
            jobs.push(j);
        }

        assert_eq!(jobs.len(), 5, "expected 5 RowJobs");
        for (i, job) in jobs.iter().enumerate() {
            assert_eq!(job.seq, i as u64, "seq mismatch at index {i}");
            assert_eq!(job.meta.row_index, i as u64);
            assert!(!job.meta.dry_run);
        }
    }

    // -----------------------------------------------------------------------
    // 2. reader_basic_jsonl_yields_all_rows
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn reader_basic_jsonl_yields_all_rows() {
        let content = r#"{"id":0}
{"id":1}
{"id":2}
{"id":3}
{"id":4}
"#;
        let f = temp_jsonl(content);
        let input: Box<dyn InputStream> =
            Box::new(JsonlInputStream::open(f.path(), &[]).unwrap());

        let (tx, mut rx) = mpsc::channel::<RowJob>(16);
        let result = reader_task(input, default_config(), tx, None).await;
        assert!(result.is_ok(), "reader_task failed: {:?}", result);

        let mut jobs = Vec::new();
        while let Ok(j) = rx.try_recv() {
            jobs.push(j);
        }

        assert_eq!(jobs.len(), 5, "expected 5 RowJobs");
        for (i, job) in jobs.iter().enumerate() {
            assert_eq!(job.seq, i as u64, "seq mismatch at index {i}");
        }
    }

    // -----------------------------------------------------------------------
    // 3. reader_skip_seqs_filtered
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn reader_skip_seqs_filtered() {
        // 5 rows; skip seq 1 and 3 → 3 rows with seqs 0/2/4
        let f = temp_csv("id,val\n0,a\n1,b\n2,c\n3,d\n4,e\n");
        let input: Box<dyn InputStream> =
            Box::new(CsvInputStream::open(f.path(), &[]).unwrap());

        let config = ReaderConfig {
            skip_seqs: [1u64, 3u64].iter().cloned().collect(),
            ..default_config()
        };

        let (tx, mut rx) = mpsc::channel::<RowJob>(16);
        let result = reader_task(input, config, tx, None).await;
        assert!(result.is_ok());

        let mut seqs: Vec<u64> = Vec::new();
        while let Ok(j) = rx.try_recv() {
            seqs.push(j.seq);
        }

        assert_eq!(seqs, vec![0, 2, 4], "expected seqs [0,2,4], got {:?}", seqs);
    }

    // -----------------------------------------------------------------------
    // 4. reader_row_limit_truncates
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn reader_row_limit_truncates() {
        // 10 rows; row_limit=3 → exactly 3 RowJobs
        let mut csv = "n\n".to_string();
        for i in 0..10 {
            csv.push_str(&format!("{i}\n"));
        }
        let f = temp_csv(&csv);
        let input: Box<dyn InputStream> =
            Box::new(CsvInputStream::open(f.path(), &[]).unwrap());

        let config = ReaderConfig {
            row_limit: Some(3),
            ..default_config()
        };

        let (tx, mut rx) = mpsc::channel::<RowJob>(16);
        let result = reader_task(input, config, tx, None).await;
        assert!(result.is_ok());

        let mut jobs = Vec::new();
        while let Ok(j) = rx.try_recv() {
            jobs.push(j);
        }

        assert_eq!(jobs.len(), 3, "expected exactly 3 RowJobs, got {}", jobs.len());
    }

    // -----------------------------------------------------------------------
    // 5. reader_field_map_renames_keys
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn reader_field_map_renames_keys() {
        // Input row {a:1, b:2}; field_map = {a → x}; result should be {x:1, b:2}
        let f = temp_csv("a,b\n1,2\n");
        let input: Box<dyn InputStream> =
            Box::new(CsvInputStream::open(f.path(), &[]).unwrap());

        let mut field_map = BTreeMap::new();
        field_map.insert("a".to_string(), "x".to_string());

        let config = ReaderConfig {
            field_map,
            ..default_config()
        };

        let (tx, mut rx) = mpsc::channel::<RowJob>(4);
        let result = reader_task(input, config, tx, None).await;
        assert!(result.is_ok());

        let job = rx.try_recv().expect("expected one RowJob");
        assert!(
            job.data.contains_key("x"),
            "expected key 'x' in data, got: {:?}",
            job.data.keys().collect::<Vec<_>>()
        );
        assert!(
            job.data.contains_key("b"),
            "expected key 'b' still present, got: {:?}",
            job.data.keys().collect::<Vec<_>>()
        );
        assert!(
            !job.data.contains_key("a"),
            "key 'a' should have been renamed, but still found"
        );
        assert_eq!(
            job.data.get("x").unwrap(),
            &serde_json::Value::String("1".into())
        );
        assert_eq!(
            job.data.get("b").unwrap(),
            &serde_json::Value::String("2".into())
        );
    }

    // -----------------------------------------------------------------------
    // 6. reader_cancel_before_loop
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn reader_cancel_before_loop() {
        // Pre-cancel the token; reader_task should return Ok with zero rows sent.
        let f = temp_csv("id,val\n0,a\n1,b\n2,c\n");
        let input: Box<dyn InputStream> =
            Box::new(CsvInputStream::open(f.path(), &[]).unwrap());

        let token = CancellationToken::new();
        token.cancel(); // pre-cancel

        let (tx, mut rx) = mpsc::channel::<RowJob>(16);
        let result = reader_task(input, default_config(), tx, Some(token)).await;
        assert!(result.is_ok(), "expected Ok, got {:?}", result);

        // No rows should have been sent.
        assert!(
            rx.try_recv().is_err(),
            "expected zero rows sent after pre-cancel"
        );
    }

    // -----------------------------------------------------------------------
    // 7. reader_cancel_unblocks_full_channel  (acceptance criterion §16)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn reader_cancel_unblocks_full_channel() {
        use std::time::Duration;
        use tokio::time::timeout;

        // Build a CSV with 100 rows.
        let mut csv = "n\n".to_string();
        for i in 0..100 {
            csv.push_str(&format!("{i}\n"));
        }
        let f = temp_csv(&csv);
        let input: Box<dyn InputStream> =
            Box::new(CsvInputStream::open(f.path(), &[]).unwrap());

        // Small channel capacity so the reader blocks after 2 rows.
        let (tx, rx) = mpsc::channel::<RowJob>(2);
        let token = CancellationToken::new();
        let token_clone = token.clone();

        let handle = tokio::spawn(async move {
            reader_task(input, default_config(), tx, Some(token_clone)).await
        });

        // Do NOT read from rx — let the channel fill up so reader blocks.
        // Give the reader a moment to fill the channel and block on send.
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Fire cancellation.
        token.cancel();

        // Reader must unblock and complete within 500 ms.
        let task_result = timeout(Duration::from_millis(500), handle)
            .await
            .expect("reader_task did not complete within 500ms after cancel")
            .expect("reader_task JoinError");

        // Cancel is not an error path for reader.
        assert!(
            task_result.is_ok(),
            "reader_task should return Ok on cancel, got: {:?}",
            task_result
        );

        // Keep rx alive so the channel isn't prematurely dropped before cancel fires.
        drop(rx);
    }
}
