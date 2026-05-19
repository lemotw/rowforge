//! End-to-end test for `rowforge pack` bundle assembly.
//!
//! Uses the real example handlers under `examples/handlers/` plus a fake
//! "rowforge" binary (any non-empty file) to exercise `assemble_bundle`
//! end-to-end. The cross-compile path (`cargo zigbuild`) is exercised
//! manually per B3 and is intentionally not invoked here so the test can
//! run on machines without zigbuild installed.

use std::fs::{self, File};
use std::io::Read;
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    // tests/ lives at crates/rowforge-cli/tests/. Walk up two parents to
    // reach the workspace root (.../crates/rowforge-cli -> .../crates -> .../).
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root above crates/rowforge-cli")
        .to_path_buf()
}

#[test]
fn pack_assembly_produces_runnable_layout() {
    let root = workspace_root();
    let py_handler = root.join("examples/handlers/python3-uppercase");
    let go_handler = root.join("examples/handlers/golang-uppercase");
    assert!(py_handler.exists(), "missing example handler: {}", py_handler.display());
    assert!(go_handler.exists(), "missing example handler: {}", go_handler.display());

    let scratch = tempfile::tempdir().unwrap();
    let fake_bin = scratch.path().join("fake-rowforge");
    fs::write(&fake_bin, b"#!/bin/sh\necho hi\n").unwrap();

    let out_zip = scratch.path().join("bundle.zip");
    let target = rowforge_cli::pack_cmd::parse_target("linux-x86_64").unwrap();

    rowforge_cli::pack_cmd::assemble_bundle(
        &fake_bin,
        &[py_handler, go_handler],
        &target,
        &out_zip,
    )
    .expect("assemble_bundle should succeed");

    assert!(out_zip.exists(), "expected zip at {}", out_zip.display());

    let f = File::open(&out_zip).unwrap();
    let mut zr = zip::ZipArchive::new(f).unwrap();
    let names: Vec<String> = (0..zr.len())
        .map(|i| zr.by_index(i).unwrap().name().to_string())
        .collect();

    assert!(
        names.iter().any(|n| n == "rowforge"),
        "expected top-level rowforge binary, got {:?}",
        names
    );
    assert!(
        names.iter().any(|n| n == "README.md"),
        "expected README.md, got {:?}",
        names
    );
    assert!(
        names.iter().any(|n| n.starts_with("handlers/python3-uppercase/")),
        "expected handlers/python3-uppercase/* entries, got {:?}",
        names
    );
    assert!(
        names.iter().any(|n| n.starts_with("handlers/golang-uppercase/")),
        "expected handlers/golang-uppercase/* entries, got {:?}",
        names
    );

    // Read README and assert it mentions both handlers + the target alias.
    let mut readme = String::new();
    zr.by_name("README.md")
        .expect("README.md present")
        .read_to_string(&mut readme)
        .unwrap();
    assert!(readme.contains("python3-uppercase"), "README missing python3-uppercase: {}", readme);
    assert!(readme.contains("golang-uppercase"), "README missing golang-uppercase: {}", readme);
    assert!(readme.contains("linux-x86_64"), "README missing target alias: {}", readme);
}

#[test]
fn pack_rejects_unknown_target() {
    let res = rowforge_cli::pack_cmd::parse_target("freebsd-x86_64");
    assert!(res.is_err(), "expected Err for unknown target, got {:?}", res.is_ok());
}
