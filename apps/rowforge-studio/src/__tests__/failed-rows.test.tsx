import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { FailedRowsTable } from "@/components/FailedRowsTable";
import type React from "react";

vi.mock("@tauri-apps/plugin-opener", () => ({ openPath: vi.fn(), openUrl: vi.fn() }));

describe("FailedRowsTable", () => {
  let qc: QueryClient;
  beforeEach(() => {
    vi.clearAllMocks();
    qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  });

  function wrap(node: React.ReactNode) {
    return <QueryClientProvider client={qc}>{node}</QueryClientProvider>;
  }

  it("renders failed rows and expands raw_record on click", async () => {
    (invoke as any).mockImplementation((cmd: string) => {
      if (cmd === "attempt_failed_page")
        return Promise.resolve({
          rows: [
            {
              seq: 1,
              kind: "error",
              error_code: "BILLING_NOT_FOUND",
              message: "no billing row for billid",
              raw_record: { billid: "b0042" },
              dur_ms: 12,
            },
          ],
          next_offset: null,
          total_known: null,
        });
      throw new Error("unexpected " + cmd);
    });

    render(wrap(<FailedRowsTable executionId="e1" attemptId="a1" pathsOutcomes="/tmp/out.jsonl" />));
    expect(await screen.findByText("BILLING_NOT_FOUND")).toBeInTheDocument();
    fireEvent.click(screen.getByText("raw"));
    expect(await screen.findByText(/"billid": "b0042"/i)).toBeInTheDocument();
  });
});
