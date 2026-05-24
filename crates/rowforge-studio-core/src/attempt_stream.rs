//! AttemptStream abstraction: unifies live (from SessionRegistry) and
//! replay (from outcomes.jsonl). Spec part-6 §6.4.

use std::pin::Pin;
use std::sync::Arc;

use futures::Stream;
use tokio_stream::wrappers::BroadcastStream;

use crate::aggregator::ProgressSnapshot;
use crate::events::ProgressEvent;
use crate::session::Session;

/// Common interface for live + replay attempt event streams.
///
/// `snapshot()` returns counters at the time of subscription;
/// `events()` returns an async stream of subsequent events.
pub trait AttemptStream: Send {
    fn snapshot(&self) -> ProgressSnapshot;
    fn events(self: Box<Self>) -> Pin<Box<dyn Stream<Item = ProgressEvent> + Send>>;
}

/// Wraps a live `Session` from the SessionRegistry — exposes its
/// aggregator's broadcast as the event stream.
pub struct LiveAttemptStream {
    session: Arc<Session>,
}

impl LiveAttemptStream {
    pub fn new(session: Arc<Session>) -> Self {
        Self { session }
    }
}

impl AttemptStream for LiveAttemptStream {
    fn snapshot(&self) -> ProgressSnapshot {
        self.session.aggregator.snapshot()
    }

    fn events(self: Box<Self>) -> Pin<Box<dyn Stream<Item = ProgressEvent> + Send>> {
        use futures::StreamExt;
        let rx = self.session.aggregator.subscribe();
        Box::pin(BroadcastStream::new(rx).filter_map(|r| async move { r.ok() }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregator::ProgressAggregator;
    use crate::events::{Phase, ProgressEvent};
    use crate::run_handle::{RunHandle, RunStatus};
    use crate::session::Session;
    use std::sync::Mutex;
    use std::time::Instant;
    use tokio::sync::watch;
    use tokio_util::sync::CancellationToken;

    fn fake_session() -> Arc<Session> {
        let (tick_stop, _) = watch::channel(false);
        Arc::new(Session {
            handle: RunHandle::new(),
            execution_id: "e1".into(),
            aggregator: Arc::new(ProgressAggregator::new()),
            cancel_token: CancellationToken::new(),
            tick_stop,
            status: Mutex::new(RunStatus::Running),
            started_at: Instant::now(),
        })
    }

    #[tokio::test]
    async fn live_snapshot_reflects_aggregator() {
        let session = fake_session();
        session.aggregator.on_outcome_success(0, 5);
        session.aggregator.on_outcome_success(1, 6);
        let live = LiveAttemptStream::new(session);
        let snap = live.snapshot();
        assert_eq!(snap.processed, 2);
        assert_eq!(snap.success, 2);
    }

    #[tokio::test]
    async fn live_events_receives_phase_change() {
        use futures::StreamExt;
        let session = fake_session();
        let live = Box::new(LiveAttemptStream::new(session.clone()));
        let mut events = live.events();

        // Emit a phase change AFTER subscription.
        tokio::spawn({
            let agg = session.aggregator.clone();
            async move {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                agg.set_phase(Phase::Running);
            }
        });

        let event = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            events.next(),
        ).await.expect("timed out").expect("stream ended");

        assert!(matches!(
            event,
            ProgressEvent::PhaseChanged { phase: Phase::Running, .. }
        ));
    }
}
