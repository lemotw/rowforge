// Migrated from run_pool (P11): rewritten to use run_pool_streaming.
// Handler returns `batch_result` with one fewer entry than the dispatched
// batch. The worker's bijection validation must synthesize a
// `BATCH_PROTOCOL_ERROR` for every row in the offending batch.

use rowforge_core::input_stream::CsvInputStream;
use rowforge_core::manifest::{Entry, Manifest};
use rowforge_core::pool::{BatchOutcome, RowOutcome};
use rowforge_core::pool_streaming::{run_pool_streaming, StreamingPoolConfig};
use rowforge_core::runtime::{Mode, Runtime};
use std::collections::{BTreeMap, HashSet};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;

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

fn batch_short_manifest() -> Arc<Manifest> {
    Arc::new(Manifest {
        name: "batch-short".into(),
        version: "0.0.0".into(),
        description: String::new(),
        language: String::new(),
        entry: Entry {
            cmd: vec![
                test_handler_path().to_string_lossy().into(),
                "batch-short".into(),
            ],
            build: None,
            cwd: ".".into(),
            env: Default::default(),
            startup_timeout_ms: 5000,
        },
        required_input: vec![],
        config: BTreeMap::new(),
        runtime: Some(Runtime {
            mode: Mode::Batch,
            batch_size: Some(5),
            max_batch_bytes: 16 * 1024 * 1024,
            batch_bytes_target: 4 * 1024 * 1024,
            idempotent: Some(true),
            stateful: false,
        }),
        output: None,
    })
}

fn write_csv(dir: &TempDir, name: &str, content: &str) -> PathBuf {
    let path = dir.path().join(name);
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(content.as_bytes()).unwrap();
    path
}

fn collect_jsonl_outcomes(path: &std::path::Path) -> Vec<RowOutcome> {
    let content = std::fs::read_to_string(path).unwrap_or_default();
    let mut outcomes = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let bo: BatchOutcome = serde_json::from_str(trimmed).unwrap();
        outcomes.extend(bo.outcomes);
    }
    outcomes
}

#[tokio::test]
async fn length_mismatch_yields_batch_protocol_error_per_row() {
    let dir = tempfile::tempdir().unwrap();
    // Exactly one full batch of 5 rows.
    let csv_path = write_csv(&dir, "input.csv", "x\nv0\nv1\nv2\nv3\nv4\n");

    let manifest = batch_short_manifest();
    let runtime = manifest.runtime.clone().unwrap();

    let cfg = StreamingPoolConfig {
        handler_dir: std::env::temp_dir(),
        manifest,
        workers: 1,
        run_id: "test-batch-short".into(),
        config: BTreeMap::new(),
        shutdown_grace: Duration::from_secs(2),
        cancel: None,
        runtime,
        jsonl_path: dir.path().join("outcomes.jsonl"),
        fsync_outcomes: false,
        stall_timeout: Some(Duration::from_secs(5)),
        stall_poll_interval: Some(Duration::from_millis(100)),
        on_row_done: None,
    };

    let input = Box::new(CsvInputStream::open(&csv_path, &[]).unwrap());
    let _report = run_pool_streaming(
        input,
        HashSet::new(),
        None,
        BTreeMap::new(),
        false,
        cfg,
    )
    .await
    .unwrap();

    let jsonl_path = dir.path().join("outcomes.jsonl");
    let outcomes = collect_jsonl_outcomes(&jsonl_path);
    assert_eq!(outcomes.len(), 5, "one outcome per input row");

    let mut seqs_seen = std::collections::BTreeSet::new();
    for o in &outcomes {
        match o {
            RowOutcome::Error { seq, code, message, .. } => {
                assert_eq!(
                    code, "BATCH_PROTOCOL_ERROR",
                    "all rows in misbehaving batch must report BATCH_PROTOCOL_ERROR; got {} ({})",
                    code, message
                );
                seqs_seen.insert(*seq);
            }
            other => panic!("expected Error outcome, got {:?}", other),
        }
    }
    assert_eq!(
        seqs_seen,
        (0..5u64).collect(),
        "every dispatched seq must appear exactly once"
    );
}
