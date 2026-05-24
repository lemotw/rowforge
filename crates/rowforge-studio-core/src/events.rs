//! Live progress events. Spec part-6 §6.1 (12-variant taxonomy).
//!
//! These cross the IPC boundary as `run:<handle>` Tauri events.
//! adjacently tagged JSON shape: `{ "type": "...", ... }`.

use crate::failed::RowOutcomeKind;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProgressEvent {
    // Lifecycle
    PhaseChanged { phase: Phase, at_ms: u64 },
    WorkerSpawned { worker_id: u32 },
    HandlerReady { worker_id: u32, handler_version: String, startup_ms: u32 },
    WorkerCrashed(WorkerCrashRecord),
    StallWarning { silent_secs: u32 },

    // Hot path
    Tick {
        seq: u64,
        at_ms: u64,
        processed: u64,
        total: Option<u64>,
        success: u64,
        failed: u64,
        crashed: u64,
        in_flight: u32,
        queue_depth: u32,
        rate_1s: f32,
        rate_10s: f32,
        eta_ms: Option<u64>,
    },
    OutcomeSample {
        row_index: u64,
        kind: RowOutcomeKind,
        code: Option<String>,
        message: Option<String>,
        dur_ms: u32,
    },
    BatchSummary {
        first_seq: u64,
        n: u32,
        success: u32,
        failed: u32,
        dur_ms: u32,
    },

    // Distinct from row failures
    PipelineWarning { code: String, message: String },
    HandlerStderr { worker_id: u32, line: String },

    // Terminal
    Done(RunReport),
    Aborted {
        reason: AbortReason,
        at_phase: Phase,
        partial_report: RunReport,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Phase {
    Initializing,
    Snapshotting,
    Starting,
    Running,
    Cancelling,
    Persisting,
}

#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct WorkerCrashRecord {
    pub worker_id: u32,
    pub last_seq: Option<u64>,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub stderr_tail: String, // ≤ 64 KiB
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum AbortReason {
    UserCancelled,
    HandlerStartupTimeout { failed_workers: u32, last_stderr: String },
    AllWorkersCrashed { crashes: Vec<WorkerCrashRecord> },
    Stalled { silent_secs: u32, last_seq: Option<u64> },
    MissingRequiredInput { columns: Vec<String> },
    SnapshotHashMismatch { path: std::path::PathBuf, expected: String, actual: String },
    OrphanedOnRestart,
    Crashed { panic_message: String },
    Internal { message: String },
}

#[derive(Debug, Clone, Default, Serialize)]
#[non_exhaustive]
pub struct RunReport {
    pub processed: u64,
    pub success: u64,
    pub failed: u64,
    pub crashed: u64,
    pub dur_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_serializes_with_type_field() {
        let ev = ProgressEvent::Tick {
            seq: 1,
            at_ms: 250,
            processed: 100,
            total: Some(1000),
            success: 95,
            failed: 5,
            crashed: 0,
            in_flight: 4,
            queue_depth: 12,
            rate_1s: 400.0,
            rate_10s: 380.0,
            eta_ms: Some(2_250),
        };
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(v.get("type").and_then(|t| t.as_str()), Some("tick"));
        assert_eq!(v.get("processed").and_then(|p| p.as_u64()), Some(100));
        assert_eq!(v.get("rate_1s").and_then(|p| p.as_f64()), Some(400.0));
    }

    #[test]
    fn abort_reason_serializes_as_kind_tagged() {
        let r = AbortReason::UserCancelled;
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v.get("kind").and_then(|k| k.as_str()), Some("user_cancelled"));
    }

    #[test]
    fn aborted_event_contains_reason_and_report() {
        let ev = ProgressEvent::Aborted {
            reason: AbortReason::Stalled { silent_secs: 30, last_seq: Some(42) },
            at_phase: Phase::Running,
            partial_report: RunReport {
                processed: 42, success: 40, failed: 2, crashed: 0, dur_ms: 5000,
            },
        };
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(v.get("type").and_then(|t| t.as_str()), Some("aborted"));
        assert_eq!(
            v.get("reason").and_then(|r| r.get("kind")).and_then(|k| k.as_str()),
            Some("stalled")
        );
        assert_eq!(
            v.get("partial_report").and_then(|p| p.get("processed")).and_then(|n| n.as_u64()),
            Some(42)
        );
    }

    #[test]
    fn worker_crashed_uses_newtype_payload() {
        // ProgressEvent::WorkerCrashed(WorkerCrashRecord) is a newtype tuple
        // variant. With serde adjacent tagging this serializes as:
        // { "type": "worker_crashed", "worker_id": ..., ... } — the inner
        // struct fields are inlined.
        let ev = ProgressEvent::WorkerCrashed(WorkerCrashRecord {
            worker_id: 2,
            last_seq: Some(99),
            exit_code: None,
            signal: Some(11),
            stderr_tail: "boom".into(),
        });
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(v.get("type").and_then(|t| t.as_str()), Some("worker_crashed"));
        // Lock the inlined shape — TS mirror at apps/rowforge-studio/src/ipc/types.ts
        // depends on top-level worker_id/last_seq/etc. fields.
        assert_eq!(v.get("worker_id").and_then(|w| w.as_u64()), Some(2));
        assert_eq!(v.get("last_seq").and_then(|s| s.as_u64()), Some(99));
        assert_eq!(v.get("signal").and_then(|s| s.as_i64()), Some(11));
        assert_eq!(v.get("stderr_tail").and_then(|s| s.as_str()), Some("boom"));
    }
}
