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
    manifest_validate: vi.fn().mockResolvedValue({
      manifest: { name: "h", version: "1.0", language: "go", build: null, run: "bin/handler" },
      errors: [],
      warnings: [],
    }),
    run_start: vi.fn(),
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
  it("step 1 → Next is disabled without name and input", () => {
    renderWizard();
    const next = screen.getByRole("button", { name: /next/i });
    expect((next as HTMLButtonElement).disabled).toBe(true);
  });

  it("renders step 1 fields", () => {
    renderWizard();
    expect(screen.getByLabelText(/name/i)).toBeTruthy();
    expect(screen.getByText(/input file/i)).toBeTruthy();
  });
});
