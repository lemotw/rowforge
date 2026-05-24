//! ExecDetail projection — entity page. Spec part-2 §2.2.3 + W-3.

use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::{AttemptCountsStub, AttemptId, ExecSummary};

#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct ExecDetail {
    pub summary: ExecSummary,
    pub input_path_snapshot: PathBuf,
    pub input_format: InputFormat,
    pub handler_binding: HandlerBindingView,
    pub attempts: Vec<AttemptSummary>,
    pub field_mapping: Option<FieldMapping>,
    pub config_overrides: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum InputFormat {
    Csv,
    Jsonl,
    Ndjson,
}

#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct HandlerBindingView {
    pub handler_id: Option<String>,
    pub handler_instance_id: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct AttemptSummary {
    pub id: AttemptId,
    pub state: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub run_type: String,
    pub stats: Option<AttemptCountsStub>,
}

#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct FieldMapping {
    pub fields: BTreeMap<String, String>,
}
