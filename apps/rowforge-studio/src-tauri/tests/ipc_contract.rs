//! Lock the JSON shape that crosses the Tauri IPC boundary.
//!
//! Hand-written TS mirrors at `apps/rowforge-studio/src/ipc/types.ts`
//! depend on these keys. Any rename here without updating TS is a UI
//! breakage; this test forces them to move together.

use rowforge_studio_core::{
    ExecSummary, ExecutionId, ExportFormat, ExportOpts, ManifestReport, ManifestSource, UiError,
};

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
fn ui_error_workspace_locked_shape() {
    let err = UiError::WorkspaceLocked("no home".into());
    let v = serde_json::to_value(&err).unwrap();
    assert!(v.get("kind").is_some(), "kind missing: {v:?}");
    let kind = v.get("kind").and_then(|k| k.as_str()).unwrap();
    assert_eq!(kind, "workspace_locked", "kind value");
    assert_eq!(
        v.get("message").and_then(|m| m.as_str()),
        Some("no home"),
        "UiError content field must be 'message' (adjacent tagging): {v:?}"
    );
}

#[test]
fn ui_error_not_found_shape() {
    let err = UiError::NotFound("execution e1 not found".into());
    let v = serde_json::to_value(&err).unwrap();
    assert_eq!(v.get("kind").and_then(|k| k.as_str()).unwrap(), "not_found");
    assert_eq!(
        v.get("message").and_then(|m| m.as_str()),
        Some("execution e1 not found"),
        "message field expected"
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

// ---------------------------------------------------------------------------
// T9 — Plan 5 new command boundary types
// ---------------------------------------------------------------------------

/// exec_start: ExecutionId crosses IPC boundary as a string scalar.
#[test]
fn plan5_execution_id_json_shape() {
    let id = ExecutionId::from("exec-abc123");
    let v = serde_json::to_value(&id).unwrap();
    assert_eq!(v.as_str(), Some("exec-abc123"), "ExecutionId must serialise as a bare string: {v:?}");

    let roundtrip: ExecutionId = serde_json::from_value(v).unwrap();
    assert_eq!(roundtrip.as_str(), "exec-abc123");
}

/// exec_export: ExportOpts is the arg type — must round-trip through JSON.
#[test]
fn plan5_export_opts_json_roundtrip() {
    let json = r#"{"format":"csv","require_complete":false,"output_dir":null}"#;
    let opts: ExportOpts = serde_json::from_str(json).expect("deserialize ExportOpts");
    assert_eq!(opts.format, ExportFormat::Csv);
    assert!(!opts.require_complete);

    let re = serde_json::to_string(&opts).unwrap();
    // round-trip: can deserialise what we serialised
    let _back: ExportOpts = serde_json::from_str(&re).unwrap();
}

/// manifest_validate: ManifestSource is the arg type — must deserialise from
/// the tagged JSON shape the React layer will send.
#[test]
fn plan5_manifest_source_json_shape() {
    let json = r#"{"type":"path","path":"/some/handler"}"#;
    let src: ManifestSource = serde_json::from_str(json).expect("deserialize ManifestSource");
    match &src {
        ManifestSource::Path { path } => {
            assert_eq!(path.to_str().unwrap(), "/some/handler");
        }
        _ => panic!("unexpected variant"),
    }
}

/// manifest_validate: ManifestReport (the return type) has the expected keys.
#[test]
fn plan5_manifest_report_json_keys() {
    // ManifestReport is #[non_exhaustive] — construct via round-trip from JSON.
    let json = r#"{"manifest":null,"errors":[],"warnings":[]}"#;
    let report: ManifestReport = serde_json::from_str(json).expect("deserialize ManifestReport");
    let v = serde_json::to_value(&report).unwrap();
    assert!(v.get("manifest").is_some(), "manifest key missing: {v:?}");
    assert!(v.get("errors").is_some(), "errors key missing: {v:?}");
    assert!(v.get("warnings").is_some(), "warnings key missing: {v:?}");
}
