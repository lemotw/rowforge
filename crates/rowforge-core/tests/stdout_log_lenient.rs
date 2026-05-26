// Migrated from run_pool (P11): pool_run_completes_with_noisy_handler
// rewritten to use run_pool_streaming. Worker-level handshake test retained as-is.
//
// Lenient stdout parsing: handler can print plain-text lines to stdout without
// breaking the wire protocol. Non-JSON lines are treated as log entries while
// protocol JSON lines are still consumed normally.

use rowforge_core::input_stream::CsvInputStream;
use rowforge_core::manifest::{Entry, Manifest};
use rowforge_core::pool::{BatchOutcome, RowOutcome};
use rowforge_core::pool_streaming::{run_pool_streaming, StreamingPoolConfig};
use rowforge_core::protocol::{Inbound, Outbound, RowMeta};
use rowforge_core::runtime::Runtime;
use rowforge_core::worker::Worker;
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

fn noisy_manifest() -> Manifest {
    Manifest {
        name: "echo-noisy".into(),
        version: "0.0.0".into(),
        description: String::new(),
        language: String::new(),
        entry: Entry {
            cmd: vec![
                test_handler_path().to_string_lossy().into(),
                "echo-noisy".into(),
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

// Worker-level test: handshake must succeed despite plain-text lines on stdout.
#[tokio::test]
async fn handshake_skips_non_protocol_stdout_lines() {
    let m = noisy_manifest();
    let mut w = Worker::spawn(0, &std::env::temp_dir(), &m, "t", &BTreeMap::new(), &[])
        .await
        .expect("spawn should succeed despite plain-text lines on stdout");
    assert_eq!(w.handler_version, "0.0.0");

    let row = Outbound::Row {
        seq: 0,
        data: serde_json::Map::from_iter([("x".into(), serde_json::json!("hi"))]),
        meta: RowMeta {
            dry_run: false,
            row_index: 0,
        },
    };
    w.send_row(&row).await.unwrap();
    match w.recv().await.unwrap().unwrap() {
        Inbound::Result { seq, .. } => assert_eq!(seq, 0),
        other => panic!("expected Result, got {:?}", other),
    }
    let _ = w.shutdown(Duration::from_secs(2)).await.unwrap();
}

// Pool-level (streaming) test: every row triggers a plain-text log line on
// stdout BEFORE the result; the streaming pool should still produce N successes.
#[tokio::test]
async fn pool_run_completes_with_noisy_handler() {
    let dir = tempfile::tempdir().unwrap();
    let mut csv = "x\n".to_string();
    for i in 0..10 {
        csv.push_str(&format!("{}\n", i));
    }
    let csv_path = write_csv(&dir, "input.csv", &csv);

    let manifest = Arc::new(noisy_manifest());
    let cfg = StreamingPoolConfig {
        handler_dir: std::env::temp_dir(),
        manifest,
        workers: 2,
        run_id: "t".into(),
        config: BTreeMap::new(),
        shutdown_grace: Duration::from_secs(2),
        cancel: None,
        runtime: Runtime::default(),
        jsonl_path: dir.path().join("outcomes.jsonl"),
        fsync_outcomes: false,
        stall_timeout: None,
        stall_poll_interval: None,
        on_row_done: None,
        on_handler_log: None,
        capture_raw_stdout: false,
        hard_cancel: None,
    };

    let input = Box::new(CsvInputStream::open(&csv_path, &[]).unwrap());
    let report = run_pool_streaming(
        input,
        HashSet::new(),
        None, // only_row_ids
        None,
        BTreeMap::new(),
        false,
        cfg,
    )
    .await
    .unwrap();

    assert!(!report.aborted, "expected not aborted: {:?}", report.abort_reason);

    let outcomes = collect_jsonl_outcomes(&dir.path().join("outcomes.jsonl"));
    let succ = outcomes
        .iter()
        .filter(|o| matches!(o, RowOutcome::Success { .. }))
        .count();
    assert_eq!(succ, 10, "all 10 rows should succeed; outcomes: {:?}", outcomes);
}
