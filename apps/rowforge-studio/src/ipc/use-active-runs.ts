import { useEffect, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { RunRollupTick } from "./types";

export function useActiveRunsLive(): RunRollupTick {
  const [tick, setTick] = useState<RunRollupTick>({
    active_runs: 0,
    total_processed: 0,
    total_failed: 0,
    total_rate: 0,
    slowest_run: null,
  });

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let cancelled = false;
    listen<RunRollupTick>("runs:active", (e) => {
      setTick(e.payload);
    }).then((f) => {
      if (cancelled) f();
      else unlisten = f;
    });
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, []);

  return tick;
}
