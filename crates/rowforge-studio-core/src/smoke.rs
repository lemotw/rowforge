//! Plan 13 — Handler smoke test types and runner.
//!
//! See `docs/superpowers/specs/2026-05-26-studio-plan-13-handler-smoke-test-design.md`.

use serde::{Deserialize, Serialize};

/// One row's outcome from a smoke run. Status mirrors the wire protocol's
/// `Inbound::Result` / `Inbound::Error` variants, plus a `"crash"` sentinel
/// when the handler exited mid-run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct SmokeOutcome {
    pub seq: u64,
    pub status: String, // "success" | "error" | "crash"
    pub code: Option<String>,
    pub message: Option<String>,
    pub dur_ms: u64,
    pub data: Option<serde_json::Value>,
}

/// Request payload for `StudioCore::handler_smoke_run`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SmokeRunRequest {
    pub handler_name: String,
    pub rows: Vec<serde_json::Map<String, serde_json::Value>>,
}

/// Result returned by `StudioCore::handler_smoke_run`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SmokeRunResult {
    pub outcomes: Vec<SmokeOutcome>,
    pub stderr_tail: String,
    pub exit_code: Option<i32>,
    pub elapsed_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke_outcome_serializes_with_camel_compatible_snake() {
        let o = SmokeOutcome {
            seq: 3,
            status: "success".into(),
            code: None,
            message: None,
            dur_ms: 42,
            data: Some(serde_json::json!({"sent": true})),
        };
        let v = serde_json::to_value(&o).unwrap();
        assert_eq!(v["seq"], serde_json::json!(3));
        assert_eq!(v["status"], serde_json::json!("success"));
        assert_eq!(v["dur_ms"], serde_json::json!(42));
        assert_eq!(v["data"]["sent"], serde_json::json!(true));
        // None fields render as null (not omitted) — keeps TS type stable.
        assert_eq!(v["code"], serde_json::Value::Null);
    }

    #[test]
    fn smoke_run_request_roundtrip() {
        let req = SmokeRunRequest {
            handler_name: "alpha".into(),
            rows: vec![
                serde_json::Map::from_iter([
                    ("id".to_string(), serde_json::json!("1")),
                ]),
            ],
        };
        let s = serde_json::to_string(&req).unwrap();
        let parsed: SmokeRunRequest = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed.handler_name, "alpha");
        assert_eq!(parsed.rows.len(), 1);
    }

    #[test]
    fn smoke_run_result_roundtrip() {
        let r = SmokeRunResult {
            outcomes: vec![],
            stderr_tail: "boot\n".into(),
            exit_code: Some(0),
            elapsed_ms: 100,
        };
        let s = serde_json::to_string(&r).unwrap();
        let parsed: SmokeRunResult = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed.exit_code, Some(0));
        assert_eq!(parsed.stderr_tail, "boot\n");
    }
}
