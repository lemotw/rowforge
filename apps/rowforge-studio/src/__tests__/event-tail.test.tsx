import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { EventTail } from "@/components/EventTail";
import type { OutcomeSampleEntry } from "@/ipc/run-state";

// useVirtualizer relies on DOM layout measurements unavailable in jsdom.
// Mock it to return all items directly so filter logic is testable.
vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: ({ count, estimateSize }: { count: number; estimateSize: () => number }) => ({
    getVirtualItems: () =>
      Array.from({ length: count }, (_, i) => ({
        key: i,
        index: i,
        start: i * estimateSize(),
        size: estimateSize(),
      })),
    getTotalSize: () => count * estimateSize(),
  }),
}));

const samples: OutcomeSampleEntry[] = [
  { row_index: 1, kind: "error", code: "X", message: "boom", dur_ms: 10, arrived_at_ms: 0 },
  { row_index: 2, kind: "crash", code: null, message: "died", dur_ms: 20, arrived_at_ms: 0 },
  { row_index: 3, kind: "too_large", code: "TOO_LARGE", message: "huge", dur_ms: 5, arrived_at_ms: 0 },
];

describe("EventTail", () => {
  it("default filter is errors-only — shows error + crash + too_large", () => {
    render(<EventTail samples={samples} />);
    expect(screen.getByText("row 1")).toBeInTheDocument();
    expect(screen.getByText("row 2")).toBeInTheDocument();
    expect(screen.getByText("row 3")).toBeInTheDocument();
  });

  it("crashes filter shows only crash kind", () => {
    render(<EventTail samples={samples} />);
    fireEvent.click(screen.getByText("Crashes"));
    expect(screen.queryByText("row 1")).not.toBeInTheDocument();
    expect(screen.getByText("row 2")).toBeInTheDocument();
    expect(screen.queryByText("row 3")).not.toBeInTheDocument();
  });

  it("renders empty state when no samples", () => {
    render(<EventTail samples={[]} />);
    expect(screen.getByText(/no events yet/i)).toBeInTheDocument();
  });
});
