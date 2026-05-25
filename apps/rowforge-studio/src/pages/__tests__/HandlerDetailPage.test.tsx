import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { HandlerDetailPage } from "@/pages/HandlerDetailPage";
import type { HandlerDetail } from "@/ipc/types";
import type React from "react";

describe("HandlerDetailPage", () => {
  let qc: QueryClient;

  beforeEach(() => {
    vi.clearAllMocks();
    qc = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
  });

  function wrap(node: React.ReactNode, handlerName = "alpha") {
    return (
      <QueryClientProvider client={qc}>
        <MemoryRouter initialEntries={[`/handlers/${handlerName}`]}>
          <Routes>
            <Route path="/handlers/:name" element={node} />
            <Route path="/handlers" element={<div data-testid="handlers-list">Handlers List</div>} />
          </Routes>
        </MemoryRouter>
      </QueryClientProvider>
    );
  }

  function mockInvoke(detail: HandlerDetail | null, rejectWith?: unknown, buildReject?: unknown) {
    (invoke as any).mockImplementation((cmd: string) => {
      if (cmd === "workspace_current")
        return Promise.resolve({ root: "/tmp/ws", schema_version: 2 });
      if (cmd === "handler_show") {
        if (rejectWith !== undefined) return Promise.reject(rejectWith);
        return Promise.resolve(detail);
      }
      if (cmd === "handler_build") {
        if (buildReject !== undefined) return Promise.reject(buildReject);
        return Promise.resolve({
          started_at: "2026-05-25T10:00:00Z",
          finished_at: "2026-05-25T10:00:01Z",
          exit_code: 0,
          command: ["sh", "-c", "go build ."],
          stdout: "ok\n",
          stderr: "",
        });
      }
      throw new Error("unexpected invoke: " + cmd);
    });
  }

  const validDetail: HandlerDetail = {
    summary: {
      name: "alpha",
      path: "/tmp/ws/handlers/alpha",
      manifest_status: "valid",
      last_modified: new Date(Date.now() - 3600_000).toISOString(),
      version: "1.2.0",
      language: "go",
    },
    manifest: {
      // rowforge-core manifest fields (defensive any in component)
      kind: "row",
      primary_field: "order_id",
      entry: { cmd: ["./alpha", "--serve"] },
      batch_size: null,
      row_timeout: "30s",
      // exec-side fields also present in the TS Manifest type
      name: "alpha",
      version: "1.2.0",
      language: "go",
      entry_cmd: ["./alpha", "--serve"],
      entry_build: null,
    } as any,
    manifest_errors: [],
    manifest_warnings: [],
    source_files: [
      { name: "main.go", size_bytes: 4096, is_directory: false },
      { name: "fixtures", size_bytes: 0, is_directory: true },
    ],
    has_fixtures_dir: true,
    last_build: null,
  };

  const invalidDetail: HandlerDetail = {
    summary: {
      name: "broken",
      path: "/tmp/ws/handlers/broken",
      manifest_status: "invalid",
      last_modified: new Date().toISOString(),
      version: null,
      language: null,
    },
    manifest: null,
    manifest_errors: [
      { kind: "parse_failed", message: "unexpected field 'typ'" },
      { kind: "parse_failed", message: "missing required field 'kind'" },
    ],
    manifest_warnings: [],
    source_files: [
      { name: "handler.go", size_bytes: 200, is_directory: false },
    ],
    has_fixtures_dir: false,
    last_build: null,
  };

  const missingDetail: HandlerDetail = {
    summary: {
      name: "bare",
      path: "/tmp/ws/handlers/bare",
      manifest_status: "missing",
      last_modified: new Date().toISOString(),
      version: null,
      language: null,
    },
    manifest: null,
    manifest_errors: [],
    manifest_warnings: [],
    source_files: [
      { name: "main.go", size_bytes: 8192, is_directory: false },
    ],
    has_fixtures_dir: false,
    last_build: null,
  };

  // ── 1. Loading state ────────────────────────────────────────────────────────

  it("renders loading state while handler_show is pending", () => {
    (invoke as any).mockImplementation((cmd: string) => {
      if (cmd === "workspace_current")
        return Promise.resolve({ root: "/tmp/ws", schema_version: 2 });
      if (cmd === "handler_show") return new Promise(() => {});
      throw new Error("unexpected invoke: " + cmd);
    });
    render(wrap(<HandlerDetailPage />, "alpha"));
    expect(screen.getByText(/Loading handler/i)).toBeInTheDocument();
  });

  // ── 2. Valid manifest ───────────────────────────────────────────────────────

  it("renders handler name, path, and valid manifest fields", async () => {
    mockInvoke(validDetail);
    render(wrap(<HandlerDetailPage />, "alpha"));

    // Handler name in header (appears multiple times — h1 and manifest name row)
    const alphaEls = await screen.findAllByText("alpha");
    expect(alphaEls.length).toBeGreaterThanOrEqual(1);
    // Path below the name
    expect(await screen.findByText("/tmp/ws/handlers/alpha")).toBeInTheDocument();

    // Status badge
    expect(await screen.findByText("valid")).toBeInTheDocument();

    // Manifest key-value rows
    expect(await screen.findByText("kind")).toBeInTheDocument();
    expect(await screen.findByText("row")).toBeInTheDocument();
    expect(await screen.findByText("primary_field")).toBeInTheDocument();
    expect(await screen.findByText("order_id")).toBeInTheDocument();
    expect(await screen.findByText("entry.cmd")).toBeInTheDocument();
    expect(await screen.findByText("./alpha --serve")).toBeInTheDocument();
    expect(await screen.findByText("row_timeout")).toBeInTheDocument();
    expect(await screen.findByText("30s")).toBeInTheDocument();
  });

  it("renders entry_cmd array joined as string when entry.cmd is absent", async () => {
    const detail: HandlerDetail = {
      ...validDetail,
      manifest: {
        name: "alpha",
        version: "1.0",
        language: "go",
        entry_cmd: ["./runner", "--port", "9000"],
        entry_build: null,
      },
    };
    mockInvoke(detail);
    render(wrap(<HandlerDetailPage />, "alpha"));
    expect(await screen.findByText("entry_cmd")).toBeInTheDocument();
    expect(await screen.findByText("./runner --port 9000")).toBeInTheDocument();
  });

  // ── 3. Invalid manifest ─────────────────────────────────────────────────────

  it("renders invalid status badge and error list", async () => {
    mockInvoke(invalidDetail);
    render(wrap(<HandlerDetailPage />, "broken"));

    expect(await screen.findByText("invalid")).toBeInTheDocument();
    expect(await screen.findByText("Errors")).toBeInTheDocument();
    expect(
      await screen.findByText("unexpected field 'typ'")
    ).toBeInTheDocument();
    expect(
      await screen.findByText("missing required field 'kind'")
    ).toBeInTheDocument();
  });

  // ── 4. Missing manifest ─────────────────────────────────────────────────────

  it("renders missing status badge and no-yaml message", async () => {
    mockInvoke(missingDetail);
    render(wrap(<HandlerDetailPage />, "bare"));

    expect(await screen.findByText("missing")).toBeInTheDocument();
    expect(
      await screen.findByText(/No rowforge\.yaml in this handler directory/i)
    ).toBeInTheDocument();
  });

  // ── 5. HandlerNotFound error ────────────────────────────────────────────────

  it("renders not-found copy with back link when UiError kind is handler_not_found", async () => {
    mockInvoke(null, { kind: "handler_not_found", message: { name: "alpha" } });
    render(wrap(<HandlerDetailPage />, "alpha"));

    expect(
      await screen.findByText(/may have been deleted or renamed/i)
    ).toBeInTheDocument();
    const backLink = await screen.findByRole("link", { name: /back to handlers/i });
    expect(backLink).toBeInTheDocument();
    expect(backLink).toHaveAttribute("href", "/handlers");
  });

  it("renders generic error message for non-not-found errors", async () => {
    mockInvoke(null, { kind: "io", message: "permission denied" });
    render(wrap(<HandlerDetailPage />, "alpha"));

    expect(
      await screen.findByText(/Failed to load handler/i)
    ).toBeInTheDocument();
    expect(
      await screen.findByText(/\[io\] permission denied/)
    ).toBeInTheDocument();
  });

  // ── 6. Source files ─────────────────────────────────────────────────────────

  it("renders source files with formatted byte sizes and directory markers", async () => {
    mockInvoke(validDetail);
    render(wrap(<HandlerDetailPage />, "alpha"));

    // main.go: 4096 bytes → 4.0 KB
    expect(await screen.findByText("main.go")).toBeInTheDocument();
    expect(await screen.findByText("4.0 KB")).toBeInTheDocument();

    // rowforge.yaml is excluded from source_files (it appears in the Manifest
    // section instead) — assert it does NOT appear in the Files list.
    await screen.findByText("main.go"); // wait for render
    expect(screen.queryByText("rowforge.yaml")).not.toBeInTheDocument();

    // fixtures directory: shown with trailing slash, size is —
    expect(await screen.findByText("fixtures/")).toBeInTheDocument();

    // fixtures/ directory hint
    expect(
      await screen.findByText(/fixtures\/ directory present/i)
    ).toBeInTheDocument();
  });

  it("renders file count in section title", async () => {
    mockInvoke(validDetail);
    render(wrap(<HandlerDetailPage />, "alpha"));

    // 2 source files (rowforge.yaml is in the Manifest section, not here)
    expect(await screen.findByText(/Files \(2\)/i)).toBeInTheDocument();
  });

  it("renders warnings list when manifest_warnings is non-empty", async () => {
    const detailWithWarnings: HandlerDetail = {
      ...validDetail,
      manifest_warnings: [
        { kind: "path_lookup_failed", field: "fixtures", token: "jq" },
      ],
    };
    mockInvoke(detailWithWarnings);
    render(wrap(<HandlerDetailPage />, "alpha"));

    expect(await screen.findByText("Warnings")).toBeInTheDocument();
  });

  // ── 7. Build button ─────────────────────────────────────────────────────────

  const buildableDetail: HandlerDetail = {
    ...validDetail,
    manifest: {
      ...validDetail.manifest,
      entry: { cmd: ["./alpha", "--serve"], build: ["sh", "-c", "go build ."] },
    } as any,
  };

  it("renders Build button when manifest has entry.build", async () => {
    mockInvoke(buildableDetail);
    render(wrap(<HandlerDetailPage />, "alpha"));

    expect(await screen.findByRole("button", { name: /^Build$/ })).toBeInTheDocument();
  });

  it("hides Build button when manifest has no entry.build", async () => {
    mockInvoke(validDetail);
    render(wrap(<HandlerDetailPage />, "alpha"));

    // Wait for page to fully render (path is unique on the page)
    await screen.findByText("/tmp/ws/handlers/alpha");
    expect(screen.queryByRole("button", { name: /^Build$/ })).toBeNull();
  });

  it("clicking Build invokes handler_build with handler name", async () => {
    mockInvoke(buildableDetail);
    render(wrap(<HandlerDetailPage />, "alpha"));

    const buildBtn = await screen.findByRole("button", { name: /^Build$/ });
    fireEvent.click(buildBtn);

    // Wait a tick for the mutation to fire
    await vi.waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("handler_build", { name: "alpha" });
    });
  });
});
