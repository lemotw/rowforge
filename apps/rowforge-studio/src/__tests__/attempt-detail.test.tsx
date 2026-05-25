import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { AttemptDetailPage } from "@/pages/AttemptDetail";

vi.mock("@tauri-apps/plugin-opener", () => ({ openPath: vi.fn(), openUrl: vi.fn() }));

describe("AttemptDetail", () => {
  let qc: QueryClient;
  beforeEach(() => {
    vi.clearAllMocks();
    qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  });

  function wrap(initialEntry: string) {
    return (
      <QueryClientProvider client={qc}>
        <MemoryRouter initialEntries={[initialEntry]}>
          <Routes>
            <Route path="/exec/:id/attempt/:aid" element={<AttemptDetailPage />} />
          </Routes>
        </MemoryRouter>
      </QueryClientProvider>
    );
  }

  function fakeAttempt(isTerminal: boolean) {
    return {
      id: "a1",
      execution_id: "e1",
      state: isTerminal ? "done" : "running",
      run_type: "full",
      started_at: "2026-05-24T12:00:00Z",
      finished_at: isTerminal ? "2026-05-24T12:00:05Z" : null,
      stats: { success: 3, failed: 1, crashed: 0 },
      by_error_code: { BILLING_NOT_FOUND: 1 },
      handler_instance: { id: null, handler_id: null, version: null },
      paths: { meta_json: "/tmp/m", outcomes_jsonl: "/tmp/o", handler_stderr_log: "/tmp/s" },
      is_terminal: isTerminal,
    };
  }

  it("renders stats and no stale banner for terminal attempt", async () => {
    (invoke as any).mockImplementation((cmd: string) => {
      if (cmd === "workspace_current")
        return Promise.resolve({ root: "/tmp/ws", schema_version: 2 });
      if (cmd === "attempt_show") return Promise.resolve(fakeAttempt(true));
      throw new Error("unexpected " + cmd);
    });
    render(wrap("/exec/e1/attempt/a1"));
    expect(await screen.findByText(/Attempt a1/i)).toBeInTheDocument();
    expect(screen.queryByText(/may still be running/i)).not.toBeInTheDocument();
  });

  it("shows stale-banner for non-terminal attempt", async () => {
    (invoke as any).mockImplementation((cmd: string) => {
      if (cmd === "workspace_current")
        return Promise.resolve({ root: "/tmp/ws", schema_version: 2 });
      if (cmd === "attempt_show") return Promise.resolve(fakeAttempt(false));
      throw new Error("unexpected " + cmd);
    });
    render(wrap("/exec/e1/attempt/a1"));
    expect(await screen.findByText(/may still be running/i)).toBeInTheDocument();
  });
});
