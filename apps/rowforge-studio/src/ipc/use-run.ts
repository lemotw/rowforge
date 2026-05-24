import { useEffect, useReducer } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { initialRunState, reduceRun } from "./run-state";
import { ipc } from "./client";
import type { ProgressEvent, RunHandle } from "./types";

/**
 * Subscribe to a run's Tauri event channel and accumulate state.
 * `handle` may be null to disable (e.g. when no run is active).
 *
 * Bootstrap protocol (fix for race: Tauri events are fire-and-forget,
 * so any tick / phase_changed emitted before `listen()` lands is lost):
 *
 * 1. Attach the listener first so subsequent events are captured.
 * 2. Once the listener is attached, fetch the current ProgressSnapshot
 *    via `ipc.run_snapshot` and dispatch a synthetic `_bootstrap` action
 *    to fill in counters that the listener missed.
 *
 * Real events arriving between step 1 and step 2 are accumulated normally;
 * the bootstrap dispatch overwrites counter fields with the snapshot, but
 * the next real Tick (250ms later at the latest) overrides again with
 * authoritative numbers. Brief flicker is acceptable; missing the entire
 * run is not.
 */
export function useRun(handle: RunHandle | null) {
  const [state, dispatch] = useReducer(reduceRun, initialRunState);

  useEffect(() => {
    if (!handle) return;
    let unlisten: UnlistenFn | undefined;
    let cancelled = false;

    listen<ProgressEvent>(`run:${handle}`, (e) => {
      dispatch(e.payload);
    }).then(async (f) => {
      if (cancelled) {
        f();
        return;
      }
      unlisten = f;

      // Listener is attached — bootstrap from snapshot to recover any
      // counters that fired before we landed. Best-effort: if the run
      // already finished (handle no longer in the registry), the call
      // errors with UnknownHandle and we just keep going.
      try {
        const snap = await ipc.run_snapshot({ handle });
        if (!cancelled) {
          dispatch({ type: "_bootstrap", snapshot: snap });
        }
      } catch {
        // run_snapshot rejected (typically UnknownHandle — run finished
        // before listener attached, session removed from registry).
        // Signal the page so it can fall back to attempt_show static data.
        if (!cancelled) {
          dispatch({ type: "_terminal_before_listen" });
        }
      }
    });

    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, [handle]);

  return state;
}
