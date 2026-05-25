import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter, Route, Routes, useLocation } from "react-router-dom";
import { DeleteHandlerDialog } from "@/components/DeleteHandlerDialog";
import type React from "react";

// ── Mocks ──────────────────────────────────────────────────────────────────

vi.mock("sonner", () => ({
  toast: {
    success: vi.fn(),
    error: vi.fn(),
    loading: vi.fn().mockReturnValue("toast-1"),
    dismiss: vi.fn(),
  },
}));

vi.mock("@/ipc/client", () => ({
  ipc: {
    workspace_current: vi.fn().mockResolvedValue({ root: "/tmp/ws", schema_version: 2 }),
    handler_list: vi.fn().mockResolvedValue([]),
    handler_delete: vi.fn(),
  },
}));

// ── Helpers ────────────────────────────────────────────────────────────────

function LocationDisplay() {
  const loc = useLocation();
  return <div data-testid="location">{loc.pathname}</div>;
}

function wrap(node: React.ReactNode, initialPath = "/handlers/my-handler") {
  const qc = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });
  return (
    <QueryClientProvider client={qc}>
      <MemoryRouter initialEntries={[initialPath]}>
        <Routes>
          <Route path="/handlers/:name" element={node} />
          <Route path="/handlers" element={<LocationDisplay />} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>
  );
}

const noop = () => {};

beforeEach(() => {
  vi.clearAllMocks();
});

// ── Tests ──────────────────────────────────────────────────────────────────

describe("DeleteHandlerDialog", () => {
  // 1. Renders title with handler name
  it("renders the dialog title with the handler name", () => {
    render(wrap(<DeleteHandlerDialog open={true} onOpenChange={noop} name="my-handler" />));
    expect(screen.getByText(/Delete handler "my-handler"\?/i)).toBeInTheDocument();
  });

  // 2. Delete button disabled when token empty
  it("disables the Delete button when token is empty", () => {
    render(wrap(<DeleteHandlerDialog open={true} onOpenChange={noop} name="my-handler" />));
    const deleteBtn = screen.getByRole("button", { name: /^Delete$/i });
    expect(deleteBtn).toBeDisabled();
  });

  // 3. Delete button disabled when token doesn't match exactly (case-sensitive)
  it("disables the Delete button when token doesn't match (case-sensitive)", () => {
    render(wrap(<DeleteHandlerDialog open={true} onOpenChange={noop} name="my-handler" />));
    const input = screen.getByLabelText(/Type.*to confirm/i);
    fireEvent.change(input, { target: { value: "My-Handler" } });
    const deleteBtn = screen.getByRole("button", { name: /^Delete$/i });
    expect(deleteBtn).toBeDisabled();
  });

  // 4. Delete button enabled when token matches exactly
  it("enables the Delete button when token matches exactly", () => {
    render(wrap(<DeleteHandlerDialog open={true} onOpenChange={noop} name="my-handler" />));
    const input = screen.getByLabelText(/Type.*to confirm/i);
    fireEvent.change(input, { target: { value: "my-handler" } });
    const deleteBtn = screen.getByRole("button", { name: /^Delete$/i });
    expect(deleteBtn).not.toBeDisabled();
  });

  // 5. Happy path: calls ipc.handler_delete → toast + close + navigate to /handlers
  it("calls ipc.handler_delete, toasts, closes, and navigates to /handlers on success", async () => {
    const { ipc } = await import("@/ipc/client");
    const { toast } = await import("sonner");
    (ipc.handler_delete as ReturnType<typeof vi.fn>).mockResolvedValue(undefined);

    const onOpenChange = vi.fn();
    render(wrap(<DeleteHandlerDialog open={true} onOpenChange={onOpenChange} name="my-handler" />));

    const input = screen.getByLabelText(/Type.*to confirm/i);
    fireEvent.change(input, { target: { value: "my-handler" } });

    fireEvent.click(screen.getByRole("button", { name: /^Delete$/i }));

    await waitFor(() => {
      expect(ipc.handler_delete).toHaveBeenCalledWith({ name: "my-handler" });
    });

    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith('Handler "my-handler" deleted');
    });

    expect(onOpenChange).toHaveBeenCalledWith(false);

    // Navigation should reach /handlers
    expect(await screen.findByTestId("location")).toHaveTextContent("/handlers");
  });

  // 6. Error path: HandlerNotFound → shows inline error, stays open
  it("shows inline error banner on HandlerNotFound and keeps dialog open", async () => {
    const { ipc } = await import("@/ipc/client");
    (ipc.handler_delete as ReturnType<typeof vi.fn>).mockRejectedValue({
      kind: "handler_not_found",
      message: { name: "my-handler" },
    });

    const onOpenChange = vi.fn();
    render(wrap(<DeleteHandlerDialog open={true} onOpenChange={onOpenChange} name="my-handler" />));

    const input = screen.getByLabelText(/Type.*to confirm/i);
    fireEvent.change(input, { target: { value: "my-handler" } });

    fireEvent.click(screen.getByRole("button", { name: /^Delete$/i }));

    await waitFor(() => {
      expect(
        screen.getByText(/\[handler_not_found\]/i)
      ).toBeInTheDocument();
    });

    // Dialog should NOT have been closed
    expect(onOpenChange).not.toHaveBeenCalledWith(false);
  });
});
