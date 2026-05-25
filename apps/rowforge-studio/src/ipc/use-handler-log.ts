import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import { ipc } from "./client";
import type { HandlerLogLine } from "./types";

export function useHandlerLogTail(execId: string, attemptId: string, maxLines = 5000) {
  return useQuery({
    queryKey: ["handler_log_tail", execId, attemptId, maxLines],
    queryFn: () =>
      ipc.handler_log_tail({ exec_id: execId, attempt_id: attemptId, max_lines: maxLines }),
    enabled: !!attemptId,
  });
}

export interface LiveStreamState {
  lines: HandlerLogLine[];
  dropped: number;
}

/**
 * Subscribe to live handler log lines for an attempt. Pumps new lines
 * into local state; subscriber is torn down on unmount.
 *
 * @param enabled toggle the subscription off when not on the Logs tab
 *                or when the attempt is no longer live, to avoid an
 *                unnecessary IPC subscribe + event listener.
 */
export function useHandlerLogLive(execId: string, attemptId: string, enabled: boolean): LiveStreamState {
  const [state, setState] = useState<LiveStreamState>({ lines: [], dropped: 0 });

  useEffect(() => {
    if (!enabled || !attemptId) return;

    let unlisten: UnlistenFn | null = null;
    let cancelled = false;

    ipc
      .handler_log_subscribe({ exec_id: execId, attempt_id: attemptId })
      .then(() => {
        if (cancelled) return undefined;
        return listen<{ lines: HandlerLogLine[]; dropped: number }>(
          `handler_log:${attemptId}`,
          (event) => {
            setState((prev) => ({
              lines: [...prev.lines, ...event.payload.lines],
              dropped: prev.dropped + event.payload.dropped,
            }));
          },
        );
      })
      .then((un) => {
        if (un) unlisten = un;
      })
      .catch(() => {
        // attempt may be inactive; tail (file read) covers static view
      });

    return () => {
      cancelled = true;
      if (unlisten) unlisten();
      ipc.handler_log_unsubscribe({ attempt_id: attemptId }).catch(() => {});
    };
  }, [execId, attemptId, enabled]);

  return state;
}
