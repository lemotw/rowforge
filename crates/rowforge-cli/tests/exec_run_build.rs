//! Integration tests for auto-build gate in `exec run`.
//!
//! These tests exercise the path where `rowforge exec run` detects a stale
//! (or missing) binary and invokes the handler's `entry.build` command before
//! spawning workers.

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

#[test]
fn exec_run_auto_builds_stale_handler() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path();
    let handler_dir = workspace.join("handlers/stub");
    std::fs::create_dir_all(&handler_dir).unwrap();

    // Handler manifest: the build step simply creates a stub-bin file.
    // We don't need a fully functional handler here — the test only asserts
    // that (a) the build banner appears in stderr and (b) stub-bin exists
    // after exec run kicks off the build. The actual row-dispatch attempt
    // will fail/abort because stub-bin isn't a real handler; that's fine.
    std::fs::write(
        handler_dir.join("rowforge.yaml"),
        "name: stub\nversion: 0.0.0\nentry:\n  cmd: [\"./stub-bin\"]\n  build: [\"sh\", \"-c\", \"touch stub-bin && chmod +x stub-bin\"]\n",
    )
    .unwrap();

    // A source file so needs_build sees a source file newer than any binary.
    std::fs::write(handler_dir.join("handler.go"), "// placeholder\n").unwrap();

    // Input CSV with one data row.
    let csv = workspace.join("input.csv");
    std::fs::write(&csv, "id\n1\n").unwrap();

    let exe = env!("CARGO_BIN_EXE_rowforge");

    // Create the execution.
    let exec_start = Command::new(exe)
        .env("ROWFORGE_HOME", workspace)
        .args([
            "exec",
            "start",
            "--csv",
            csv.to_str().unwrap(),
            "--name",
            "smoke",
        ])
        .output()
        .expect("exec start runs");
    assert!(
        exec_start.status.success(),
        "exec start failed:\nSTDERR: {}",
        String::from_utf8_lossy(&exec_start.stderr)
    );

    let exec_id = parse_exec_id(&String::from_utf8_lossy(&exec_start.stdout));

    // Run the attempt — this should auto-build before spawning workers.
    // The dispatch itself will fail (stub-bin is not a real handler), but we
    // only care that the build banner appeared and stub-bin was created.
    let run = Command::new(exe)
        .env("ROWFORGE_HOME", workspace)
        .args([
            "exec",
            "run",
            &exec_id,
            "--handler",
            handler_dir.to_str().unwrap(),
            "--workers",
            "1",
        ])
        .output()
        .expect("exec run invocation should not panic");

    let stderr = String::from_utf8_lossy(&run.stderr);
    let stdout = String::from_utf8_lossy(&run.stdout);

    // The build banner must appear — this is the key assertion.
    assert!(
        stderr.contains("building stub"),
        "expected '[rowforge] building stub' banner in stderr;\nSTDOUT:\n{}\nSTDERR:\n{}",
        stdout,
        stderr
    );

    // stub-bin must exist in the handler dir after the build.
    assert!(
        handler_dir.join("stub-bin").exists(),
        "stub-bin should exist in handler dir after auto-build"
    );
}

#[test]
fn exec_run_exits_nonzero_when_build_fails() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path();
    let handler_dir = workspace.join("handlers/broken");
    std::fs::create_dir_all(&handler_dir).unwrap();

    // Handler whose build command always fails.
    std::fs::write(
        handler_dir.join("rowforge.yaml"),
        "name: broken\nversion: 0.0.0\nentry:\n  cmd: [\"./broken-bin\"]\n  build: [\"sh\", \"-c\", \"echo build-fail >&2; exit 7\"]\n",
    )
    .unwrap();
    std::fs::write(handler_dir.join("handler.go"), "// placeholder\n").unwrap();

    let csv = workspace.join("input.csv");
    std::fs::write(&csv, "id\n1\n").unwrap();

    let exe = env!("CARGO_BIN_EXE_rowforge");

    let exec_start = Command::new(exe)
        .env("ROWFORGE_HOME", workspace)
        .args([
            "exec",
            "start",
            "--csv",
            csv.to_str().unwrap(),
            "--name",
            "smoke",
        ])
        .output()
        .unwrap();
    assert!(
        exec_start.status.success(),
        "exec start failed: {}",
        String::from_utf8_lossy(&exec_start.stderr)
    );

    let exec_id = parse_exec_id(&String::from_utf8_lossy(&exec_start.stdout));

    let run = Command::new(exe)
        .env("ROWFORGE_HOME", workspace)
        .args([
            "exec",
            "run",
            &exec_id,
            "--handler",
            handler_dir.to_str().unwrap(),
            "--workers",
            "1",
        ])
        .output()
        .unwrap();

    assert!(
        !run.status.success(),
        "expected non-zero exit when build fails; got exit {:?}",
        run.status.code()
    );

    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(
        stderr.contains("build failed"),
        "expected 'build failed' in stderr; got:\n{}",
        stderr
    );
}
