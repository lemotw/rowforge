import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { ExecDetailPage } from "@/pages/ExecDetail";

describe("ExecDetail", () => {
  let qc: QueryClient;

  beforeEach(() => {
    vi.clearAllMocks();
    qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  });

  function wrap(initialEntry: string) {
    return (
      <QueryClientProvider client={qc}>
        <MemoryRouter initialEntries={[initialEntry]}>
          <Routes>
            <Route path="/exec/:id" element={<ExecDetailPage />} />
          </Routes>
        </MemoryRouter>
      </QueryClientProvider>
    );
  }

  it("renders attempts table when exec has attempts", async () => {
    (invoke as any).mockImplementation((cmd: string) => {
      if (cmd === "workspace_current")
        return Promise.resolve({ root: "/tmp/ws", schema_version: 2 });
      if (cmd === "exec_show")
        return Promise.resolve({
          summary: {
            id: "e1",
            name: "smoke",
            created_at: "2026-05-24T12:00:00Z",
            input_rows: 5,
            attempts_count: 1,
            last_attempt_state: "done",
            last_attempt_counts: null,
          },
          input_path_snapshot: "/tmp/in.csv",
          input_format: "csv",
          handler_binding: { handler_id: null, handler_instance_id: null, version: null },
          attempts: [
            {
              id: "a1",
              state: "done",
              started_at: "2026-05-24T12:00:00Z",
              finished_at: "2026-05-24T12:00:05Z",
              run_type: "full",
              stats: null,
            },
          ],
          field_mapping: null,
          config_overrides: {},
        });
      throw new Error("unexpected " + cmd);
    });

    render(wrap("/exec/e1"));
    expect(await screen.findByRole("heading", { name: /smoke/i })).toBeInTheDocument();
    expect(await screen.findByText(/open ⏵/)).toBeInTheDocument();
  });

  it("renders empty-state when exec has no attempts", async () => {
    (invoke as any).mockImplementation((cmd: string) => {
      if (cmd === "workspace_current")
        return Promise.resolve({ root: "/tmp/ws", schema_version: 2 });
      if (cmd === "exec_show")
        return Promise.resolve({
          summary: {
            id: "e1",
            name: "empty",
            created_at: "2026-05-24T12:00:00Z",
            input_rows: 0,
            attempts_count: 0,
            last_attempt_state: null,
            last_attempt_counts: null,
          },
          input_path_snapshot: "/tmp/in.csv",
          input_format: "csv",
          handler_binding: { handler_id: null, handler_instance_id: null, version: null },
          attempts: [],
          field_mapping: null,
          config_overrides: {},
        });
      throw new Error("unexpected " + cmd);
    });

    render(wrap("/exec/e1"));
    expect(await screen.findByText(/never been run/i)).toBeInTheDocument();
  });
});
