//! RunHandle: opaque session ID returned by start_run; passed back to
//! cancel/subscribe/status. Serializable so React side can store it.
//!
//! Spec part-2 §2.2.8, part-3 §3.3.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RunHandle(String);

impl RunHandle {
    pub fn new() -> Self {
        Self(format!("run-{}", ulid::Ulid::new()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for RunHandle {
    fn default() -> Self { Self::new() }
}

impl std::fmt::Display for RunHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for RunHandle {
    fn from(s: String) -> Self { Self(s) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RunStatus {
    Pending,
    Starting,
    Running,
    Cancelling,
    Done,
    Aborted,
    Crashed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CancelMode {
    Soft,
    Hard,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_handle_serializes_transparently() {
        let h = RunHandle::from("run-abc".to_string());
        let v = serde_json::to_value(&h).unwrap();
        assert_eq!(v, serde_json::Value::String("run-abc".into()));
    }

    #[test]
    fn run_handle_deserializes_from_string() {
        let h: RunHandle = serde_json::from_str(r#""run-xyz""#).unwrap();
        assert_eq!(h.as_str(), "run-xyz");
    }

    #[test]
    fn run_handle_new_has_run_prefix() {
        let h = RunHandle::new();
        assert!(h.as_str().starts_with("run-"));
        assert!(h.as_str().len() > 4, "{} too short", h);
    }

    #[test]
    fn run_status_snake_case_serialization() {
        let v = serde_json::to_value(&RunStatus::Cancelling).unwrap();
        assert_eq!(v, serde_json::Value::String("cancelling".into()));
    }

    #[test]
    fn cancel_mode_snake_case_serialization() {
        let v = serde_json::to_value(&CancelMode::Hard).unwrap();
        assert_eq!(v, serde_json::Value::String("hard".into()));
    }
}
