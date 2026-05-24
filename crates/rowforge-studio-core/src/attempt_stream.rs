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

// ---------------------------------------------------------------------------
// ReplayAttemptStream
// ---------------------------------------------------------------------------

use std::path::PathBuf;
use std::time::Duration;

use crate::events::{Phase, RunReport};

/// Replay stream — reads `outcomes.jsonl` and synthesizes events at
/// 4 Hz (scaled by `speed`). Use for terminal attempts.
pub struct ReplayAttemptStream {
    snapshot: ProgressSnapshot,
    outcomes_path: PathBuf,
    speed: f32,
}

impl ReplayAttemptStream {
    /// `attempt_dir` is `<workspace>/executions/<exec>/attempts/<aid>`.
    /// `speed` is the playback multiplier (1.0 = real-time 250 ms ticks,
    /// 10.0 = 25 ms between synthesized ticks).
    pub fn from_attempt(
        attempt_dir: &std::path::Path,
        speed: f32,
    ) -> Result<Self, std::io::Error> {
        let meta_path = attempt_dir.join("meta.json");
        let snapshot = read_initial_snapshot(&meta_path);
        let outcomes_path = attempt_dir.join("outcomes.jsonl");
        Ok(Self {
            snapshot,
            outcomes_path,
            speed: speed.max(0.1).min(20.0),
        })
    }
}

fn read_initial_snapshot(meta_path: &std::path::Path) -> ProgressSnapshot {
    let mut snap = ProgressSnapshot::default();
    // meta.json carries total + final stats. Replay's "initial snapshot" is
    // empty (counters start at zero and build up as outcomes are read), but
    // we use meta.json to populate `total` so the progress bar percent works.
    if let Ok(bytes) = std::fs::read(meta_path) {
        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            if let Some(t) = v.get("input_row_count").and_then(|x| x.as_u64()) {
                snap.total = Some(t);
            }
        }
    }
    snap.phase = Some(Phase::Running);
    snap
}

impl AttemptStream for ReplayAttemptStream {
    fn snapshot(&self) -> ProgressSnapshot {
        self.snapshot.clone()
    }

    fn events(self: Box<Self>) -> Pin<Box<dyn Stream<Item = ProgressEvent> + Send>> {
        let outcomes_path = self.outcomes_path.clone();
        let speed = self.speed;
        let total = self.snapshot.total;
        Box::pin(async_stream::stream! {
            use std::io::{BufRead, BufReader};
            use std::time::Instant;

            let started = Instant::now();
            let tick_dur = Duration::from_millis((250.0 / speed) as u64);
            let mut tick_seq: u64 = 0;
            let mut processed: u64 = 0;
            let mut success: u64 = 0;
            let mut failed: u64 = 0;
            let mut crashed: u64 = 0;
            let mut last_tick = Instant::now();

            // Emit initial PhaseChanged so the UI shows the running phase.
            yield ProgressEvent::PhaseChanged {
                phase: Phase::Running,
                at_ms: 0,
            };

            let f = match std::fs::File::open(&outcomes_path) {
                Ok(f) => f,
                Err(_) => {
                    // No outcomes file — emit empty Done.
                    yield ProgressEvent::Done(RunReport {
                        processed: 0, success: 0, failed: 0, crashed: 0,
                        dur_ms: 0,
                    });
                    return;
                }
            };

            for line_res in BufReader::new(f).lines() {
                let line = match line_res {
                    Ok(l) => l,
                    Err(_) => continue,
                };
                let v: serde_json::Value = match serde_json::from_str(&line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let outcomes = match v.get("outcomes").and_then(|o| o.as_array()) {
                    Some(o) => o.clone(),
                    None => continue,
                };
                for outcome in &outcomes {
                    processed += 1;
                    let kind = outcome.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    match kind {
                        "success" => success += 1,
                        "error" => failed += 1,
                        "crash" => crashed += 1,
                        _ => {}
                    }

                    // Emit Tick on time intervals (scaled by speed).
                    if last_tick.elapsed() >= tick_dur {
                        tokio::time::sleep(tick_dur).await;
                        tick_seq += 1;
                        last_tick = Instant::now();
                        yield ProgressEvent::Tick {
                            seq: tick_seq,
                            at_ms: started.elapsed().as_millis() as u64,
                            processed,
                            total,
                            success,
                            failed,
                            crashed,
                            in_flight: 0,
                            queue_depth: 0,
                            rate_1s: 0.0,
                            rate_10s: 0.0,
                            eta_ms: None,
                        };
                    }
                }
            }

            // Final Tick + Done.
            yield ProgressEvent::Tick {
                seq: tick_seq + 1,
                at_ms: started.elapsed().as_millis() as u64,
                processed, total, success, failed, crashed,
                in_flight: 0, queue_depth: 0, rate_1s: 0.0, rate_10s: 0.0,
                eta_ms: None,
            };
            yield ProgressEvent::Done(RunReport {
                processed, success, failed, crashed,
                dur_ms: started.elapsed().as_millis() as u64,
            });
        })
    }
}

#[cfg(test)]
mod replay_tests {
    use super::*;
    use std::io::Write;

    #[tokio::test]
    async fn replay_streams_events_from_outcomes_jsonl() {
        use futures::StreamExt;
        let tmp = tempfile::tempdir().unwrap();
        let outcomes = tmp.path().join("outcomes.jsonl");
        let mut f = std::fs::File::create(&outcomes).unwrap();
        // Two batched lines, 4 outcomes total.
        writeln!(f, r#"{{"first_seq":0,"seqs":[0,1],"outcomes":[{{"type":"success","seq":0,"dur_ms":10}},{{"type":"error","seq":1,"code":"X","dur_ms":11}}]}}"#).unwrap();
        writeln!(f, r#"{{"first_seq":2,"seqs":[2,3],"outcomes":[{{"type":"success","seq":2,"dur_ms":12}},{{"type":"crash","seq":3,"dur_ms":13}}]}}"#).unwrap();
        drop(f);

        // Also write a minimal meta.json so snapshot.total is populated.
        std::fs::write(tmp.path().join("meta.json"), r#"{"input_row_count":4}"#).unwrap();

        let stream = ReplayAttemptStream::from_attempt(tmp.path(), 10.0).unwrap();
        assert_eq!(stream.snapshot().total, Some(4));

        let mut events = Box::new(stream).events();
        let mut got_done = false;
        let mut tick_count = 0;
        while let Some(event) = events.next().await {
            match event {
                ProgressEvent::Done(r) => {
                    got_done = true;
                    assert_eq!(r.processed, 4);
                    assert_eq!(r.success, 2);
                    assert_eq!(r.failed, 1);
                    assert_eq!(r.crashed, 1);
                    break;
                }
                ProgressEvent::Tick { .. } => tick_count += 1,
                _ => {}
            }
        }
        assert!(got_done, "replay should yield Done");
        assert!(tick_count >= 1, "replay should yield ≥ 1 Tick");
    }

    #[tokio::test]
    async fn replay_with_missing_outcomes_yields_empty_done() {
        use futures::StreamExt;
        let tmp = tempfile::tempdir().unwrap();
        let stream = ReplayAttemptStream::from_attempt(tmp.path(), 10.0).unwrap();
        let mut events = Box::new(stream).events();
        // First event should be PhaseChanged, then Done.
        let mut got_done = false;
        while let Some(event) = events.next().await {
            if let ProgressEvent::Done(_) = event {
                got_done = true;
                break;
            }
        }
        assert!(got_done);
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
