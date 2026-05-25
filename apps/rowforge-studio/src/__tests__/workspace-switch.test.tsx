import { describe, it, expect, vi, beforeEach } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
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

const mockNavigate = vi.fn();
vi.mock("react-router-dom", async () => {
  const actual = await vi.importActual<typeof import("react-router-dom")>("react-router-dom");
  return { ...actual, useNavigate: () => mockNavigate };
});

function wrapWith(qc: QueryClient, node: React.ReactNode) {
  return (
    <QueryClientProvider client={qc}>
      <MemoryRouter>{node}</MemoryRouter>
    </QueryClientProvider>
  );
}

function freshQc() {
  return new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
}

beforeEach(() => { vi.clearAllMocks(); });

describe("WorkspaceSwitchButton", () => {
  it("button is enabled when no active runs", async () => {
    render(wrapWith(freshQc(), <WorkspaceSwitchButton />));
    const btn = await screen.findByRole("button", { name: /switch workspace/i });
    await waitFor(() => expect((btn as HTMLButtonElement).disabled).toBe(false));
  });

  it("button is disabled with warning when active_runs > 0", async () => {
    const { ipc } = await import("@/ipc/client");
    (ipc.run_active as any).mockResolvedValue(["run-1", "run-2"]);
    render(wrapWith(freshQc(), <WorkspaceSwitchButton />));
    const btn = await screen.findByRole("button", { name: /switch workspace/i });
    await waitFor(() => expect((btn as HTMLButtonElement).disabled).toBe(true));
    expect(await screen.findByText(/2 active runs/i)).toBeInTheDocument();
  });

  it("switch primes settings + workspace cache before navigate; drops other queries", async () => {
    const { ipc } = await import("@/ipc/client");
    const { open: openDialog } = await import("@tauri-apps/plugin-dialog");
    // Previous test mutated run_active mock to return 2 handles; reset
    // to empty so this test starts with the button enabled.
    (ipc.run_active as any).mockResolvedValue([]);
    (openDialog as any).mockResolvedValue("/tmp/new");

    const qc = freshQc();
    // Seed stale state from the OLD workspace.
    qc.setQueryData(["settings"], {
      schema_version: 1, workspace_root: "/tmp/old",
      default_workers: null, max_concurrent_runs: null, telemetry_opt_in: false,
    });
    qc.setQueryData(["workspace"], { root: "/tmp/old", schema_version: 3 });
    qc.setQueryData(["exec_list"], [{ id: "e_OLD", name: "from-old-workspace" }]);
    qc.setQueryData(["exec_show", "e_OLD"], { summary: { id: "e_OLD" } });

    render(wrapWith(qc, <WorkspaceSwitchButton />));
    const btn = await screen.findByRole("button", { name: /switch workspace/i });
    await waitFor(() => expect((btn as HTMLButtonElement).disabled).toBe(false));

    await act(async () => {
      fireEvent.click(btn);
      // Drain promise microtasks for: dialog open → settings_load →
      // settings_save → workspace_open → onSuccess → navigate.
      await new Promise((r) => setTimeout(r, 0));
      await new Promise((r) => setTimeout(r, 0));
    });
    await waitFor(() => expect(mockNavigate).toHaveBeenCalledWith("/"), { timeout: 2000 });

    // settings + workspace: primed to NEW values, not stale.
    expect((qc.getQueryData(["settings"]) as any)?.workspace_root).toBe("/tmp/new");
    expect((qc.getQueryData(["workspace"]) as any)?.root).toBe("/tmp/new");
    // Other old-workspace queries: removed (undefined), not just invalidated.
    // Critical: BootGate on "/" must not see a cached exec list from the
    // previous workspace.
    expect(qc.getQueryData(["exec_list"])).toBeUndefined();
    expect(qc.getQueryData(["exec_show", "e_OLD"])).toBeUndefined();

    // workspace_settings_save was called with the new root in payload.
    expect(ipc.workspace_settings_save).toHaveBeenCalledWith({
      settings: expect.objectContaining({ workspace_root: "/tmp/new" }),
    });
  });
});
