import { useEffect, useReducer } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { initialRunState, reduceRun } from "./run-state";
import type { ProgressEvent, RunHandle } from "./types";

/**
 * Subscribe to a run's Tauri event channel and accumulate state.
 * `handle` may be null to disable (e.g. when no run is active).
 */
export function useRun(handle: RunHandle | null) {
  const [state, dispatch] = useReducer(reduceRun, initialRunState);

  useEffect(() => {
    if (!handle) return;
    let unlisten: UnlistenFn | undefined;
    let cancelled = false;
    listen<ProgressEvent>(`run:${handle}`, (e) => {
      dispatch(e.payload);
    }).then((f) => {
      if (cancelled) {
        f();
      } else {
        unlisten = f;
      }
    });
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, [handle]);

  return state;
}
