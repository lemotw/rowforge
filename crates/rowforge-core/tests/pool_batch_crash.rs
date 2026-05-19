// Migrated from run_pool (P11): rewritten to use run_pool_streaming.
// Batch-mode crash semantics (T7).
//
// Two variants:
//   A: idempotent=true  → batch crash produces WORKER_CRASH
//   B: idempotent=false → batch crash produces WORKER_CRASH_UNSAFE
//
// The `batch-crash` test-handler behavior reads exactly one `batch`
// envelope and exits non-zero without replying.

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

fn batch_crash_manifest(idempotent: bool) -> Arc<Manifest> {
    Arc::new(Manifest {
        name: "batch-crash".into(),
        version: "0.0.0".into(),
        description: String::new(),
        language: String::new(),
        entry: Entry {
            cmd: vec![
                test_handler_path().to_string_lossy().into(),
                "batch-crash".into(),
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
            idempotent: Some(idempotent),
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
async fn batch_crash_idempotent_true_yields_worker_crash() {
    let dir = tempfile::tempdir().unwrap();
    // 5 rows = exactly one batch (batch_size=5)
    let csv_path = write_csv(&dir, "input.csv", "x\n0\n1\n2\n3\n4\n");

    let manifest = batch_crash_manifest(true);
    let runtime = manifest.runtime.clone().unwrap();

    let cfg = StreamingPoolConfig {
        handler_dir: std::env::temp_dir(),
        manifest,
        workers: 1,
        run_id: "t-crash-safe".into(),
        config: BTreeMap::new(),
        shutdown_grace: Duration::from_secs(2),
        cancel: None,
        runtime,
        jsonl_path: dir.path().join("outcomes.jsonl"),
        fsync_outcomes: false,
        stall_timeout: Some(Duration::from_secs(5)),
        stall_poll_interval: Some(Duration::from_millis(100)),
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

    let outcomes = collect_jsonl_outcomes(&dir.path().join("outcomes.jsonl"));
    // All 5 rows in the in-flight batch must crash with WORKER_CRASH (safe).
    let crash_codes: Vec<&str> = outcomes
        .iter()
        .filter_map(|o| match o {
            RowOutcome::Error { code, .. } => Some(code.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        !crash_codes.is_empty(),
        "expected at least one crash outcome, got {:?}",
        outcomes
    );
    for code in &crash_codes {
        assert_eq!(
            *code, "WORKER_CRASH",
            "idempotent=true → WORKER_CRASH (got {})",
            code
        );
    }
}

#[tokio::test]
async fn batch_crash_idempotent_false_yields_worker_crash_unsafe() {
    let dir = tempfile::tempdir().unwrap();
    let csv_path = write_csv(&dir, "input.csv", "x\n0\n1\n2\n3\n4\n");

    let manifest = batch_crash_manifest(false);
    let runtime = manifest.runtime.clone().unwrap();

    let cfg = StreamingPoolConfig {
        handler_dir: std::env::temp_dir(),
        manifest,
        workers: 1,
        run_id: "t-crash-unsafe".into(),
        config: BTreeMap::new(),
        shutdown_grace: Duration::from_secs(2),
        cancel: None,
        runtime,
        jsonl_path: dir.path().join("outcomes.jsonl"),
        fsync_outcomes: false,
        stall_timeout: Some(Duration::from_secs(5)),
        stall_poll_interval: Some(Duration::from_millis(100)),
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

    let outcomes = collect_jsonl_outcomes(&dir.path().join("outcomes.jsonl"));
    let crash_codes: Vec<&str> = outcomes
        .iter()
        .filter_map(|o| match o {
            RowOutcome::Error { code, .. } => Some(code.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        !crash_codes.is_empty(),
        "expected at least one crash outcome, got {:?}",
        outcomes
    );
    for code in &crash_codes {
        assert_eq!(
            *code, "WORKER_CRASH_UNSAFE",
            "idempotent=false → WORKER_CRASH_UNSAFE (got {})",
            code
        );
    }
}
