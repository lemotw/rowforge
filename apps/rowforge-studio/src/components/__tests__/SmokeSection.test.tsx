import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { SmokeSection } from "@/components/SmokeSection";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import { open as dialogOpen } from "@tauri-apps/plugin-dialog";

const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;
const dialogOpenMock = dialogOpen as unknown as ReturnType<typeof vi.fn>;

function wrap(ui: React.ReactElement) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(<QueryClientProvider client={qc}>{ui}</QueryClientProvider>);
}

beforeEach(() => {
  invokeMock.mockReset();
  dialogOpenMock.mockReset();
});

describe("SmokeSection", () => {
  it("renders paste mode by default with run disabled", () => {
    wrap(<SmokeSection handlerName="alpha" defaultRows={5} />);
    expect(screen.getByRole("button", { name: /run smoke test/i })).toBeDisabled();
  });

  it("parses pasted JSON lines and enables run", () => {
    wrap(<SmokeSection handlerName="alpha" defaultRows={5} />);
    const textarea = screen.getByPlaceholderText(/email/i) as HTMLTextAreaElement;
    fireEvent.change(textarea, {
      target: { value: '{"id":"1"}\n{"id":"2"}' },
    });
    expect(screen.getByText(/2 rows parsed/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /run smoke test/i })).toBeEnabled();
  });

  it("flags invalid JSON line", () => {
    wrap(<SmokeSection handlerName="alpha" defaultRows={5} />);
    const textarea = screen.getByPlaceholderText(/email/i) as HTMLTextAreaElement;
    fireEvent.change(textarea, { target: { value: "{not json}" } });
    expect(screen.getByText(/line 1:/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /run smoke test/i })).toBeDisabled();
  });

  it("calls handler_smoke_run with the parsed rows", async () => {
    invokeMock.mockResolvedValueOnce({
      outcomes: [
        { seq: 1, status: "success", code: null, message: null, dur_ms: 5, data: { ok: true } },
      ],
      stderr_tail: "",
      exit_code: 0,
      elapsed_ms: 10,
    });
    wrap(<SmokeSection handlerName="alpha" defaultRows={5} />);
    const textarea = screen.getByPlaceholderText(/email/i) as HTMLTextAreaElement;
    fireEvent.change(textarea, { target: { value: '{"id":"1"}' } });
    fireEvent.click(screen.getByRole("button", { name: /run smoke test/i }));
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("handler_smoke_run", {
        request: {
          handler_name: "alpha",
          rows: [{ id: "1" }],
        },
      });
    });
    await waitFor(() => {
      expect(screen.getByText(/✓ 1 success/)).toBeInTheDocument();
    });
  });

  it("fixtures mode: picking a file calls handler_smoke_load_fixtures", async () => {
    dialogOpenMock.mockResolvedValueOnce("/tmp/fx.jsonl");
    invokeMock.mockResolvedValueOnce([{ id: "1" }, { id: "2" }]);
    wrap(<SmokeSection handlerName="alpha" defaultRows={5} />);
    fireEvent.click(screen.getByLabelText(/fixtures/i));
    fireEvent.click(screen.getByRole("button", { name: /pick file/i }));
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("handler_smoke_load_fixtures", {
        path: "/tmp/fx.jsonl",
        limit: 100,
      });
    });
    await waitFor(() => {
      expect(screen.getByText(/2 rows loaded/)).toBeInTheDocument();
    });
  });

  it("synthetic mode shows the row preview", () => {
    wrap(<SmokeSection handlerName="alpha" defaultRows={5} />);
    fireEvent.click(screen.getByLabelText(/one synthetic row/i));
    expect(screen.getByText(/single row/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /run smoke test/i })).toBeEnabled();
  });

  it("clamps row count input to 1..100", () => {
    wrap(<SmokeSection handlerName="alpha" defaultRows={5} />);
    const input = screen.getByRole("spinbutton") as HTMLInputElement;
    fireEvent.change(input, { target: { value: "200" } });
    expect(input.value).toBe("100");
    fireEvent.change(input, { target: { value: "0" } });
    expect(input.value).toBe("1");
  });

  it("renders handler_busy error from ipc", async () => {
    invokeMock.mockRejectedValueOnce({
      kind: "handler_busy",
      message: { name: "alpha" },
    });
    wrap(<SmokeSection handlerName="alpha" defaultRows={5} />);
    const textarea = screen.getByPlaceholderText(/email/i) as HTMLTextAreaElement;
    fireEvent.change(textarea, { target: { value: '{"id":"1"}' } });
    fireEvent.click(screen.getByRole("button", { name: /run smoke test/i }));
    await waitFor(() => {
      expect(screen.getByText(/handler|busy/i)).toBeInTheDocument();
    });
  });
});
