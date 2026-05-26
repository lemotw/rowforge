// P11 re-enable: rewritten for v3.3.
//
// v3.3: schema.failed_output removed (P1). Column discovery moved to exec export
// (P10). This test verifies that a handler returning error-with-data produces
// an outcomes.jsonl where each error row carries the handler-emitted `data`
// payload. The CSV layout for the export (if requested) is tested separately in
// exec_export.rs.

use rowforge_core::csv_io::FieldMap;
use rowforge_core::pool::{BatchOutcome, RowOutcome};
use rowforge_core::run::{execute, RunRequest};
use std::collections::BTreeMap;
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

fn write_handler_dir() -> tempfile::TempDir {
    // The `error-with-data` handler behavior returns an error envelope whose
    // `data` field carries `{"billid": <input_billid>}`.
    let dir = tempfile::tempdir().unwrap();
    let yaml = format!(
        "name: failtest\n\
         version: 0.0.0\n\
         entry:\n\
         \x20\x20cmd: ['{}', 'error-with-data']\n\
         \x20\x20startup_timeout_ms: 5000\n",
        test_handler_path().display(),
    );
    std::fs::write(dir.path().join("rowforge.yaml"), yaml).unwrap();
    dir
}

#[tokio::test]
async fn failed_output_data_preserved_in_outcomes_jsonl() {
    let h = write_handler_dir();
    let workdir = tempfile::tempdir().unwrap();
    let input = workdir.path().join("in.csv");
    std::fs::write(&input, "billid\nB1\nB2\nB3\n").unwrap();
    let out_dir = workdir.path().join("out");

    let req = RunRequest {
        run_id: "t-fd".into(),
        parent_run_id: None,
        handler_dir: h.path().to_path_buf(),
        input_csv: input.clone(),
        output_dir: out_dir.clone(),
        workers: 1,
        dry_run: false,
        dry_run_sample: 0,
        row_limit: None,
        skip_seqs: Default::default(),
        field_map: FieldMap::new(),
        config_overrides: BTreeMap::new(),
        shutdown_grace: Duration::from_secs(2),
        on_progress: None,
        on_handler_log: None,
        cancel: None,
        input_format: None,
        fsync_outcomes: false,
        capture_raw_stdout: false,
        only_row_ids: None,
    };

    let report = execute(req).await.expect("execute");
    assert_eq!(report.success_count, 0);
    assert_eq!(report.failed_count, 3);

    // outcomes.jsonl must exist and contain 3 error outcomes.
    let jsonl_path = out_dir.join("outcomes.jsonl");
    let content = std::fs::read_to_string(&jsonl_path)
        .expect("outcomes.jsonl must exist");

    let mut outcomes: Vec<RowOutcome> = Vec::new();
    for line in content.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let bo: BatchOutcome = serde_json::from_str(t)
            .expect("parse BatchOutcome from outcomes.jsonl");
        outcomes.extend(bo.outcomes);
    }

    assert_eq!(outcomes.len(), 3, "expected 3 outcomes, got: {:?}", outcomes);

    // Sort by seq for deterministic assertions.
    outcomes.sort_by_key(|o| match o {
        RowOutcome::Error { seq, .. } => *seq,
        RowOutcome::Success { seq, .. } => *seq,
        RowOutcome::Crash { seq, .. } => *seq,
    });

    let expected_billids = ["B1", "B2", "B3"];
    for (i, o) in outcomes.iter().enumerate() {
        match o {
            RowOutcome::Error { seq, code, data, .. } => {
                assert_eq!(*seq, i as u64, "unexpected seq at position {}", i);
                assert_eq!(code, "DEMO_FAIL", "expected DEMO_FAIL code");
                let data_map = data
                    .as_ref()
                    .expect("error-with-data handler must populate data payload");
                let billid = data_map
                    .get("billid")
                    .and_then(|v| v.as_str())
                    .expect("data.billid must be a string");
                assert_eq!(
                    billid, expected_billids[i],
                    "data.billid for seq {} must be {}, got {}",
                    i, expected_billids[i], billid
                );
            }
            other => panic!("expected Error outcome at seq {}, got {:?}", i, other),
        }
    }
}
