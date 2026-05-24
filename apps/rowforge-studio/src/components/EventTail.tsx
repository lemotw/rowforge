import { useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { cn } from "@/lib/utils";
import type { OutcomeSampleEntry } from "@/ipc/run-state";

type FilterMode = "all" | "errors" | "crashes";

export function EventTail({ samples }: { samples: OutcomeSampleEntry[] }) {
  const [filter, setFilter] = useState<FilterMode>("errors");

  const filtered = samples.filter((s) => {
    if (filter === "all") return true;
    if (filter === "crashes") return s.kind === "crash";
    return s.kind === "error" || s.kind === "crash" || s.kind === "too_large";
  });

  const parentRef = useRef<HTMLDivElement>(null);
  const rowVirtualizer = useVirtualizer({
    count: filtered.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 28,
    overscan: 12,
  });

  return (
    <div className="rounded-lg border border-border bg-neutral-900">
      <div className="flex items-center justify-between border-b border-border px-3 py-2">
        <span className="text-sm font-medium">Recent events</span>
        <div className="flex gap-1">
          <FilterChip active={filter === "all"} onClick={() => setFilter("all")}>
            All
          </FilterChip>
          <FilterChip active={filter === "errors"} onClick={() => setFilter("errors")}>
            Errors
          </FilterChip>
          <FilterChip active={filter === "crashes"} onClick={() => setFilter("crashes")}>
            Crashes
          </FilterChip>
        </div>
      </div>

      {filtered.length === 0 ? (
        <div className="p-6 text-center text-sm text-muted-foreground">
          No events yet.
        </div>
      ) : (
        <div
          ref={parentRef}
          className="overflow-auto"
          style={{ height: "320px" }}
        >
          <div
            style={{
              height: `${rowVirtualizer.getTotalSize()}px`,
              width: "100%",
              position: "relative",
            }}
          >
            {rowVirtualizer.getVirtualItems().map((vRow) => {
              const sample = filtered[vRow.index];
              return (
                <EventRow
                  key={vRow.key}
                  sample={sample}
                  style={{
                    position: "absolute",
                    top: 0,
                    left: 0,
                    width: "100%",
                    transform: `translateY(${vRow.start}px)`,
                    height: `${vRow.size}px`,
                  }}
                />
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}

function FilterChip({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "rounded px-2 py-0.5 text-xs",
        active
          ? "bg-primary/20 text-foreground"
          : "text-muted-foreground hover:bg-muted"
      )}
    >
      {children}
    </button>
  );
}

function EventRow({
  sample,
  style,
}: {
  sample: OutcomeSampleEntry;
  style: React.CSSProperties;
}) {
  const toneClass =
    sample.kind === "error" ? "border-l-red-500" :
    sample.kind === "crash" ? "border-l-red-600" :
    "border-l-amber-500";
  return (
    <div
      style={style}
      className={cn(
        "flex items-center gap-3 border-l-2 px-3 text-xs font-mono",
        toneClass
      )}
    >
      <span className="text-muted-foreground">row {sample.row_index}</span>
      {sample.code && (
        <span className="rounded bg-neutral-800 px-1.5 py-0.5">{sample.code}</span>
      )}
      <span className="flex-1 truncate" title={sample.message ?? ""}>
        {sample.message}
      </span>
      <span className="tabular-nums text-muted-foreground">{sample.dur_ms}ms</span>
    </div>
  );
}
