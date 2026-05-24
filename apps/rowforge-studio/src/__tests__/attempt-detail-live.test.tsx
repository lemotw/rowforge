import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { AttemptDetailPage } from "@/pages/AttemptDetail";

// Mock @tauri-apps/api/event listen so useRun's effect runs.
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

vi.mock("@tauri-apps/plugin-shell", () => ({ open: vi.fn() }));

function wrap(initial: string) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return (
    <QueryClientProvider client={qc}>
      <MemoryRouter initialEntries={[initial]}>
        <Routes>
          <Route path="/exec/:id/attempt/:aid" element={<AttemptDetailPage />} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>
  );
}

const fakeAttempt = (isTerminal: boolean) => ({
  id: "a1",
  execution_id: "e1",
  state: isTerminal ? "done" : "running",
  run_type: "full",
  started_at: "2026-05-24T12:00:00Z",
  finished_at: isTerminal ? "2026-05-24T12:00:05Z" : null,
  stats: { success: 3, failed: 1, crashed: 0 },
  by_error_code: {},
  handler_instance: { id: null, handler_id: null, version: null },
  paths: { meta_json: "/tmp/m", outcomes_jsonl: "/tmp/o", handler_stderr_log: "/tmp/s" },
  is_terminal: isTerminal,
});

describe("AttemptDetail Live integration", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders Live tab when ?run=<handle> is present", async () => {
    (invoke as any).mockImplementation((cmd: string) => {
      if (cmd === "workspace_current")
        return Promise.resolve({ root: "/tmp/ws", schema_version: 2 });
      if (cmd === "attempt_show") return Promise.resolve(fakeAttempt(false));
      throw new Error("unexpected " + cmd);
    });

    render(wrap("/exec/e1/attempt/a1?run=run-test-1"));

    expect(await screen.findByText(/^Live$/)).toBeInTheDocument();
    // Cancel button exists (CancelDialog renders a Cancel button when status is not "cancelling")
    expect(screen.getByRole("button", { name: /^Cancel$/i })).toBeInTheDocument();
  });

  it("does NOT render Live tab when no ?run param", async () => {
    (invoke as any).mockImplementation((cmd: string) => {
      if (cmd === "workspace_current")
        return Promise.resolve({ root: "/tmp/ws", schema_version: 2 });
      if (cmd === "attempt_show") return Promise.resolve(fakeAttempt(false));
      throw new Error("unexpected " + cmd);
    });

    render(wrap("/exec/e1/attempt/a1"));
    // Live tab should not be present.
    expect(screen.queryByText(/^Live$/)).not.toBeInTheDocument();
    // Stale banner SHOULD be present (non-terminal + no live).
    expect(await screen.findByText(/may still be running/i)).toBeInTheDocument();
  });
});
