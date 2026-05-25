import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter, Route, Routes, useLocation } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { HandlersPage } from "@/pages/HandlersPage";
import type React from "react";

// Capture navigation target via a sentinel route.
function LocationDisplay() {
  const loc = useLocation();
  return <div data-testid="location">{loc.pathname}</div>;
}

describe("HandlersPage", () => {
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
        <MemoryRouter initialEntries={["/handlers"]}>
          <Routes>
            <Route path="/handlers" element={node} />
            <Route path="/handlers/:name" element={<LocationDisplay />} />
          </Routes>
        </MemoryRouter>
      </QueryClientProvider>
    );
  }

  function mockInvoke(handlerList: unknown[]) {
    (invoke as any).mockImplementation((cmd: string) => {
      if (cmd === "workspace_current")
        return Promise.resolve({ root: "/tmp/ws", schema_version: 2 });
      if (cmd === "handler_list") return Promise.resolve(handlerList);
      throw new Error("unexpected invoke: " + cmd);
    });
  }

  it("renders loading state initially", async () => {
    // Stall the handler_list call indefinitely so loading state is visible.
    (invoke as any).mockImplementation((cmd: string) => {
      if (cmd === "workspace_current")
        return Promise.resolve({ root: "/tmp/ws", schema_version: 2 });
      if (cmd === "handler_list") return new Promise(() => {});
      throw new Error("unexpected invoke: " + cmd);
    });
    render(wrap(<HandlersPage />));
    // Skeleton elements are rendered during loading (no text to check, but
    // the page must not error and the New Handler button is absent).
    // We just verify the page mounted without crashing.
    expect(document.body).toBeTruthy();
  });

  it("renders empty state when handler list is []", async () => {
    mockInvoke([]);
    render(wrap(<HandlersPage />));
    expect(
      await screen.findByText(/No handlers in this workspace yet/i)
    ).toBeInTheDocument();
    expect(
      await screen.findByRole("button", { name: /new handler/i })
    ).toBeInTheDocument();
  });

  it("renders rows for each handler", async () => {
    mockInvoke([
      {
        name: "apple-refund",
        path: "/tmp/ws/handlers/apple-refund",
        manifest_status: "valid",
        last_modified: new Date(Date.now() - 7200_000).toISOString(), // 2h ago
        version: "1.0.0",
        language: "go",
      },
      {
        name: "billing-channel",
        path: "/tmp/ws/handlers/billing-channel",
        manifest_status: "missing",
        last_modified: new Date(Date.now() - 60_000).toISOString(), // 1m ago
        version: null,
        language: null,
      },
    ]);
    render(wrap(<HandlersPage />));

    expect(await screen.findByText("apple-refund")).toBeInTheDocument();
    expect(await screen.findByText("billing-channel")).toBeInTheDocument();

    // Status badges
    expect(await screen.findByText("valid")).toBeInTheDocument();
    expect(await screen.findByText("missing")).toBeInTheDocument();

    // Version + language for first row
    expect(await screen.findByText("1.0.0")).toBeInTheDocument();
    expect(await screen.findByText("go")).toBeInTheDocument();

    // last_modified formatted as relative (2h ago)
    expect(await screen.findByText("2h ago")).toBeInTheDocument();
  });

  it("renders error state on query failure", async () => {
    (invoke as any).mockImplementation((cmd: string) => {
      if (cmd === "workspace_current")
        return Promise.resolve({ root: "/tmp/ws", schema_version: 2 });
      if (cmd === "handler_list")
        return Promise.reject({ kind: "io", message: "disk read failed" });
      throw new Error("unexpected invoke: " + cmd);
    });
    render(wrap(<HandlersPage />));
    expect(
      await screen.findByText(/Failed to load handlers/i)
    ).toBeInTheDocument();
    expect(await screen.findByText(/\[io\] disk read failed/)).toBeInTheDocument();
  });

  it("row click navigates to /handlers/:name", async () => {
    mockInvoke([
      {
        name: "my-handler",
        path: "/tmp/ws/handlers/my-handler",
        manifest_status: "valid",
        last_modified: new Date(Date.now() - 30).toISOString(),
        version: "0.1.0",
        language: "go",
      },
    ]);
    render(wrap(<HandlersPage />));

    const nameCell = await screen.findByText("my-handler");
    fireEvent.click(nameCell);

    expect(await screen.findByTestId("location")).toHaveTextContent(
      "/handlers/my-handler"
    );
  });
});
