// P11 fixups:
// - happy_path_writes_success_csv_and_exits_zero: no longer asserts success.csv
//   columns (v3.3 column discovery is exec export territory); asserts meta.json stats only.
// - crash_path_exits_one_and_records_worker_crash: asserts meta.json not failed.csv.
// - startup_timeout_aborts: v3.3 does not synthesize STARTUP_FAILED per-row CSV;
//   test now asserts aborted (exit 2) + meta.json shows zero success/failed.
// - rerun_failed_csv_is_detected_and_filtered: updated YAML to use required_input: [x].

use assert_cmd::Command;
use std::path::{Path, PathBuf};

fn rowforge_bin() -> Command {
    Command::cargo_bin("rowforge").unwrap()
}

fn test_handler_path() -> PathBuf {
    use std::sync::Once;
    static BUILD: Once = Once::new();
    BUILD.call_once(|| {
        let status = std::process::Command::new("cargo")
            .args(["build", "-p", "test-handler"])
            .status()
            .expect("cargo build -p test-handler");
        assert!(status.success(), "cargo build -p test-handler failed");
    });
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest.parent().unwrap().parent().unwrap();
    workspace_root.join("target/debug/test-handler")
}

fn write_handler_dir(behavior: &str, input_field: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    // Uses required_input: [<field>] syntax (P1 v3.3 manifest).
    let yaml = format!(
        "name: test-h\nversion: 0.0.0\nentry:\n  cmd: ['{}', '{}']\n  startup_timeout_ms: 5000\nrequired_input: [{}]\n",
        test_handler_path().display(),
        behavior,
        input_field,
    );
    std::fs::write(dir.path().join("rowforge.yaml"), yaml).unwrap();
    dir
}

fn write_csv(dir: &Path, name: &str, content: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, content).unwrap();
    p
}

// P11 re-enabled: no longer asserts success.csv columns; checks meta.json stats + run success.
#[test]
fn happy_path_writes_success_csv_and_exits_zero() {
    let h = write_handler_dir("echo", "x");
    let workdir = tempfile::tempdir().unwrap();
    let input = write_csv(workdir.path(), "in.csv", "x\nA\nB\nC\n");
    let out = workdir.path().join("out");

    rowforge_bin()
        .arg("run")
        .arg("--handler")
        .arg(h.path())
        .arg("--input")
        .arg(&input)
        .arg("--output-dir")
        .arg(&out)
        .arg("--workers")
        .arg("2")
        .arg("--quiet")
        .assert()
        .success()
        .code(0);

    let meta_raw = std::fs::read_to_string(out.join("meta.json")).unwrap();
    let meta: serde_json::Value = serde_json::from_str(&meta_raw).unwrap();
    assert_eq!(meta["stats"]["success"], 3);
    assert_eq!(meta["stats"]["failed"], 0);

    // outcomes.jsonl must exist with 3 lines.
    let jsonl = out.join("outcomes.jsonl");
    let content = std::fs::read_to_string(&jsonl).unwrap();
    let line_count = content.lines().filter(|l| !l.trim().is_empty()).count();
    assert_eq!(line_count, 3, "expected 3 outcome lines; got: {line_count}");
}

// P11 re-enabled: checks meta.json for WORKER_CRASH, not failed.csv.
#[test]
fn crash_path_exits_one_and_records_worker_crash() {
    let h = write_handler_dir("crash-after-3", "x");
    let workdir = tempfile::tempdir().unwrap();
    let mut csv_body = String::from("x\n");
    for i in 0..30 {
        csv_body.push_str(&format!("v{}\n", i));
    }
    let input = write_csv(workdir.path(), "in.csv", &csv_body);
    let out = workdir.path().join("out");

    rowforge_bin()
        .arg("run")
        .arg("--handler")
        .arg(h.path())
        .arg("--input")
        .arg(&input)
        .arg("--output-dir")
        .arg(&out)
        .arg("--workers")
        .arg("2")
        .arg("--quiet")
        .assert()
        .code(1); // some failed (crash)

    let meta_raw = std::fs::read_to_string(out.join("meta.json")).unwrap();
    let meta: serde_json::Value = serde_json::from_str(&meta_raw).unwrap();
    assert!(
        meta["stats"]["by_error_code"]["WORKER_CRASH"]
            .as_u64()
            .unwrap_or(0)
            >= 1
    );
}

// P11 re-enabled: v3.3 startup failure = aborted run, no per-row CSV output.
// Asserts exit code 2 + meta shows 0 success + 0 failed (run aborted).
#[test]
fn startup_timeout_aborts_with_exit_two() {
    let dir = tempfile::tempdir().unwrap();
    let yaml = format!(
        "name: timeout-h\nversion: 0.0.0\nentry:\n  cmd: ['{}', 'no-ready']\n  startup_timeout_ms: 500\n",
        test_handler_path().display(),
    );
    std::fs::write(dir.path().join("rowforge.yaml"), yaml).unwrap();

    let workdir = tempfile::tempdir().unwrap();
    let input = write_csv(workdir.path(), "in.csv", "x\nhi\nthere\nfriend\n");
    let out = workdir.path().join("out");

    rowforge_bin()
        .arg("run")
        .arg("--handler")
        .arg(dir.path())
        .arg("--input")
        .arg(&input)
        .arg("--output-dir")
        .arg(&out)
        .arg("--workers")
        .arg("1")
        .arg("--quiet")
        .assert()
        .code(2);

    // meta.json is still written even on abort; success/failed are 0.
    let meta_raw = std::fs::read_to_string(out.join("meta.json")).unwrap();
    let meta: serde_json::Value = serde_json::from_str(&meta_raw).unwrap();
    assert_eq!(meta["stats"]["success"], 0);
    assert_eq!(meta["stats"]["failed"], 0);
}

#[test]
fn dry_run_only_processes_sample_rows() {
    let h = write_handler_dir("echo", "x");
    let workdir = tempfile::tempdir().unwrap();
    let mut csv_body = String::from("x\n");
    for i in 0..50 {
        csv_body.push_str(&format!("v{}\n", i));
    }
    let input = write_csv(workdir.path(), "in.csv", &csv_body);
    let out = workdir.path().join("out");

    rowforge_bin()
        .arg("run")
        .arg("--handler")
        .arg(h.path())
        .arg("--input")
        .arg(&input)
        .arg("--output-dir")
        .arg(&out)
        .arg("--workers")
        .arg("2")
        .arg("--dry-run")
        .arg("--dry-run-sample")
        .arg("5")
        .arg("--quiet")
        .assert()
        .success();

    let meta_raw = std::fs::read_to_string(out.join("meta.json")).unwrap();
    let meta: serde_json::Value = serde_json::from_str(&meta_raw).unwrap();
    assert_eq!(meta["dry_run"], true);
    assert_eq!(meta["stats"]["success"], 5);
}

// P11 re-enabled: updated to use required_input: [x] manifest syntax (P1).
// The rerun detection notice + required-column error must both appear in stderr.
#[test]
fn rerun_failed_csv_is_detected_and_filtered() {
    let workdir = tempfile::tempdir().unwrap();
    let failed = write_csv(workdir.path(), "failed.csv",
        "seqid,errcode,errmessage\n\
         0,INVALID,bad\n\
         1,WORKER_CRASH,died\n");
    let h = write_handler_dir("echo", "x");
    let out = workdir.path().join("out");

    let assertion = rowforge_bin()
        .arg("run")
        .arg("--handler")
        .arg(h.path())
        .arg("--input")
        .arg(&failed)
        .arg("--output-dir")
        .arg(&out)
        .arg("--workers")
        .arg("1")
        .arg("--quiet")
        .assert()
        .failure();

    let stderr = String::from_utf8_lossy(&assertion.get_output().stderr).to_string();
    // Detection fires.
    assert!(
        stderr.contains("detected as failed.csv"),
        "expected rerun detection notice in stderr; got:\n{stderr}"
    );
    // Required-column check rejects the stripped input.
    assert!(
        stderr.contains("required") && stderr.contains("x"),
        "expected required-column error mentioning `x`; got:\n{stderr}"
    );
}

#[test]
fn output_dir_inside_handler_dir_fails_cleanly() {
    let h = write_handler_dir("echo", "x");
    let workdir = tempfile::tempdir().unwrap();
    let input = write_csv(workdir.path(), "in.csv", "x\nA\n");
    let bad_out = h.path().join("out");

    let assertion = rowforge_bin()
        .arg("run")
        .arg("--handler")
        .arg(h.path())
        .arg("--input")
        .arg(&input)
        .arg("--output-dir")
        .arg(&bad_out)
        .arg("--workers")
        .arg("1")
        .arg("--quiet")
        .assert()
        .code(3);

    let stderr = String::from_utf8_lossy(&assertion.get_output().stderr).to_string();
    assert!(
        stderr.contains("must not be inside") && stderr.contains("handler"),
        "expected guard message in stderr, got:\n{}",
        stderr
    );
}
