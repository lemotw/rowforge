import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { ExecListPage } from "@/pages/ExecList";
import type React from "react";

// Helper: produce an ExecSummary fixture
function makeExec(overrides: Partial<{
  id: string;
  name: string;
  input_rows: number | null;
  attempts_count: number;
  last_attempt_state: string | null;
  size_bytes: number | null;
}> = {}) {
  return {
    id: overrides.id ?? "e1",
    name: overrides.name ?? "smoke",
    created_at: "2026-05-24T12:00:00Z",
    input_rows: overrides.input_rows ?? 5,
    attempts_count: overrides.attempts_count ?? 0,
    last_attempt_state: overrides.last_attempt_state ?? null,
    last_attempt_counts: null,
    last_handler_dir: null,
    size_bytes: overrides.size_bytes ?? null,
  };
}

describe("ExecList — Select mode", () => {
  let qc: QueryClient;

  beforeEach(() => {
    vi.clearAllMocks();
    qc = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
  });

  function wrap(node: React.ReactNode) {
    return (
      <QueryClientProvider client={qc}>
        <MemoryRouter>{node}</MemoryRouter>
      </QueryClientProvider>
    );
  }

  function mockInvoke(execs: unknown[]) {
    (invoke as any).mockImplementation((cmd: string) => {
      if (cmd === "workspace_current")
        return Promise.resolve({ root: "/tmp/ws", schema_version: 2 });
      if (cmd === "exec_list") return Promise.resolve(execs);
      if (cmd === "execution_delete_bulk")
        return Promise.resolve({ deleted: [], failed: [] });
      throw new Error("unexpected invoke: " + cmd);
    });
  }

  // Test 1: Select toggle reveals checkbox column
  it("clicking Select reveals a checkbox column", async () => {
    mockInvoke([makeExec({ id: "e1", name: "alpha" })]);
    render(wrap(<ExecListPage />));

    // Wait for table to appear
    await screen.findByText("alpha");

    // No checkboxes yet
    expect(screen.queryAllByRole("checkbox")).toHaveLength(0);

    // Click Select button
    fireEvent.click(screen.getByRole("button", { name: /^Select$/i }));

    // Checkbox column should appear: 1 master + 1 row = 2 total
    await waitFor(() => {
      expect(screen.getAllByRole("checkbox")).toHaveLength(2);
    });
  });

  // Test 2: Active-run row checkbox is disabled with tooltip
  it("active-run row has a disabled checkbox with tooltip", async () => {
    mockInvoke([
      makeExec({ id: "e-active", name: "running-exec", last_attempt_state: "running" }),
      makeExec({ id: "e-idle", name: "idle-exec", last_attempt_state: null }),
    ]);
    render(wrap(<ExecListPage />));

    await screen.findByText("running-exec");

    // Enter select mode
    fireEvent.click(screen.getByRole("button", { name: /^Select$/i }));

    await waitFor(() => {
      // 1 master + 2 row checkboxes = 3 total
      const checkboxes = screen.getAllByRole("checkbox");
      expect(checkboxes.length).toBe(3);
    });

    const checkboxes = screen.getAllByRole("checkbox");
    // Master is rendered first (header row). Skip it; row checkboxes follow.
    const runningCheckbox = checkboxes[1] as HTMLButtonElement;
    const idleCheckbox = checkboxes[2] as HTMLButtonElement;

    expect(runningCheckbox.disabled).toBe(true);
    expect(runningCheckbox.title).toBe("Cancel active run first");
    expect(idleCheckbox.disabled).toBe(false);
  });

  // Test 3: Delete N button is disabled when nothing selected
  it("Delete button is disabled when no executions are selected", async () => {
    mockInvoke([makeExec({ id: "e1", name: "alpha" })]);
    render(wrap(<ExecListPage />));

    await screen.findByText("alpha");

    fireEvent.click(screen.getByRole("button", { name: /^Select$/i }));

    // The delete button should be present but disabled
    await waitFor(() => {
      const deleteBtn = screen.getByRole("button", { name: /Delete 0 execution/i });
      expect(deleteBtn).toBeDisabled();
    });
  });

  // Test 4: Clicking Delete N (with selection) opens the dialog
  it("clicking Delete N opens confirmation dialog when executions are selected", async () => {
    mockInvoke([makeExec({ id: "e1", name: "alpha" })]);
    render(wrap(<ExecListPage />));

    await screen.findByText("alpha");

    // Enter select mode
    fireEvent.click(screen.getByRole("button", { name: /^Select$/i }));

    // Select the row (click the row checkbox, not the master)
    await waitFor(() => {
      expect(screen.getAllByRole("checkbox")).toHaveLength(2);
    });

    const rowCheckbox = screen.getAllByRole("checkbox")[1];
    fireEvent.click(rowCheckbox);

    // Delete button should now show count 1 and be enabled
    await waitFor(() => {
      const deleteBtn = screen.getByRole("button", { name: /Delete 1 execution/i });
      expect(deleteBtn).not.toBeDisabled();
      fireEvent.click(deleteBtn);
    });

    // Dialog should open
    await waitFor(() => {
      expect(screen.getByRole("dialog")).toBeInTheDocument();
    });
  });

  // Test 5: Size column renders formatted bytes
  it("Size column renders formatted bytes", async () => {
    // 5_242_880 bytes = 5.0 MB
    mockInvoke([makeExec({ id: "e1", name: "big-exec", size_bytes: 5_242_880 })]);
    render(wrap(<ExecListPage />));

    await screen.findByText("big-exec");
    expect(screen.getByText("5.0 MB")).toBeInTheDocument();
  });

  // Test 6: Master checkbox selects all non-active rows
  it("master checkbox toggles all selectable rows; active rows stay unchecked", async () => {
    mockInvoke([
      makeExec({ id: "e-active", name: "running-exec", last_attempt_state: "running" }),
      makeExec({ id: "e1", name: "idle-1" }),
      makeExec({ id: "e2", name: "idle-2" }),
    ]);
    render(wrap(<ExecListPage />));

    await screen.findByText("idle-1");
    fireEvent.click(screen.getByRole("button", { name: /^Select$/i }));

    await waitFor(() => {
      // 1 master + 3 rows = 4
      expect(screen.getAllByRole("checkbox")).toHaveLength(4);
    });

    // Click master → should select e1 + e2 but NOT e-active
    const master = screen.getAllByRole("checkbox")[0];
    fireEvent.click(master);

    await waitFor(() => {
      const deleteBtn = screen.getByRole("button", { name: /Delete 2 executions/i });
      expect(deleteBtn).not.toBeDisabled();
    });

    // Click master again → deselects all
    fireEvent.click(master);
    await waitFor(() => {
      const deleteBtn = screen.getByRole("button", { name: /Delete 0 execution/i });
      expect(deleteBtn).toBeDisabled();
    });
  });

  // Test 7: Name cell has title attribute equal to exec_id
  it("Name cell has title equal to exec_id for hover tooltip", async () => {
    mockInvoke([makeExec({ id: "exec-id-full-string", name: "my-exec" })]);
    render(wrap(<ExecListPage />));

    await screen.findByText("my-exec");

    // The <td> for the name should have title="exec-id-full-string"
    const nameCell = screen.getByTitle("exec-id-full-string");
    expect(nameCell).toBeInTheDocument();
    expect(nameCell.textContent).toBe("my-exec");
  });
});
