// P11 re-enable: dropped success.csv file assertion (v3.3 no longer writes it).
// Now asserts on RunReport.success_count only.
//
// `skip_seqs` filters input rows BEFORE `row_limit` applies. With
// skip_seqs={0,1,2} and row_limit=Some(3) on a 10-row CSV, the dispatched
// seqs are {3,4,5} — the first 3 unskipped rows. Verifies spec I5
// (RowResolution monotonicity foundation).

use rowforge_core::csv_io::FieldMap;
use rowforge_core::run::{execute, RunRequest};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

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

fn make_handler_and_input(rows: usize) -> (tempfile::TempDir, tempfile::TempDir) {
    let handler_dir = tempfile::tempdir().expect("handler tempdir");
    let input_dir = tempfile::tempdir().expect("input tempdir");

    let handler_cmd = test_handler_path();
    let handler_cmd_str = handler_cmd.to_string_lossy();
    let yaml = format!(
        r#"name: echo
version: 0.0.0
entry:
  cmd: ["{cmd}", "echo"]
  startup_timeout_ms: 5000
"#,
        cmd = handler_cmd_str,
    );
    std::fs::write(handler_dir.path().join("rowforge.yaml"), yaml)
        .expect("write manifest");

    let csv_path = input_dir.path().join("input.csv");
    let mut buf = String::from("seq\n");
    for i in 0..rows {
        buf.push_str(&format!("{}\n", i));
    }
    std::fs::write(&csv_path, buf).expect("write csv");

    (handler_dir, input_dir)
}

#[tokio::test]
async fn skip_seqs_applies_before_row_limit() {
    let (handler_dir, input_dir) = make_handler_and_input(10);
    let output_dir = tempfile::tempdir().expect("output tempdir");
    let csv_path = input_dir.path().join("input.csv");

    let mut skip = HashSet::new();
    skip.insert(0u64);
    skip.insert(1u64);
    skip.insert(2u64);

    let req = RunRequest {
        run_id: "test-skip-seqs".into(),
        parent_run_id: None,
        handler_dir: handler_dir.path().to_path_buf(),
        input_csv: csv_path,
        output_dir: output_dir.path().to_path_buf(),
        workers: 1,
        dry_run: false,
        dry_run_sample: 0,
        row_limit: Some(3),
        skip_seqs: skip,
        field_map: FieldMap::new(),
        config_overrides: BTreeMap::new(),
        shutdown_grace: Duration::from_secs(2),
        on_progress: None,
        cancel: None,
        input_format: None,
        fsync_outcomes: false,
    };

    let report = execute(req).await.expect("execute should succeed");

    assert_eq!(
        report.success_count, 3,
        "expected 3 successes (seqs 3,4,5); got {} (failed={}, by_code={:?})",
        report.success_count, report.failed_count, report.by_error_code
    );
    assert_eq!(report.failed_count, 0);

    // Verify outcomes.jsonl contains exactly 3 entries with seqs 3,4,5.
    let jsonl_path = output_dir.path().join("outcomes.jsonl");
    let content = std::fs::read_to_string(&jsonl_path).expect("read outcomes.jsonl");
    let mut seqs_found: Vec<u64> = Vec::new();
    for line in content.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let bo: rowforge_core::pool::BatchOutcome =
            serde_json::from_str(t).expect("parse BatchOutcome");
        seqs_found.extend(bo.seqs.iter().copied());
    }
    seqs_found.sort_unstable();
    assert_eq!(
        seqs_found,
        vec![3u64, 4, 5],
        "outcomes.jsonl seqs must be exactly [3,4,5], got {:?}",
        seqs_found
    );
}
