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
    (invoke as any).mockResolvedValue("run-abc");
    render(wrap(<RunButton executionId="e1" lastHandlerDir="/handlers/foo" />));
    fireEvent.click(screen.getByRole("button", { name: /^Run$/i }));
    await new Promise((r) => setTimeout(r, 10));
    expect(openDialog).not.toHaveBeenCalled();
    expect(invoke).toHaveBeenCalledWith("run_start", { executionId: "e1", handlerDir: "/handlers/foo" });
    // After success, "Started" button shows.
    expect(await screen.findByRole("button", { name: /Started/i })).toBeInTheDocument();
  });
});
