import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import { WorkspaceSwitchButton } from "@/components/WorkspaceSwitchButton";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

vi.mock("@/ipc/client", () => ({
  ipc: {
    run_active: vi.fn().mockResolvedValue([]),
    workspace_settings_load: vi.fn().mockResolvedValue({
      schema_version: 1,
      workspace_root: "/tmp/old",
      default_workers: null,
      max_concurrent_runs: null,
      telemetry_opt_in: false,
    }),
    workspace_settings_save: vi.fn().mockResolvedValue(undefined),
    workspace_open: vi.fn().mockResolvedValue({ root: "/tmp/new", schema_version: 3 }),
  },
}));

function wrap(node: React.ReactNode) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return (
    <QueryClientProvider client={qc}>
      <MemoryRouter>{node}</MemoryRouter>
    </QueryClientProvider>
  );
}

beforeEach(() => { vi.clearAllMocks(); });

describe("WorkspaceSwitchButton", () => {
  it("button is enabled when no active runs", async () => {
    render(wrap(<WorkspaceSwitchButton />));
    const btn = await screen.findByRole("button", { name: /switch workspace/i });
    await waitFor(() => expect((btn as HTMLButtonElement).disabled).toBe(false));
  });

  it("button is disabled with warning when active_runs > 0", async () => {
    const { ipc } = await import("@/ipc/client");
    (ipc.run_active as any).mockResolvedValue(["run-1", "run-2"]);
    render(wrap(<WorkspaceSwitchButton />));
    const btn = await screen.findByRole("button", { name: /switch workspace/i });
    await waitFor(() => expect((btn as HTMLButtonElement).disabled).toBe(true));
    // Amber warning text mentions the active count.
    expect(await screen.findByText(/2 active runs/i)).toBeInTheDocument();
  });
});
