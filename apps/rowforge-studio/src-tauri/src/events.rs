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
    loop {
        match rx.recv().await {
            Ok(event) => {
                if let Err(e) = app.emit(&channel, &event) {
                    tracing::warn!("failed to emit {channel}: {e}");
                }
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                // Spec §6.2: emit PipelineWarning EVENT_LAG when receiver lags.
                let warning = ProgressEvent::PipelineWarning {
                    code: "EVENT_LAG".into(),
                    message: format!("{} events dropped", n),
                };
                let _ = app.emit(&channel, &warning);
            }
            Err(broadcast::error::RecvError::Closed) => {
                // Sender dropped — terminal.
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
