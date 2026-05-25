import { describe, it, expect } from "vitest";
import { initialRunState, reduceRun } from "@/ipc/run-state";
import type { ProgressEvent } from "@/ipc/types";

describe("reduceRun", () => {
  it("tick updates counters and rates", () => {
    const tick: ProgressEvent = {
      type: "tick",
      seq: 1,
      at_ms: 250,
      processed: 100,
      total: 1000,
      success: 95,
      failed: 5,
      crashed: 0,
      in_flight: 4,
      queue_depth: 12,
      rate_1s: 400,
      rate_10s: 380,
      eta_ms: 2250,
    };
    const after = reduceRun(initialRunState, tick);
    expect(after.processed).toBe(100);
    expect(after.failed).toBe(5);
    expect(after.rate_10s).toBe(380);
  });

  it("phase_changed sets status correctly", () => {
    const evt: ProgressEvent = { type: "phase_changed", phase: "running", at_ms: 0 };
    const after = reduceRun(initialRunState, evt);
    expect(after.phase).toBe("running");
    expect(after.status).toBe("running");
  });

  it("outcome_sample prepends and caps at 200", () => {
    let s = initialRunState;
    for (let i = 0; i < 250; i++) {
      s = reduceRun(s, {
        type: "outcome_sample",
        row_index: i,
        kind: "error",
        code: "X",
        message: null,
        dur_ms: 1,
      });
    }
    expect(s.recentSamples.length).toBe(200);
    expect(s.recentSamples[0].row_index).toBe(249);
  });

  it("done sets status, finalReport, and syncs visible counters", () => {
    // Pre-populate stale counters that a final tick might have left behind.
    const stale = reduceRun(initialRunState, {
      type: "tick",
      seq: 99, at_ms: 24750,
      processed: 80, total: 100, success: 70, failed: 10, crashed: 0,
      in_flight: 4, queue_depth: 16, rate_1s: 320, rate_10s: 300, eta_ms: 6250,
    });
    expect(stale.processed).toBe(80);

    const evt: ProgressEvent = {
      type: "done",
      processed: 100, success: 95, failed: 5, crashed: 0, dur_ms: 1000,
    };
    const after = reduceRun(stale, evt);
    expect(after.status).toBe("done");
    expect(after.finalReport?.success).toBe(95);
    // Visible counters must reflect the authoritative final report — backend
    // stops the tick loop on terminal, so without sync the display would
    // freeze at the stale tick (80 / 70 / 10).
    expect(after.processed).toBe(100);
    expect(after.success).toBe(95);
    expect(after.failed).toBe(5);
    expect(after.in_flight).toBe(0);
    expect(after.queue_depth).toBe(0);
    expect(after.eta_ms).toBe(0);
  });

  it("aborted maps crashed reason to crashed status and syncs counters from partial_report", () => {
    const stale = reduceRun(initialRunState, {
      type: "tick",
      seq: 5, at_ms: 1250,
      processed: 30, total: 100, success: 25, failed: 5, crashed: 0,
      in_flight: 8, queue_depth: 60, rate_1s: 24, rate_10s: 20, eta_ms: 3500,
    });

    const evt: ProgressEvent = {
      type: "aborted",
      reason: { kind: "crashed", panic_message: "boom" },
      at_phase: "running",
      partial_report: { processed: 50, success: 40, failed: 10, crashed: 0, dur_ms: 500 },
    };
    const after = reduceRun(stale, evt);
    expect(after.status).toBe("crashed");
    expect(after.abortReason?.kind).toBe("crashed");
    expect(after.processed).toBe(50);
    expect(after.success).toBe(40);
    expect(after.failed).toBe(10);
    expect(after.in_flight).toBe(0);
  });

  it("_bootstrap fills counter + phase + rate fields from snapshot", () => {
    const after = reduceRun(initialRunState, {
      type: "_bootstrap",
      snapshot: {
        processed: 50,
        total: 100,
        success: 45,
        failed: 5,
        crashed: 0,
        in_flight: 4,
        queue_depth: 16,
        phase: "running",
        rate_10s: 12.5,
      },
    });
    expect(after.processed).toBe(50);
    expect(after.total).toBe(100);
    expect(after.success).toBe(45);
    expect(after.failed).toBe(5);
    expect(after.in_flight).toBe(4);
    expect(after.queue_depth).toBe(16);
    expect(after.phase).toBe("running");
    // Plan 6 (review fix): rate also bootstrapped.
    expect(after.rate_10s).toBe(12.5);
    // Status derived from phase per the phase_changed logic.
    expect(after.status).toBe("running");
  });

  it("_bootstrap does not touch event-only accumulators", () => {
    let s = initialRunState;
    s = reduceRun(s, {
      type: "outcome_sample",
      row_index: 7, kind: "error", code: "X", message: null, dur_ms: 1,
    });
    s = reduceRun(s, {
      type: "_bootstrap",
      snapshot: {
        processed: 100, total: 100, success: 95, failed: 5, crashed: 0,
        in_flight: 0, queue_depth: 0, phase: null, rate_10s: 0,
      },
    });
    // recentSamples survives the bootstrap.
    expect(s.recentSamples.length).toBe(1);
    expect(s.recentSamples[0].row_index).toBe(7);
  });

  it("worker_crashed adds a banner", () => {
    const evt: ProgressEvent = {
      type: "worker_crashed",
      worker_id: 2,
      last_seq: 99,
      exit_code: null,
      signal: 11,
      stderr_tail: "boom",
    };
    const after = reduceRun(initialRunState, evt);
    expect(after.banners.length).toBe(1);
    expect(after.banners[0].kind).toBe("worker_crashed");
    expect(after.banners[0].stderr_tail).toBe("boom");
  });
});
