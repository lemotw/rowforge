import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { AttemptLogsTab } from "@/pages/AttemptLogsTab";
import type { HandlerLogLine } from "@/ipc/types";

// useVirtualizer relies on DOM layout measurements unavailable in jsdom.
vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: ({ count, estimateSize }: { count: number; estimateSize: () => number }) => ({
    getVirtualItems: () =>
      Array.from({ length: count }, (_, i) => ({
        key: i,
        index: i,
        start: i * estimateSize(),
        size: estimateSize(),
      })),
    getTotalSize: () => count * estimateSize(),
  }),
}));

vi.mock("@tauri-apps/plugin-shell", () => ({ open: vi.fn() }));

function wrap(node: React.ReactNode) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return <QueryClientProvider client={qc}>{node}</QueryClientProvider>;
}

const line1: HandlerLogLine = {
  timestamp: "2026-05-25T10:00:00Z",
  worker_id: 0,
  stream: "stderr",
  line: "booting up",
};
const line2: HandlerLogLine = {
  timestamp: "2026-05-25T10:00:01Z",
  worker_id: 1,
  stream: "stdout",
  line: "processing row 1",
};
const line3: HandlerLogLine = {
  timestamp: "2026-05-25T10:00:02Z",
  worker_id: 0,
  stream: "stderr",
  line: "error: something failed",
};

describe("AttemptLogsTab", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // Default: listen returns a no-op unlisten
    (listen as any).mockResolvedValue(() => {});
  });

  it("shows loading state while tail query is in-flight", () => {
    // Never resolves
    (invoke as any).mockReturnValue(new Promise(() => {}));
    render(wrap(<AttemptLogsTab execId="e1" attemptId="a1" isLive={false} />));
    expect(screen.getByText(/loading logs/i)).toBeInTheDocument();
  });

  it("shows 'no log file' empty state for a terminal attempt with no lines", async () => {
    (invoke as any).mockImplementation((cmd: string) => {
      if (cmd === "handler_log_tail") return Promise.resolve([]);
      return Promise.reject(new Error("unexpected: " + cmd));
    });
    render(wrap(<AttemptLogsTab execId="e1" attemptId="a1" isLive={false} />));
    expect(
      await screen.findByText(/no log file.*predates plan 9/i)
    ).toBeInTheDocument();
  });

  it("shows 'no output yet' empty state for a live attempt with no lines", async () => {
    (invoke as any).mockImplementation((cmd: string) => {
      if (cmd === "handler_log_tail") return Promise.resolve([]);
      if (cmd === "handler_log_subscribe") return Promise.resolve(undefined);
      if (cmd === "handler_log_unsubscribe") return Promise.resolve(undefined);
      return Promise.reject(new Error("unexpected: " + cmd));
    });
    render(wrap(<AttemptLogsTab execId="e1" attemptId="a1" isLive={true} />));
    expect(
      await screen.findByText("Handler has not produced any output yet.", {}, { timeout: 3000 })
    ).toBeInTheDocument();
  });

  it("renders log lines from tail snapshot", async () => {
    (invoke as any).mockImplementation((cmd: string) => {
      if (cmd === "handler_log_tail") return Promise.resolve([line1, line2]);
      return Promise.reject(new Error("unexpected: " + cmd));
    });
    render(wrap(<AttemptLogsTab execId="e1" attemptId="a1" isLive={false} />));
    expect(await screen.findByText("booting up")).toBeInTheDocument();
    expect(screen.getByText("processing row 1")).toBeInTheDocument();
  });

  it("displays new lines that arrive from live stream after mount", async () => {
    let listener: ((e: { payload: { lines: HandlerLogLine[]; dropped: number } }) => void) | undefined;

    (invoke as any).mockImplementation((cmd: string) => {
      if (cmd === "handler_log_tail") return Promise.resolve([line1]);
      if (cmd === "handler_log_subscribe") return Promise.resolve(undefined);
      if (cmd === "handler_log_unsubscribe") return Promise.resolve(undefined);
      return Promise.reject(new Error("unexpected: " + cmd));
    });
    (listen as any).mockImplementation(async (_name: string, fn: any) => {
      listener = fn;
      return () => {};
    });

    render(wrap(<AttemptLogsTab execId="e1" attemptId="a1" isLive={true} />));

    // Wait for the tail to load and the listener to be registered
    expect(await screen.findByText("booting up")).toBeInTheDocument();
    await waitFor(() => expect(listener).toBeDefined());

    // Fire a live event
    listener!({ payload: { lines: [line2], dropped: 0 } });

    expect(await screen.findByText("processing row 1")).toBeInTheDocument();
  });

  it("renders dropped banner when live.dropped > 0", async () => {
    let listener: ((e: { payload: { lines: HandlerLogLine[]; dropped: number } }) => void) | undefined;

    (invoke as any).mockImplementation((cmd: string) => {
      if (cmd === "handler_log_tail") return Promise.resolve([line1]);
      if (cmd === "handler_log_subscribe") return Promise.resolve(undefined);
      if (cmd === "handler_log_unsubscribe") return Promise.resolve(undefined);
      return Promise.reject(new Error("unexpected: " + cmd));
    });
    (listen as any).mockImplementation(async (_name: string, fn: any) => {
      listener = fn;
      return () => {};
    });

    render(wrap(<AttemptLogsTab execId="e1" attemptId="a1" isLive={true} />));
    expect(await screen.findByText("booting up")).toBeInTheDocument();
    await waitFor(() => expect(listener).toBeDefined());

    listener!({ payload: { lines: [], dropped: 42 } });

    expect(await screen.findByText(/42 log lines dropped/i)).toBeInTheDocument();
  });

  it("worker filter narrows visible lines", async () => {
    (invoke as any).mockImplementation((cmd: string) => {
      if (cmd === "handler_log_tail") return Promise.resolve([line1, line2, line3]);
      return Promise.reject(new Error("unexpected: " + cmd));
    });
    render(wrap(<AttemptLogsTab execId="e1" attemptId="a1" isLive={false} />));

    // Wait for lines to render
    expect(await screen.findByText("booting up")).toBeInTheDocument();
    expect(screen.getByText("processing row 1")).toBeInTheDocument();

    // Click worker #1 chip to filter to worker 1 only
    const worker1Chip = screen.getByRole("button", { name: /worker 1/i });
    fireEvent.click(worker1Chip);

    // Worker 0 lines should disappear, worker 1 line stays
    expect(screen.queryByText("booting up")).not.toBeInTheDocument();
    expect(screen.getByText("processing row 1")).toBeInTheDocument();
    expect(screen.queryByText("error: something failed")).not.toBeInTheDocument();
  });

  it("stream filter narrows visible lines", async () => {
    (invoke as any).mockImplementation((cmd: string) => {
      if (cmd === "handler_log_tail") return Promise.resolve([line1, line2, line3]);
      return Promise.reject(new Error("unexpected: " + cmd));
    });
    render(wrap(<AttemptLogsTab execId="e1" attemptId="a1" isLive={false} />));

    // Wait for all lines
    expect(await screen.findByText("booting up")).toBeInTheDocument();

    // Click the "stdout" stream filter
    fireEvent.click(screen.getByRole("button", { name: "stdout", hidden: false }));

    // Only stdout line visible
    expect(screen.queryByText("booting up")).not.toBeInTheDocument();
    expect(screen.getByText("processing row 1")).toBeInTheDocument();
    expect(screen.queryByText("error: something failed")).not.toBeInTheDocument();
  });
});
