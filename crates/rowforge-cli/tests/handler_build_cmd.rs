//! Integration tests for `rowforge handler build [name] [--force]`.

use std::process::Command;

fn exe() -> &'static str {
    env!("CARGO_BIN_EXE_rowforge")
}

/// Write a minimal rowforge.yaml with an optional build command.
fn write_manifest(dir: &std::path::Path, name: &str, build: Option<&str>) {
    let build_line = match build {
        Some(cmd) => format!("  build: [\"sh\", \"-c\", \"{}\"]\n", cmd),
        None => String::new(),
    };
    std::fs::write(
        dir.join("rowforge.yaml"),
        format!(
            "name: {name}\nversion: 0.1.0\nentry:\n  cmd: [\"./handler\"]\n{build_line}",
        ),
    )
    .unwrap();
}

// ──────────────────────────────────────────────────────────────────────────────
// Test 1: build all handlers — one with build command, one without
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn handler_build_builds_all_handlers() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path();

    // Handler "alpha": has a build command that creates ./handler
    let alpha_dir = workspace.join("handlers/alpha");
    std::fs::create_dir_all(&alpha_dir).unwrap();
    write_manifest(
        &alpha_dir,
        "alpha",
        Some("touch handler && chmod +x handler"),
    );
    // A source file so needs_build sees alpha as stale (no binary yet).
    std::fs::write(alpha_dir.join("main.go"), "// placeholder\n").unwrap();

    // Handler "no-build": has NO build command
    let no_build_dir = workspace.join("handlers/no-build");
    std::fs::create_dir_all(&no_build_dir).unwrap();
    write_manifest(&no_build_dir, "no-build", None);

    let out = Command::new(exe())
        .env("ROWFORGE_HOME", workspace)
        .args(["handler", "build"])
        .output()
        .expect("rowforge handler build runs");

    let stderr = String::from_utf8_lossy(&out.stderr);

    // alpha should build successfully
    assert!(
        stderr.contains("[alpha] ok"),
        "expected '[alpha] ok' in stderr; got:\n{}",
        stderr
    );

    // no-build should be skipped
    assert!(
        stderr.contains("[no-build] skipped"),
        "expected '[no-build] skipped' in stderr; got:\n{}",
        stderr
    );

    // The binary must exist after the build
    assert!(
        alpha_dir.join("handler").exists(),
        "handler binary should exist in alpha dir after build"
    );

    // Exit code 0 (no failures)
    assert_eq!(
        out.status.code(),
        Some(0),
        "expected exit 0; stderr:\n{}",
        stderr
    );
}

// ──────────────────────────────────────────────────────────────────────────────
// Test 2: build a specific handler by name
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn handler_build_builds_specific_handler() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path();

    // Handler "alpha"
    let alpha_dir = workspace.join("handlers/alpha");
    std::fs::create_dir_all(&alpha_dir).unwrap();
    write_manifest(
        &alpha_dir,
        "alpha",
        Some("touch handler && chmod +x handler"),
    );
    std::fs::write(alpha_dir.join("main.go"), "// placeholder\n").unwrap();

    // Handler "beta" — should NOT be built
    let beta_dir = workspace.join("handlers/beta");
    std::fs::create_dir_all(&beta_dir).unwrap();
    write_manifest(
        &beta_dir,
        "beta",
        Some("touch handler && chmod +x handler"),
    );
    std::fs::write(beta_dir.join("main.go"), "// placeholder\n").unwrap();

    let out = Command::new(exe())
        .env("ROWFORGE_HOME", workspace)
        .args(["handler", "build", "alpha"])
        .output()
        .expect("rowforge handler build alpha runs");

    let stderr = String::from_utf8_lossy(&out.stderr);

    // alpha built
    assert!(
        stderr.contains("[alpha] ok"),
        "expected '[alpha] ok' in stderr; got:\n{}",
        stderr
    );

    // beta NOT mentioned (wasn't selected)
    assert!(
        !stderr.contains("[beta]"),
        "beta should not appear when only alpha is requested; got:\n{}",
        stderr
    );

    // alpha binary exists, beta binary does not
    assert!(
        alpha_dir.join("handler").exists(),
        "alpha handler binary should exist"
    );
    assert!(
        !beta_dir.join("handler").exists(),
        "beta handler binary should NOT exist when not selected"
    );

    assert_eq!(out.status.code(), Some(0));
}

// ──────────────────────────────────────────────────────────────────────────────
// Test 3: --force rebuilds even when the binary is already up to date
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn handler_build_force_rebuilds_even_when_fresh() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path();

    let alpha_dir = workspace.join("handlers/alpha");
    std::fs::create_dir_all(&alpha_dir).unwrap();
    // Build command appends a line to a log so we can count invocations.
    write_manifest(
        &alpha_dir,
        "alpha",
        Some("echo built >> build.log && touch handler && chmod +x handler"),
    );
    std::fs::write(alpha_dir.join("main.go"), "// placeholder\n").unwrap();

    // First build: stale (no binary yet) → must build
    let out1 = Command::new(exe())
        .env("ROWFORGE_HOME", workspace)
        .args(["handler", "build", "alpha"])
        .output()
        .expect("first build");
    let stderr1 = String::from_utf8_lossy(&out1.stderr);
    assert!(
        stderr1.contains("[alpha] ok"),
        "first build should succeed; stderr:\n{}",
        stderr1
    );
    assert!(alpha_dir.join("handler").exists());

    // Second build without --force: binary is fresh → "up to date"
    let out2 = Command::new(exe())
        .env("ROWFORGE_HOME", workspace)
        .args(["handler", "build", "alpha"])
        .output()
        .expect("second build (no force)");
    let stderr2 = String::from_utf8_lossy(&out2.stderr);
    assert!(
        stderr2.contains("[alpha] up to date"),
        "second build without --force should print 'up to date'; stderr:\n{}",
        stderr2
    );

    // Third build with --force: must rebuild regardless
    let out3 = Command::new(exe())
        .env("ROWFORGE_HOME", workspace)
        .args(["handler", "build", "--force", "alpha"])
        .output()
        .expect("third build (--force)");
    let stderr3 = String::from_utf8_lossy(&out3.stderr);
    assert!(
        stderr3.contains("[alpha] ok"),
        "--force should rebuild even when fresh; stderr:\n{}",
        stderr3
    );

    // Confirm build command ran at least twice (first + forced)
    let log = std::fs::read_to_string(alpha_dir.join("build.log")).unwrap_or_default();
    let count = log.lines().count();
    assert!(
        count >= 2,
        "expected at least 2 build invocations (initial + forced); got {}",
        count
    );

    assert_eq!(out3.status.code(), Some(0));
}
