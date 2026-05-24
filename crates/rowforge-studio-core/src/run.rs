//! Run lifecycle. Spawns rowforge-core pipeline in-process, attaches
//! ProgressAggregator as the progress sink, registers the session,
//! returns RunHandle. Spec part-3 §3.1 (in-process) + §3.3 (state machine).
//!
//! Integration path: **Path A** — rowforge-core already exposes a per-row
//! `ProgressCallback = Box<dyn Fn(RunProgressEvent) + Send + Sync>` on
//! `RunRequest.on_progress`, plus `RunRequest.cancel: Option<CancellationToken>`.
//! We wire the aggregator's `on_outcome*` calls directly into that callback.
//!
//! The callback receives three variants:
//!   - `Started { total_rows }` → `aggregator.set_total()` + phase → Running
//!   - `RowDone { seq, success }` → `on_outcome_success` / `on_outcome` (error)
//!   - `Completed { success, failed }` → ignored (we derive final counts from
//!     the aggregator's own counters at `emit_done` time)
//!
//! Before spawning the pipeline, `start_run` synchronously:
//!   1. Validates the execution exists in the store.
//!   2. Registers a handler instance (content-addressed, idempotent).
//!   3. Creates an `Attempt` row in the store (state = Running).
//! These three steps happen under the store Mutex lock and complete before the
//! function returns, so subscribers can always call `show()` / `attempt()`
//! and see a valid row.
//!
//! After the pipeline task finishes, it calls `store.finish_attempt()` to mark
//! the attempt terminal, stops the tick loop, and removes the session from the
//! registry.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, watch};
use tokio_util::sync::CancellationToken;

use rowforge_core::execution_store::{FinishAttempt, NewAttempt, NewHandlerInstance, RunType, Simulation, Source};
use rowforge_core::run::{RunProgressEvent, RunRequest};

use crate::aggregator::{ProgressAggregator, ProgressSnapshot};
use crate::error::BusyScope;
use crate::events::{AbortReason, Phase, ProgressEvent, RunReport};
use crate::ids::ExecutionId;
use crate::run_handle::{CancelMode, RunHandle, RunStatus};
use crate::session::{BusyReason, Session};
use crate::{StudioCore, UiError};

// ---------------------------------------------------------------------------
// Workspace rollup (spec §6.6)
// ---------------------------------------------------------------------------

/// Workspace-level rollup emitted by [`StudioCore::active_runs_stream`] at 1 Hz.
/// Used by the header pill and dock badge in the UI.
#[derive(Debug, Clone, serde::Serialize)]
#[non_exhaustive]
pub struct RunRollupTick {
    /// Number of currently active (in-registry) runs.
    pub active_runs: u32,
    /// Sum of processed row counts across all active sessions.
    pub total_processed: u64,
    /// Sum of failed + crashed row counts across all active sessions.
    pub total_failed: u64,
    /// Aggregate rows/s across all active sessions.
    /// Ships as 0.0 in Plan 4 (per-session rate not yet cached in registry).
    pub total_rate: f32,
    /// The handle of the session with the lowest throughput, if any.
    /// Ships as None in Plan 4.
    pub slowest_run: Option<RunHandle>,
}

/// Options for launching a run via `StudioCore::start_run`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RunOpts {
    /// Path to the handler directory (must contain `rowforge.yaml`).
    pub handler_dir: PathBuf,
    /// Number of worker processes. None → rowforge-core default (from manifest).
    pub workers: Option<u32>,
    /// When true, dispatch only rows that failed in the most recent attempt.
    pub retry_failed: bool,
    /// When true, every row is dispatched with `meta.dry_run = true`.
    pub dry_run: bool,
}

impl RunOpts {
    pub fn new(handler_dir: PathBuf) -> Self {
        Self {
            handler_dir,
            workers: None,
            retry_failed: false,
            dry_run: false,
        }
    }
}

/// Returned by `StudioCore::subscribe`.
#[non_exhaustive]
pub struct RunStream {
    pub handle: RunHandle,
    pub rx: broadcast::Receiver<ProgressEvent>,
    pub snapshot: ProgressSnapshot,
}

/// Returned by `StudioCore::start_run`.
///
/// Carries both the `RunHandle` (for subscribing / cancelling) and the
/// `attempt_id` created synchronously by `start_run`. The caller can use
/// `attempt_id` to construct a direct URL to the attempt's Live tab without
/// a follow-up `exec_show` roundtrip.
#[non_exhaustive]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RunStartedHandle {
    pub handle: RunHandle,
    pub attempt_id: String,
}

impl RunStartedHandle {
    pub(crate) fn new(handle: RunHandle, attempt_id: String) -> Self {
        Self { handle, attempt_id }
    }
}

impl StudioCore {
    /// Start an in-process rowforge-core pipeline for `execution_id`.
    ///
    /// Returns a `RunHandle` immediately. The pipeline runs in a background
    /// tokio task; subscribe via `subscribe()` to receive `ProgressEvent`s.
    ///
    /// Returns `UiError::RunBusy` if the execution already has an active run
    /// (per-exec limit = 1, workspace limit = 3 by default).
    ///
    /// Returns `UiError::NotFound` if the execution does not exist.
    pub fn start_run(
        &self,
        execution_id: &ExecutionId,
        opts: RunOpts,
    ) -> Result<RunStartedHandle, UiError> {
        // 1. Concurrency check (fast path, no store I/O).
        let workspace_limit = self.sessions.workspace_limit();
        let per_exec_limit = self.sessions.per_exec_limit();
        self.sessions
            .can_start(execution_id.as_str())
            .map_err(|reason| busy_reason_to_ui_error(reason, workspace_limit, per_exec_limit))?;

        // 2. Resolve execution + create attempt (store writes happen here,
        //    synchronously, so we fail fast if exec doesn't exist or the
        //    handler dir is missing).
        let (attempt_id, attempt_dir, input_snapshot) = {
            let mut store = self
                .store
                .lock()
                .unwrap_or_else(|p| p.into_inner());

            let exec = store
                .get_execution(execution_id.as_str())
                .map_err(|e| UiError::Internal(e.to_string()))?
                .ok_or_else(|| UiError::NotFound(format!("execution {} not found", execution_id)))?;

            // Find the input snapshot inside the execution dir (may be .csv or .jsonl).
            let input_snapshot = ["input.jsonl", "input.ndjson", "input.csv"]
                .iter()
                .map(|n| exec.dir.join(n))
                .find(|p| p.is_file())
                .ok_or_else(|| {
                    UiError::NotFound(format!(
                        "no input snapshot found in {}",
                        exec.dir.display()
                    ))
                })?;

            // Register handler instance (content-addressed, idempotent).
            // This fails fast if the handler dir doesn't contain a manifest,
            // which is the desired behaviour — we surface the error before
            // spawning anything.
            let handler_canon = std::fs::canonicalize(&opts.handler_dir)
                .map_err(|e| UiError::InvalidArg(format!(
                    "canonicalize handler dir {}: {}",
                    opts.handler_dir.display(), e
                )))?;
            let (manifest, manifest_path) =
                rowforge_core::manifest::Manifest::load_from_dir(&handler_canon)
                    .map_err(|e| UiError::InvalidArg(e.to_string()))?;
            let manifest_bytes =
                std::fs::read(&manifest_path).map_err(|e| UiError::Io(e.to_string()))?;
            let manifest_hash = {
                use sha2::{Digest, Sha256};
                format!("sha256:{:x}", Sha256::digest(&manifest_bytes))
            };

            let hi = store
                .register_handler_instance(NewHandlerInstance {
                    handler_id: manifest.name.clone(),
                    manifest_hash,
                    source_snapshot_dir: handler_canon.clone(),
                    binary_hash: None,
                })
                .map_err(|e| UiError::Internal(e.to_string()))?;

            let simulation = if opts.dry_run {
                Simulation::Dry
            } else {
                Simulation::Real
            };
            let attempt = store
                .create_attempt(NewAttempt {
                    execution_id: exec.id.clone(),
                    handler_instance_id: hi.id.clone(),
                    parent_attempt_id: None,
                    run_type: RunType {
                        source: Source::Full,
                        simulation,
                    },
                })
                .map_err(|e| UiError::Internal(e.to_string()))?;

            (attempt.id, attempt.dir, input_snapshot)
        };

        // 3. Allocate handle + aggregator + cancel token + tick stop watch.
        let handle = RunHandle::new();
        let aggregator = Arc::new(ProgressAggregator::new());
        let cancel_token = CancellationToken::new();
        let (tick_stop_tx, tick_stop_rx) = watch::channel(false);

        // 4. Spawn the 4 Hz tick loop.
        let agg_for_tick = aggregator.clone();
        tokio::spawn(async move {
            agg_for_tick.tick_loop(tick_stop_rx).await;
        });

        // 5. Register the session BEFORE spawning the pipeline so that
        //    subscribers can find it the instant start_run returns.
        let session = Arc::new(Session {
            handle: handle.clone(),
            execution_id: execution_id.as_str().to_string(),
            aggregator: aggregator.clone(),
            cancel_token: cancel_token.clone(),
            tick_stop: tick_stop_tx.clone(),
            status: Mutex::new(RunStatus::Starting),
            started_at: Instant::now(),
        });
        self.sessions.register(session.clone());

        // 6. Spawn the actual pipeline task.
        let sessions_arc = self.sessions.clone();
        let store_arc = self.store.clone();
        let handle_for_task = handle.clone();
        let aggregator_for_task = aggregator.clone();
        let cancel_for_task = cancel_token.clone();
        let opts_for_task = opts.clone();
        let attempt_id_for_task = attempt_id.clone();
        let started = Instant::now();

        tokio::spawn(async move {
            aggregator_for_task.set_phase(Phase::Starting);

            let run_result = run_pipeline_in_process(
                &attempt_id_for_task,
                attempt_dir,
                input_snapshot,
                opts_for_task,
                aggregator_for_task.clone(),
                cancel_for_task.clone(),
            )
            .await;

            // Compose final event and mark attempt terminal in store.
            let dur_ms = started.elapsed().as_millis() as u64;

            // Helper: persist FinishAttempt; if it fails, emit a
            // PipelineWarning so the UI sees the divergence between
            // in-memory terminal state and on-disk attempt row. The DB
            // row stays `Running` and orphan recovery (spec §3.7) will
            // clean it up on the next workspace open.
            let try_finish = |finish: FinishAttempt| -> Option<String> {
                let mut store = store_arc.lock().unwrap_or_else(|p| p.into_inner());
                match store.finish_attempt(&attempt_id_for_task, finish) {
                    Ok(_) => None,
                    Err(e) => Some(e.to_string()),
                }
            };
            let emit_persist_warning = |err: String| {
                aggregator_for_task.emit(ProgressEvent::PipelineWarning {
                    code: "PERSIST_FAILED".to_string(),
                    message: format!(
                        "failed to persist terminal attempt state to sqlite: {err} \
                         (orphan recovery will clean up on next workspace open)"
                    ),
                });
            };

            match run_result {
                Ok(report) => {
                    if let Some(err) = try_finish(FinishAttempt {
                        success_count: report.success_count,
                        failed_count: report.failed_count,
                        aborted: report.aborted,
                        aborted_reason: report.abort_reason.clone(),
                    }) {
                        emit_persist_warning(err);
                    }

                    if report.aborted {
                        let reason_msg = report.abort_reason.unwrap_or_default();
                        aggregator_for_task.emit(ProgressEvent::Aborted {
                            reason: AbortReason::Internal { message: reason_msg },
                            at_phase: aggregator_for_task
                                .snapshot()
                                .phase
                                .unwrap_or(Phase::Running),
                            partial_report: build_partial(&aggregator_for_task, dur_ms),
                        });
                        *session.status.lock().unwrap_or_else(|p| p.into_inner()) =
                            RunStatus::Aborted;
                    } else {
                        aggregator_for_task.emit_done(dur_ms);
                        *session.status.lock().unwrap_or_else(|p| p.into_inner()) =
                            RunStatus::Done;
                    }
                }
                Err(RunFailure::Cancelled(report)) => {
                    // Persist the partial counts from rowforge-core so a
                    // cancelled run with completed rows is recorded
                    // accurately, not as 0/0.
                    if let Some(err) = try_finish(FinishAttempt {
                        success_count: report.success_count,
                        failed_count: report.failed_count,
                        aborted: true,
                        aborted_reason: report
                            .abort_reason
                            .clone()
                            .or_else(|| Some("cancelled by operator".into())),
                    }) {
                        emit_persist_warning(err);
                    }
                    aggregator_for_task.emit(ProgressEvent::Aborted {
                        reason: AbortReason::UserCancelled,
                        at_phase: aggregator_for_task
                            .snapshot()
                            .phase
                            .unwrap_or(Phase::Running),
                        partial_report: build_partial(&aggregator_for_task, dur_ms),
                    });
                    *session.status.lock().unwrap_or_else(|p| p.into_inner()) =
                        RunStatus::Aborted;
                }
                Err(RunFailure::Panic(msg)) => {
                    if let Some(err) = try_finish(FinishAttempt {
                        success_count: 0,
                        failed_count: 0,
                        aborted: true,
                        aborted_reason: Some(format!("panic: {msg}")),
                    }) {
                        emit_persist_warning(err);
                    }
                    aggregator_for_task.emit(ProgressEvent::Aborted {
                        reason: AbortReason::Crashed { panic_message: msg },
                        at_phase: aggregator_for_task
                            .snapshot()
                            .phase
                            .unwrap_or(Phase::Running),
                        partial_report: build_partial(&aggregator_for_task, dur_ms),
                    });
                    *session.status.lock().unwrap_or_else(|p| p.into_inner()) =
                        RunStatus::Crashed;
                }
                Err(RunFailure::Other(msg)) => {
                    if let Some(err) = try_finish(FinishAttempt {
                        success_count: 0,
                        failed_count: 0,
                        aborted: true,
                        aborted_reason: Some(msg.clone()),
                    }) {
                        emit_persist_warning(err);
                    }
                    aggregator_for_task.emit(ProgressEvent::Aborted {
                        reason: AbortReason::Internal { message: msg },
                        at_phase: aggregator_for_task
                            .snapshot()
                            .phase
                            .unwrap_or(Phase::Running),
                        partial_report: build_partial(&aggregator_for_task, dur_ms),
                    });
                    *session.status.lock().unwrap_or_else(|p| p.into_inner()) =
                        RunStatus::Aborted;
                }
            }

            // Stop the tick loop and remove from registry.
            let _ = tick_stop_tx.send(true);
            sessions_arc.remove(&handle_for_task);
        });

        Ok(RunStartedHandle::new(handle, attempt_id))
    }

    /// Subscribe to live `ProgressEvent`s for a running pipeline.
    ///
    /// Returns a `RunStream` containing the current `ProgressSnapshot` and a
    /// `broadcast::Receiver<ProgressEvent>`. The receiver may lag if the
    /// consumer is slow (capacity = 256); lagged receivers return
    /// `RecvError::Lagged`.
    ///
    /// Returns `UiError::UnknownHandle` if the handle is not in the registry
    /// (i.e. the run has already completed or the handle is bogus).
    pub fn subscribe(&self, h: &RunHandle) -> Result<RunStream, UiError> {
        let session = self
            .sessions
            .get(h)
            .ok_or_else(|| UiError::UnknownHandle(h.to_string()))?;
        Ok(RunStream {
            handle: h.clone(),
            rx: session.aggregator.subscribe(),
            snapshot: session.aggregator.snapshot(),
        })
    }

    /// Request cancellation of an active run.
    ///
    /// `CancelMode::Soft` — fires the cancellation token; the pool stops
    /// dispatching new rows and waits for in-flight workers to complete.
    ///
    /// `CancelMode::Hard` — same as Soft for now. A future plan can add
    /// SIGKILL plumbing once rowforge-core exposes a kill handle.
    ///
    /// Returns `UiError::UnknownHandle` if the handle is not in the registry.
    pub fn cancel(&self, h: &RunHandle, mode: CancelMode) -> Result<(), UiError> {
        let session = self
            .sessions
            .get(h)
            .ok_or_else(|| UiError::UnknownHandle(h.to_string()))?;
        {
            let mut s = session.status.lock().unwrap_or_else(|p| p.into_inner());
            *s = RunStatus::Cancelling;
        }
        match mode {
            CancelMode::Soft => session.cancel_token.cancel(),
            CancelMode::Hard => {
                // Hard cancel: fire the token immediately. rowforge-core does
                // not currently expose a SIGKILL handle, so Hard == Soft for
                // now. When pool_streaming gains process-kill plumbing, wire
                // it here.
                session.cancel_token.cancel();
                tracing::warn!(
                    handle = %h,
                    "Hard cancel requested; rowforge-core has no kill handle yet — \
                     falling back to soft cancel (token fire)"
                );
            }
        }
        Ok(())
    }

    /// Return the current `RunStatus` of an active run.
    ///
    /// Returns `UiError::UnknownHandle` if the handle is not in the registry.
    pub fn status(&self, h: &RunHandle) -> Result<RunStatus, UiError> {
        let session = self
            .sessions
            .get(h)
            .ok_or_else(|| UiError::UnknownHandle(h.to_string()))?;
        let status = *session.status.lock().unwrap_or_else(|p| p.into_inner());
        Ok(status)
    }

    /// Return handles for all currently-active runs in this workspace.
    pub fn active_runs(&self) -> Vec<RunHandle> {
        self.sessions.handles()
    }

    /// Returns a stream that yields a [`RunRollupTick`] every 1 second.
    ///
    /// The stream lives as long as the caller holds it; dropping it stops the
    /// 1 Hz interval. Spec §6.6.
    pub fn active_runs_stream(&self) -> impl futures::Stream<Item = RunRollupTick> + Send + 'static {
        let sessions = self.sessions.clone();
        async_stream::stream! {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                let snapshots = sessions.snapshots();
                let active = snapshots.len() as u32;
                let total_processed: u64 = snapshots.iter().map(|(_, s)| s.processed).sum();
                let total_failed: u64 = snapshots.iter().map(|(_, s)| s.failed + s.crashed).sum();
                // total_rate: aggregate from per-session rate. ProgressSnapshot
                // doesn't carry rate — that's only in Tick events. Plan 4 ships
                // 0.0 for now; spec §6.6 is satisfied by counter aggregation.
                // Future: SessionRegistry could cache last-Tick rate per session.
                let total_rate = 0.0_f32;
                // slowest_run: heuristic — the session with the lowest rate.
                // Without per-session rate, pick the one with the longest
                // started_at duration. Plan 4 ships None for simplicity.
                let slowest_run = None;

                yield RunRollupTick {
                    active_runs: active,
                    total_processed,
                    total_failed,
                    total_rate,
                    slowest_run,
                };
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn busy_reason_to_ui_error(
    reason: BusyReason,
    _workspace_limit: u32,
    per_exec_limit: u32,
) -> UiError {
    match reason {
        BusyReason::PerExec { execution_id } => UiError::RunBusy {
            execution_id,
            limit: per_exec_limit,
            scope: BusyScope::PerExec,
        },
        BusyReason::Workspace { limit } => UiError::RunBusy {
            execution_id: String::new(),
            limit,
            scope: BusyScope::PerWorkspace,
        },
    }
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

enum RunFailure {
    /// Carries the partial `RunReport` from rowforge-core so persisted
    /// attempt stats reflect the work that completed before cancellation.
    Cancelled(rowforge_core::run::RunReport),
    Panic(String),
    Other(String),
}

fn build_partial(agg: &ProgressAggregator, dur_ms: u64) -> RunReport {
    let s = agg.snapshot();
    RunReport {
        processed: s.processed,
        success: s.success,
        failed: s.failed,
        crashed: s.crashed,
        dur_ms,
    }
}

// ---------------------------------------------------------------------------
// Core pipeline invocation (Path A)
// ---------------------------------------------------------------------------

/// Spawn rowforge-core's pipeline in-process.
///
/// Translates `RunProgressEvent` callbacks into `ProgressAggregator` calls:
/// - `Started` → `set_total` + `set_phase(Running)`
/// - `RowDone { success: true }` → `on_outcome_success`
/// - `RowDone { success: false }` → `on_outcome` with kind=Error, no code
/// - `Completed` → ignored (aggregator already has the counts)
///
/// Cancellation is wired via `RunRequest.cancel`. When the token fires,
/// `execute` returns `Ok(RunReport { aborted: true, .. })`, which we map
/// to `RunFailure::Cancelled`.
async fn run_pipeline_in_process(
    attempt_id: &str,
    output_dir: PathBuf,
    input_csv: PathBuf,
    opts: RunOpts,
    aggregator: Arc<ProgressAggregator>,
    cancel: CancellationToken,
) -> Result<rowforge_core::run::RunReport, RunFailure> {
    let handler_canon = match std::fs::canonicalize(&opts.handler_dir) {
        Ok(p) => p,
        Err(e) => return Err(RunFailure::Other(format!(
            "canonicalize handler dir: {e}"
        ))),
    };

    let workers = opts.workers.unwrap_or(1);
    let dry_run = opts.dry_run;

    // Build the per-row progress callback. The closure captures `aggregator`
    // and updates counters for every RowDone event. `Started` wires the total
    // so the Tick ETA calculation works.
    let agg_cb = aggregator.clone();
    let on_progress: rowforge_core::run::ProgressCallback = Box::new(move |ev| {
        match ev {
            RunProgressEvent::Started { total_rows } => {
                agg_cb.set_total(total_rows);
                agg_cb.set_phase(Phase::Running);
            }
            RunProgressEvent::RowDone { seq, success } => {
                if success {
                    agg_cb.on_outcome_success(seq, 0);
                } else {
                    agg_cb.on_outcome(
                        seq,
                        crate::failed::RowOutcomeKind::Error,
                        None,  // error code not available in RowDone
                        None,
                        0,
                    );
                }
            }
            RunProgressEvent::Completed { .. } => {
                // Aggregator already has accurate counts from per-row calls.
            }
        }
    });

    let req = RunRequest {
        run_id: attempt_id.to_string(),
        parent_run_id: None,
        handler_dir: handler_canon,
        input_csv,
        output_dir,
        workers,
        dry_run,
        dry_run_sample: 0,
        row_limit: None,
        skip_seqs: std::collections::HashSet::new(),
        field_map: rowforge_core::reader::FieldMap::new(),
        config_overrides: BTreeMap::new(),
        shutdown_grace: Duration::from_secs(5),
        on_progress: Some(on_progress),
        cancel: Some(cancel.clone()),
        input_format: None,
        fsync_outcomes: false,
    };

    aggregator.set_phase(Phase::Snapshotting);

    // Spawn in a child task so panics inside rowforge-core don't propagate
    // as unwinding through our async boundary. The child task is joined and
    // its JoinError inspected to detect panics.
    let join = tokio::spawn(async move {
        rowforge_core::run::execute(req).await
    });

    match join.await {
        Ok(Ok(report)) => {
            if report.aborted && cancel.is_cancelled() {
                Err(RunFailure::Cancelled(report))
            } else {
                Ok(report)
            }
        }
        Ok(Err(e)) => Err(RunFailure::Other(e.to_string())),
        Err(join_err) => {
            if join_err.is_panic() {
                let msg = join_err
                    .into_panic()
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "unknown panic".to_string());
                Err(RunFailure::Panic(msg))
            } else {
                // Task was cancelled by the tokio runtime (shutdown) — we
                // have no report to forward. Use a zero-count synthetic
                // so the attempt is still marked aborted with the right
                // reason. Counts will be `0/0`, but this only fires on
                // process shutdown so orphan recovery handles it next boot.
                Err(RunFailure::Cancelled(rowforge_core::run::RunReport {
                    success_count: 0,
                    failed_count: 0,
                    by_error_code: BTreeMap::new(),
                    run_dir: PathBuf::new(),
                    aborted: true,
                    abort_reason: Some("runtime shutdown".into()),
                }))
            }
        }
    }
}
