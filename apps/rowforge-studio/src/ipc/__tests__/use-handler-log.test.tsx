import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor, act } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useHandlerLogTail, useHandlerLogLive } from "@/ipc/use-handler-log";
import type { HandlerLogLine } from "@/ipc/types";

function wrap({ children }: { children: React.ReactNode }) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
}

describe("useHandlerLogTail", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("invokes handler_log_tail with the right args", async () => {
    const lines: HandlerLogLine[] = [
      { timestamp: "2026-05-25T10:00:00Z", worker_id: 0, stream: "stderr", line: "hi" },
    ];
    (invoke as any).mockResolvedValue(lines);
    const { result } = renderHook(() => useHandlerLogTail("e1", "a1"), { wrapper: wrap });
    await waitFor(() => expect(result.current.data).toEqual(lines));
    expect(invoke).toHaveBeenCalledWith("handler_log_tail", { exec_id: "e1", attempt_id: "a1", max_lines: 5000 });
  });
});

describe("useHandlerLogLive", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    (listen as any).mockResolvedValue(() => {}); // unlisten fn
  });

  it("does not subscribe when disabled", () => {
    renderHook(() => useHandlerLogLive("e1", "a1", false), { wrapper: wrap });
    expect(invoke).not.toHaveBeenCalled();
  });

  it("subscribes and accumulates lines from event", async () => {
    let listener: any;
    (listen as any).mockImplementation(async (_name: string, fn: any) => {
      listener = fn;
      return () => {};
    });
    (invoke as any).mockResolvedValue(undefined);

    const { result } = renderHook(() => useHandlerLogLive("e1", "a1", true), {
      wrapper: wrap,
    });

    // wait for subscribe + listen to settle
    await waitFor(() => expect(listener).toBeDefined());

    act(() => {
      listener({
        payload: {
          lines: [{ timestamp: "2026-05-25T10:00:00Z", worker_id: 0, stream: "stderr", line: "hi" }],
          dropped: 0,
        },
      });
    });

    expect(result.current.lines).toHaveLength(1);
    expect(result.current.lines[0].line).toBe("hi");
  });

  it("unsubscribes on unmount", async () => {
    (invoke as any).mockResolvedValue(undefined);
    const { unmount } = renderHook(() => useHandlerLogLive("e1", "a1", true), {
      wrapper: wrap,
    });
    // Wait for subscribe call to happen
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("handler_log_subscribe", { exec_id: "e1", attempt_id: "a1" }));
    unmount();
    expect(invoke).toHaveBeenCalledWith("handler_log_unsubscribe", { attempt_id: "a1" });
  });
});
