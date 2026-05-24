import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { ExecListPage } from "@/pages/ExecList";
import type React from "react";

describe("ExecList", () => {
  let qc: QueryClient;

  beforeEach(() => {
    vi.clearAllMocks();
    qc = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
  });

  function wrap(node: React.ReactNode) {
    return <QueryClientProvider client={qc}>{node}</QueryClientProvider>;
  }

  it("renders empty state when list is []", async () => {
    (invoke as any).mockImplementation((cmd: string) => {
      if (cmd === "workspace_settings_load")
        return Promise.resolve({
          workspace_root: "/tmp/ws",
          schema_version: 1,
          default_workers: null,
          max_concurrent_runs: null,
          telemetry_opt_in: false,
        });
      if (cmd === "exec_list") return Promise.resolve([]);
      throw new Error("unexpected invoke: " + cmd);
    });
    render(wrap(<ExecListPage />));
    expect(await screen.findByText(/No executions yet/i)).toBeInTheDocument();
  });

  it("renders rows from invoke result", async () => {
    (invoke as any).mockImplementation((cmd: string) => {
      if (cmd === "workspace_settings_load")
        return Promise.resolve({
          workspace_root: "/tmp/ws",
          schema_version: 1,
          default_workers: null,
          max_concurrent_runs: null,
          telemetry_opt_in: false,
        });
      if (cmd === "exec_list")
        return Promise.resolve([
          {
            id: "e1",
            name: "smoke",
            created_at: "2026-05-24T12:00:00Z",
            input_rows: 5,
            attempts_count: 0,
            last_attempt_state: null,
            last_attempt_counts: null,
          },
        ]);
      throw new Error("unexpected invoke: " + cmd);
    });
    render(wrap(<ExecListPage />));
    expect(await screen.findByText("smoke")).toBeInTheDocument();
  });
});
