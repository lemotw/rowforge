import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { DeleteExecutionsDialog } from "@/components/DeleteExecutionsDialog";
import type { ExecSummary } from "@/ipc/types";

// ── Helpers ────────────────────────────────────────────────────────────────

function makeExec(overrides: Partial<ExecSummary> & { id: string }): ExecSummary {
  return {
    name: overrides.id,
    created_at: "2026-01-01T00:00:00Z",
    input_rows: null,
    attempts_count: 0,
    last_attempt_state: null,
    last_attempt_counts: null,
    last_handler_dir: null,
    size_bytes: null,
    ...overrides,
  };
}

const noop = () => {};

beforeEach(() => {
  vi.clearAllMocks();
});

// ── Tests ──────────────────────────────────────────────────────────────────

describe("DeleteExecutionsDialog", () => {
  // 1. Title shows correct selection count
  it("renders title with selection count (plural)", () => {
    const selected = [makeExec({ id: "e1" }), makeExec({ id: "e2" })];
    render(
      <DeleteExecutionsDialog
        open={true}
        onOpenChange={noop}
        selected={selected}
        onConfirm={noop}
        isPending={false}
      />,
    );
    expect(screen.getByText(/Delete 2 executions\?/i)).toBeInTheDocument();
  });

  it("renders title with selection count (singular)", () => {
    const selected = [makeExec({ id: "e1" })];
    render(
      <DeleteExecutionsDialog
        open={true}
        onOpenChange={noop}
        selected={selected}
        onConfirm={noop}
        isPending={false}
      />,
    );
    expect(screen.getByText(/Delete 1 execution\?/i)).toBeInTheDocument();
  });

  // 2. Total size renders correctly
  it("renders formatted total size in the description", () => {
    const selected = [
      makeExec({ id: "e1", size_bytes: 1024 }),
      makeExec({ id: "e2", size_bytes: 2048 }),
    ];
    render(
      <DeleteExecutionsDialog
        open={true}
        onOpenChange={noop}
        selected={selected}
        onConfirm={noop}
        isPending={false}
      />,
    );
    // 3072 bytes = 3.0 KB
    expect(screen.getByText(/3\.0 KB/)).toBeInTheDocument();
  });

  // 3. "… and N more" truncation when > MAX_LISTED (10) selected
  it('shows "… and N more" when more than 10 executions are selected', () => {
    const selected = Array.from({ length: 13 }, (_, i) =>
      makeExec({ id: `exec-${i}`, size_bytes: 100 }),
    );
    render(
      <DeleteExecutionsDialog
        open={true}
        onOpenChange={noop}
        selected={selected}
        onConfirm={noop}
        isPending={false}
      />,
    );
    expect(screen.getByText(/… and 3 more/i)).toBeInTheDocument();
    // Only first 10 should appear by name
    expect(screen.getByText("exec-0")).toBeInTheDocument();
    expect(screen.queryByText("exec-10")).not.toBeInTheDocument();
  });

  // 4. Delete button is disabled when isPending
  it("disables the Delete button when isPending is true", () => {
    const selected = [makeExec({ id: "e1" })];
    render(
      <DeleteExecutionsDialog
        open={true}
        onOpenChange={noop}
        selected={selected}
        onConfirm={noop}
        isPending={true}
      />,
    );
    const deleteBtn = screen.getByRole("button", { name: /Deleting…/i });
    expect(deleteBtn).toBeDisabled();
  });

  // 5. onConfirm is called when Delete button is clicked
  it("calls onConfirm when the Delete button is clicked", () => {
    const onConfirm = vi.fn();
    const selected = [makeExec({ id: "e1" }), makeExec({ id: "e2" })];
    render(
      <DeleteExecutionsDialog
        open={true}
        onOpenChange={noop}
        selected={selected}
        onConfirm={onConfirm}
        isPending={false}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /Delete 2 executions/i }));
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });
});
