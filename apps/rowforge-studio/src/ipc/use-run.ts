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
 *
 * If `run_snapshot` rejects (typically UnknownHandle — run finished
 * before listener attached, session already removed from registry),
 * dispatches `_terminal_before_listen` so AttemptDetail can pivot to
 * the Summary tab and refetch attempt_show for the final stats.
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

      try {
        const snap = await ipc.run_snapshot({ handle });
        if (!cancelled) {
          dispatch({ type: "_bootstrap", snapshot: snap });
        }
      } catch (e) {
        if (cancelled) return;
        // Only UnknownHandle implies "run finished before we attached —
        // pivot to Summary tab + refetch attempt_show". Any other error
        // (workspace_locked, internal, io, ...) means the bootstrap
        // failed for an unrelated reason; keep the listener attached
        // and let live events populate state as they arrive.
        const kind = (e as { kind?: string } | null)?.kind;
        if (kind === "unknown_handle") {
          dispatch({ type: "_terminal_before_listen" });
        } else {
          // eslint-disable-next-line no-console
          console.warn("[useRun] run_snapshot failed unexpectedly:", e);
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
