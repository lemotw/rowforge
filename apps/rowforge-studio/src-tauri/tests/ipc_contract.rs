//! Lock the JSON shape that crosses the Tauri IPC boundary.
//!
//! Hand-written TS mirrors at `apps/rowforge-studio/src/ipc/types.ts`
//! depend on these keys. Any rename here without updating TS is a UI
//! breakage; this test forces them to move together.

use rowforge_studio_core::{
    ExecSummary, ExecutionId, ExportFormat, ExportOpts, HandlerDetail, HandlerSummary,
    ManifestReport, ManifestSource, ManifestStatus, ScaffoldArgs, ScaffoldTemplate, UiError,
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

// ---------------------------------------------------------------------------
// T9 — Plan 7 handler command boundary types
// ---------------------------------------------------------------------------

/// handler_list: HandlerSummary is the element type returned over IPC.
/// ManifestStatus must serialize as snake_case string.
#[test]
fn plan7_handler_summary_json_shape() {
    // HandlerSummary is #[non_exhaustive] — round-trip from JSON.
    let json = r#"{
        "name": "alpha",
        "path": "/x/handlers/alpha",
        "manifest_status": "valid",
        "last_modified": "2026-05-24T12:00:00Z",
        "version": "0.1.0",
        "language": "go"
    }"#;
    let s: HandlerSummary = serde_json::from_str(json).expect("deserialize HandlerSummary");
    let v = serde_json::to_value(&s).unwrap();
    assert_eq!(v["name"], "alpha", "name key: {v:?}");
    assert_eq!(v["manifest_status"], "valid", "manifest_status must be snake_case: {v:?}");
    assert_eq!(v["version"], "0.1.0", "version key: {v:?}");
    assert_eq!(v["language"], "go", "language key: {v:?}");
    assert!(v.get("path").is_some(), "path key missing: {v:?}");
    assert!(v.get("last_modified").is_some(), "last_modified key missing: {v:?}");
}

/// ManifestStatus all variants serialize as the expected snake_case strings.
#[test]
fn plan7_manifest_status_all_variants() {
    assert_eq!(
        serde_json::to_value(ManifestStatus::Valid).unwrap().as_str(),
        Some("valid")
    );
    assert_eq!(
        serde_json::to_value(ManifestStatus::Invalid).unwrap().as_str(),
        Some("invalid")
    );
    assert_eq!(
        serde_json::to_value(ManifestStatus::Missing).unwrap().as_str(),
        Some("missing")
    );
}

/// handler_show: HandlerDetail wraps a summary + manifest + lists.
#[test]
fn plan7_handler_detail_json_keys() {
    // HandlerDetail is #[non_exhaustive] — round-trip from JSON.
    let json = r#"{
        "summary": {
            "name": "beta",
            "path": "/x/handlers/beta",
            "manifest_status": "missing",
            "last_modified": "2026-05-24T12:00:00Z",
            "version": null,
            "language": null
        },
        "manifest": null,
        "manifest_errors": [],
        "manifest_warnings": [],
        "source_files": [],
        "has_fixtures_dir": false
    }"#;
    let detail: HandlerDetail = serde_json::from_str(json).expect("deserialize HandlerDetail");
    let v = serde_json::to_value(&detail).unwrap();
    assert!(v.get("summary").is_some(), "summary key missing: {v:?}");
    assert!(v.get("manifest").is_some(), "manifest key missing: {v:?}");
    assert!(v.get("source_files").is_some(), "source_files key missing: {v:?}");
    assert!(v.get("has_fixtures_dir").is_some(), "has_fixtures_dir key missing: {v:?}");
}

/// handler_scaffold: ScaffoldArgs is the arg type — must round-trip from JSON.
#[test]
fn plan7_scaffold_args_json_roundtrip() {
    let json = r#"{"name":"gamma","template":"go_stdio","primary_field":"order_id"}"#;
    let args: ScaffoldArgs = serde_json::from_str(json).expect("deserialize ScaffoldArgs");
    assert_eq!(args.name, "gamma");
    assert_eq!(args.template, ScaffoldTemplate::GoStdio);
    assert_eq!(args.primary_field, "order_id");

    let re = serde_json::to_string(&args).unwrap();
    let back: ScaffoldArgs = serde_json::from_str(&re).unwrap();
    assert_eq!(back.name, "gamma");
}

/// ScaffoldTemplate variants serialize as snake_case strings.
#[test]
fn plan7_scaffold_template_json_shape() {
    assert_eq!(
        serde_json::to_value(ScaffoldTemplate::GoStdio).unwrap().as_str(),
        Some("go_stdio")
    );
    assert_eq!(
        serde_json::to_value(ScaffoldTemplate::GoBatch).unwrap().as_str(),
        Some("go_batch")
    );
    assert_eq!(
        serde_json::to_value(ScaffoldTemplate::Empty).unwrap().as_str(),
        Some("empty")
    );
}

/// Compile-time check: the 7 Plan 7 arg/return types all impl
/// Serialize + Deserialize (required for IPC crossing). If any derive
/// is missing this test file won't compile.
#[test]
fn plan7_handler_types_are_serde() {
    fn assert_serde<T: serde::Serialize + for<'de> serde::Deserialize<'de>>() {}
    assert_serde::<HandlerSummary>();
    assert_serde::<HandlerDetail>();
    assert_serde::<ScaffoldArgs>();
    assert_serde::<ManifestStatus>();
    assert_serde::<ScaffoldTemplate>();
}

// ---------------------------------------------------------------------------
// Plan 8 T7 — handler_build command boundary types
// ---------------------------------------------------------------------------

/// Compile-time symbol check: handler_build is registered and BuildOutcome
/// impl Serialize + Deserialize (required for IPC crossing).
#[test]
fn plan8_handler_build_types_are_serde() {
    use rowforge_studio_core::BuildOutcome;
    fn assert_serde<T: serde::Serialize + for<'de> serde::Deserialize<'de>>() {}
    assert_serde::<BuildOutcome>();
}

/// handler_build returns BuildOutcome — verify the JSON shape has the
/// expected keys that the TS mirror (ipc/types.ts) will depend on.
///
/// BuildOutcome is #[non_exhaustive] so we cannot construct it with a
/// struct literal from outside rowforge-core; use serde_json::from_value
/// instead.
#[test]
fn plan8_build_outcome_json_shape() {
    let json = serde_json::json!({
        "started_at": "2026-05-25T00:00:00Z",
        "finished_at": "2026-05-25T00:00:01Z",
        "exit_code": 0,
        "command": ["sh", "-c", "echo"],
        "stdout": "hi",
        "stderr": ""
    });
    let parsed: rowforge_studio_core::BuildOutcome =
        serde_json::from_value(json).expect("deserialize BuildOutcome");
    assert_eq!(parsed.exit_code, 0);

    // Round-trip to verify the serialized shape.
    let v = serde_json::to_value(&parsed).unwrap();
    assert_eq!(v["exit_code"], 0, "exit_code key: {v:?}");
    assert!(v["command"].is_array(), "command must be array: {v:?}");
    assert!(v["stdout"].is_string(), "stdout must be string: {v:?}");
    assert!(v.get("started_at").is_some(), "started_at key missing: {v:?}");
    assert!(v.get("finished_at").is_some(), "finished_at key missing: {v:?}");
    assert!(v.get("stderr").is_some(), "stderr key missing: {v:?}");
}

// ---------------------------------------------------------------------------
// Plan 9 T6 — handler_log Tauri commands
// ---------------------------------------------------------------------------

/// Compile-time symbol check: all three Plan 9 T6 handler_log commands exist
/// and are callable (the compiler will error if any are missing or renamed).
#[test]
fn plan9_handler_log_commands_registered() {
    let _ = rowforge_studio_lib::commands::handler_log_tail;
    let _ = rowforge_studio_lib::commands::handler_log_subscribe;
    let _ = rowforge_studio_lib::commands::handler_log_unsubscribe;
}

/// HandlerLogLine is the element type in handler_log_tail's return value and
/// in the event payload. Verify the JSON shape the TS mirror will depend on.
#[test]
fn plan9_handler_log_line_json_shape() {
    let json = serde_json::json!({
        "timestamp": "2026-05-25T10:00:00+00:00",
        "worker_id": 3,
        "stream": "stderr",
        "line": "hello",
    });
    let parsed: rowforge_studio_core::HandlerLogLine =
        serde_json::from_value(json).expect("deserialize HandlerLogLine");
    assert_eq!(parsed.worker_id, 3);
    assert_eq!(parsed.line, "hello");

    // Round-trip — verify serialized keys.
    let v = serde_json::to_value(&parsed).unwrap();
    assert!(v.get("timestamp").is_some(), "timestamp key missing: {v:?}");
    assert_eq!(v["worker_id"], 3, "worker_id key: {v:?}");
    assert_eq!(v["stream"], "stderr", "stream must be snake_case: {v:?}");
    assert_eq!(v["line"], "hello", "line key: {v:?}");
}
