import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter, Route, Routes, useLocation } from "react-router-dom";
import { ScaffoldDialog } from "@/components/ScaffoldDialog";
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
    handler_scaffold: vi.fn(),
  },
}));

// ── Helpers ────────────────────────────────────────────────────────────────

function LocationDisplay() {
  const loc = useLocation();
  return <div data-testid="location">{loc.pathname}</div>;
}

function wrap(node: React.ReactNode) {
  const qc = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });
  return (
    <QueryClientProvider client={qc}>
      <MemoryRouter initialEntries={["/handlers"]}>
        <Routes>
          <Route path="/handlers" element={node} />
          <Route path="/handlers/:name" element={<LocationDisplay />} />
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

describe("ScaffoldDialog", () => {
  // 1. Renders all fields
  it("renders name input, 3 template radios, and primary field input", () => {
    render(wrap(<ScaffoldDialog open={true} onOpenChange={noop} />));

    expect(screen.getByLabelText(/^Name$/i)).toBeInTheDocument();
    expect(screen.getByLabelText("Go (row mode)")).toBeInTheDocument();
    expect(screen.getByLabelText("Go (batch mode)")).toBeInTheDocument();
    expect(screen.getByLabelText("Empty")).toBeInTheDocument();
    expect(screen.getByLabelText(/^Primary field$/i)).toBeInTheDocument();
  });

  // 2. Create disabled when name is empty
  it("disables the Create button when name is empty", () => {
    render(wrap(<ScaffoldDialog open={true} onOpenChange={noop} />));
    const createBtn = screen.getByRole("button", { name: /^Create$/i });
    expect(createBtn).toBeDisabled();
  });

  // 3. Create disabled with invalid name
  it("disables the Create button when name contains uppercase letters", () => {
    render(wrap(<ScaffoldDialog open={true} onOpenChange={noop} />));
    const nameInput = screen.getByLabelText(/^Name$/i);
    fireEvent.change(nameInput, { target: { value: "BadName" } });
    const createBtn = screen.getByRole("button", { name: /^Create$/i });
    expect(createBtn).toBeDisabled();
  });

  // 4. Shows inline name error for invalid name
  it("shows inline error when name is invalid", () => {
    render(wrap(<ScaffoldDialog open={true} onOpenChange={noop} />));
    const nameInput = screen.getByLabelText(/^Name$/i);
    fireEvent.change(nameInput, { target: { value: "BadName" } });
    expect(
      screen.getByText(/Lowercase letters, numbers, and hyphens/i)
    ).toBeInTheDocument();
  });

  // 5. Calls handler_scaffold with correct args on Create
  it("calls ipc.handler_scaffold with correct args when form is submitted", async () => {
    const { ipc } = await import("@/ipc/client");
    (ipc.handler_scaffold as ReturnType<typeof vi.fn>).mockResolvedValue("new-handler");

    render(wrap(<ScaffoldDialog open={true} onOpenChange={noop} />));

    const nameInput = screen.getByLabelText(/^Name$/i);
    fireEvent.change(nameInput, { target: { value: "new-handler" } });

    // Switch to go_batch
    const batchRadio = screen.getByLabelText("Go (batch mode)");
    fireEvent.click(batchRadio);

    const primaryInput = screen.getByLabelText(/^Primary field$/i);
    fireEvent.change(primaryInput, { target: { value: "bill_id" } });

    const createBtn = screen.getByRole("button", { name: /^Create$/i });
    fireEvent.click(createBtn);

    await waitFor(() => {
      expect(ipc.handler_scaffold).toHaveBeenCalledWith({
        name: "new-handler",
        template: "go_batch",
        primary_field: "bill_id",
      });
    });
  });

  // 6. Happy path: toast + close + navigate
  it("toasts success, calls onOpenChange(false), and navigates on success", async () => {
    const { ipc } = await import("@/ipc/client");
    const { toast } = await import("sonner");
    (ipc.handler_scaffold as ReturnType<typeof vi.fn>).mockResolvedValue("my-handler");

    const onOpenChange = vi.fn();
    render(wrap(<ScaffoldDialog open={true} onOpenChange={onOpenChange} />));

    const nameInput = screen.getByLabelText(/^Name$/i);
    fireEvent.change(nameInput, { target: { value: "my-handler" } });

    fireEvent.click(screen.getByRole("button", { name: /^Create$/i }));

    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith('Handler "my-handler" created');
    });

    expect(onOpenChange).toHaveBeenCalledWith(false);

    // Navigation should reach /handlers/my-handler
    expect(await screen.findByTestId("location")).toHaveTextContent(
      "/handlers/my-handler"
    );
  });

  // 7a. Error path: HandlerExists shows inline banner, dialog stays open
  it("shows inline error banner on HandlerExists and keeps dialog open", async () => {
    const { ipc } = await import("@/ipc/client");
    (ipc.handler_scaffold as ReturnType<typeof vi.fn>).mockRejectedValue({
      kind: "handler_exists",
      message: { name: "my-handler" },
    });

    const onOpenChange = vi.fn();
    render(wrap(<ScaffoldDialog open={true} onOpenChange={onOpenChange} />));

    const nameInput = screen.getByLabelText(/^Name$/i);
    fireEvent.change(nameInput, { target: { value: "my-handler" } });

    fireEvent.click(screen.getByRole("button", { name: /^Create$/i }));

    await waitFor(() => {
      expect(
        screen.getByText(/\[handler_exists\].*my-handler.*already exists/i)
      ).toBeInTheDocument();
    });

    // Dialog should NOT have been closed
    expect(onOpenChange).not.toHaveBeenCalledWith(false);
  });

  // 7b. Error path: InvalidHandlerName shows inline banner
  it("shows inline error banner on InvalidHandlerName", async () => {
    const { ipc } = await import("@/ipc/client");
    (ipc.handler_scaffold as ReturnType<typeof vi.fn>).mockRejectedValue({
      kind: "invalid_handler_name",
      message: { name: "bad_name" },
    });

    render(wrap(<ScaffoldDialog open={true} onOpenChange={noop} />));

    // Use a technically valid-looking client-side name so the button is enabled,
    // but the server rejects it as invalid
    const nameInput = screen.getByLabelText(/^Name$/i);
    fireEvent.change(nameInput, { target: { value: "bad" } });

    fireEvent.click(screen.getByRole("button", { name: /^Create$/i }));

    await waitFor(() => {
      expect(
        screen.getByText(/\[invalid_handler_name\]/i)
      ).toBeInTheDocument();
    });
  });
});
