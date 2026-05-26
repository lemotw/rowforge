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
    /// Cap how many rows are dispatched. `None` = run all rows; `Some(n)` =
    /// stop after `n` rows. Useful for sampling against a slow handler /
    /// expensive external API.
    pub row_limit: Option<u64>,
    /// When true, compute `RowResolution` for this execution and pass every
    /// already-attempted seq (success / failed / crashed — anything that's
    /// not `NeverAttempted`) into the pipeline as `skip_seqs`. Combine with
    /// `row_limit` to sample successive batches of fresh rows across
    /// repeated runs.
    pub skip_attempted: bool,
    /// When `Some`, only the rows whose `seq` values appear in this list are
    /// dispatched. Used by Plan 11's Re-run failed flow. Takes priority over
    /// `skip_seqs` inside rowforge-core: if a seq is in `only_row_ids` it is
    /// dispatched regardless of `skip_seqs`.
    pub only_row_ids: Option<Vec<u64>>,
}

impl RunOpts {
    pub fn new(handler_dir: PathBuf) -> Self {
        Self {
            handler_dir,
            workers: None,
            retry_failed: false,
            dry_run: false,
            row_limit: None,
            skip_attempted: false,
            only_row_ids: None,
        }
    }

    pub fn with_row_limit(mut self, n: u64) -> Self {
        self.row_limit = Some(n);
        self
    }

    pub fn with_workers(mut self, n: u32) -> Self {
        self.workers = Some(n);
        self
    }

    pub fn with_dry_run(mut self, b: bool) -> Self {
        self.dry_run = b;
        self
    }

    pub fn with_skip_attempted(mut self, b: bool) -> Self {
        self.skip_attempted = b;
        self
    }

    /// Restrict dispatch to specific row seq values. Pass `None` to clear.
    ///
    /// Used by Plan 11's Re-run failed flow: callers set this to the
    /// `Vec<u64>` returned by `StudioCore::attempt_failed_row_ids`.
    pub fn with_only_row_ids(mut self, ids: Option<Vec<u64>>) -> Self {
        self.only_row_ids = ids;
        self
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
        // 1a. Cross-process active-attempt gate (sqlite is source of truth).
        //
        // A CLI process opens a fresh StudioCore with an empty in-process
        // SessionRegistry; without this check it could start a new attempt
        // while Studio already has a running attempt for the same exec —
        // corrupting state. Plan 10 added the same gate to execution_delete;
        // mirrored here for start_run.
        {
            let store = self.store.lock().unwrap_or_else(|p| p.into_inner());
            let sqlite_active = store
                .has_active_attempt(execution_id.as_str())
                .map_err(|e| UiError::Io(format!("active-attempt check: {}", e)))?;
            if sqlite_active {
                return Err(UiError::ExecutionInUse {
                    exec_id: execution_id.as_str().to_string(),
                });
            }
        }

        // 1b. In-process concurrency check (catches the brief window between
        //     attempt-start and the sqlite state being committed; fast path,
        //     no additional store I/O beyond step 1a).
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

            // Plan 6 T4: persist the handler dir so the RunButton can default
            // to it on next visit (replaces the Plan 5 localStorage hack — T7
            // drops the LS_HANDLER_DIR plumbing). Non-fatal: a sqlite write
            // failure here shouldn't block the actual run from starting; we
            // log via tracing and continue.
            if let Err(e) = store.set_last_handler_dir(execution_id.as_str(), &handler_canon) {
                tracing::warn!(
                    execution_id = %execution_id,
                    error = %e,
                    "failed to persist last_handler_dir; continuing",
                );
            }

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
        //    Allocate the handler-log broadcast channel here (cap 4096);
        //    the sender is stashed on the session for `handler_log_subscribe`,
        //    and a clone is forwarded into the pipeline as `on_handler_log`.
        let (handler_log_tx, _) =
            broadcast::channel::<rowforge_core::handler_log::HandlerLogLine>(
                crate::session::HANDLER_LOG_CHANNEL_CAP,
            );
        let handler_log_tx_for_pipeline = handler_log_tx.clone();
        let session = Arc::new(Session {
            handle: handle.clone(),
            execution_id: execution_id.as_str().to_string(),
            attempt_id: attempt_id.clone(),
            aggregator: aggregator.clone(),
            cancel_token: cancel_token.clone(),
            tick_stop: tick_stop_tx.clone(),
            status: Mutex::new(RunStatus::Starting),
            started_at: Instant::now(),
            handler_log_tx,
        });
        self.sessions.register(session.clone());

        // 6. Compute skip_seqs (already-attempted rows) if requested. Done
        //    here, before the spawn, so we can use the locked store. The
        //    HashSet is moved into the task.
        let skip_seqs: std::collections::HashSet<u64> = if opts.skip_attempted {
            let store = self.store.lock().unwrap_or_else(|p| p.into_inner());
            match rowforge_core::row_resolution::compute_resolution(
                &store,
                execution_id.as_str(),
            ) {
                Ok(res) => res.attempted_seqs(),
                Err(_) => std::collections::HashSet::new(),
            }
        } else {
            std::collections::HashSet::new()
        };

        // 7. Spawn the actual pipeline task.
        let sessions_arc = self.sessions.clone();
        let store_arc = self.store.clone();
        let handle_for_task = handle.clone();
        let aggregator_for_task = aggregator.clone();
        let cancel_for_task = cancel_token.clone();
        let opts_for_task = opts.clone();
        let attempt_id_for_task = attempt_id.clone();
        let started = Instant::now();
        // Plan 9 T5: snapshot Settings.handler_log_capture_raw_stdout at
        // attempt-start. Changes to the setting after this point do NOT affect
        // the current attempt (intentional — snapshotted into RunRequest).
        let capture_raw_stdout = self.capture_raw_stdout();

        // Plan 11: snapshot only_row_ids from RunOpts so the filter is
        // captured into the spawned task rather than borrowed.
        let only_row_ids = opts.only_row_ids.clone();

        tokio::spawn(async move {
            aggregator_for_task.set_phase(Phase::Starting);

            let run_result = run_pipeline_in_process(
                &attempt_id_for_task,
                attempt_dir,
                input_snapshot,
                opts_for_task,
                aggregator_for_task.clone(),
                cancel_for_task.clone(),
                skip_seqs,
                handler_log_tx_for_pipeline,
                capture_raw_stdout,
                only_row_ids,
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

    /// Look up the live run handle for a given attempt, if one exists in
    /// the registry. Used by AttemptDetail to offer "Watch live" when the
    /// user lands on the page without `?run=` in the URL (e.g. by
    /// navigating from the Executions list rather than via the Run
    /// button's auto-navigate).
    ///
    /// Returns `None` if the attempt is not currently running.
    pub fn active_handle_for_attempt(&self, attempt_id: &str) -> Option<RunHandle> {
        self.sessions.lookup_by_attempt(attempt_id)
    }

    /// Return the current [`ProgressSnapshot`] for a run.
    ///
    /// Used by the UI to bootstrap state when subscribing to a run that's
    /// already in flight — Tauri events are fire-and-forget, so a fresh
    /// `listen` only sees events from now on. Calling `snapshot` after
    /// `listen` is set up fills in the counters that fired before the
    /// listener attached.
    ///
    /// Returns `UiError::UnknownHandle` if the handle is not in the registry.
    pub fn snapshot(&self, h: &RunHandle) -> Result<crate::aggregator::ProgressSnapshot, UiError> {
        let session = self
            .sessions
            .get(h)
            .ok_or_else(|| UiError::UnknownHandle(h.to_string()))?;
        Ok(session.aggregator.snapshot())
    }

    /// Returns a stream that yields a [`RunRollupTick`] every 1 second.
    ///
    /// The stream lives as long as the caller holds it; dropping it stops the
    /// 1 Hz interval. Spec §6.6.
    ///
    /// Delegates to [`SessionRegistry::rollup_tick`] for the actual
    /// rollup computation — total_rate / slowest_run derivation lives
    /// there since Plan 6 T6 (was hardcoded `0.0` / `None` here in
    /// Plan 4). Keep one source of truth so the Tauri bridge and this
    /// stream emit identical rollups.
    pub fn active_runs_stream(&self) -> impl futures::Stream<Item = RunRollupTick> + Send + 'static {
        let sessions = self.sessions.clone();
        async_stream::stream! {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                yield sessions.rollup_tick();
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
    skip_seqs: std::collections::HashSet<u64>,
    handler_log_tx: tokio::sync::broadcast::Sender<rowforge_core::handler_log::HandlerLogLine>,
    capture_raw_stdout: bool,
    only_row_ids: Option<Vec<u64>>,
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
    //
    // in_flight / queue_depth are HEURISTIC because rowforge-core's
    // RunProgressEvent doesn't carry a RowDispatched event — we can't know
    // exactly which rows are in worker hands at any moment. The pool is
    // assumed to stay full while rows remain:
    //   in_flight = min(workers, total - processed)
    //   queue_depth = max(0, total - processed - in_flight)
    // This is accurate to within `workers` rows for steady-state runs and
    // tracks correctly during the final wind-down.
    let agg_cb = aggregator.clone();
    let workers_for_cb = workers;
    let on_progress: rowforge_core::run::ProgressCallback = Arc::new(move |ev| {
        let update_in_flight = |agg: &Arc<ProgressAggregator>, total: u64, processed: u64| {
            let remaining = total.saturating_sub(processed);
            let in_flight = (workers_for_cb as u64).min(remaining) as u32;
            let queue = remaining.saturating_sub(in_flight as u64) as u32;
            agg.set_in_flight(in_flight, queue);
        };

        match ev {
            RunProgressEvent::Started { total_rows } => {
                agg_cb.set_total(total_rows);
                agg_cb.set_phase(Phase::Running);
                update_in_flight(&agg_cb, total_rows, 0);
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
                let snap = agg_cb.snapshot();
                if let Some(total) = snap.total {
                    update_in_flight(&agg_cb, total, snap.processed);
                }
            }
            RunProgressEvent::Completed { .. } => {
                // Pipeline finished — zero out in_flight/queue_depth in case
                // the last RowDone left them at 1/0 momentarily.
                agg_cb.set_in_flight(0, 0);
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
        // rowforge-core has a legacy CLI quirk: when dry_run=true AND
        // row_limit=None, it uses dry_run_sample as the effective limit
        // (matches the old `rowforge run --dry-run --dry-run-sample N`
        // semantic). For Studio's dry-run-without-sample case ("dry-run
        // everything"), we have to pass usize::MAX to avoid getting
        // capped at zero. When row_limit is set, dry_run_sample is
        // ignored (the Some(n) arm takes priority).
        dry_run_sample: if opts.dry_run && opts.row_limit.is_none() {
            usize::MAX
        } else {
            0
        },
        // Sample / cap dispatched rows. Studio's RunOpts.row_limit maps
        // directly to rowforge-core's row_limit (usize). Cast u64→usize is
        // safe on any reasonable input (rowforge-core itself caps reads).
        row_limit: opts.row_limit.map(|n| n as usize),
        skip_seqs,
        field_map: rowforge_core::reader::FieldMap::new(),
        config_overrides: BTreeMap::new(),
        shutdown_grace: Duration::from_secs(5),
        on_progress: Some(on_progress),
        on_handler_log: Some(std::sync::Arc::new(move |line| {
            // Ignore SendError when there are no active subscribers.
            let _ = handler_log_tx.send(line);
        })),
        cancel: Some(cancel.clone()),
        input_format: None,
        fsync_outcomes: false,
        capture_raw_stdout,
        only_row_ids,
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
