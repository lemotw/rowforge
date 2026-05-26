// Plan 11 T1: integration tests for RunRequest.only_row_ids filter.
//
// Three scenarios:
//  1. only_row_ids = Some([3,5,7]) on a 10-row input — exactly those 3 rows dispatched.
//  2. only_row_ids = Some([]) — vacuous noop, 0 outcomes, no error.
//  3. only_row_ids overrides skip_seqs: second run with only_row_ids=Some([1,2,3])
//     + skip_seqs={0..=9} (simulating "all previously attempted") still dispatches
//     the 3 listed rows (re-run intent overrides resume intent).
//  4. Started event reports len(only_row_ids) not the full input row count
//     (Plan 11 review fix — Live tab denominator regression).

use rowforge_core::csv_io::FieldMap;
use rowforge_core::pool::BatchOutcome;
use rowforge_core::run::{execute, RunProgressEvent, RunRequest};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
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

/// Build a handler dir with `rowforge.yaml` using the `echo` test-handler behavior,
/// and an input dir with a `input.csv` containing `rows` data rows.
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
    std::fs::write(handler_dir.path().join("rowforge.yaml"), yaml).expect("write manifest");

    let csv_path = input_dir.path().join("input.csv");
    let mut buf = String::from("n\n");
    for i in 0..rows {
        buf.push_str(&format!("{}\n", i));
    }
    std::fs::write(&csv_path, buf).expect("write csv");

    (handler_dir, input_dir)
}

/// Read outcomes.jsonl and return a sorted Vec of all seq values found.
fn collect_seqs_from_jsonl(jsonl_path: &std::path::Path) -> Vec<u64> {
    let content = std::fs::read_to_string(jsonl_path).unwrap_or_default();
    let mut seqs = Vec::new();
    for line in content.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if let Ok(bo) = serde_json::from_str::<BatchOutcome>(t) {
            seqs.extend(bo.seqs.iter().copied());
        }
    }
    seqs.sort_unstable();
    seqs
}

fn base_req(
    run_id: &str,
    handler_dir: &tempfile::TempDir,
    input_dir: &tempfile::TempDir,
    output_dir: &tempfile::TempDir,
) -> RunRequest {
    RunRequest {
        run_id: run_id.into(),
        parent_run_id: None,
        handler_dir: handler_dir.path().to_path_buf(),
        input_csv: input_dir.path().join("input.csv"),
        output_dir: output_dir.path().to_path_buf(),
        workers: 1,
        dry_run: false,
        dry_run_sample: 0,
        row_limit: None,
        skip_seqs: HashSet::new(),
        field_map: FieldMap::new(),
        config_overrides: BTreeMap::new(),
        shutdown_grace: Duration::from_secs(5),
        on_progress: None,
        on_handler_log: None,
        cancel: None,
        input_format: None,
        fsync_outcomes: false,
        capture_raw_stdout: false,
        only_row_ids: None,
    }
}

// ---------------------------------------------------------------------------
// Test 1: only_row_ids dispatches just those rows
// ---------------------------------------------------------------------------

#[tokio::test]
async fn only_row_ids_dispatches_just_those_rows() {
    // 10-row input; filter to seqs 3, 5, 7.
    let (handler_dir, input_dir) = make_handler_and_input(10);
    let output_dir = tempfile::tempdir().expect("output tempdir");

    let req = RunRequest {
        run_id: "t-only-filter".into(),
        only_row_ids: Some(vec![3, 5, 7]),
        ..base_req("t-only-filter", &handler_dir, &input_dir, &output_dir)
    };

    let report = execute(req).await.expect("execute should succeed");

    assert_eq!(
        report.success_count, 3,
        "expected 3 successes (seqs 3,5,7); got {} (failed={}, by_code={:?})",
        report.success_count, report.failed_count, report.by_error_code
    );
    assert_eq!(report.failed_count, 0);
    assert!(!report.aborted, "run should not be aborted");

    let jsonl_path = output_dir.path().join("outcomes.jsonl");
    let seqs = collect_seqs_from_jsonl(&jsonl_path);
    assert_eq!(
        seqs,
        vec![3u64, 5, 7],
        "outcomes.jsonl seqs must be exactly [3,5,7], got {:?}",
        seqs
    );
}

// ---------------------------------------------------------------------------
// Test 2: only_row_ids = Some([]) runs vacuously (noop)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn only_row_ids_empty_vec_runs_vacuously() {
    // 10-row input; filter to empty set → dispatch nothing.
    let (handler_dir, input_dir) = make_handler_and_input(10);
    let output_dir = tempfile::tempdir().expect("output tempdir");

    let req = RunRequest {
        run_id: "t-only-empty".into(),
        only_row_ids: Some(vec![]),
        ..base_req("t-only-empty", &handler_dir, &input_dir, &output_dir)
    };

    let report = execute(req).await.expect("execute should succeed with empty filter");

    assert_eq!(
        report.success_count, 0,
        "expected 0 successes with empty only_row_ids; got {}",
        report.success_count
    );
    assert_eq!(report.failed_count, 0);
    assert!(
        !report.aborted,
        "empty only_row_ids should not abort the run; reason={:?}",
        report.abort_reason
    );

    // outcomes.jsonl should be empty (or absent — both are acceptable).
    let jsonl_path = output_dir.path().join("outcomes.jsonl");
    let seqs = collect_seqs_from_jsonl(&jsonl_path);
    assert!(
        seqs.is_empty(),
        "expected 0 outcomes in outcomes.jsonl; got {:?}",
        seqs
    );
}

// ---------------------------------------------------------------------------
// Test 3: only_row_ids overrides skip_seqs (re-run intent > resume intent)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn only_row_ids_overrides_skip_attempted() {
    // Simulate "all 10 rows previously attempted" by putting all seqs in skip_seqs.
    // With only_row_ids=Some([1,2,3]), those 3 rows MUST be re-dispatched even
    // though skip_seqs would have excluded them.
    let (handler_dir, input_dir) = make_handler_and_input(10);
    let output_dir = tempfile::tempdir().expect("output tempdir");

    let all_seqs: HashSet<u64> = (0u64..10).collect();

    let req = RunRequest {
        run_id: "t-only-override".into(),
        // Mark all seqs as "already attempted" — simulates skip_attempted=true.
        skip_seqs: all_seqs,
        // But explicitly request re-run of seqs 1, 2, 3.
        only_row_ids: Some(vec![1, 2, 3]),
        ..base_req("t-only-override", &handler_dir, &input_dir, &output_dir)
    };

    let report = execute(req).await.expect("execute should succeed");

    // only_row_ids wins: 3 rows dispatched despite skip_seqs covering them.
    assert_eq!(
        report.success_count, 3,
        "only_row_ids must override skip_seqs: expected 3 successes; got {} (failed={}, by_code={:?})",
        report.success_count, report.failed_count, report.by_error_code
    );
    assert_eq!(report.failed_count, 0);
    assert!(!report.aborted);

    let jsonl_path = output_dir.path().join("outcomes.jsonl");
    let seqs = collect_seqs_from_jsonl(&jsonl_path);
    assert_eq!(
        seqs,
        vec![1u64, 2, 3],
        "outcomes.jsonl must contain exactly seqs [1,2,3]; got {:?}",
        seqs
    );
}

// ---------------------------------------------------------------------------
// Test 4: Started event reports len(only_row_ids), not full input row count
// ---------------------------------------------------------------------------

/// Regression: Live tab denominator must reflect the filtered count.
///
/// A 10-row input with only_row_ids=[1,3,5] must emit `Started { total_rows:
/// 3 }` — not `Started { total_rows: 10 }`. Without this fix, the Live tab
/// shows "0 / 10" → "3 / 10" instead of "0 / 3" → "3 / 3".
#[tokio::test]
async fn only_row_ids_started_event_uses_filtered_count() {
    // 10-row input; filter is [1, 3, 5] (3 rows).
    let (handler_dir, input_dir) = make_handler_and_input(10);
    let output_dir = tempfile::tempdir().expect("output tempdir");

    // Capture the total_rows value reported by the Started event.
    let started_total: Arc<Mutex<Option<u64>>> = Arc::new(Mutex::new(None));
    let started_total_cb = started_total.clone();

    let on_progress: rowforge_core::run::ProgressCallback = Arc::new(move |ev| {
        if let RunProgressEvent::Started { total_rows } = ev {
            *started_total_cb.lock().unwrap() = Some(total_rows);
        }
    });

    let req = RunRequest {
        run_id: "t-started-denom".into(),
        only_row_ids: Some(vec![1, 3, 5]),
        on_progress: Some(on_progress),
        ..base_req("t-started-denom", &handler_dir, &input_dir, &output_dir)
    };

    let report = execute(req).await.expect("execute should succeed");
    assert_eq!(report.success_count, 3, "expected 3 successes; got {}", report.success_count);

    let captured = *started_total.lock().unwrap();
    assert_eq!(
        captured,
        Some(3u64),
        "Started event must report total_rows=3 (filter length), not 10 (input length); got {:?}",
        captured
    );
}
