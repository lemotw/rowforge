//! Lock the JSON shape that crosses the Tauri IPC boundary.
//!
//! Hand-written TS mirrors at `apps/rowforge-studio/src/ipc/types.ts`
//! depend on these keys. Any rename here without updating TS is a UI
//! breakage; this test forces them to move together.

use rowforge_studio_core::{ExecSummary, UiError};

#[test]
fn workspace_json_keys() {
    // Workspace is #[non_exhaustive] — construct via StudioCore::open with a
    // temporary real workspace, then round-trip through JSON to verify keys.
    use rowforge_studio_core::OpenOpts;
    let tmp = std::env::temp_dir().join("rowforge_ipc_contract_ws");
    std::fs::create_dir_all(&tmp).ok();
    let core = rowforge_studio_core::StudioCore::open(
        OpenOpts::new().with_workspace(tmp.clone()),
    )
    .expect("open tmp workspace");
    let v = serde_json::to_value(core.workspace()).unwrap();
    std::fs::remove_dir_all(&tmp).ok();
    assert!(v.get("root").is_some(), "root key missing: got {v:?}");
    assert!(v.get("schema_version").is_some(), "schema_version key missing: got {v:?}");
}

#[test]
fn exec_summary_json_keys() {
    // ExecSummary is #[non_exhaustive] — we can deserialize but cannot
    // construct externally with a struct literal. Deserialization is
    // sufficient for this shape test.
    let json = r#"{
        "id":"e1","name":"x","created_at":"2026-05-24T12:00:00Z",
        "input_rows":42,"attempts_count":0,
        "last_attempt_state":null,"last_attempt_counts":null
    }"#;
    let parsed: ExecSummary = serde_json::from_str(json).expect("deserialize");
    assert_eq!(parsed.id.as_str(), "e1");
    assert_eq!(parsed.input_rows, Some(42));
}

#[test]
fn ui_error_workspace_unavailable_shape() {
    let err = UiError::WorkspaceUnavailable("no home".into());
    let v = serde_json::to_value(&err).unwrap();
    assert!(v.get("kind").is_some(), "kind missing: {v:?}");
    let kind = v.get("kind").and_then(|k| k.as_str()).unwrap();
    assert_eq!(kind, "workspace_unavailable", "kind value");
    assert_eq!(
        v.get("message").and_then(|m| m.as_str()),
        Some("no home"),
        "UiError content field must be 'message' (adjacent tagging): {v:?}"
    );
}

#[test]
fn ui_error_internal_shape() {
    let err = UiError::Internal("boom".into());
    let v = serde_json::to_value(&err).unwrap();
    assert_eq!(v.get("kind").and_then(|k| k.as_str()).unwrap(), "internal");
    assert_eq!(
        v.get("message").and_then(|m| m.as_str()),
        Some("boom"),
        "UiError content field must be 'message' (adjacent tagging): {v:?}"
    );
}
