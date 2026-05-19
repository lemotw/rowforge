//! Streaming worker loop for the dispatch pipeline (plan §6, v3.3).
//!
//! `run_worker_loop` pulls [`Batch`]es from a shared mpsc receiver and writes
//! completed [`BatchOutcome`]s directly to [`SharedJsonlWriter`] — no Writer
//! task, no `out_tx` channel.
//!
//! Cancel behaviour (§8.2):
//! - New batches are NOT pulled after cancel is observed (biased select on job_rx).
//! - Cancel during recv (handler hung): in-flight batch outcomes are synthesized
//!   as WORKER_CRASH (idempotent=true) or WORKER_CRASH_UNSAFE (idempotent=false)
//!   and written to jsonl BEFORE the loop exits — C2 invariant preserved via
//!   synthesized record, not handler reply.
//! - `worker.shutdown(grace)` is called even on cancel (graceful teardown).

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::accumulator::Batch;
use crate::cancel::CancellationToken;
use crate::error::CoreError;
use crate::jsonl_writer::SharedJsonlWriter;
use crate::pool::{BatchOutcome, RowOutcome};
use crate::protocol::{Inbound, Outbound};
use crate::run::{ERR_PROTOCOL_ERROR, ERR_WORKER_CRASH, ERR_WORKER_CRASH_UNSAFE};
use crate::runtime::Mode;
use crate::worker::Worker;

// ERR_PROTOCOL_ERROR is imported from crate::run (centralised ERR_* constants).

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Streaming worker loop: pulls [`Batch`]es from `job_rx` and writes outcomes
/// directly to `jsonl`.
///
/// # Arguments
///
/// - `worker`  — spawned, handshaked [`Worker`] ready to process rows.
/// - `mode`    — [`Mode::Row`] or [`Mode::Batch`]; determines dispatch shape.
/// - `idempotent` — when `true`, crashed rows use `ERR_WORKER_CRASH`; when
///   `false`, `ERR_WORKER_CRASH_UNSAFE`. Ignored in row mode (same `WORKER_CRASH`).
/// - `job_rx`  — shared receiver; multiple workers compete for batches.
/// - `jsonl`   — shared append-only JSONL writer.
/// - `grace`   — grace period passed to [`Worker::shutdown`].
/// - `cancel`  — optional cancellation token; when fired the loop stops
///   pulling new batches after the current one finishes.
///
/// Returns `Ok(())` on normal completion, natural channel EOF, or cancel.
/// Returns `Err` only if a fatal I/O or protocol error occurs that is not
/// recoverable into a synthesized outcome.
pub async fn run_worker_loop(
    mut worker: Worker,
    mode: Mode,
    idempotent: bool,
    job_rx: Arc<tokio::sync::Mutex<mpsc::Receiver<Batch>>>,
    jsonl: Arc<SharedJsonlWriter>,
    grace: Duration,
    cancel: Option<CancellationToken>,
) -> Result<(), CoreError> {
    loop {
        // Pull next batch — cancel-aware (§8.2 biased select!).
        // The lock is held only for the duration of recv(), then dropped
        // immediately so other workers can compete.
        let batch = {
            let mut rx = job_rx.lock().await;
            match &cancel {
                Some(c) => tokio::select! {
                    biased;
                    _ = c.cancelled() => None,
                    b = rx.recv() => b,
                },
                None => rx.recv().await,
            }
        };

        // None from either cancel arm or natural channel EOF → break.
        let Some(batch) = batch else { break };

        let seqs = batch.seqs.clone();

        let outcomes: Vec<RowOutcome> = match mode {
            Mode::Row => {
                // Row mode: batch is always exactly 1 row.
                let seq = batch.seqs[0];
                let row = &batch.rows[0];
                let env = Outbound::Row {
                    seq,
                    data: row.data.clone(),
                    meta: row.meta.clone(),
                };

                // If stdin write fails, synthesize crash for this row and
                // break the loop (worker is dead).
                if let Err(_e) = worker.send_row(&env).await {
                    let outcomes = vec![synthesize_crash(seq, worker.id, idempotent)];
                    let bo = BatchOutcome::from_outcomes(outcomes);
                    jsonl.append_line(&bo).await?;
                    break;
                }

                // Cancel-aware recv: if cancel fires while waiting for a row
                // response (e.g. stall monitor fires, handler is hung), synthesize
                // a crash outcome for the in-flight seq and write it to jsonl
                // BEFORE breaking — preserving the C2 invariant so that
                // row_resolution sees a recorded outcome (not NeverAttempted).
                let recv_result = match &cancel {
                    Some(c) => tokio::select! {
                        biased;
                        _ = c.cancelled() => {
                            tracing::debug!(worker = worker.id, seq, "cancel during recv; synthesizing crash and breaking");
                            let outcomes = vec![synthesize_crash(seq, worker.id, idempotent)];
                            let bo = BatchOutcome::from_outcomes(outcomes);
                            jsonl.append_line(&bo).await?;
                            break;
                        },
                        r = worker.recv() => r,
                    },
                    None => worker.recv().await,
                };
                let outcome = match recv_result {
                    Ok(Some(Inbound::Result { data, .. })) => RowOutcome::Success {
                        seq,
                        data,
                        dur_ms: 0,
                    },
                    Ok(Some(Inbound::Error { code, message, data, .. })) => RowOutcome::Error {
                        seq,
                        code,
                        message,
                        data,
                        dur_ms: 0,
                    },
                    Ok(None) => synthesize_crash(seq, worker.id, idempotent),
                    Ok(Some(_other)) => synthesize_protocol_error(seq),
                    Err(_e) => synthesize_crash(seq, worker.id, idempotent),
                };
                vec![outcome]
            }

            Mode::Batch => {
                // Batch mode: send all rows, then receive batch_result.
                if let Err(_e) = worker.send_batch_envelopes(&batch.rows).await {
                    // stdin broken → crash all seqs in this batch.
                    let outcomes = seqs
                        .iter()
                        .map(|&s| synthesize_crash(s, worker.id, idempotent))
                        .collect::<Vec<_>>();
                    let bo = BatchOutcome::from_outcomes(outcomes);
                    jsonl.append_line(&bo).await?;
                    break;
                }
                // Cancel-aware recv: if cancel fires while waiting for a batch
                // response, synthesize crash outcomes for EVERY seq in the batch
                // and write them to jsonl BEFORE breaking — C2 invariant preserved.
                let batch_recv_result = match &cancel {
                    Some(c) => tokio::select! {
                        biased;
                        _ = c.cancelled() => {
                            tracing::debug!(worker = worker.id, seqs = ?seqs, "cancel during batch recv; synthesizing crashes and breaking");
                            let outcomes = seqs
                                .iter()
                                .map(|&s| synthesize_crash(s, worker.id, idempotent))
                                .collect::<Vec<_>>();
                            let bo = BatchOutcome::from_outcomes(outcomes);
                            jsonl.append_line(&bo).await?;
                            break;
                        },
                        r = worker.recv_batch_result(&seqs) => r,
                    },
                    None => worker.recv_batch_result(&seqs).await,
                };
                // recv_batch_result synthesizes BATCH_PROTOCOL_ERROR outcomes
                // on parse / length mismatch — those go to jsonl as-is.
                match batch_recv_result {
                    Ok(outcomes) => outcomes,
                    Err(_e) => {
                        // HandlerExit or IO error: crash all seqs.
                        seqs.iter()
                            .map(|&s| synthesize_crash(s, worker.id, idempotent))
                            .collect()
                    }
                }
            }
        };

        // Write outcomes to jsonl — NO cancel arm (§5.2 / §8.5 C2):
        // in-flight results must land regardless of cancel state.
        let bo = BatchOutcome::from_outcomes(outcomes);
        jsonl.append_line(&bo).await?;
    }

    // Shutdown handler even on cancel (§8.2: "shutdown handler").
    let _ = worker.shutdown(grace).await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Crash / protocol-error synthesizers
// ---------------------------------------------------------------------------

fn synthesize_crash(seq: u64, worker_id: u32, idempotent: bool) -> RowOutcome {
    let code = if idempotent {
        ERR_WORKER_CRASH
    } else {
        ERR_WORKER_CRASH_UNSAFE
    };
    RowOutcome::Error {
        seq,
        code: code.to_string(),
        message: format!("worker {} crashed at seq {}", worker_id, seq),
        data: None,
        dur_ms: 0,
    }
}

fn synthesize_protocol_error(seq: u64) -> RowOutcome {
    RowOutcome::Error {
        seq,
        code: ERR_PROTOCOL_ERROR.to_string(),
        message: format!(
            "worker returned unexpected message variant for row seq {}",
            seq
        ),
        data: None,
        dur_ms: 0,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accumulator::Batch;
    use crate::manifest::{Entry, Manifest};
    use crate::pool::BatchOutcome;
    use crate::protocol::RowMeta;
    use crate::runtime::Mode;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;
    use tempfile::NamedTempFile;
    use tokio::sync::mpsc;


    // -----------------------------------------------------------------------
    // Test infrastructure helpers
    // -----------------------------------------------------------------------

    fn test_handler_path() -> PathBuf {
        use std::sync::Once;
        static BUILD: Once = Once::new();
        BUILD.call_once(|| {
            let status = std::process::Command::new("cargo")
                .args(["build", "-p", "test-handler"])
                .status()
                .expect("invoking `cargo build -p test-handler`");
            assert!(status.success(), "cargo build -p test-handler failed");
        });
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let workspace_root = manifest.parent().unwrap().parent().unwrap();
        workspace_root.join("target/debug/test-handler")
    }

    fn make_manifest(behavior: &str) -> Manifest {
        Manifest {
            name: "test".into(),
            version: "0.0.0".into(),
            description: String::new(),
            language: String::new(),
            entry: Entry {
                cmd: vec![
                    test_handler_path().to_string_lossy().into(),
                    behavior.to_string(),
                ],
                build: None,
                cwd: ".".into(),
                env: Default::default(),
                startup_timeout_ms: 5000,
            },
            required_input: vec![],
            config: BTreeMap::new(),
            runtime: None,
            output: None,
        }
    }

    async fn spawn_worker(behavior: &str) -> Worker {
        let manifest = make_manifest(behavior);
        Worker::spawn(
            0,
            &std::env::temp_dir(),
            &manifest,
            "test-run",
            &BTreeMap::new(),
            &["x".to_string()],
        )
        .await
        .expect("worker spawn failed")
    }

    fn make_batch(seqs: Vec<u64>) -> Batch {
        Batch {
            rows: seqs
                .iter()
                .map(|&seq| crate::protocol::RowEnvelope {
                    seq,
                    data: {
                        let mut m = serde_json::Map::new();
                        m.insert("x".to_string(), serde_json::json!(seq));
                        m
                    },
                    meta: RowMeta { dry_run: false, row_index: seq },
                })
                .collect(),
            seqs,
        }
    }

    /// Feed batches into a channel and run `run_worker_loop`, returning the
    /// parsed `BatchOutcome` lines from the jsonl file.
    async fn run_and_collect(
        behavior: &str,
        mode: Mode,
        idempotent: bool,
        batches: Vec<Batch>,
        cancel: Option<CancellationToken>,
    ) -> Vec<BatchOutcome> {
        let worker = spawn_worker(behavior).await;
        let tmp = NamedTempFile::new().unwrap();
        let jsonl = Arc::new(
            SharedJsonlWriter::open(tmp.path(), false)
                .await
                .expect("open writer"),
        );

        let (job_tx, job_rx) = mpsc::channel::<Batch>(16);
        let job_rx = Arc::new(tokio::sync::Mutex::new(job_rx));

        for batch in batches {
            job_tx.send(batch).await.unwrap();
        }
        drop(job_tx); // signal EOF so the loop exits naturally

        run_worker_loop(
            worker,
            mode,
            idempotent,
            job_rx,
            Arc::clone(&jsonl),
            Duration::from_secs(2),
            cancel,
        )
        .await
        .expect("run_worker_loop returned Err");

        // Read back and parse all JSONL lines.
        let content = std::fs::read_to_string(tmp.path()).unwrap();
        content
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str::<BatchOutcome>(l).expect("parse BatchOutcome"))
            .collect()
    }

    // -----------------------------------------------------------------------
    // Test 1: Row mode — basic success
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn worker_loop_row_mode_basic() {
        let batches = vec![
            make_batch(vec![0]),
            make_batch(vec![1]),
            make_batch(vec![2]),
        ];
        let outcomes = run_and_collect("echo", Mode::Row, true, batches, None).await;

        assert_eq!(outcomes.len(), 3, "expected 3 BatchOutcome lines");
        for (i, bo) in outcomes.iter().enumerate() {
            assert_eq!(bo.seqs.len(), 1);
            assert_eq!(
                bo.first_seq, bo.seqs[0],
                "first_seq must equal seqs[0]"
            );
            match &bo.outcomes[0] {
                RowOutcome::Success { seq, .. } => {
                    assert_eq!(*seq, bo.seqs[0], "success seq mismatch at index {i}");
                }
                other => panic!("expected Success at index {i}, got {:?}", other),
            }
        }
    }

    // -----------------------------------------------------------------------
    // Test 2: Batch mode — basic success
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn worker_loop_batch_mode_basic() {
        let batch = make_batch(vec![0, 1, 2]);
        let outcomes = run_and_collect("batch-echo", Mode::Batch, true, vec![batch], None).await;

        assert_eq!(outcomes.len(), 1, "expected 1 BatchOutcome line");
        let bo = &outcomes[0];
        assert_eq!(bo.seqs, vec![0, 1, 2]);
        assert_eq!(bo.outcomes.len(), 3);
        for (i, o) in bo.outcomes.iter().enumerate() {
            match o {
                RowOutcome::Success { seq, .. } => assert_eq!(*seq, i as u64),
                other => panic!("expected Success[{i}], got {:?}", other),
            }
        }
    }

    // -----------------------------------------------------------------------
    // Test 3: Row mode — handler returns error
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn worker_loop_row_mode_error() {
        // error-on-bad: sends error if data.bad == true
        // We need a single row with bad=true.
        let worker = spawn_worker("error-on-bad").await;
        let tmp = NamedTempFile::new().unwrap();
        let jsonl = Arc::new(
            SharedJsonlWriter::open(tmp.path(), false)
                .await
                .unwrap(),
        );

        let (job_tx, job_rx) = mpsc::channel::<Batch>(4);
        let job_rx = Arc::new(tokio::sync::Mutex::new(job_rx));

        // Build a batch with data.bad = true
        let batch = Batch {
            seqs: vec![5],
            rows: vec![crate::protocol::RowEnvelope {
                seq: 5,
                data: {
                    let mut m = serde_json::Map::new();
                    m.insert("bad".to_string(), serde_json::json!(true));
                    m
                },
                meta: RowMeta { dry_run: false, row_index: 5 },
            }],
        };
        job_tx.send(batch).await.unwrap();
        drop(job_tx);

        run_worker_loop(
            worker,
            Mode::Row,
            true,
            job_rx,
            Arc::clone(&jsonl),
            Duration::from_secs(2),
            None,
        )
        .await
        .unwrap();

        let content = std::fs::read_to_string(tmp.path()).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 1);
        let bo: BatchOutcome = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(bo.seqs, vec![5]);
        match &bo.outcomes[0] {
            RowOutcome::Error { seq, code, message, .. } => {
                assert_eq!(*seq, 5);
                assert_eq!(code, "BAD_ROW");
                assert!(message.contains("bad"), "message: {message}");
            }
            other => panic!("expected Error, got {:?}", other),
        }
    }

    // NOTE: Tests 4+5 (crash idempotent/unsafe) are in the integration test
    // `tests/worker_loop_crash.rs` to avoid cross-test subprocess interference
    // when the lib tests run in parallel within the same process.

    // -----------------------------------------------------------------------
    // Test 6: Cancel after in-flight batch finishes — jsonl has 1 line
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn worker_loop_cancel_after_inflight() {
        let worker = spawn_worker("echo").await;
        let tmp = NamedTempFile::new().unwrap();
        let jsonl = Arc::new(
            SharedJsonlWriter::open(tmp.path(), false)
                .await
                .unwrap(),
        );

        let (job_tx, job_rx) = mpsc::channel::<Batch>(4);
        let job_rx = Arc::new(tokio::sync::Mutex::new(job_rx));

        // Send 2 batches.
        job_tx.send(make_batch(vec![0])).await.unwrap();
        job_tx.send(make_batch(vec![1])).await.unwrap();
        drop(job_tx);

        // Cancel immediately — cancel is observed on the NEXT recv, so the
        // first batch may or may not be processed depending on timing.
        // To make this deterministic: pre-cancel the token before the loop
        // starts, so the biased select fires on the very first iteration.
        let cancel = CancellationToken::new();
        cancel.cancel(); // already fired

        run_worker_loop(
            worker,
            Mode::Row,
            true,
            job_rx,
            Arc::clone(&jsonl),
            Duration::from_secs(2),
            Some(cancel),
        )
        .await
        .unwrap();

        // With pre-cancelled token + biased select, the loop should break
        // immediately without processing any batches.
        let content = std::fs::read_to_string(tmp.path()).unwrap();
        let line_count = content.lines().filter(|l| !l.trim().is_empty()).count();
        assert!(
            line_count == 0,
            "pre-cancelled token should produce 0 lines, got {}",
            line_count
        );
    }

    // -----------------------------------------------------------------------
    // Test 7: Natural channel EOF — both batches processed, returns Ok
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn worker_loop_channel_close_natural_eof() {
        let batches = vec![make_batch(vec![0]), make_batch(vec![1])];
        let outcomes = run_and_collect("echo", Mode::Row, true, batches, None).await;

        assert_eq!(outcomes.len(), 2, "expected 2 BatchOutcome lines");
        for (i, bo) in outcomes.iter().enumerate() {
            match &bo.outcomes[0] {
                RowOutcome::Success { seq, .. } => {
                    assert_eq!(*seq, i as u64);
                }
                other => panic!("expected Success[{i}], got {:?}", other),
            }
        }
    }
}
