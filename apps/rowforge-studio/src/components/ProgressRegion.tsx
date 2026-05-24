import { Activity, Layers } from "lucide-react";
import type { RunState } from "@/ipc/run-state";

export function ProgressRegion({ state }: { state: RunState }) {
  const pct = state.total && state.total > 0
    ? (state.processed / state.total) * 100
    : null;

  const etaText = formatEta(state.eta_ms);

  return (
    <div className="grid grid-cols-[2fr_1fr_1fr] gap-6 rounded-lg border border-border bg-neutral-900 p-4">
      {/* Left — progress bar */}
      <div>
        <div className="h-3 w-full rounded-full bg-neutral-800 overflow-hidden">
          <div
            className="h-full bg-emerald-500 transition-all duration-150 ease-out"
            style={{ width: pct == null ? "0%" : `${Math.min(pct, 100)}%` }}
          />
        </div>
        <div className="mt-2 font-mono text-sm tabular-nums text-muted-foreground">
          {state.processed.toLocaleString()} / {state.total != null ? state.total.toLocaleString() : "—"}
          {pct != null && ` (${pct.toFixed(1)}%)`}
        </div>
      </div>

      {/* Middle — rate + ETA */}
      <div className="flex flex-col gap-1">
        <div className="flex items-baseline gap-3">
          <Stat label="rate/1s" value={state.rate_1s.toFixed(0)} />
          <Stat label="rate/10s" value={state.rate_10s.toFixed(0)} />
        </div>
        <div className="mt-2">
          <Stat label="ETA" value={etaText} />
        </div>
      </div>

      {/* Right — in_flight + queue_depth */}
      <div className="flex flex-col gap-2">
        <Stat
          label="in-flight"
          value={state.in_flight.toString()}
          icon={<Activity className="h-3 w-3" />}
        />
        <Stat
          label="queue"
          value={state.queue_depth.toString()}
          icon={<Layers className="h-3 w-3" />}
        />
      </div>
    </div>
  );
}

function Stat({ label, value, icon }: { label: string; value: string; icon?: React.ReactNode }) {
  return (
    <div>
      <div className="text-xl tabular-nums">{value}</div>
      <div className="flex items-center gap-1 text-xs text-muted-foreground">
        {icon}
        <span>{label}</span>
      </div>
    </div>
  );
}

function formatEta(ms: number | null): string {
  if (ms == null) return "—";
  const totalSec = Math.floor(ms / 1000);
  if (totalSec < 60) return `${totalSec}s`;
  const min = Math.floor(totalSec / 60);
  const sec = totalSec % 60;
  if (min < 60) return `${min}m ${sec.toString().padStart(2, "0")}s`;
  const hr = Math.floor(min / 60);
  const m = min % 60;
  return `${hr}h ${m.toString().padStart(2, "0")}m`;
}
