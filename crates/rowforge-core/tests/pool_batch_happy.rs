// Migrated from run_pool (P11): rewritten to use run_pool_streaming.
// Happy path: handler in batch mode processes 350 rows in batches of 100.
// All 350 outcomes must be Success with `echoed` payload preserved per row.

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

fn batch_echo_manifest() -> Arc<Manifest> {
    Arc::new(Manifest {
        name: "batch-echo".into(),
        version: "0.0.0".into(),
        description: String::new(),
        language: String::new(),
        entry: Entry {
            cmd: vec![
                test_handler_path().to_string_lossy().into(),
                "batch-echo".into(),
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
            batch_size: Some(100),
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
async fn batch_mode_processes_all_rows_with_echoed_payload() {
    let dir = tempfile::tempdir().unwrap();
    let mut csv = "x\n".to_string();
    for i in 0..350 {
        csv.push_str(&format!("v{}\n", i));
    }
    let csv_path = write_csv(&dir, "input.csv", &csv);

    let manifest = batch_echo_manifest();
    let runtime = manifest.runtime.clone().unwrap();

    let cfg = StreamingPoolConfig {
        handler_dir: std::env::temp_dir(),
        manifest,
        workers: 1, // single worker: deterministic 4-batch sequence
        run_id: "test-batch-happy".into(),
        config: BTreeMap::new(),
        shutdown_grace: Duration::from_secs(2),
        cancel: None,
        runtime,
        jsonl_path: dir.path().join("outcomes.jsonl"),
        fsync_outcomes: false,
        stall_timeout: None,
        stall_poll_interval: None,
    };

    let input = Box::new(CsvInputStream::open(&csv_path, &[]).unwrap());
    let report = run_pool_streaming(
        input,
        HashSet::new(),
        None,
        BTreeMap::new(),
        false,
        cfg,
    )
    .await
    .unwrap();

    assert!(
        !report.aborted,
        "expected not aborted: {:?}",
        report.abort_reason
    );

    let jsonl_path = dir.path().join("outcomes.jsonl");
    let outcomes = collect_jsonl_outcomes(&jsonl_path);
    assert_eq!(outcomes.len(), 350, "one outcome per input row");

    // All Success, sorted by seq, each carrying an `echoed` key with the original `x`.
    let mut by_seq: std::collections::BTreeMap<u64, RowOutcome> =
        std::collections::BTreeMap::new();
    for o in outcomes {
        let seq = match &o {
            RowOutcome::Success { seq, .. } => *seq,
            other => panic!("expected Success, got {:?}", other),
        };
        by_seq.insert(seq, o);
    }
    assert_eq!(by_seq.len(), 350, "every seq 0..350 represented exactly once");

    for i in 0..350u64 {
        let o = by_seq.get(&i).expect("missing seq");
        match o {
            RowOutcome::Success { data, .. } => {
                let echoed = data.get("echoed").expect("echoed key present");
                assert_eq!(
                    echoed.get("x").and_then(|v| v.as_str()),
                    Some(format!("v{}", i).as_str()),
                    "echoed.x matches input for seq {}",
                    i
                );
            }
            _ => unreachable!(),
        }
    }
}
