import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import { NewExecutionWizardPage } from "@/pages/NewExecutionWizard";

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
}));
vi.mock("@/ipc/client", () => ({
  ipc: {
    exec_start: vi.fn().mockResolvedValue("e_01TEST"),
    workspace_current: vi.fn().mockResolvedValue(null),
  },
}));

function renderWizard() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <MemoryRouter>
      <QueryClientProvider client={qc}>
        <NewExecutionWizardPage />
      </QueryClientProvider>
    </MemoryRouter>
  );
}

beforeEach(() => { vi.clearAllMocks(); });

describe("NewExecutionWizard", () => {
  it("Create button is disabled without name and input", () => {
    renderWizard();
    const create = screen.getByRole("button", { name: /create execution/i });
    expect((create as HTMLButtonElement).disabled).toBe(true);
  });

  it("renders name + input file fields (no handler picker)", () => {
    renderWizard();
    expect(screen.getByLabelText(/name/i)).toBeTruthy();
    // "Input file" label (not the inline help text mentioning it)
    expect(screen.getAllByText(/input file/i).length).toBeGreaterThan(0);
    // Wizard no longer collects handler dir — handler is per-Run on ExecDetail.
    expect(screen.queryByText(/handler directory/i)).toBeNull();
    expect(screen.queryByText(/start a run immediately/i)).toBeNull();
  });
});
