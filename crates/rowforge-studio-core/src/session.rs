//! In-memory registry of active run sessions. One entry per RunHandle.
//! Enforces concurrency limits (spec §3.4): default 1 per exec, 3 per workspace.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::aggregator::ProgressAggregator;
use crate::run_handle::{RunHandle, RunStatus};

#[non_exhaustive]
pub struct Session {
    pub handle: RunHandle,
    pub execution_id: String,
    /// Attempt id this session is producing rows into. Used by
    /// `SessionRegistry::lookup_by_attempt` so a user landing on
    /// AttemptDetail without `?run=` can be offered to re-attach to
    /// the live stream.
    pub attempt_id: String,
    pub aggregator: Arc<ProgressAggregator>,
    pub cancel_token: CancellationToken,
    /// Drop sender to stop the per-session tick loop on shutdown.
    pub tick_stop: watch::Sender<bool>,
    pub status: Mutex<RunStatus>,
    pub started_at: Instant,
}

pub struct SessionRegistry {
    inner: Mutex<HashMap<RunHandle, Arc<Session>>>,
    workspace_limit: u32,
    per_exec_limit: u32,
}

impl Default for SessionRegistry {
    fn default() -> Self {
        Self::new(3, 1)
    }
}

impl SessionRegistry {
    pub fn new(workspace_limit: u32, per_exec_limit: u32) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            workspace_limit,
            per_exec_limit,
        }
    }

    /// Check if a new run for `execution_id` can start. Returns Err with the
    /// specific limit that's blocked.
    pub fn can_start(&self, execution_id: &str) -> Result<(), BusyReason> {
        let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if inner.len() as u32 >= self.workspace_limit {
            return Err(BusyReason::Workspace {
                limit: self.workspace_limit,
            });
        }
        let per_exec_count = inner
            .values()
            .filter(|s| s.execution_id == execution_id)
            .count() as u32;
        if per_exec_count >= self.per_exec_limit {
            return Err(BusyReason::PerExec {
                execution_id: execution_id.to_string(),
            });
        }
        Ok(())
    }

    pub fn register(&self, session: Arc<Session>) {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.insert(session.handle.clone(), session);
    }

    pub fn get(&self, h: &RunHandle) -> Option<Arc<Session>> {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(h)
            .cloned()
    }

    pub fn remove(&self, h: &RunHandle) -> Option<Arc<Session>> {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(h)
    }

    /// Find the live RunHandle associated with a given attempt, if any.
    /// Used by AttemptDetail to offer a "Watch live" affordance when the
    /// user lands on the page without `?run=` in the URL.
    pub fn lookup_by_attempt(&self, attempt_id: &str) -> Option<RunHandle> {
        let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner
            .values()
            .find(|s| s.attempt_id == attempt_id)
            .map(|s| s.handle.clone())
    }

    pub fn handles(&self) -> Vec<RunHandle> {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .keys()
            .cloned()
            .collect()
    }

    pub fn snapshots(&self) -> Vec<(RunHandle, crate::aggregator::ProgressSnapshot)> {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .map(|s| (s.handle.clone(), s.aggregator.snapshot()))
            .collect()
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).len()
    }

    pub fn workspace_limit(&self) -> u32 {
        self.workspace_limit
    }

    pub fn per_exec_limit(&self) -> u32 {
        self.per_exec_limit
    }

    /// Build a [`crate::run::RunRollupTick`] from the current registry state.
    ///
    /// Used by the Tauri event bridge to emit `runs:active` without needing a
    /// `StudioCore` reference (only `Arc<SessionRegistry>` is available there).
    pub fn rollup_tick(&self) -> crate::run::RunRollupTick {
        let snaps = self.snapshots();
        let active = snaps.len() as u32;
        let total_processed: u64 = snaps.iter().map(|(_, s)| s.processed).sum();
        let total_failed: u64 = snaps.iter().map(|(_, s)| s.failed + s.crashed).sum();
        let total_rate: f32 = snaps.iter().map(|(_, s)| s.rate_10s).sum();
        // slowest_run: pick the session with the lowest positive rate_10s.
        // Sessions with rate_10s == 0 are still warming up the sliding
        // window (< ~10s since start) — exclude them so they're not
        // false-positive flagged as slow.
        let slowest_run = snaps
            .iter()
            .filter(|(_, s)| s.rate_10s > 0.0)
            .min_by(|(_, a), (_, b)| {
                a.rate_10s
                    .partial_cmp(&b.rate_10s)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(h, _)| h.clone());
        crate::run::RunRollupTick {
            active_runs: active,
            total_processed,
            total_failed,
            total_rate,
            slowest_run,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BusyReason {
    PerExec { execution_id: String },
    Workspace { limit: u32 },
}

impl std::fmt::Display for BusyReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BusyReason::PerExec { execution_id } => write!(
                f,
                "execution {} already has an active run",
                execution_id
            ),
            BusyReason::Workspace { limit } => write!(
                f,
                "workspace concurrent-run limit reached ({})",
                limit
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_session(exec: &str) -> Arc<Session> {
        let (tick_stop, _) = watch::channel(false);
        Arc::new(Session {
            handle: RunHandle::new(),
            execution_id: exec.into(),
            attempt_id: format!("a_{}", exec),
            aggregator: Arc::new(ProgressAggregator::new()),
            cancel_token: CancellationToken::new(),
            tick_stop,
            status: Mutex::new(RunStatus::Running),
            started_at: Instant::now(),
        })
    }

    fn fake_session_with_rate(exec: &str, rate_10s: f32) -> Arc<Session> {
        let (tick_stop, _) = watch::channel(false);
        let agg = Arc::new(ProgressAggregator::new());
        agg.set_rate_for_test(rate_10s);
        Arc::new(Session {
            handle: RunHandle::new(),
            execution_id: exec.into(),
            attempt_id: format!("a_{}", exec),
            aggregator: agg,
            cancel_token: CancellationToken::new(),
            tick_stop,
            status: Mutex::new(RunStatus::Running),
            started_at: Instant::now(),
        })
    }

    #[test]
    fn per_exec_limit_enforced() {
        let r = SessionRegistry::default();
        r.register(fake_session("e1"));
        match r.can_start("e1") {
            Err(BusyReason::PerExec { execution_id }) => assert_eq!(execution_id, "e1"),
            other => panic!("expected PerExec, got {other:?}"),
        }
    }

    #[test]
    fn workspace_limit_enforced() {
        let r = SessionRegistry::default();
        r.register(fake_session("e1"));
        r.register(fake_session("e2"));
        r.register(fake_session("e3"));
        match r.can_start("e4") {
            Err(BusyReason::Workspace { limit }) => assert_eq!(limit, 3),
            other => panic!("expected Workspace, got {other:?}"),
        }
    }

    #[test]
    fn can_start_succeeds_with_room() {
        let r = SessionRegistry::default();
        r.register(fake_session("e1"));
        assert!(r.can_start("e2").is_ok());
    }

    #[test]
    fn register_remove_get() {
        let r = SessionRegistry::default();
        let s = fake_session("e1");
        let h = s.handle.clone();
        r.register(s);
        assert!(r.get(&h).is_some());
        let removed = r.remove(&h);
        assert!(removed.is_some());
        assert!(r.get(&h).is_none());
    }

    #[test]
    fn handles_and_snapshots() {
        let r = SessionRegistry::default();
        r.register(fake_session("e1"));
        r.register(fake_session("e2"));
        assert_eq!(r.handles().len(), 2);
        assert_eq!(r.snapshots().len(), 2);
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn rollup_tick_sums_rate_across_sessions() {
        let reg = SessionRegistry::new(3, 1);
        let s1 = fake_session_with_rate("e1", 100.0);
        let s2 = fake_session_with_rate("e2", 50.0);
        reg.register(s1.clone());
        reg.register(s2.clone());

        let tick = reg.rollup_tick();
        assert_eq!(tick.active_runs, 2);
        // Allow ±10 slack for set_rate_for_test integer-bucket rounding.
        assert!(
            (tick.total_rate - 150.0).abs() < 10.0,
            "total_rate ≈ 150, got {}",
            tick.total_rate,
        );
    }

    #[test]
    fn rollup_tick_slowest_run_is_min_positive_rate() {
        let reg = SessionRegistry::new(3, 1);
        let fast = fake_session_with_rate("e_fast", 100.0);
        let slow = fake_session_with_rate("e_slow", 20.0);
        reg.register(fast.clone());
        reg.register(slow.clone());

        let tick = reg.rollup_tick();
        assert_eq!(tick.slowest_run, Some(slow.handle.clone()));
    }

    #[test]
    fn rollup_tick_excludes_zero_rate_from_slowest() {
        let reg = SessionRegistry::new(3, 1);
        let working = fake_session_with_rate("e_work", 50.0);
        let warming = fake_session_with_rate("e_warm", 0.0); // still warming up
        reg.register(working.clone());
        reg.register(warming.clone());

        let tick = reg.rollup_tick();
        // Warming-up session is NOT picked as slowest; the working one is.
        assert_eq!(tick.slowest_run, Some(working.handle.clone()));
    }

    #[test]
    fn rollup_tick_slowest_run_is_none_when_all_warming() {
        let reg = SessionRegistry::new(3, 1);
        let w1 = fake_session_with_rate("e1", 0.0);
        let w2 = fake_session_with_rate("e2", 0.0);
        reg.register(w1.clone());
        reg.register(w2.clone());

        let tick = reg.rollup_tick();
        assert_eq!(tick.slowest_run, None);
    }
}
