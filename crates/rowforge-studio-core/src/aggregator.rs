//! Coalesces per-row outcome events into 4 Hz Tick + 20/s OutcomeSample
//! before broadcasting. Spec part-6 §6.2.
//!
//! Pattern:
//! - `on_outcome*` is called by rowforge-core's progress sink for every row.
//!   It updates internal counters + a small ring buffer for rate, and may
//!   emit an OutcomeSample subject to token-bucket budget.
//! - `tick_loop` is a tokio task that wakes every 250 ms, composes a Tick
//!   from the current snapshot, and broadcasts it.
//! - Lifecycle events (`emit`) bypass coalescing and broadcast immediately.

use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, watch};
use tokio::time;

use crate::events::{Phase, ProgressEvent, RunReport};
use crate::failed::RowOutcomeKind;

const TICK_INTERVAL: Duration = Duration::from_millis(250);
const OUTCOME_TOKENS_PER_SEC: f32 = 20.0;
const ERROR_BUDGET_RATIO: f32 = 0.9;
const BROADCAST_CAPACITY: usize = 256;

#[derive(Debug, Clone, Default)]
pub struct ProgressSnapshot {
    pub processed: u64,
    pub total: Option<u64>,
    pub success: u64,
    pub failed: u64,
    pub crashed: u64,
    pub in_flight: u32,
    pub queue_depth: u32,
    pub phase: Option<Phase>,
}

pub struct ProgressAggregator {
    inner: Mutex<Inner>,
    tx: broadcast::Sender<ProgressEvent>,
    started: Instant,
}

struct Inner {
    snapshot: ProgressSnapshot,
    tick_seq: u64,
    // Per-second sampled processed counts for rate calculation.
    // 4 samples = 1 second window; 40 = 10 seconds.
    rate_1s_buf: Vec<u64>,
    rate_10s_buf: Vec<u64>,
    last_processed_for_rate: u64,
    // Token bucket
    error_tokens: f32,
    last_token_refill: Instant,
}

impl ProgressAggregator {
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel(BROADCAST_CAPACITY);
        let now = Instant::now();
        Self {
            inner: Mutex::new(Inner {
                snapshot: ProgressSnapshot::default(),
                tick_seq: 0,
                rate_1s_buf: vec![0; 4],
                rate_10s_buf: vec![0; 40],
                last_processed_for_rate: 0,
                error_tokens: OUTCOME_TOKENS_PER_SEC * ERROR_BUDGET_RATIO,
                last_token_refill: now,
            }),
            tx,
            started: now,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ProgressEvent> {
        self.tx.subscribe()
    }

    pub fn snapshot(&self) -> ProgressSnapshot {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).snapshot.clone()
    }

    pub fn set_total(&self, total: u64) {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.snapshot.total = Some(total);
    }

    pub fn set_phase(&self, phase: Phase) {
        {
            let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            inner.snapshot.phase = Some(phase);
        }
        let at_ms = self.started.elapsed().as_millis() as u64;
        let _ = self.tx.send(ProgressEvent::PhaseChanged { phase, at_ms });
    }

    pub fn set_in_flight(&self, in_flight: u32, queue_depth: u32) {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.snapshot.in_flight = in_flight;
        inner.snapshot.queue_depth = queue_depth;
    }

    /// Called per-row outcome by rowforge-core's ProgressSink.
    /// Increments counters and may emit an OutcomeSample if token budget allows.
    pub fn on_outcome(
        &self,
        row_index: u64,
        kind: RowOutcomeKind,
        code: Option<String>,
        message: Option<String>,
        dur_ms: u32,
    ) {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.snapshot.processed += 1;
        match kind {
            RowOutcomeKind::Error => inner.snapshot.failed += 1,
            RowOutcomeKind::Crash => inner.snapshot.crashed += 1,
            RowOutcomeKind::TooLarge => inner.snapshot.failed += 1,
        }
        Self::refill_tokens(&mut inner);
        let emit = inner.error_tokens >= 1.0;
        if emit {
            inner.error_tokens -= 1.0;
        }
        drop(inner);
        if emit {
            let _ = self.tx.send(ProgressEvent::OutcomeSample {
                row_index,
                kind,
                code,
                message,
                dur_ms,
            });
        }
    }

    /// Called per-row success outcome by rowforge-core's ProgressSink.
    ///
    /// Updates counters so Tick reflects the correct `success` count.
    /// Does NOT emit an OutcomeSample — `RowOutcomeKind` has no `Success`
    /// variant; the UI sees success progress via Tick's `success` field instead.
    pub fn on_outcome_success(&self, _row_index: u64, _dur_ms: u32) {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.snapshot.processed += 1;
        inner.snapshot.success += 1;
        // No OutcomeSample emit for success — RowOutcomeKind has no Success
        // variant. Tick events show success count instead.
    }

    fn refill_tokens(inner: &mut Inner) {
        let now = Instant::now();
        let elapsed_s = now.duration_since(inner.last_token_refill).as_secs_f32();
        if elapsed_s > 0.0 {
            let refill = elapsed_s * OUTCOME_TOKENS_PER_SEC;
            let err_cap = OUTCOME_TOKENS_PER_SEC * ERROR_BUDGET_RATIO;
            inner.error_tokens = (inner.error_tokens + refill * ERROR_BUDGET_RATIO).min(err_cap);
            inner.last_token_refill = now;
        }
    }

    /// Drive the 4 Hz Tick timer. Spawn as a tokio task; stop via `stop_rx`.
    pub async fn tick_loop(
        self: std::sync::Arc<Self>,
        mut stop_rx: watch::Receiver<bool>,
    ) {
        let mut interval = time::interval(TICK_INTERVAL);
        interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let event = self.compose_tick();
                    let _ = self.tx.send(event);
                }
                changed = stop_rx.changed() => {
                    if changed.is_err() || *stop_rx.borrow() {
                        break;
                    }
                }
            }
        }
    }

    fn compose_tick(&self) -> ProgressEvent {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.tick_seq += 1;
        let seq = inner.tick_seq;
        let processed = inner.snapshot.processed;
        // Slide the rate windows
        let delta = processed.saturating_sub(inner.last_processed_for_rate);
        inner.last_processed_for_rate = processed;
        inner.rate_1s_buf.rotate_left(1);
        if let Some(last) = inner.rate_1s_buf.last_mut() {
            *last = delta;
        }
        inner.rate_10s_buf.rotate_left(1);
        if let Some(last) = inner.rate_10s_buf.last_mut() {
            *last = delta;
        }
        let rate_1s = inner.rate_1s_buf.iter().sum::<u64>() as f32; // 4 samples = 1s
        let rate_10s = (inner.rate_10s_buf.iter().sum::<u64>() as f32) / 10.0; // 40 samples = 10s
        let snap = inner.snapshot.clone();
        drop(inner);

        let eta_ms = match snap.total {
            Some(total) if rate_10s > 0.0 && total >= snap.processed => {
                let remaining = (total - snap.processed) as f32;
                Some(((remaining / rate_10s) * 1000.0) as u64)
            }
            _ => None,
        };

        ProgressEvent::Tick {
            seq,
            at_ms: self.started.elapsed().as_millis() as u64,
            processed: snap.processed,
            total: snap.total,
            success: snap.success,
            failed: snap.failed,
            crashed: snap.crashed,
            in_flight: snap.in_flight,
            queue_depth: snap.queue_depth,
            rate_1s,
            rate_10s,
            eta_ms,
        }
    }

    pub fn emit(&self, event: ProgressEvent) {
        let _ = self.tx.send(event);
    }

    pub fn emit_done(&self, dur_ms: u64) {
        let snap = self.snapshot();
        let _ = self.tx.send(ProgressEvent::Done(RunReport {
            processed: snap.processed,
            success: snap.success,
            failed: snap.failed,
            crashed: snap.crashed,
            dur_ms,
        }));
    }
}

impl Default for ProgressAggregator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test]
    async fn tick_loop_emits_at_4hz() {
        let agg = Arc::new(ProgressAggregator::new());
        let mut rx = agg.subscribe();
        let (stop_tx, stop_rx) = watch::channel(false);
        let h = tokio::spawn({
            let agg = agg.clone();
            async move { agg.tick_loop(stop_rx).await }
        });
        tokio::time::sleep(Duration::from_millis(700)).await;
        let _ = stop_tx.send(true);
        h.await.unwrap();
        let mut count = 0;
        while let Ok(_) = rx.try_recv() {
            count += 1;
        }
        assert!(count >= 2 && count <= 5, "got {count} ticks in 700 ms");
    }

    #[test]
    fn outcome_counters_increment() {
        let agg = ProgressAggregator::new();
        agg.on_outcome_success(0, 10);
        agg.on_outcome_success(1, 11);
        agg.on_outcome(2, RowOutcomeKind::Error, Some("X".into()), None, 5);
        let s = agg.snapshot();
        assert_eq!(s.processed, 3);
        assert_eq!(s.success, 2);
        assert_eq!(s.failed, 1);
    }

    #[test]
    fn outcome_sample_token_bucket_drops_excess() {
        // Burst 100 errors quickly; should NOT all emit. Token bucket caps
        // at ~18 (20 tokens/s × 0.9 error ratio); some may emit beyond on
        // refill but we should see strictly fewer emits than calls.
        let agg = ProgressAggregator::new();
        let mut rx = agg.subscribe();
        for i in 0..100 {
            agg.on_outcome(i, RowOutcomeKind::Error, Some("X".into()), None, 1);
        }
        let mut emits = 0;
        while let Ok(_) = rx.try_recv() {
            emits += 1;
        }
        assert!(emits < 100, "all 100 emitted? token bucket broken (emits={emits})");
        assert!(emits >= 1, "no emits at all? token bucket broken (emits={emits})");
        // And counters reflect ALL 100, not just sampled:
        assert_eq!(agg.snapshot().failed, 100);
    }

    #[test]
    fn set_phase_emits_event_and_updates_snapshot() {
        let agg = ProgressAggregator::new();
        let mut rx = agg.subscribe();
        agg.set_phase(Phase::Running);
        assert!(matches!(
            rx.try_recv().unwrap(),
            ProgressEvent::PhaseChanged { phase: Phase::Running, .. }
        ));
        assert_eq!(agg.snapshot().phase, Some(Phase::Running));
    }
}
