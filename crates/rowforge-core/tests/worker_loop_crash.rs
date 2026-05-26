//! Integration tests for `run_worker_loop` crash semantics.
//!
//! Running crash tests in their own binary avoids cross-test interference from
//! parallel in-process execution (SIGCHLD, FD inheritance, etc.).
//!
//! Test coverage:
//!   T4: idempotent=true  → crash outcomes carry ERR_WORKER_CRASH
//!   T5: idempotent=false → crash outcomes carry ERR_WORKER_CRASH_UNSAFE

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
                meta: RowMeta {
                    dry_run: false,
                    row_index: seq,
                },
            })
            .collect(),
        seqs,
    }
}

async fn run_crash_test(idempotent: bool) -> Vec<BatchOutcome> {
    let worker = spawn_worker("crash-on-first").await;
    let tmp = NamedTempFile::new().unwrap();
    let jsonl = Arc::new(
        SharedJsonlWriter::open(tmp.path(), false)
            .await
            .expect("open writer"),
    );

    let (job_tx, job_rx) = mpsc::channel::<Batch>(4);
    let job_rx = Arc::new(tokio::sync::Mutex::new(job_rx));

    job_tx.send(make_batch(vec![0])).await.unwrap();
    drop(job_tx);

    run_worker_loop(
        worker,
        Mode::Row,
        idempotent,
        job_rx,
        Arc::clone(&jsonl),
        Duration::from_secs(2),
        None::<CancellationToken>,
        None,
        None,
    )
    .await
    .expect("run_worker_loop returned Err");

    // Drop the writer before reading so any pending OS flush is complete.
    drop(jsonl);

    let content = std::fs::read_to_string(tmp.path()).unwrap();
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str::<BatchOutcome>(l).expect("parse BatchOutcome"))
        .collect()
}

// T4+T5: Handler crash semantics — idempotent=true and idempotent=false.
//
// Both sub-cases are run sequentially in a single test function so that the
// Worker::spawn / subprocess lifecycle is always serialized.  The
// `multi_thread` flavor ensures tokio's process watcher uses its own
// IO-driver thread rather than a per-test `current_thread` reactor.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn worker_loop_crash_idempotent_true_then_false() {
    // --- T4: idempotent=true → WORKER_CRASH ---
    let outcomes = run_crash_test(true).await;
    assert_eq!(
        outcomes.len(),
        1,
        "T4: expected 1 BatchOutcome; got: {:?}",
        outcomes
    );
    match &outcomes[0].outcomes[0] {
        RowOutcome::Error { seq, code, .. } => {
            assert_eq!(*seq, 0);
            assert_eq!(
                code, ERR_WORKER_CRASH,
                "idempotent=true must produce ERR_WORKER_CRASH"
            );
        }
        other => panic!("T4: expected Error outcome, got {:?}", other),
    }

    // --- T5: idempotent=false → WORKER_CRASH_UNSAFE ---
    let outcomes = run_crash_test(false).await;
    assert_eq!(
        outcomes.len(),
        1,
        "T5: expected 1 BatchOutcome; got: {:?}",
        outcomes
    );
    match &outcomes[0].outcomes[0] {
        RowOutcome::Error { seq, code, .. } => {
            assert_eq!(*seq, 0);
            assert_eq!(
                code, ERR_WORKER_CRASH_UNSAFE,
                "idempotent=false must produce ERR_WORKER_CRASH_UNSAFE"
            );
        }
        other => panic!("T5: expected Error outcome, got {:?}", other),
    }
}
