use rowforge_core::error::CoreError;
use rowforge_core::manifest::{Entry, Manifest};
use rowforge_core::worker::Worker;
use std::collections::BTreeMap;
use std::path::PathBuf;

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

fn no_ready_manifest(timeout_ms: u64) -> Manifest {
    Manifest {
        name: "noready".into(),
        version: "0.0.0".into(),
        description: String::new(),
        language: String::new(),
        entry: Entry {
            cmd: vec![
                test_handler_path().to_string_lossy().into(),
                "no-ready".into(),
            ],
            build: None,
            cwd: ".".into(),
            env: Default::default(),
            startup_timeout_ms: timeout_ms,
        },
        required_input: vec![],
        config: BTreeMap::new(),
        runtime: None,
        output: None,
    }
}

#[tokio::test]
async fn spawn_returns_startup_timeout_when_no_ready() {
    let m = no_ready_manifest(500);
    let res = Worker::spawn(
        0,
        &std::env::temp_dir(),
        &m,
        "t",
        &BTreeMap::new(),
        &["x".into()],
    )
    .await;
    match res {
        Err(CoreError::StartupTimeout { timeout_ms }) => assert_eq!(timeout_ms, 500),
        Err(e) => panic!("expected StartupTimeout, got error: {}", e),
        Ok(_) => panic!("expected StartupTimeout, got Ok"),
    }
}
