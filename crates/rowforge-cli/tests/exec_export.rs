//! Integration tests for `exec export --format / --strict / completeness`.
//!
//! Tests exercise the export path directly using ExecutionStore + hand-crafted
//! outcomes.jsonl files (no real handler subprocess needed).

use rowforge_core::execution_store::{
    AttemptState, ExecutionStore, FinishAttempt, NewAttempt, NewExecution, NewHandlerInstance,
    RunType, Simulation, Source,
};
use rowforge_core::pool::{BatchOutcome, RowOutcome};
use serde_json::Value;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_store(home: &Path) -> ExecutionStore {
    ExecutionStore::open(home).unwrap()
}

fn make_csv(p: &Path, rows: usize) {
    let mut s = String::from("col\n");
    for i in 0..rows {
        s.push_str(&format!("{i}\n"));
    }
    std::fs::write(p, s).unwrap();
}

/// Create an execution with `row_count` rows. Returns (TempDir-for-csv, exec_id, hi_id).
fn new_execution(store: &mut ExecutionStore, row_count: usize) -> (TempDir, String, String) {
    let src = tempfile::tempdir().unwrap();
    let csv = src.path().join("in.csv");
    make_csv(&csv, row_count);
    let exec = store
        .create_execution(NewExecution {
            name: None,
            input_csv_id: "c".into(),
            input_csv_path: csv,
            current_handler_instance_id: None,
        })
        .unwrap();
    let hi = store
        .register_handler_instance(NewHandlerInstance {
            handler_id: "h".into(),
            manifest_hash: "sha256:m".into(),
            source_snapshot_dir: PathBuf::from("/tmp/snap"),
            binary_hash: None,
        })
        .unwrap();
    (src, exec.id, hi.id)
}

/// Write outcomes.jsonl for an attempt with specific success data maps and
/// error pairs.
///
/// `successes`: list of (seq, data_map) where data_map keys are the handler keys.
/// `failures`: list of (seq, errcode, errmessage, optional_data_map).
fn write_outcomes(
    dir: &Path,
    successes: &[(u64, serde_json::Map<String, Value>)],
    failures: &[(u64, &str, &str, Option<serde_json::Map<String, Value>>)],
) {
    let path = dir.join("outcomes.jsonl");
    let mut lines = Vec::new();
    for (seq, data) in successes {
        let bo = BatchOutcome {
            first_seq: *seq,
            seqs: vec![*seq],
            outcomes: vec![RowOutcome::Success {
                seq: *seq,
                data: data.clone(),
                dur_ms: 1,
            }],
        };
        lines.push(serde_json::to_string(&bo).unwrap());
    }
    for (seq, code, msg, data) in failures {
        let bo = BatchOutcome {
            first_seq: *seq,
            seqs: vec![*seq],
            outcomes: vec![RowOutcome::Error {
                seq: *seq,
                code: code.to_string(),
                message: msg.to_string(),
                dur_ms: 1,
                data: data.clone(),
            }],
        };
        lines.push(serde_json::to_string(&bo).unwrap());
    }
    std::fs::write(&path, lines.join("\n")).unwrap();
}

/// Create and finish an attempt (Completed or Aborted).
fn create_attempt(
    store: &mut ExecutionStore,
    exec_id: &str,
    hi_id: &str,
    successes: &[(u64, serde_json::Map<String, Value>)],
    failures: &[(u64, &str, &str, Option<serde_json::Map<String, Value>>)],
    aborted: bool,
) -> String {
    let at = store
        .create_attempt(NewAttempt {
            execution_id: exec_id.to_string(),
            handler_instance_id: hi_id.to_string(),
            parent_attempt_id: None,
            run_type: RunType {
                source: Source::Full,
                simulation: Simulation::Real,
            },
        })
        .unwrap();
    write_outcomes(&at.dir, successes, failures);
    store
        .finish_attempt(
            &at.id,
            FinishAttempt {
                success_count: successes.len() as u64,
                failed_count: failures.len() as u64,
                aborted,
                aborted_reason: if aborted {
                    Some("test aborted".into())
                } else {
                    None
                },
            },
        )
        .unwrap();
    at.id
}

fn kv(k: &str, v: &str) -> serde_json::Map<String, Value> {
    let mut m = serde_json::Map::new();
    m.insert(k.to_string(), Value::String(v.to_string()));
    m
}

fn kv2(k1: &str, v1: &str, k2: &str, v2: &str) -> serde_json::Map<String, Value> {
    let mut m = serde_json::Map::new();
    m.insert(k1.to_string(), Value::String(v1.to_string()));
    m.insert(k2.to_string(), Value::String(v2.to_string()));
    m
}

fn empty_map() -> serde_json::Map<String, Value> {
    serde_json::Map::new()
}

/// Run the export subcommand via the rowforge binary and return assert output.
fn run_export(
    home: &Path,
    exec_id: &str,
    extra_args: &[&str],
    out_dir: &Path,
) -> std::process::Output {
    let mut cmd = assert_cmd::Command::cargo_bin("rowforge").unwrap();
    cmd.env("ROWFORGE_HOME", home);
    cmd.arg("exec").arg("export").arg(exec_id);
    cmd.arg("--output-dir").arg(out_dir);
    for a in extra_args {
        cmd.arg(a);
    }
    cmd.output().unwrap()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Test 1: default format → success.csv + failed.csv created, no jsonl.
#[test]
fn exec_export_csv_default() {
    let home = tempfile::tempdir().unwrap();
    let mut store = make_store(home.path());
    let (_src, exec_id, hi_id) = new_execution(&mut store, 2);

    // 1 success (seq=0), 1 failure (seq=1).
    create_attempt(
        &mut store,
        &exec_id,
        &hi_id,
        &[(0, kv("result", "ok"))],
        &[(1, "MY_ERROR", "bad thing", None)],
        false,
    );
    drop(store);

    let out = tempfile::tempdir().unwrap();
    let output = run_export(home.path(), &exec_id, &[], out.path());
    assert!(
        output.status.success(),
        "export should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(out.path().join("success.csv").exists(), "success.csv should exist");
    assert!(out.path().join("failed.csv").exists(), "failed.csv should exist");
    assert!(
        !out.path().join("success.jsonl").exists(),
        "success.jsonl should NOT exist with default csv format"
    );
    assert!(
        !out.path().join("failed.jsonl").exists(),
        "failed.jsonl should NOT exist with default csv format"
    );
    assert!(out.path().join("resolution.json").exists(), "resolution.json should exist");
}

/// Test 2: --format jsonl → success.jsonl + failed.jsonl, no csv.
#[test]
fn exec_export_jsonl_only() {
    let home = tempfile::tempdir().unwrap();
    let mut store = make_store(home.path());
    let (_src, exec_id, hi_id) = new_execution(&mut store, 2);

    create_attempt(
        &mut store,
        &exec_id,
        &hi_id,
        &[(0, kv("x", "1"))],
        &[(1, "ERR", "oops", None)],
        false,
    );
    drop(store);

    let out = tempfile::tempdir().unwrap();
    let output = run_export(home.path(), &exec_id, &["--format", "jsonl"], out.path());
    assert!(
        output.status.success(),
        "export should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(!out.path().join("success.csv").exists(), "success.csv should NOT exist");
    assert!(!out.path().join("failed.csv").exists(), "failed.csv should NOT exist");
    assert!(out.path().join("success.jsonl").exists(), "success.jsonl should exist");
    assert!(out.path().join("failed.jsonl").exists(), "failed.jsonl should exist");
    assert!(out.path().join("resolution.json").exists(), "resolution.json should exist");
}

/// Test 3: --format both → all 4 files.
#[test]
fn exec_export_both() {
    let home = tempfile::tempdir().unwrap();
    let mut store = make_store(home.path());
    let (_src, exec_id, hi_id) = new_execution(&mut store, 2);

    create_attempt(
        &mut store,
        &exec_id,
        &hi_id,
        &[(0, kv("v", "hi"))],
        &[(1, "E", "fail", None)],
        false,
    );
    drop(store);

    let out = tempfile::tempdir().unwrap();
    let output = run_export(home.path(), &exec_id, &["--format", "both"], out.path());
    assert!(
        output.status.success(),
        "export should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    for name in &["success.csv", "failed.csv", "success.jsonl", "failed.jsonl"] {
        assert!(out.path().join(name).exists(), "{name} should exist with --format both");
    }
}

/// Test 4: column discovery across 2 attempts with different key sets.
/// Attempt 1: keys {a, b}; Attempt 2: keys {a, b, c}.
/// Merged success.csv header should be [seqid, a, b, c].
#[test]
fn exec_export_column_discovery() {
    let home = tempfile::tempdir().unwrap();
    let mut store = make_store(home.path());
    let (_src, exec_id, hi_id) = new_execution(&mut store, 3);

    // Attempt 1: seq=0 success with {a, b}; seq=1 success with {a, b}.
    create_attempt(
        &mut store,
        &exec_id,
        &hi_id,
        &[
            (0, kv2("a", "1", "b", "2")),
            (1, kv2("a", "3", "b", "4")),
        ],
        &[],
        false,
    );
    // Attempt 2: seq=2 success with {a, b, c}.
    create_attempt(
        &mut store,
        &exec_id,
        &hi_id,
        &[(2, {
            let mut m = serde_json::Map::new();
            m.insert("a".into(), Value::String("5".into()));
            m.insert("b".into(), Value::String("6".into()));
            m.insert("c".into(), Value::String("7".into()));
            m
        })],
        &[],
        false,
    );
    drop(store);

    let out = tempfile::tempdir().unwrap();
    let output = run_export(home.path(), &exec_id, &[], out.path());
    assert!(
        output.status.success(),
        "export should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let csv_content = std::fs::read_to_string(out.path().join("success.csv")).unwrap();
    let header = csv_content.lines().next().unwrap();
    assert_eq!(
        header, "seqid,a,b,c",
        "column discovery should produce alphabetical union: got '{header}'"
    );

    // Verify 3 data rows.
    let rows: Vec<&str> = csv_content.lines().skip(1).collect();
    assert_eq!(rows.len(), 3, "should have 3 data rows");
}

/// Test 5: JSONL output has explicit null for missing keys (D1).
/// Handler returns {a:1} for some rows, {a:1, b:2} for others.
#[test]
fn exec_export_jsonl_null_for_missing() {
    let home = tempfile::tempdir().unwrap();
    let mut store = make_store(home.path());
    let (_src, exec_id, hi_id) = new_execution(&mut store, 2);

    // seq=0: only has key "a"; seq=1: has keys "a" and "b".
    create_attempt(
        &mut store,
        &exec_id,
        &hi_id,
        &[
            (0, kv("a", "10")),
            (1, kv2("a", "20", "b", "30")),
        ],
        &[],
        false,
    );
    drop(store);

    let out = tempfile::tempdir().unwrap();
    let output = run_export(home.path(), &exec_id, &["--format", "jsonl"], out.path());
    assert!(
        output.status.success(),
        "export should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let jsonl = std::fs::read_to_string(out.path().join("success.jsonl")).unwrap();
    let lines: Vec<&str> = jsonl.lines().collect();
    assert_eq!(lines.len(), 2, "should have 2 JSONL lines");

    // The first line (seq=0) should have b: null since it was missing.
    let row0: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(
        row0.get("seqid").and_then(|v| v.as_u64()),
        Some(0),
        "seq=0 should be first"
    );
    assert_eq!(
        row0.get("b"),
        Some(&Value::Null),
        "missing key 'b' should be explicit null in seq=0 JSONL row: {row0}"
    );

    // The second line (seq=1) should have b: "30".
    let row1: Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(
        row1.get("b"),
        Some(&Value::String("30".into())),
        "seq=1 should have b='30'"
    );
}

/// Test 6: NeverAttempted rows appear in failed.csv with errcode=NEVER_ATTEMPTED.
#[test]
fn exec_export_never_attempted() {
    let home = tempfile::tempdir().unwrap();
    let mut store = make_store(home.path());
    // 3 rows, but we only do 1 attempt covering seq=0 only (sample=1, but we
    // simulate by writing outcomes for seq=0 only).
    let (_src, exec_id, hi_id) = new_execution(&mut store, 3);

    // Only seq=0 gets an outcome — seqs 1 and 2 remain NeverAttempted.
    create_attempt(
        &mut store,
        &exec_id,
        &hi_id,
        &[(0, kv("x", "ok"))],
        &[],
        false,
    );
    drop(store);

    let out = tempfile::tempdir().unwrap();
    let output = run_export(home.path(), &exec_id, &[], out.path());
    assert!(
        output.status.success(),
        "export should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let failed_csv = std::fs::read_to_string(out.path().join("failed.csv")).unwrap();
    // Should have 2 NEVER_ATTEMPTED rows (seq=1 and seq=2).
    assert!(
        failed_csv.contains("NEVER_ATTEMPTED"),
        "failed.csv should contain NEVER_ATTEMPTED:\n{failed_csv}"
    );
    let data_lines: Vec<&str> = failed_csv.lines().skip(1).collect();
    assert_eq!(data_lines.len(), 2, "should have 2 NEVER_ATTEMPTED rows:\n{failed_csv}");

    // Also test JSONL format.
    let out2 = tempfile::tempdir().unwrap();
    let home2 = tempfile::tempdir().unwrap();
    // Re-open store to re-export as jsonl.
    let out2_output = {
        let mut cmd = assert_cmd::Command::cargo_bin("rowforge").unwrap();
        cmd.env("ROWFORGE_HOME", home.path());
        cmd.arg("exec")
            .arg("export")
            .arg(&exec_id)
            .arg("--output-dir")
            .arg(out2.path())
            .arg("--format")
            .arg("jsonl");
        cmd.output().unwrap()
    };
    let _ = home2;
    assert!(out2_output.status.success());
    let failed_jsonl = std::fs::read_to_string(out2.path().join("failed.jsonl")).unwrap();
    assert!(
        failed_jsonl.contains("NEVER_ATTEMPTED"),
        "failed.jsonl should contain NEVER_ATTEMPTED:\n{failed_jsonl}"
    );
}

/// Test 7: NeverAttempted rows → resolution.json has fully_processed=false.
#[test]
fn export_warn_on_never_attempted() {
    let home = tempfile::tempdir().unwrap();
    let mut store = make_store(home.path());
    let (_src, exec_id, hi_id) = new_execution(&mut store, 3);

    // Only seq=0 covered; seqs 1 & 2 are NeverAttempted.
    create_attempt(
        &mut store,
        &exec_id,
        &hi_id,
        &[(0, empty_map())],
        &[],
        false,
    );
    drop(store);

    let out = tempfile::tempdir().unwrap();
    let output = run_export(home.path(), &exec_id, &[], out.path());
    assert!(output.status.success());

    let res_json: Value =
        serde_json::from_str(&std::fs::read_to_string(out.path().join("resolution.json")).unwrap())
            .unwrap();
    assert_eq!(
        res_json["completeness"]["fully_processed"],
        Value::Bool(false),
        "fully_processed should be false when never_attempted > 0"
    );
    assert_eq!(
        res_json["completeness"]["aborted_attempts"],
        Value::Number(0.into()),
        "aborted_attempts should be 0"
    );
}

/// Test 8: Aborted attempt → resolution.json includes aborted_attempt_ids.
#[test]
fn export_warn_on_aborted_attempts() {
    let home = tempfile::tempdir().unwrap();
    let mut store = make_store(home.path());
    let (_src, exec_id, hi_id) = new_execution(&mut store, 2);

    // 1 Aborted attempt covering seq=0.
    let attempt_id = create_attempt(
        &mut store,
        &exec_id,
        &hi_id,
        &[(0, empty_map())],
        &[],
        true, // aborted
    );
    drop(store);

    let out = tempfile::tempdir().unwrap();
    let output = run_export(home.path(), &exec_id, &[], out.path());
    assert!(
        output.status.success(),
        "export should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let res_json: Value =
        serde_json::from_str(&std::fs::read_to_string(out.path().join("resolution.json")).unwrap())
            .unwrap();
    assert_eq!(
        res_json["completeness"]["fully_processed"],
        Value::Bool(false),
        "fully_processed should be false when there are aborted attempts"
    );
    assert_eq!(
        res_json["completeness"]["aborted_attempts"],
        Value::Number(1.into()),
        "aborted_attempts should be 1"
    );
    let ids = res_json["completeness"]["aborted_attempt_ids"]
        .as_array()
        .expect("aborted_attempt_ids should be array");
    assert!(
        ids.iter().any(|v| v.as_str() == Some(&attempt_id)),
        "aborted_attempt_ids should contain {attempt_id}: {ids:?}"
    );
}

/// Test 9: --strict on incomplete execution → exit code 3, no export files.
#[test]
fn export_strict_fails_when_incomplete() {
    let home = tempfile::tempdir().unwrap();
    let mut store = make_store(home.path());
    // 2 rows but only 1 covered → NeverAttempted.
    let (_src, exec_id, hi_id) = new_execution(&mut store, 2);

    create_attempt(
        &mut store,
        &exec_id,
        &hi_id,
        &[(0, empty_map())],
        &[],
        false,
    );
    drop(store);

    let out = tempfile::tempdir().unwrap();
    let output = run_export(home.path(), &exec_id, &["--strict"], out.path());

    assert_eq!(
        output.status.code(),
        Some(3),
        "strict should exit 3 when incomplete"
    );

    // No export files should have been written.
    assert!(
        !out.path().join("success.csv").exists(),
        "success.csv should NOT be written when strict fails"
    );
    assert!(
        !out.path().join("failed.csv").exists(),
        "failed.csv should NOT be written when strict fails"
    );
    assert!(
        !out.path().join("resolution.json").exists(),
        "resolution.json should NOT be written when strict fails"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not fully processed"),
        "stderr should mention 'not fully processed': {stderr}"
    );
}

/// Test 10: --strict on complete execution → exit 0, exports written.
#[test]
fn export_strict_passes_when_complete() {
    let home = tempfile::tempdir().unwrap();
    let mut store = make_store(home.path());
    let (_src, exec_id, hi_id) = new_execution(&mut store, 2);

    // All rows covered in one Completed attempt.
    create_attempt(
        &mut store,
        &exec_id,
        &hi_id,
        &[(0, kv("r", "a")), (1, kv("r", "b"))],
        &[],
        false,
    );
    drop(store);

    let out = tempfile::tempdir().unwrap();
    let output = run_export(home.path(), &exec_id, &["--strict"], out.path());

    assert_eq!(
        output.status.code(),
        Some(0),
        "strict should exit 0 when complete: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(out.path().join("success.csv").exists(), "success.csv should be written");
    assert!(out.path().join("resolution.json").exists(), "resolution.json should be written");

    let res_json: Value = serde_json::from_str(
        &std::fs::read_to_string(out.path().join("resolution.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        res_json["completeness"]["fully_processed"],
        Value::Bool(true),
        "fully_processed should be true when all rows covered + no aborted attempts"
    );
}
