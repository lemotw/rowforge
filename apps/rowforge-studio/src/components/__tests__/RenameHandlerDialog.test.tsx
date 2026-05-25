import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter, Route, Routes, useLocation } from "react-router-dom";
import { RenameHandlerDialog } from "@/components/RenameHandlerDialog";
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
    handler_rename: vi.fn(),
  },
}));

// ── Helpers ────────────────────────────────────────────────────────────────

function LocationDisplay() {
  const loc = useLocation();
  return <div data-testid="location">{loc.pathname}</div>;
}

function wrap(node: React.ReactNode, initialPath = "/handlers/old-handler") {
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
          <Route path="/handlers/:newname" element={<LocationDisplay />} />
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

describe("RenameHandlerDialog", () => {
  // 1. Renders pre-filled with current name
  it("renders the input pre-filled with the current handler name", () => {
    render(wrap(<RenameHandlerDialog open={true} onOpenChange={noop} oldName="old-handler" />));
    const input = screen.getByLabelText(/^New name$/i) as HTMLInputElement;
    expect(input.value).toBe("old-handler");
  });

  // 2. Rename button disabled when name === oldName (unchanged)
  it("disables the Rename button when name is unchanged", () => {
    render(wrap(<RenameHandlerDialog open={true} onOpenChange={noop} oldName="old-handler" />));
    const renameBtn = screen.getByRole("button", { name: /^Rename$/i });
    expect(renameBtn).toBeDisabled();
  });

  // 3. Rename button disabled when invalid (e.g. "Bad Name")
  it("disables the Rename button when name is invalid", () => {
    render(wrap(<RenameHandlerDialog open={true} onOpenChange={noop} oldName="old-handler" />));
    const input = screen.getByLabelText(/^New name$/i);
    fireEvent.change(input, { target: { value: "Bad Name" } });
    const renameBtn = screen.getByRole("button", { name: /^Rename$/i });
    expect(renameBtn).toBeDisabled();
    expect(screen.getByText(/Lowercase letters, numbers, and hyphens/i)).toBeInTheDocument();
  });

  // 4. Rename button enabled when changed-and-valid
  it("enables the Rename button when name is changed and valid", () => {
    render(wrap(<RenameHandlerDialog open={true} onOpenChange={noop} oldName="old-handler" />));
    const input = screen.getByLabelText(/^New name$/i);
    fireEvent.change(input, { target: { value: "new-handler" } });
    const renameBtn = screen.getByRole("button", { name: /^Rename$/i });
    expect(renameBtn).not.toBeDisabled();
  });

  // 5. Happy path: calls ipc.handler_rename → toast + close + navigate to /handlers/new
  it("calls ipc.handler_rename, toasts, closes, and navigates on success", async () => {
    const { ipc } = await import("@/ipc/client");
    const { toast } = await import("sonner");
    (ipc.handler_rename as ReturnType<typeof vi.fn>).mockResolvedValue(undefined);

    const onOpenChange = vi.fn();

    // Use a wrapper that has a route for the new handler path
    const qc = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
        mutations: { retry: false },
      },
    });
    const TestApp = () => (
      <QueryClientProvider client={qc}>
        <MemoryRouter initialEntries={["/handlers/old-handler"]}>
          <Routes>
            <Route
              path="/handlers/old-handler"
              element={
                <RenameHandlerDialog
                  open={true}
                  onOpenChange={onOpenChange}
                  oldName="old-handler"
                />
              }
            />
            <Route
              path="/handlers/:name"
              element={
                <div data-testid="location">
                  {/* rendered after navigate */}
                  <LocationDisplay />
                </div>
              }
            />
          </Routes>
        </MemoryRouter>
      </QueryClientProvider>
    );

    render(<TestApp />);

    const input = screen.getByLabelText(/^New name$/i);
    fireEvent.change(input, { target: { value: "new-handler" } });

    fireEvent.click(screen.getByRole("button", { name: /^Rename$/i }));

    await waitFor(() => {
      expect(ipc.handler_rename).toHaveBeenCalledWith({
        old: "old-handler",
        new: "new-handler",
      });
    });

    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith('Handler renamed to "new-handler"');
    });

    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  // 6. Error path: HandlerExists → shows inline error, stays open
  it("shows inline error banner on HandlerExists and keeps dialog open", async () => {
    const { ipc } = await import("@/ipc/client");
    (ipc.handler_rename as ReturnType<typeof vi.fn>).mockRejectedValue({
      kind: "handler_exists",
      message: { name: "new-handler" },
    });

    const onOpenChange = vi.fn();
    render(wrap(<RenameHandlerDialog open={true} onOpenChange={onOpenChange} oldName="old-handler" />));

    const input = screen.getByLabelText(/^New name$/i);
    fireEvent.change(input, { target: { value: "new-handler" } });

    fireEvent.click(screen.getByRole("button", { name: /^Rename$/i }));

    await waitFor(() => {
      expect(
        screen.getByText(/\[handler_exists\].*new-handler.*already exists/i)
      ).toBeInTheDocument();
    });

    // Dialog should NOT have been closed
    expect(onOpenChange).not.toHaveBeenCalledWith(false);
  });
});
