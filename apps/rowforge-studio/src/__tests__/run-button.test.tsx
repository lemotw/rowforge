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
    // run_start must not fire when user cancels; handler_list may have been
    // called on mount (Plan 7 workspace handler dropdown).
    expect(invoke).not.toHaveBeenCalledWith("run_start", expect.anything());
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
      skipAttempted: null,
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
      skipAttempted: null,
    });
  });

  it("options panel 'Skip already-attempted' forwards skipAttempted=true", async () => {
    (invoke as any).mockResolvedValue({ handle: "run-abc", attempt_id: "att-1" });
    render(wrap(<RunButton executionId="e1" lastHandlerDir="/handlers/foo" />));

    fireEvent.click(screen.getByRole("button", { name: /Run options/i }));
    fireEvent.change(screen.getByPlaceholderText(/e\.g\. 10/), { target: { value: "2" } });
    fireEvent.click(screen.getByLabelText(/skip rows already attempted/i));
    fireEvent.click(screen.getByRole("button", { name: /^Start run$/i }));
    await new Promise((r) => setTimeout(r, 10));

    expect(invoke).toHaveBeenCalledWith("run_start", expect.objectContaining({
      rowLimit: 2,
      skipAttempted: true,
    }));
  });

  it("workspace handler dropdown sets handlerDir and Start run uses it", async () => {
    (invoke as any).mockImplementation((cmd: string) => {
      if (cmd === "handler_list") {
        return Promise.resolve([
          {
            name: "alpha",
            path: "/ws/handlers/alpha",
            manifest_status: "valid",
            last_modified: "2026-05-25T00:00:00Z",
            version: "0.1.0",
            language: "go",
          },
          {
            name: "broken",
            path: "/ws/handlers/broken",
            manifest_status: "invalid",
            last_modified: "2026-05-25T00:00:00Z",
            version: null,
            language: null,
          },
        ]);
      }
      if (cmd === "run_start") {
        return Promise.resolve({ handle: "run-abc", attempt_id: "att-1" });
      }
      return Promise.resolve(null);
    });

    render(wrap(<RunButton executionId="e1" />));
    fireEvent.click(screen.getByRole("button", { name: /Run options/i }));

    // Wait for handler_list query to settle so the dropdown renders.
    await new Promise((r) => setTimeout(r, 10));

    const select = screen.getByLabelText(/Workspace handler/i) as HTMLSelectElement;
    fireEvent.change(select, { target: { value: "alpha" } });

    fireEvent.click(screen.getByRole("button", { name: /^Start run$/i }));
    await new Promise((r) => setTimeout(r, 10));

    expect(invoke).toHaveBeenCalledWith("run_start", expect.objectContaining({
      executionId: "e1",
      handlerDir: "/ws/handlers/alpha",
    }));
  });

  it("invalid workspace handlers are not selectable", async () => {
    (invoke as any).mockImplementation((cmd: string) => {
      if (cmd === "handler_list") {
        return Promise.resolve([
          {
            name: "broken",
            path: "/ws/handlers/broken",
            manifest_status: "invalid",
            last_modified: "2026-05-25T00:00:00Z",
            version: null,
            language: null,
          },
        ]);
      }
      return Promise.resolve(null);
    });

    render(wrap(<RunButton executionId="e1" />));
    fireEvent.click(screen.getByRole("button", { name: /Run options/i }));
    await new Promise((r) => setTimeout(r, 10));

    const broken = screen.getByRole("option", { name: /broken \(invalid\)/i }) as HTMLOptionElement;
    expect(broken.disabled).toBe(true);
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
