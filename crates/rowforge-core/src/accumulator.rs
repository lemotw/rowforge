//! Streaming Accumulator task for the dispatch pipeline (plan §5, v3.3).
//!
//! `accumulator_task` drains [`RowJob`]s from `row_rx`, groups them into
//! [`Batch`]es respecting three-tier byte caps, and sends each completed
//! [`Batch`] to `job_tx` for a worker to consume. Oversized rows (exceeding
//! [`ROW_HARD_CAP_BYTES`]) bypass the batch entirely and are written directly
//! to `jsonl` as a synthesized [`ROW_TOO_LARGE`] outcome.
//!
//! Cancel behaviour (v3.3 §8.2 方案 A):
//! - When `cancel` fires, pending rows are **dropped** (not dispatched, not
//!   written to jsonl). The next attempt's `compute_resolution` will treat
//!   those seqs as `NeverAttempted` and re-ingest them.
//! - The flush helper is cancel-aware (v3.2): a `tokio::select!` cancel arm
//!   prevents the channel-full + cancel deadlock scenario (§8.4 C6).

use std::sync::Arc;

use tokio::sync::mpsc;
use tracing;

use crate::cancel::CancellationToken;
use crate::error::CoreError;
use crate::jsonl_writer::SharedJsonlWriter;
use crate::pool::{BatchOutcome, RowJob, RowOutcome};
use crate::protocol::{RowEnvelope, RowMeta};
use crate::run::ERR_ROW_TOO_LARGE;
use crate::runtime::{Mode, ROW_HARD_CAP_BYTES, Runtime};

// ---------------------------------------------------------------------------
// JOB_CHANNEL_CAP
// ---------------------------------------------------------------------------

/// Capacity of the bounded `job_tx` / `job_rx` channel between the Accumulator
/// task and the Worker(s) (plan §11).
pub const JOB_CHANNEL_CAP: usize = 64;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Reason that triggered a batch flush from the accumulator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlushReason {
    /// Row count hit `max_count` (normal path).
    CountCap,
    /// Accumulated bytes would exceed the *soft* target (`batch_bytes_target`).
    SoftByteCap,
    /// Accumulated bytes would exceed the *hard* maximum (`max_batch_bytes`).
    HardByteCap,
    /// `row_rx` closed (natural EOF after the Reader finishes).
    Eof,
}

/// A unit of work sent from the Accumulator to a Worker.
///
/// `seqs` and `rows` are positionally aligned: `seqs[i]` is the seq number of
/// `rows[i]`. Both are strictly ascending within a `Batch` (invariant I2).
///
/// Note: this is the streaming-pipeline type for the dispatch pipeline.
/// `pool::BatchJob` (`Vec<RowJob>`) and `run_pool`/`accumulate_batches` were
/// removed in P11; all callers now use `Batch` and `accumulator_task`.
#[derive(Debug)]
pub struct Batch {
    pub seqs: Vec<u64>,
    pub rows: Vec<RowEnvelope>,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Accumulator task: pulls [`RowJob`]s, groups them into [`Batch`]es, handles
/// oversized rows, and forwards batches to workers via `job_tx`.
///
/// Returns `Ok(())` on natural EOF or after a cancel-triggered drop. Returns
/// `Err` if the downstream channel is dropped unexpectedly or if a flush is
/// interrupted in an unrecoverable way.
pub async fn accumulator_task(
    runtime: Runtime,
    mut row_rx: mpsc::Receiver<RowJob>,
    job_tx: mpsc::Sender<Batch>,
    jsonl: Arc<SharedJsonlWriter>,
    cancel: Option<CancellationToken>,
) -> Result<(), CoreError> {
    let max_count: usize = match runtime.mode {
        Mode::Row => 1,
        Mode::Batch => runtime.batch_size.unwrap_or(100) as usize,
    };
    let bytes_target: u64 = runtime.batch_bytes_target;
    let bytes_max: u64 = runtime.max_batch_bytes;

    let mut pending_seqs: Vec<u64> = Vec::new();
    let mut pending_rows: Vec<RowEnvelope> = Vec::new();
    let mut pending_bytes: u64 = 0;
    let mut downsized_batches: u32 = 0;
    let mut warned_once = false;
    let mut exited_by_cancel = false;

    loop {
        // Cancel-aware receive (v3.2 §8.4 C6).
        let job_opt = match &cancel {
            Some(c) => tokio::select! {
                biased;
                _ = c.cancelled() => { exited_by_cancel = true; None }
                j = row_rx.recv() => j,
            },
            None => row_rx.recv().await,
        };

        let Some(job) = job_opt else { break };

        let row_bytes = estimate_size(&job);

        // (1) Per-row hard cap: oversized rows bypass the batch entirely.
        // We do NOT wrap this await in a cancel arm (§5.2): in-flight
        // synthesized outcomes must be written to satisfy C2/C3.
        if row_bytes > ROW_HARD_CAP_BYTES {
            let outcome = synthesize_row_too_large(job.seq, row_bytes);
            let bo = BatchOutcome::from_outcomes(vec![outcome]);
            jsonl.append_line(&bo).await?;
            continue;
        }

        // (2) Per-batch hard cap: flush before this row would overflow.
        if !pending_rows.is_empty() && pending_bytes + row_bytes > bytes_max {
            flush(
                FlushReason::HardByteCap,
                &job_tx,
                &mut pending_seqs,
                &mut pending_rows,
                &mut pending_bytes,
                &cancel,
                &mut downsized_batches,
                &mut warned_once,
            )
            .await?;
        }
        // (3) Per-batch soft cap: flush before this row would exceed target.
        else if !pending_rows.is_empty() && pending_bytes + row_bytes > bytes_target {
            flush(
                FlushReason::SoftByteCap,
                &job_tx,
                &mut pending_seqs,
                &mut pending_rows,
                &mut pending_bytes,
                &cancel,
                &mut downsized_batches,
                &mut warned_once,
            )
            .await?;
        }

        let envelope = envelope_of(&job);
        pending_seqs.push(job.seq);
        pending_rows.push(envelope);
        pending_bytes += row_bytes;

        // (4) Row count cap.
        if pending_rows.len() >= max_count {
            flush(
                FlushReason::CountCap,
                &job_tx,
                &mut pending_seqs,
                &mut pending_rows,
                &mut pending_bytes,
                &cancel,
                &mut downsized_batches,
                &mut warned_once,
            )
            .await?;
        }
    }

    // Post-loop handling.
    if !exited_by_cancel && !pending_rows.is_empty() {
        // Natural EOF: flush remainder.
        flush(
            FlushReason::Eof,
            &job_tx,
            &mut pending_seqs,
            &mut pending_rows,
            &mut pending_bytes,
            &cancel,
            &mut downsized_batches,
            &mut warned_once,
        )
        .await?;
    } else if exited_by_cancel && !pending_rows.is_empty() {
        // Cancel path (v3.3 §8.2 方案 A): drop pending, do NOT dispatch.
        tracing::info!(
            dropped_pending = pending_rows.len(),
            dropped_seqs_first = pending_seqs.first(),
            dropped_seqs_last = pending_seqs.last(),
            "cancel: dropping {} pending rows (seqs {}..={}); next attempt will re-ingest as NeverAttempted",
            pending_rows.len(),
            pending_seqs.first().copied().unwrap_or(0),
            pending_seqs.last().copied().unwrap_or(0),
        );
    }

    if downsized_batches > 0 {
        tracing::warn!(
            downsized_batches,
            bytes_target,
            "{} batch(es) flushed early due to batch_bytes_target ({} bytes)",
            downsized_batches,
            bytes_target,
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Flush helper
// ---------------------------------------------------------------------------

/// Flush all pending rows as a single [`Batch`] to `job_tx`.
///
/// Cancel-aware (v3.2): when `cancel` fires while `job_tx` is full, the send
/// is abandoned and `Err(CoreError::Store("flush cancelled"))` is returned
/// instead of deadlocking.
#[allow(clippy::too_many_arguments)]
async fn flush(
    reason: FlushReason,
    job_tx: &mpsc::Sender<Batch>,
    pending_seqs: &mut Vec<u64>,
    pending_rows: &mut Vec<RowEnvelope>,
    pending_bytes: &mut u64,
    cancel: &Option<CancellationToken>,
    downsized_batches: &mut u32,
    warned_once: &mut bool,
) -> Result<(), CoreError> {
    if matches!(reason, FlushReason::SoftByteCap) {
        *downsized_batches += 1;
        if !*warned_once {
            tracing::warn!(
                reason = ?reason,
                bytes = *pending_bytes,
                "batch flushed early due to batch_bytes_target"
            );
            *warned_once = true;
        }
    }

    let batch = Batch {
        seqs: std::mem::take(pending_seqs),
        rows: std::mem::take(pending_rows),
    };
    *pending_bytes = 0;

    match cancel {
        Some(c) => {
            // Capture seqs before moving batch into the select! send arm.
            let dropped_seqs = batch.seqs.clone();
            tokio::select! {
                biased;
                _ = c.cancelled() => {
                    // Cancel fired while waiting to send to workers. The batch
                    // is dropped here — per v3.3 §8.2 方案 A, pending rows are
                    // discarded on cancel and will be re-ingested as NeverAttempted
                    // on the next attempt.
                    tracing::info!(
                        dropped_seqs = ?dropped_seqs,
                        "cancel: dropping {} rows mid-flush (seqs {:?}); next attempt re-ingest",
                        dropped_seqs.len(),
                        dropped_seqs,
                    );
                    Err(CoreError::Store("flush cancelled".into()))
                },
                r = job_tx.send(batch) => r.map_err(|_| CoreError::Store("worker dropped".into())),
            }
        },
        None => job_tx
            .send(batch)
            .await
            .map_err(|_| CoreError::Store("worker dropped".into())),
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn envelope_of(job: &RowJob) -> RowEnvelope {
    RowEnvelope {
        seq: job.seq,
        data: job.data.clone(),
        meta: RowMeta {
            dry_run: job.meta.dry_run,
            row_index: job.meta.row_index,
        },
    }
}

fn estimate_size(job: &RowJob) -> u64 {
    serde_json::to_vec(&envelope_of(job))
        .map(|v| v.len() as u64)
        .unwrap_or(0)
}

fn synthesize_row_too_large(seq: u64, row_bytes: u64) -> RowOutcome {
    RowOutcome::Error {
        seq,
        code: ERR_ROW_TOO_LARGE.to_string(),
        message: format!(
            "serialized row size {} bytes exceeds hard cap {} bytes",
            row_bytes, ROW_HARD_CAP_BYTES
        ),
        dur_ms: 0,
        data: None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jsonl_writer::SharedJsonlWriter;
    use crate::runtime::Mode;
    use std::sync::Arc;
    use tempfile::NamedTempFile;
    use tokio::sync::mpsc;

    /// Build a minimal `Runtime` for tests.
    fn row_runtime() -> Runtime {
        Runtime {
            mode: Mode::Row,
            batch_size: None,
            max_batch_bytes: 16 * 1024 * 1024,
            batch_bytes_target: 4 * 1024 * 1024,
            idempotent: None,
            stateful: false,
        }
    }

    fn batch_runtime(batch_size: u32) -> Runtime {
        Runtime {
            mode: Mode::Batch,
            batch_size: Some(batch_size),
            max_batch_bytes: 16 * 1024 * 1024,
            batch_bytes_target: 4 * 1024 * 1024,
            idempotent: Some(true),
            stateful: false,
        }
    }

    fn make_job(seq: u64, payload_bytes: usize) -> RowJob {
        let mut data = serde_json::Map::new();
        // A string value of the given approximate size.
        data.insert("x".to_string(), serde_json::Value::String("a".repeat(payload_bytes)));
        RowJob {
            seq,
            data,
            meta: RowMeta { dry_run: false, row_index: seq },
        }
    }

    async fn open_writer(f: &NamedTempFile) -> Arc<SharedJsonlWriter> {
        Arc::new(SharedJsonlWriter::open(f.path(), false).await.unwrap())
    }

    // -----------------------------------------------------------------------
    // Test 1: Row mode — each job → 1-row Batch
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn accumulator_basic_count_cap() {
        let (row_tx, row_rx) = mpsc::channel(16);
        let (job_tx, mut job_rx) = mpsc::channel(16);
        let tmp = NamedTempFile::new().unwrap();
        let jsonl = open_writer(&tmp).await;

        for seq in 0..5 {
            row_tx.send(make_job(seq, 10)).await.unwrap();
        }
        drop(row_tx);

        accumulator_task(row_runtime(), row_rx, job_tx, jsonl, None)
            .await
            .unwrap();

        let mut batches = Vec::new();
        while let Ok(b) = job_rx.try_recv() {
            batches.push(b);
        }
        assert_eq!(batches.len(), 5, "expected 5 single-row batches");
        for (i, b) in batches.iter().enumerate() {
            assert_eq!(b.seqs, vec![i as u64]);
            assert_eq!(b.rows.len(), 1);
        }
    }

    // -----------------------------------------------------------------------
    // Test 2: Batch mode — count cap groups correctly
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn accumulator_basic_batch_count_cap() {
        let (row_tx, row_rx) = mpsc::channel(32);
        let (job_tx, mut job_rx) = mpsc::channel(32);
        let tmp = NamedTempFile::new().unwrap();
        let jsonl = open_writer(&tmp).await;

        for seq in 0..7 {
            row_tx.send(make_job(seq, 10)).await.unwrap();
        }
        drop(row_tx);

        accumulator_task(batch_runtime(3), row_rx, job_tx, jsonl, None)
            .await
            .unwrap();

        let mut batches = Vec::new();
        while let Ok(b) = job_rx.try_recv() {
            batches.push(b);
        }
        assert_eq!(batches.len(), 3, "expected 3 batches: [3, 3, 1]");
        assert_eq!(batches[0].seqs, vec![0, 1, 2]);
        assert_eq!(batches[1].seqs, vec![3, 4, 5]);
        assert_eq!(batches[2].seqs, vec![6]);
    }

    // -----------------------------------------------------------------------
    // Test 3: Oversized row → direct jsonl write, normal rows → Batch
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn accumulator_oversized_row_direct_jsonl() {
        let (row_tx, row_rx) = mpsc::channel(16);
        let (job_tx, mut job_rx) = mpsc::channel(16);
        let tmp = NamedTempFile::new().unwrap();
        let jsonl = open_writer(&tmp).await;

        // oversized: ~4.1 MiB payload — ensure serialized size > 4 MiB
        let oversized_payload = 4 * 1024 * 1024 + 1024; // 4 MiB + 1 KiB
        row_tx.send(make_job(0, oversized_payload)).await.unwrap();
        row_tx.send(make_job(1, 10)).await.unwrap();
        row_tx.send(make_job(2, 10)).await.unwrap();
        drop(row_tx);

        let rt = batch_runtime(100);
        accumulator_task(rt, row_rx, job_tx, jsonl, None)
            .await
            .unwrap();

        // job_tx should have received 1 Batch with 2 normal rows.
        let mut batches = Vec::new();
        while let Ok(b) = job_rx.try_recv() {
            batches.push(b);
        }
        assert_eq!(batches.len(), 1, "expected 1 batch (2 normal rows)");
        assert_eq!(batches[0].seqs, vec![1, 2]);

        // outcomes.jsonl should have 1 line (the oversized ROW_TOO_LARGE outcome).
        let content = std::fs::read_to_string(tmp.path()).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 1, "expected 1 jsonl line for oversized row");
        let bo: BatchOutcome = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(bo.seqs, vec![0]);
        match &bo.outcomes[0] {
            RowOutcome::Error { code, .. } => {
                assert_eq!(code, ERR_ROW_TOO_LARGE);
            }
            other => panic!("expected Error outcome, got {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // Test 4: Soft cap triggers early flush + warns
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn accumulator_soft_cap_warns_once() {
        // batch_size=100 but each row ~2.1 MiB → 2 rows = ~4.2 MiB > 4 MiB target
        let (row_tx, row_rx) = mpsc::channel(64);
        let (job_tx, mut job_rx) = mpsc::channel(64);
        let tmp = NamedTempFile::new().unwrap();
        let jsonl = open_writer(&tmp).await;

        // Each row slightly over 2 MiB so two rows together exceed the 4 MiB target.
        let row_payload = 2 * 1024 * 1024 + 100; // ~2 MiB
        for seq in 0..6u64 {
            row_tx.send(make_job(seq, row_payload)).await.unwrap();
        }
        drop(row_tx);

        let rt = batch_runtime(100);
        accumulator_task(rt, row_rx, job_tx, jsonl, None)
            .await
            .unwrap();

        let mut batches = Vec::new();
        while let Ok(b) = job_rx.try_recv() {
            batches.push(b);
        }

        // Each batch should have been flushed before reaching 100 rows.
        assert!(
            batches.len() > 1,
            "expected multiple batches due to soft byte cap, got {}",
            batches.len()
        );
        for b in &batches {
            assert!(
                b.rows.len() < 100,
                "batch should be < 100 rows (flushed early), got {}",
                b.rows.len()
            );
        }
        // Verify all rows accounted for.
        let total: usize = batches.iter().map(|b| b.rows.len()).sum();
        assert_eq!(total, 6);
    }

    // -----------------------------------------------------------------------
    // Test 5: Hard cap flush
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn accumulator_hard_cap_flush() {
        let (row_tx, row_rx) = mpsc::channel(64);
        let (job_tx, mut job_rx) = mpsc::channel(64);
        let tmp = NamedTempFile::new().unwrap();
        let jsonl = open_writer(&tmp).await;

        // Set bytes_max to 6 MiB. Each row is ~4 MiB (but < ROW_HARD_CAP).
        // Actually we need rows < ROW_HARD_CAP (4 MiB) but large enough to
        // exceed bytes_max (set to 6 MiB) after 2 rows.
        // Use 3 MiB rows; 2 rows = 6 MiB >= bytes_max → hard cap flush.
        let rt = Runtime {
            mode: Mode::Batch,
            batch_size: Some(100),
            max_batch_bytes: 6 * 1024 * 1024, // 6 MiB hard cap
            batch_bytes_target: 8 * 1024 * 1024, // soft cap above hard cap (won't trigger)
            idempotent: Some(true),
            stateful: false,
        };
        let row_payload = 3 * 1024 * 1024 - 200; // ~3 MiB, below ROW_HARD_CAP
        for seq in 0..4u64 {
            row_tx.send(make_job(seq, row_payload)).await.unwrap();
        }
        drop(row_tx);

        accumulator_task(rt, row_rx, job_tx, jsonl, None)
            .await
            .unwrap();

        let mut batches = Vec::new();
        while let Ok(b) = job_rx.try_recv() {
            batches.push(b);
        }

        // We have 4 rows, each ~3 MiB. bytes_max = 6 MiB. After 2 rows
        // (pending ~6 MiB), the 3rd row would push to ~9 MiB → hard cap flush.
        // So we expect: [2-row batch], [2-row batch] — or similar early flushing.
        assert!(
            batches.len() >= 2,
            "expected hard-cap-triggered early flush, got {} batches",
            batches.len()
        );
        let total: usize = batches.iter().map(|b| b.rows.len()).sum();
        assert_eq!(total, 4);
    }

    // -----------------------------------------------------------------------
    // Test 6: Cancel drops pending, no synth, no batch sent (acceptance #14)
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn accumulator_cancel_drops_pending_no_synth() {
        let (row_tx, row_rx) = mpsc::channel(64);
        let (job_tx, mut job_rx) = mpsc::channel(64);
        let tmp = NamedTempFile::new().unwrap();
        let jsonl = open_writer(&tmp).await;

        let cancel = CancellationToken::new();

        // Feed 7 rows (batch_size=10 so they accumulate without flushing).
        for seq in 0..7u64 {
            row_tx.send(make_job(seq, 10)).await.unwrap();
        }
        // Cancel BEFORE dropping row_tx (rows still "in flight" or buffered).
        cancel.cancel();
        drop(row_tx);

        accumulator_task(batch_runtime(10), row_rx, job_tx, jsonl, Some(cancel))
            .await
            .unwrap();

        // job_tx should be empty — no Batch dispatched.
        assert!(
            job_rx.try_recv().is_err(),
            "expected no batches dispatched after cancel"
        );

        // outcomes.jsonl should be empty.
        let content = std::fs::read_to_string(tmp.path()).unwrap();
        assert!(
            content.trim().is_empty(),
            "expected empty jsonl after cancel-drop, got: {}",
            content
        );
    }

    // -----------------------------------------------------------------------
    // Test 7: Cancel unblocks full job channel (acceptance #13)
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn accumulator_cancel_unblocks_full_job_channel() {
        // job_tx capacity = 1, nobody reads from job_rx.
        let (row_tx, row_rx) = mpsc::channel(64);
        let (job_tx, _job_rx) = mpsc::channel::<Batch>(1); // capacity 1, never drained
        let tmp = NamedTempFile::new().unwrap();
        let jsonl = open_writer(&tmp).await;

        let cancel = CancellationToken::new();

        // batch_size=2 so after 2 rows we flush, which will fill the capacity-1 channel.
        // The 3rd+4th row produce a second flush that will block on the full channel.
        for seq in 0..4u64 {
            row_tx.send(make_job(seq, 10)).await.unwrap();
        }
        drop(row_tx);

        // Spawn the accumulator, then cancel after a tiny delay.
        let cancel_clone = cancel.clone();
        let accumulator_handle = tokio::spawn(async move {
            accumulator_task(batch_runtime(2), row_rx, job_tx, jsonl, Some(cancel_clone)).await
        });

        // Give the task time to start and block on the full channel, then cancel.
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        cancel.cancel();

        // The task should complete within 200ms of the cancel.
        let result = tokio::time::timeout(
            tokio::time::Duration::from_millis(200),
            accumulator_handle,
        )
        .await;

        match result {
            Ok(join_result) => {
                // Either Ok (flushed 1 batch and cancel hit before 2nd) or Err "flush cancelled".
                // Both are acceptable — the important thing is it didn't hang.
                let _ = join_result;
            }
            Err(_elapsed) => panic!("accumulator_task hung after cancel — deadlock not resolved"),
        }
    }

    // -----------------------------------------------------------------------
    // Test 8: EOF flushes partial batch
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn accumulator_eof_flushes_partial_batch() {
        let (row_tx, row_rx) = mpsc::channel(16);
        let (job_tx, mut job_rx) = mpsc::channel(16);
        let tmp = NamedTempFile::new().unwrap();
        let jsonl = open_writer(&tmp).await;

        for seq in 0..4u64 {
            row_tx.send(make_job(seq, 10)).await.unwrap();
        }
        drop(row_tx); // natural EOF

        accumulator_task(batch_runtime(10), row_rx, job_tx, jsonl, None)
            .await
            .unwrap();

        let mut batches = Vec::new();
        while let Ok(b) = job_rx.try_recv() {
            batches.push(b);
        }
        assert_eq!(batches.len(), 1, "expected exactly 1 partial batch on EOF");
        assert_eq!(batches[0].seqs, vec![0, 1, 2, 3]);
        assert_eq!(batches[0].rows.len(), 4);
    }
}
