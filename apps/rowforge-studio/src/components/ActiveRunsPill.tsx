import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Activity } from "lucide-react";
import { useActiveRunsLive } from "@/ipc/use-active-runs";
import { useActiveRuns } from "@/ipc/queries";

export function ActiveRunsPill() {
  const live = useActiveRunsLive();
  const handlesQ = useActiveRuns();

  if (live.active_runs === 0) return null;

  return (
    <Popover>
      <PopoverTrigger asChild>
        <button className="flex items-center gap-1.5 rounded-full border border-emerald-500/40 bg-emerald-500/10 px-3 py-1 text-xs text-emerald-300 hover:bg-emerald-500/20">
          <Activity className="h-3 w-3" />
          <span className="tabular-nums">{live.active_runs}</span>
          <span>running</span>
        </button>
      </PopoverTrigger>
      <PopoverContent align="end">
        <div className="mb-2 text-xs font-medium text-muted-foreground">
          Active runs
        </div>
        <div className="space-y-1">
          {(handlesQ.data ?? []).map((handle) => (
            <RunRow key={handle} handle={handle} />
          ))}
        </div>
        <div className="mt-3 flex justify-between border-t border-border pt-2 text-xs text-muted-foreground tabular-nums">
          <span>processed: {live.total_processed.toLocaleString()}</span>
          <span>failed: {live.total_failed.toLocaleString()}</span>
        </div>
      </PopoverContent>
    </Popover>
  );
}

function RunRow({ handle }: { handle: string }) {
  // For Plan 4, we don't know which exec/attempt the handle maps to
  // without a backend query. Plan 5 may add a richer view; here we
  // show the handle as a clickable element that scrolls to the
  // user's last-known route, OR just renders the handle.
  return (
    <div className="rounded bg-neutral-800/50 px-2 py-1.5 text-xs">
      <span className="font-mono text-muted-foreground">{handle}</span>
    </div>
  );
}
