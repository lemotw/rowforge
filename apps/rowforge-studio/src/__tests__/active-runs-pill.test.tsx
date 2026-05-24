import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ActiveRunsPill } from "@/components/ActiveRunsPill";

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(),
}));

function wrap(node: React.ReactNode) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return (
    <QueryClientProvider client={qc}>
      <MemoryRouter>{node}</MemoryRouter>
    </QueryClientProvider>
  );
}

describe("ActiveRunsPill", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    (listen as any).mockResolvedValue(() => {});
  });

  it("renders nothing when active_runs is 0 (initial state)", () => {
    (invoke as any).mockResolvedValue([]);
    render(wrap(<ActiveRunsPill />));
    expect(screen.queryByText(/running/i)).not.toBeInTheDocument();
  });

  it("renders pill when active_runs > 0", async () => {
    // Set up listen mock to immediately invoke handler with active_runs=2
    (listen as any).mockImplementation((_chan: string, cb: any) => {
      cb({ payload: { active_runs: 2, total_processed: 100, total_failed: 5, total_rate: 0, slowest_run: null } });
      return Promise.resolve(() => {});
    });
    (invoke as any).mockResolvedValue(["run-1", "run-2"]);

    render(wrap(<ActiveRunsPill />));
    expect(await screen.findByText("2")).toBeInTheDocument();
    expect(screen.getByText(/running/i)).toBeInTheDocument();
  });
});
