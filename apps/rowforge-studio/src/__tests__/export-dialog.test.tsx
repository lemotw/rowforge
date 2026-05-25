import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ExportDialog } from "@/components/ExportDialog";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openPath: vi.fn(), openUrl: vi.fn() }));
vi.mock("sonner", () => ({
  toast: {
    loading: vi.fn().mockReturnValue("toast-1"),
    success: vi.fn(),
    error: vi.fn(),
    dismiss: vi.fn(),
  },
}));
vi.mock("@/ipc/client", () => ({
  ipc: {
    exec_export: vi.fn().mockResolvedValue({
      output_dir: "/tmp/exports/x",
      written_files: ["/tmp/exports/x/success.csv", "/tmp/exports/x/failed.csv"],
      success_count: 100,
      failed_count: 5,
      warnings: [],
    }),
  },
}));

function renderDialog() {
  const qc = new QueryClient({ defaultOptions: { mutations: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <ExportDialog open execId={"e_01TEST"} onClose={() => {}} />
    </QueryClientProvider>
  );
}

beforeEach(() => { vi.clearAllMocks(); });

describe("ExportDialog", () => {
  it("renders format radio with csv as default", () => {
    renderDialog();
    const csv = screen.getByLabelText(/csv/i) as HTMLInputElement;
    expect(csv.checked).toBe(true);
  });

  it("submits exec_export with selected format and require_complete", async () => {
    renderDialog();
    fireEvent.click(screen.getByLabelText(/jsonl/i));
    fireEvent.click(screen.getByLabelText(/require complete/i));
    fireEvent.click(screen.getByRole("button", { name: /^export$/i }));
    await waitFor(async () => {
      const { ipc } = await import("@/ipc/client");
      expect(ipc.exec_export).toHaveBeenCalledWith(
        "e_01TEST",
        expect.objectContaining({ format: "jsonl", require_complete: true }),
      );
    });
  });
});
