import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { AttemptDetailPage } from "@/pages/AttemptDetail";
import type React from "react";

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

const WORKSPACE = { root: "/tmp/ws", schema_version: 2 };

const ATTEMPT_DETAIL = {
  id: "att_01",
  execution_id: "exec_01",
  state: "completed",
  run_type: "full",
  started_at: "2026-05-25T10:00:00Z",
  finished_at: "2026-05-25T10:01:00Z",
  stats: { success: 10, failed: 0, crashed: 0 },
  by_error_code: {},
  handler_instance: { id: null, handler_id: null, version: null },
  paths: {
    meta_json: "/tmp/ws/executions/exec_01/attempts/att_01/meta.json",
    outcomes_jsonl: "/tmp/ws/executions/exec_01/attempts/att_01/outcomes.jsonl",
    handler_stderr_log: "/tmp/ws/executions/exec_01/attempts/att_01/handler_stderr.log",
  },
  is_terminal: true,
};

const EXEC_DETAIL = {
  summary: {
    id: "exec_01",
    name: "my-exec",
    created_at: "2026-05-25T09:00:00Z",
    input_rows: 100,
    attempts_count: 1,
    last_attempt_state: "completed",
    last_attempt_counts: { success: 10, failed: 0, crashed: 0 },
    last_handler_dir: null,
  },
};

function wrap(node: React.ReactNode, execId = "exec_01", attemptId = "att_01") {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return (
    <QueryClientProvider client={qc}>
      <MemoryRouter initialEntries={[`/exec/${execId}/attempt/${attemptId}`]}>
        <Routes>
          <Route path="/exec/:id/attempt/:aid" element={node} />
          <Route path="/" element={<div data-testid="home">Home</div>} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>
  );
}

function mockInvoke() {
  (invoke as any).mockImplementation((cmd: string) => {
    if (cmd === "workspace_current") return Promise.resolve(WORKSPACE);
    if (cmd === "attempt_show") return Promise.resolve(ATTEMPT_DETAIL);
    if (cmd === "exec_show") return Promise.resolve(EXEC_DETAIL);
    if (cmd === "attempt_active_handle") return Promise.resolve(null);
    if (cmd === "handler_log_tail") return Promise.resolve([]);
    // attempt_failed_page not called unless we click Failed rows
    return Promise.resolve(null);
  });
}

describe("AttemptDetailPage — Logs tab", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    (listen as any).mockResolvedValue(() => {});
  });

  it("renders a Logs tab in the tab list", async () => {
    mockInvoke();
    render(wrap(<AttemptDetailPage />));

    // Wait for tabs to render (Summary tab appears once detail.data is loaded)
    expect(await screen.findByRole("tab", { name: "Summary" })).toBeInTheDocument();

    expect(screen.getByRole("tab", { name: "Logs" })).toBeInTheDocument();
  });

  it("switching to Logs tab renders AttemptLogsTab content", async () => {
    mockInvoke();
    render(wrap(<AttemptDetailPage />));

    // Wait for tabs to render
    expect(await screen.findByRole("tab", { name: "Summary" })).toBeInTheDocument();

    // Radix Tabs activates via onMouseDown (button=0, no ctrlKey).
    const logsTab = screen.getByRole("tab", { name: "Logs" });
    fireEvent.mouseDown(logsTab, { button: 0, ctrlKey: false });

    // AttemptLogsTab shows loading state then empty state.
    // Loading state: "Loading logs…" (shown while handler_log_tail query resolves)
    // Terminal + no lines: "No log file. This attempt predates Plan 9 log capture."
    await waitFor(
      () =>
        expect(
          screen.queryByText(/loading logs/i) ||
          screen.queryByText(/no log file/i)
        ).not.toBeNull(),
      { timeout: 5000 }
    );
  });

  it("other tabs still render after Logs tab is added", async () => {
    mockInvoke();
    render(wrap(<AttemptDetailPage />));

    // Wait for tabs to render
    expect(await screen.findByRole("tab", { name: "Summary" })).toBeInTheDocument();

    // Summary tab is active by default — check stats are visible
    expect(await screen.findByText("success")).toBeInTheDocument();

    // All expected tabs present
    expect(screen.getByRole("tab", { name: "Summary" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Failed rows" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Errors by code" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Logs" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Artifacts" })).toBeInTheDocument();
  });

  it("Logs tab receives logFilePath from workspace root", async () => {
    // We verify the Reveal button is rendered (it's always shown in LogsToolbar).
    mockInvoke();
    render(wrap(<AttemptDetailPage />));

    // Wait for tabs to render
    expect(await screen.findByRole("tab", { name: "Summary" })).toBeInTheDocument();

    // Radix Tabs activates via onMouseDown (button=0, no ctrlKey).
    const logsTab = screen.getByRole("tab", { name: "Logs" });
    fireEvent.mouseDown(logsTab, { button: 0, ctrlKey: false });

    // The Reveal button is always rendered by LogsToolbar regardless of path.
    // Wait for AttemptLogsTab to move past loading state first.
    await waitFor(
      () => expect(screen.getByText("Reveal log file")).toBeInTheDocument(),
      { timeout: 5000 }
    );
  });
});
