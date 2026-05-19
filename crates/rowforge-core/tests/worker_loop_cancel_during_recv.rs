//! Integration tests for cancel-during-recv crash synthesis (C2 invariant fix).
//!
//! When cancel fires while `run_worker_loop` is blocked waiting for a handler
//! reply (handler is hung), the loop must:
//!   1. Synthesize a WORKER_CRASH (idempotent=true) or WORKER_CRASH_UNSAFE
//!      (idempotent=false) outcome for the in-flight seq(s).
//!   2. Write that synthesized `BatchOutcome` to jsonl BEFORE breaking.
//!   3. Return `Ok(())`.
//!
//! This preserves the C2 invariant: every dispatched seq has a recorded
//! outcome, so `row_resolution` never treats it as NeverAttempted and a
//! non-idempotent row is never silently re-dispatched.
//!
//! Test coverage:
//!   T_ROW_IDEMPOTENT  : row mode, idempotent=true  → WORKER_CRASH
//!   T_ROW_UNSAFE      : row mode, idempotent=false → WORKER_CRASH_UNSAFE
//!   T_BATCH_IDEMPOTENT: batch mode, idempotent=true  → WORKER_CRASH (all seqs)
//!   T_BATCH_UNSAFE    : batch mode, idempotent=false → WORKER_CRASH_UNSAFE (all seqs)

use rowforge_core::accumulator::Batch;
use rowforge_core::cancel::CancellationToken;
use rowforge_core::jsonl_writer::SharedJsonlWriter;
use rowforge_core::manifest::{Entry, Manifest};
use rowforge_core::pool::{BatchOutcome, RowOutcome};
use rowforge_core::protocol::{RowEnvelope, RowMeta};
use rowforge_core::run::{ERR_WORKER_CRASH, ERR_WORKER_CRASH_UNSAFE};
use rowforge_core::runtime::Mode;
use rowforge_core::worker::Worker;
use rowforge_core::worker_loop::run_worker_loop;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tempfile::NamedTempFile;
use tokio::sync::mpsc;

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

fn make_row_batch(seq: u64) -> Batch {
    Batch {
        rows: vec![RowEnvelope {
            seq,
            data: {
                let mut m = serde_json::Map::new();
                m.insert("x".to_string(), serde_json::json!(seq));
                m
            },
            meta: RowMeta { dry_run: false, row_index: seq },
        }],
        seqs: vec![seq],
    }
}

fn make_batch(seqs: Vec<u64>) -> Batch {
    Batch {
        rows: seqs
            .iter()
            .map(|&seq| RowEnvelope {
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

/// Core test helper: spawns a hanging handler, sends one batch, waits briefly
/// for the worker to be past the send step, fires cancel, then asserts:
///   - jsonl has exactly 1 line
///   - that line contains the expected error code for every seq
///   - run_worker_loop returns Ok
async fn run_cancel_during_recv_test(
    behavior: &str,
    mode: Mode,
    idempotent: bool,
    batch: Batch,
    expected_code: &str,
) {
    let worker = spawn_worker(behavior).await;
    let tmp = NamedTempFile::new().unwrap();
    let jsonl = Arc::new(
        SharedJsonlWriter::open(tmp.path(), false)
            .await
            .expect("open writer"),
    );

    let (job_tx, job_rx) = mpsc::channel::<Batch>(4);
    let job_rx = Arc::new(tokio::sync::Mutex::new(job_rx));

    let expected_seqs = batch.seqs.clone();

    // Send exactly one batch, then close the sender.
    job_tx.send(batch).await.unwrap();
    drop(job_tx);

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();

    // Spawn the loop so we can fire cancel concurrently.
    let jsonl_clone = jsonl.clone();
    let loop_handle = tokio::spawn(async move {
        run_worker_loop(
            worker,
            mode,
            idempotent,
            job_rx,
            jsonl_clone,
            Duration::from_secs(2),
            Some(cancel_clone),
        )
        .await
    });

    // Give the worker loop time to:
    //   1. Dequeue the batch from job_rx
    //   2. Send it to the handler (stdin write)
    //   3. Block on recv() waiting for a reply that will never come
    //
    // 100 ms is generous for a local subprocess write; the handler sleeps
    // 3600 s so no spurious reply will arrive before we fire cancel.
    tokio::time::sleep(Duration::from_millis(100)).await;
    cancel.cancel();

    // The loop should return Ok within the shutdown grace period.
    let result = tokio::time::timeout(Duration::from_secs(5), loop_handle)
        .await
        .expect("run_worker_loop did not complete within 5s on cancel")
        .expect("task join error");

    assert!(result.is_ok(), "run_worker_loop returned Err: {:?}", result);

    // Drop writer so any OS-level buffering is flushed before reading.
    drop(jsonl);

    let content = std::fs::read_to_string(tmp.path()).unwrap();
    let lines: Vec<&str> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect();

    assert_eq!(
        lines.len(),
        1,
        "expected exactly 1 jsonl line after cancel-during-recv; got {}. content: {:?}",
        lines.len(),
        content
    );

    let bo: BatchOutcome =
        serde_json::from_str(lines[0]).expect("parse BatchOutcome from jsonl line");

    // Seqs must match.
    assert_eq!(
        bo.seqs, expected_seqs,
        "BatchOutcome.seqs mismatch: expected {:?}, got {:?}",
        expected_seqs, bo.seqs
    );

    // Every outcome must be an Error with the expected crash code.
    for (i, outcome) in bo.outcomes.iter().enumerate() {
        match outcome {
            RowOutcome::Error { seq, code, .. } => {
                assert_eq!(
                    code, expected_code,
                    "outcome[{i}] seq={seq}: expected code {expected_code}, got {code}"
                );
            }
            other => panic!(
                "outcome[{i}] expected Error{{code={expected_code}}}, got {:?}",
                other
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// Row mode tests — both run sequentially in one #[tokio::test] to avoid
// cross-test subprocess interference (same pattern as worker_loop_crash.rs).
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn worker_loop_cancel_during_recv_writes_synth_crash_row() {
    // T_ROW_IDEMPOTENT: idempotent=true → WORKER_CRASH
    run_cancel_during_recv_test(
        "hang-on-first",
        Mode::Row,
        true,
        make_row_batch(42),
        ERR_WORKER_CRASH,
    )
    .await;

    // T_ROW_UNSAFE: idempotent=false → WORKER_CRASH_UNSAFE
    run_cancel_during_recv_test(
        "hang-on-first",
        Mode::Row,
        false,
        make_row_batch(99),
        ERR_WORKER_CRASH_UNSAFE,
    )
    .await;
}

// ---------------------------------------------------------------------------
// Batch mode tests — also sequential in one test function.
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn worker_loop_cancel_during_recv_writes_synth_crash_batch() {
    // T_BATCH_IDEMPOTENT: idempotent=true → WORKER_CRASH for all seqs
    run_cancel_during_recv_test(
        "hang-on-first-batch",
        Mode::Batch,
        true,
        make_batch(vec![10, 11, 12]),
        ERR_WORKER_CRASH,
    )
    .await;

    // T_BATCH_UNSAFE: idempotent=false → WORKER_CRASH_UNSAFE for all seqs
    run_cancel_during_recv_test(
        "hang-on-first-batch",
        Mode::Batch,
        false,
        make_batch(vec![20, 21, 22]),
        ERR_WORKER_CRASH_UNSAFE,
    )
    .await;
}
