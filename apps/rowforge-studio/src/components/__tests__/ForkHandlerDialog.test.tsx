import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter, Route, Routes, useLocation } from "react-router-dom";
import { ForkHandlerDialog } from "@/components/ForkHandlerDialog";
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
    handler_fork: vi.fn(),
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

describe("ForkHandlerDialog", () => {
  // 1. Pre-fills name with <source>-fork
  it("pre-fills the input with '<sourceName>-fork'", () => {
    render(
      wrap(
        <ForkHandlerDialog open={true} onOpenChange={noop} sourceName="my-handler" />,
      ),
    );
    const input = screen.getByLabelText(/^New name$/i) as HTMLInputElement;
    expect(input.value).toBe("my-handler-fork");
  });

  // 2. Fork button disabled when name equals source (user clears the -fork suffix)
  it("disables the Fork button when name equals the source name", () => {
    render(
      wrap(
        <ForkHandlerDialog open={true} onOpenChange={noop} sourceName="my-handler" />,
      ),
    );
    const input = screen.getByLabelText(/^New name$/i);
    fireEvent.change(input, { target: { value: "my-handler" } });
    const forkBtn = screen.getByRole("button", { name: /^Fork$/i });
    expect(forkBtn).toBeDisabled();
    expect(screen.getByText(/Name must differ from source/i)).toBeInTheDocument();
  });

  // 3. Fork button disabled for invalid name regex (e.g. "Bad-Name")
  it("disables the Fork button when name is invalid", () => {
    render(
      wrap(
        <ForkHandlerDialog open={true} onOpenChange={noop} sourceName="my-handler" />,
      ),
    );
    const input = screen.getByLabelText(/^New name$/i);
    fireEvent.change(input, { target: { value: "Bad-Name" } });
    const forkBtn = screen.getByRole("button", { name: /^Fork$/i });
    expect(forkBtn).toBeDisabled();
    expect(screen.getByText(/Lowercase letters, numbers, and hyphens/i)).toBeInTheDocument();
  });

  // 4. Happy path: clicks Fork → ipc.handler_fork called → onSuccess → close + navigate
  it("calls ipc.handler_fork, toasts, closes, and navigates on success", async () => {
    const { ipc } = await import("@/ipc/client");
    const { toast } = await import("sonner");
    (ipc.handler_fork as ReturnType<typeof vi.fn>).mockResolvedValue(undefined);

    const onOpenChange = vi.fn();

    const qc = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
        mutations: { retry: false },
      },
    });
    const TestApp = () => (
      <QueryClientProvider client={qc}>
        <MemoryRouter initialEntries={["/handlers/my-handler"]}>
          <Routes>
            <Route
              path="/handlers/my-handler"
              element={
                <ForkHandlerDialog
                  open={true}
                  onOpenChange={onOpenChange}
                  sourceName="my-handler"
                />
              }
            />
            <Route
              path="/handlers/:name"
              element={
                <div data-testid="location">
                  <LocationDisplay />
                </div>
              }
            />
          </Routes>
        </MemoryRouter>
      </QueryClientProvider>
    );

    render(<TestApp />);

    // Default pre-fill is "my-handler-fork" which is valid — click Fork directly
    const forkBtn = screen.getByRole("button", { name: /^Fork$/i });
    expect(forkBtn).not.toBeDisabled();
    fireEvent.click(forkBtn);

    await waitFor(() => {
      expect(ipc.handler_fork).toHaveBeenCalledWith({
        sourceName: "my-handler",
        newName: "my-handler-fork",
      });
    });

    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith('Handler forked to "my-handler-fork"');
    });

    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  // 5. HandlerExists error renders inline, dialog stays open
  it("shows inline error banner on HandlerExists and keeps dialog open", async () => {
    const { ipc } = await import("@/ipc/client");
    (ipc.handler_fork as ReturnType<typeof vi.fn>).mockRejectedValue({
      kind: "handler_exists",
      message: { name: "my-handler-fork" },
    });

    const onOpenChange = vi.fn();
    render(
      wrap(
        <ForkHandlerDialog
          open={true}
          onOpenChange={onOpenChange}
          sourceName="my-handler"
        />,
      ),
    );

    const forkBtn = screen.getByRole("button", { name: /^Fork$/i });
    fireEvent.click(forkBtn);

    await waitFor(() => {
      expect(
        screen.getByText(/\[handler_exists\].*my-handler-fork.*already exists/i),
      ).toBeInTheDocument();
    });

    // Dialog should NOT have been closed
    expect(onOpenChange).not.toHaveBeenCalledWith(false);
  });
});
