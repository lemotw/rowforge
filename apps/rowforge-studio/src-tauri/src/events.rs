//! Bridges between rowforge-studio-core's broadcast channels / streams
//! and Tauri's per-window emit. React side listens with
//! `@tauri-apps/api/event::listen(`run:<handle>`)` or `listen("runs:active")`.

use std::sync::Arc;

use rowforge_studio_core::{ProgressEvent, RunHandle, SessionRegistry};
use tauri::{AppHandle, Emitter};
use tokio::sync::broadcast;

/// Forward all events from a run's broadcast Receiver to Tauri's
/// `run:<handle>` event channel. Stops when the broadcast sender
/// drops (which happens when the ProgressAggregator drops).
pub async fn forward_run_events(
    app: AppHandle,
    handle: RunHandle,
    mut rx: broadcast::Receiver<ProgressEvent>,
) {
    let channel = format!("run:{}", handle.as_str());
    eprintln!("[forward_run_events] starting for {channel}");
    let mut count: u64 = 0;
    loop {
        match rx.recv().await {
            Ok(event) => {
                count += 1;
                // Only log non-tick events to avoid spam (4Hz).
                let kind = match &event {
                    ProgressEvent::Tick { .. } => "tick",
                    ProgressEvent::PhaseChanged { .. } => "phase_changed",
                    ProgressEvent::OutcomeSample { .. } => "outcome_sample",
                    ProgressEvent::Done { .. } => "done",
                    ProgressEvent::Aborted { .. } => "aborted",
                    ProgressEvent::WorkerCrashed { .. } => "worker_crashed",
                    _ => "other",
                };
                if kind != "tick" || count <= 3 || count % 20 == 0 {
                    eprintln!("[forward_run_events] #{count} {kind} -> {channel}");
                }
                if let Err(e) = app.emit(&channel, &event) {
                    tracing::warn!("failed to emit {channel}: {e}");
                    eprintln!("[forward_run_events] EMIT FAILED: {e}");
                }
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                eprintln!("[forward_run_events] LAGGED — dropped {n} events");
                let warning = ProgressEvent::PipelineWarning {
                    code: "EVENT_LAG".into(),
                    message: format!("{} events dropped", n),
                };
                let _ = app.emit(&channel, &warning);
            }
            Err(broadcast::error::RecvError::Closed) => {
                eprintln!("[forward_run_events] CLOSED — sent {count} events total");
                break;
            }
        }
    }
}

/// Polls the workspace-level rollup stream and emits a `runs:active`
/// event every 1 second. Runs for the lifetime of the StudioCore.
pub async fn forward_active_runs(
    app: AppHandle,
    sessions: Arc<SessionRegistry>,
) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        let tick = sessions.rollup_tick();
        let _ = app.emit("runs:active", &tick);
    }
}
