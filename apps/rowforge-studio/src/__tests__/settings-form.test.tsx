import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import { SettingsForm } from "@/components/SettingsForm";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@/ipc/client", () => ({
  ipc: {
    workspace_settings_load: vi.fn().mockResolvedValue({
      schema_version: 1,
      workspace_root: "/tmp/ws",
      default_workers: 2,
      max_concurrent_runs: 3,
      telemetry_opt_in: false,
    }),
    workspace_settings_save: vi.fn().mockResolvedValue(undefined),
    run_active: vi.fn().mockResolvedValue([]),
    workspace_open: vi.fn(),
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

describe("SettingsForm", () => {
  it("loads + renders the four sections", async () => {
    render(wrap(<SettingsForm />));
    expect(await screen.findByText(/workspace/i)).toBeInTheDocument();
    expect(await screen.findByDisplayValue("2")).toBeInTheDocument();   // default_workers
    expect(await screen.findByDisplayValue("3")).toBeInTheDocument();   // max_concurrent_runs
    expect(screen.getByText(/telemetry/i)).toBeInTheDocument();
  });

  it("shows the dirty banner when max_concurrent_runs differs from loaded value", async () => {
    render(wrap(<SettingsForm />));
    const mcr = await screen.findByLabelText(/max concurrent runs/i);
    fireEvent.change(mcr, { target: { value: "5" } });
    expect(screen.getByText(/apply on next workspace open/i)).toBeInTheDocument();
  });

  it("Save calls workspace_settings_save with the form values", async () => {
    render(wrap(<SettingsForm />));
    const mcr = await screen.findByLabelText(/max concurrent runs/i);
    fireEvent.change(mcr, { target: { value: "5" } });
    fireEvent.click(screen.getByRole("button", { name: /^Save$/i }));
    await waitFor(async () => {
      const { ipc } = await import("@/ipc/client");
      expect(ipc.workspace_settings_save).toHaveBeenCalledWith({
        settings: expect.objectContaining({ max_concurrent_runs: 5 }),
      });
    });
  });
});
