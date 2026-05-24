import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { RunButton } from "@/components/RunButton";

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
}));

// Mock react-router-dom's useNavigate so we can assert calls.
const mockNavigate = vi.fn();
vi.mock("react-router-dom", async () => {
  const actual = await vi.importActual<typeof import("react-router-dom")>("react-router-dom");
  return { ...actual, useNavigate: () => mockNavigate };
});

function wrap(node: React.ReactNode) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return (
    <QueryClientProvider client={qc}>
      <MemoryRouter>{node}</MemoryRouter>
    </QueryClientProvider>
  );
}

describe("RunButton", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("clicking Run when no lastHandlerDir opens directory picker", async () => {
    (openDialog as any).mockResolvedValue(null); // user cancels
    render(wrap(<RunButton executionId="e1" />));
    fireEvent.click(screen.getByRole("button", { name: /^Run$/i }));
    // openDialog should have been called with directory: true.
    await new Promise((r) => setTimeout(r, 10));
    expect(openDialog).toHaveBeenCalledWith({ directory: true, multiple: false });
    expect(invoke).not.toHaveBeenCalled(); // user cancelled — no invoke
  });

  it("clicking Run with lastHandlerDir skips picker + invokes run_start", async () => {
    (invoke as any).mockResolvedValue({ handle: "run-abc", attempt_id: "att-1" });
    render(wrap(<RunButton executionId="e1" lastHandlerDir="/handlers/foo" />));
    fireEvent.click(screen.getByRole("button", { name: /^Run$/i }));
    await new Promise((r) => setTimeout(r, 10));
    expect(openDialog).not.toHaveBeenCalled();
    // Quick-run path passes nulls for the optional knobs.
    expect(invoke).toHaveBeenCalledWith("run_start", {
      executionId: "e1",
      handlerDir: "/handlers/foo",
      rowLimit: null,
      workers: null,
      dryRun: null,
    });
  });

  it("options panel forwards sample size and workers to run_start", async () => {
    (invoke as any).mockResolvedValue({ handle: "run-abc", attempt_id: "att-1" });
    render(wrap(<RunButton executionId="e1" lastHandlerDir="/handlers/foo" />));

    // Open the options panel (icon button next to Run).
    fireEvent.click(screen.getByRole("button", { name: /Run options/i }));

    // Set sample = 3, workers = 2.
    fireEvent.change(screen.getByPlaceholderText(/e\.g\. 10/), { target: { value: "3" } });
    fireEvent.change(screen.getByPlaceholderText(/e\.g\. 4/), { target: { value: "2" } });

    // Submit.
    fireEvent.click(screen.getByRole("button", { name: /^Start run$/i }));
    await new Promise((r) => setTimeout(r, 10));

    expect(invoke).toHaveBeenCalledWith("run_start", {
      executionId: "e1",
      handlerDir: "/handlers/foo",
      rowLimit: 3,
      workers: 2,
      dryRun: null,
    });
  });

  it("navigates to Live tab after successful run_start", async () => {
    mockNavigate.mockClear();
    (invoke as any).mockResolvedValue({ handle: "run-abc", attempt_id: "att-1" });
    render(wrap(<RunButton executionId="e_TEST" lastHandlerDir="/handlers/foo" />));
    fireEvent.click(screen.getByRole("button", { name: /^Run$/i }));
    await new Promise((r) => setTimeout(r, 10));
    expect(mockNavigate).toHaveBeenCalledWith(
      "/exec/e_TEST/attempt/att-1?run=run-abc",
    );
  });
});
