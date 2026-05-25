import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";

interface LogsToolbarProps {
  workerFilter: Set<number>;
  setWorkerFilter: (f: Set<number>) => void;
  availableWorkers: number[];
  streamFilter: "stdout" | "stderr" | "both";
  setStreamFilter: (f: "stdout" | "stderr" | "both") => void;
  searchTerm: string;
  setSearchTerm: (s: string) => void;
  autoScroll: boolean;
  setAutoScroll: (v: boolean) => void;
  paused: boolean;
  onTogglePaused: () => void;
  onReveal: () => void;
  isLive: boolean;
}

export function LogsToolbar({
  workerFilter,
  setWorkerFilter,
  availableWorkers,
  streamFilter,
  setStreamFilter,
  searchTerm,
  setSearchTerm,
  autoScroll,
  setAutoScroll,
  paused,
  onTogglePaused,
  onReveal,
  isLive,
}: LogsToolbarProps) {
  const toggleWorker = (id: number) => {
    const next = new Set(workerFilter);
    if (next.has(id)) {
      next.delete(id);
    } else {
      next.add(id);
    }
    setWorkerFilter(next);
  };

  return (
    <div className="flex flex-wrap items-center gap-2 border-b border-border bg-zinc-900/60 px-3 py-2">
      {/* Left: worker chips */}
      <div className="flex items-center gap-1.5 flex-wrap">
        {availableWorkers.length > 0 && (
          <span className="text-xs text-muted-foreground shrink-0">Workers:</span>
        )}
        {availableWorkers.map((id) => {
          const hue = (id * 60) % 360;
          const active = workerFilter.has(id);
          return (
            <button
              key={id}
              onClick={() => toggleWorker(id)}
              aria-pressed={active}
              aria-label={`worker ${id}`}
              className={cn(
                "inline-block rounded px-1.5 py-0.5 text-[10px] font-semibold transition-opacity",
                active ? "opacity-100 ring-1 ring-white/40" : "opacity-50"
              )}
              style={{
                backgroundColor: `hsl(${hue}, 50%, 25%)`,
                color: `hsl(${hue}, 70%, 80%)`,
              }}
            >
              #{id}
            </button>
          );
        })}
      </div>

      {/* Stream filter */}
      <div className="flex items-center gap-1">
        {(["both", "stdout", "stderr"] as const).map((s) => (
          <button
            key={s}
            onClick={() => setStreamFilter(s)}
            aria-pressed={streamFilter === s}
            className={cn(
              "rounded px-2 py-0.5 text-xs transition-colors",
              streamFilter === s
                ? "bg-primary/20 text-foreground"
                : "text-muted-foreground hover:bg-muted"
            )}
          >
            {s}
          </button>
        ))}
      </div>

      {/* Spacer */}
      <div className="flex-1" />

      {/* Search */}
      <Input
        value={searchTerm}
        onChange={(e) => setSearchTerm(e.target.value)}
        placeholder="Search…"
        className="h-7 w-44 text-xs"
        aria-label="search logs"
      />

      {/* Auto-scroll toggle */}
      <label className="flex items-center gap-1.5 text-xs text-muted-foreground cursor-pointer select-none">
        <input
          type="checkbox"
          checked={autoScroll}
          onChange={(e) => setAutoScroll(e.target.checked)}
          aria-label="auto-scroll"
        />
        Auto-scroll
      </label>

      {/* Pause / Resume — only meaningful while live */}
      {isLive && (
        <Button size="sm" variant="outline" onClick={onTogglePaused} className="h-7 text-xs">
          {paused ? "Resume" : "Pause"}
        </Button>
      )}

      {/* Reveal log file */}
      <Button size="sm" variant="ghost" onClick={onReveal} className="h-7 text-xs">
        Reveal log file
      </Button>
    </div>
  );
}
