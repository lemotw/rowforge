import type {
  AbortReason, Phase, ProgressEvent, RunReport, RunStatus,
} from "./types";

const RECENT_BUFFER_SIZE = 200;

export interface OutcomeSampleEntry {
  row_index: number;
  kind: "error" | "crash" | "too_large";
  code: string | null;
  message: string | null;
  dur_ms: number;
  // Synthetic ordering key — newest first.
  arrived_at_ms: number;
}

export interface LifecycleBanner {
  id: string;
  kind: "worker_crashed" | "stall_warning" | "pipeline_warning";
  message: string;
  // For worker_crashed: stderr_tail
  stderr_tail?: string;
  worker_id?: number;
}

export interface RunState {
  status: RunStatus;
  phase: Phase | null;
  /** Counters from the most recent Tick. */
  processed: number;
  total: number | null;
  success: number;
  failed: number;
  crashed: number;
  in_flight: number;
  queue_depth: number;
  rate_1s: number;
  rate_10s: number;
  eta_ms: number | null;
  /** Newest first, capped at 200. */
  recentSamples: OutcomeSampleEntry[];
  /** Active lifecycle banners (worker crashes, stalls, EVENT_LAG). */
  banners: LifecycleBanner[];
  /** Set when terminal — Done or Aborted. */
  finalReport?: RunReport;
  abortReason?: AbortReason;
}

export const initialRunState: RunState = {
  status: "pending",
  phase: null,
  processed: 0,
  total: null,
  success: 0,
  failed: 0,
  crashed: 0,
  in_flight: 0,
  queue_depth: 0,
  rate_1s: 0,
  rate_10s: 0,
  eta_ms: null,
  recentSamples: [],
  banners: [],
};

let bannerIdCounter = 0;

export function reduceRun(state: RunState, event: ProgressEvent): RunState {
  switch (event.type) {
    case "phase_changed": {
      const phase = event.phase;
      // Update status based on phase. Spec §3.3:
      // Initializing/Snapshotting/Starting → Starting
      // Running → Running
      // Cancelling → Cancelling
      // Persisting → Running (still doing work)
      const status: RunStatus =
        phase === "running" ? "running"
        : phase === "cancelling" ? "cancelling"
        : phase === "persisting" ? "running"
        : "starting";
      return { ...state, phase, status };
    }
    case "tick": {
      return {
        ...state,
        processed: event.processed,
        total: event.total,
        success: event.success,
        failed: event.failed,
        crashed: event.crashed,
        in_flight: event.in_flight,
        queue_depth: event.queue_depth,
        rate_1s: event.rate_1s,
        rate_10s: event.rate_10s,
        eta_ms: event.eta_ms,
      };
    }
    case "outcome_sample": {
      const entry: OutcomeSampleEntry = {
        row_index: event.row_index,
        kind: event.kind,
        code: event.code,
        message: event.message,
        dur_ms: event.dur_ms,
        arrived_at_ms: Date.now(),
      };
      const recentSamples = [entry, ...state.recentSamples].slice(0, RECENT_BUFFER_SIZE);
      return { ...state, recentSamples };
    }
    case "worker_crashed": {
      bannerIdCounter += 1;
      return {
        ...state,
        banners: [
          ...state.banners,
          {
            id: `worker_crashed_${bannerIdCounter}`,
            kind: "worker_crashed",
            message: `Worker ${event.worker_id} crashed (signal=${event.signal ?? "n/a"})`,
            stderr_tail: event.stderr_tail,
            worker_id: event.worker_id,
          },
        ],
      };
    }
    case "stall_warning": {
      bannerIdCounter += 1;
      return {
        ...state,
        banners: [
          ...state.banners,
          {
            id: `stall_${bannerIdCounter}`,
            kind: "stall_warning",
            message: `No progress for ${event.silent_secs}s`,
          },
        ],
      };
    }
    case "pipeline_warning": {
      bannerIdCounter += 1;
      return {
        ...state,
        banners: [
          ...state.banners,
          {
            id: `warn_${bannerIdCounter}`,
            kind: "pipeline_warning",
            message: `[${event.code}] ${event.message}`,
          },
        ],
      };
    }
    case "done": {
      const final_ = {
        processed: event.processed,
        success: event.success,
        failed: event.failed,
        crashed: event.crashed,
        dur_ms: event.dur_ms,
      };
      // Sync visible counters to the authoritative final report. The
      // backend stops the tick loop on terminal, so without this the
      // last Tick before completion would freeze ProgressRegion at
      // stale numbers.
      return {
        ...state,
        status: "done",
        phase: null,
        processed: final_.processed,
        success: final_.success,
        failed: final_.failed,
        crashed: final_.crashed,
        in_flight: 0,
        queue_depth: 0,
        eta_ms: 0,
        finalReport: final_,
      };
    }
    case "aborted": {
      const status: RunStatus =
        event.reason.kind === "crashed" ? "crashed" : "aborted";
      return {
        ...state,
        status,
        phase: null,
        processed: event.partial_report.processed,
        success: event.partial_report.success,
        failed: event.partial_report.failed,
        crashed: event.partial_report.crashed,
        in_flight: 0,
        queue_depth: 0,
        eta_ms: 0,
        finalReport: event.partial_report,
        abortReason: event.reason,
      };
    }
    // Non-state-mutating events.
    case "worker_spawned":
    case "handler_ready":
    case "batch_summary":
    case "handler_stderr":
      return state;
  }
}
