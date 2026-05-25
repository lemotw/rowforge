import { useEffect, useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { HandlerLogLine } from "@/ipc/types";

interface Props {
  lines: HandlerLogLine[];
  autoScroll: boolean;
}

export function LogsVirtualList({ lines, autoScroll }: Props) {
  const parentRef = useRef<HTMLDivElement>(null);
  const rowVirtualizer = useVirtualizer({
    count: lines.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 22,
    overscan: 20,
  });

  useEffect(() => {
    if (autoScroll && parentRef.current) {
      parentRef.current.scrollTop = parentRef.current.scrollHeight;
    }
  }, [lines.length, autoScroll]);

  return (
    <div ref={parentRef} className="flex-1 overflow-auto font-mono text-xs bg-zinc-950">
      <div style={{ height: rowVirtualizer.getTotalSize(), position: "relative", width: "100%" }}>
        {rowVirtualizer.getVirtualItems().map((item) => {
          const line = lines[item.index];
          return (
            <div
              key={item.key}
              style={{
                position: "absolute",
                top: 0,
                left: 0,
                width: "100%",
                transform: `translateY(${item.start}px)`,
              }}
              className="flex gap-2 px-2 py-0.5 hover:bg-zinc-800/40 cursor-text"
            >
              <span className="text-muted-foreground shrink-0 w-20">
                {new Date(line.timestamp).toLocaleTimeString()}
              </span>
              <WorkerBadge id={line.worker_id} />
              <StreamChip stream={line.stream} />
              <span className="whitespace-pre-wrap break-all">{line.line}</span>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function WorkerBadge({ id }: { id: number }) {
  const hue = (id * 60) % 360;
  return (
    <span
      className="inline-block rounded px-1 text-[10px] font-semibold shrink-0"
      style={{ backgroundColor: `hsl(${hue}, 50%, 25%)`, color: `hsl(${hue}, 70%, 80%)` }}
    >
      #{id}
    </span>
  );
}

function StreamChip({ stream }: { stream: "stdout" | "stderr" }) {
  const cls =
    stream === "stderr"
      ? "bg-yellow-500/15 text-yellow-300"
      : "bg-blue-500/15 text-blue-300";
  return (
    <span className={`inline-block rounded px-1 text-[10px] font-medium shrink-0 ${cls}`}>
      {stream}
    </span>
  );
}
