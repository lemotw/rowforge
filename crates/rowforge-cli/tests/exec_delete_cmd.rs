//! Integration tests for `rowforge exec delete`.

use std::process::Command;

/// Parse the exec id from `exec start` stdout.
/// The output line looks like: `created e_<id>`
fn parse_exec_id(stdout: &str) -> String {
    stdout
        .lines()
        .find_map(|l| l.strip_prefix("created "))
        .expect("exec start stdout must contain 'created <id>' line")
        .trim()
        .to_string()
}

/// Create a temp workspace with one execution seeded via `exec start`.
/// Returns (TempDir, exec_id).
fn workspace_with_exec(name: &str) -> (tempfile::TempDir, String) {
    let tmp = tempfile::TempDir::new().unwrap();
    let exe = env!("CARGO_BIN_EXE_rowforge");
    let csv = tmp.path().join("input.csv");
    std::fs::write(&csv, "id\n1\n").unwrap();
    let out = Command::new(exe)
        .env("ROWFORGE_HOME", tmp.path())
        .args(["exec", "start", "--csv", csv.to_str().unwrap(), "--name", name])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "exec start failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let exec_id = parse_exec_id(&stdout);
    (tmp, exec_id)
}

#[test]
fn exec_delete_single_succeeds() {
    let (tmp, exec_id) = workspace_with_exec("delete-single");
    let exe = env!("CARGO_BIN_EXE_rowforge");
    let out = Command::new(exe)
        .env("ROWFORGE_HOME", tmp.path())
        .args(["exec", "delete", &exec_id])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "exec delete failed: {}", stderr);
    assert!(stderr.contains("deleted"), "expected 'deleted' in stderr: {}", stderr);
    // Directory must be gone.
    assert!(
        !tmp.path().join("executions").join(&exec_id).exists(),
        "execution dir should be removed after delete"
    );
}

#[test]
fn exec_delete_missing_id_and_no_flag_fails() {
    let tmp = tempfile::TempDir::new().unwrap();
    let exe = env!("CARGO_BIN_EXE_rowforge");
    let out = Command::new(exe)
        .env("ROWFORGE_HOME", tmp.path())
        .args(["exec", "delete"])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "expected non-zero exit when neither exec_id nor --all-completed is given"
    );
}

#[test]
fn exec_delete_all_completed_works() {
    let (tmp, exec_id_a) = workspace_with_exec("delete-all-a");
    // Seed a second execution in the same workspace.
    let exe = env!("CARGO_BIN_EXE_rowforge");
    let csv = tmp.path().join("input.csv");
    let out_b = Command::new(exe)
        .env("ROWFORGE_HOME", tmp.path())
        .args(["exec", "start", "--csv", csv.to_str().unwrap(), "--name", "delete-all-b"])
        .output()
        .unwrap();
    assert!(out_b.status.success(), "second exec start failed");
    let exec_id_b = parse_exec_id(&String::from_utf8_lossy(&out_b.stdout));

    let out = Command::new(exe)
        .env("ROWFORGE_HOME", tmp.path())
        .args(["exec", "delete", "--all-completed"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "all-completed delete failed: {}", stderr);
    // Both ids should appear in output.
    assert!(stderr.contains(&exec_id_a), "expected exec_id_a in stderr: {}", stderr);
    assert!(stderr.contains(&exec_id_b), "expected exec_id_b in stderr: {}", stderr);
    // Directories must be gone.
    assert!(
        !tmp.path().join("executions").join(&exec_id_a).exists(),
        "execution A dir should be removed"
    );
    assert!(
        !tmp.path().join("executions").join(&exec_id_b).exists(),
        "execution B dir should be removed"
    );
}
