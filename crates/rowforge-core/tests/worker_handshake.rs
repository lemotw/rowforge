use rowforge_core::manifest::{Entry, Manifest};
use rowforge_core::protocol::{Inbound, Outbound, RowMeta};
use rowforge_core::worker::Worker;
use std::collections::BTreeMap;

fn test_handler_path() -> std::path::PathBuf {
    use std::sync::Once;
    static BUILD: Once = Once::new();
    BUILD.call_once(|| {
        let status = std::process::Command::new("cargo")
            .args(["build", "-p", "test-handler"])
            .status()
            .expect("invoking `cargo build -p test-handler`");
        assert!(status.success(), "cargo build -p test-handler failed");
    });
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest.parent().unwrap().parent().unwrap();
    workspace_root.join("target/debug/test-handler")
}

fn echo_manifest() -> Manifest {
    Manifest {
        name: "echo".into(),
        version: "0.0.0".into(),
        description: String::new(),
        language: String::new(),
        entry: Entry {
            cmd: vec![test_handler_path().to_string_lossy().into(), "echo".into()],
            build: None,
            cwd: ".".into(),
            env: Default::default(),
            startup_timeout_ms: 5000,
        },
        required_input: vec![],
        config: BTreeMap::new(),
        runtime: None,
        output: None,
    }
}

#[tokio::test]
async fn worker_handshake_and_single_row() {
    let m = echo_manifest();
    let dir = std::env::temp_dir(); // cwd doesn't matter for this handler
    let mut w = Worker::spawn(0, &dir, &m, "test", &BTreeMap::new(), &["x".into()])
        .await
        .expect("spawn");
    assert_eq!(w.handler_version, "0.0.0");

    let row = Outbound::Row {
        seq: 0,
        data: serde_json::Map::from_iter([("x".into(), serde_json::json!("hello"))]),
        meta: RowMeta {
            dry_run: false,
            row_index: 0,
        },
    };
    w.send_row(&row).await.unwrap();

    match w.recv().await.unwrap().unwrap() {
        Inbound::Result { seq, data } => {
            assert_eq!(seq, 0);
            assert_eq!(
                data.get("echoed").unwrap().get("x").unwrap(),
                &serde_json::json!("hello")
            );
        }
        other => panic!("expected Result, got {:?}", other),
    }

    let code = w.shutdown(std::time::Duration::from_secs(2)).await.unwrap();
    assert_eq!(code, Some(0));
}
