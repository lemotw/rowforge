import { useEffect, useMemo, useState } from "react";
import { open as shellOpen } from "@tauri-apps/plugin-shell";
import { useHandlerLogTail, useHandlerLogLive } from "@/ipc/use-handler-log";
import { LogsToolbar } from "@/components/LogsToolbar";
import { LogsVirtualList } from "@/components/LogsVirtualList";
import { uiErrorMessage } from "@/ipc/types";
import type { HandlerLogLine } from "@/ipc/types";

interface Props {
  execId: string;
  attemptId: string;
  isLive: boolean;
  /**
   * Absolute path to the handler_log.log file for this attempt.
   * When provided, "Reveal log file" opens it in the OS file manager.
   * When absent (older attempt or path not yet resolved), the button is
   * a no-op — TODO(T8.5): plumb handler_log_path from AttemptDetail.paths.
   */
  logFilePath?: string;
}

export function AttemptLogsTab({ execId, attemptId, isLive, logFilePath }: Props) {
  const tail = useHandlerLogTail(execId, attemptId);
  const live = useHandlerLogLive(execId, attemptId, isLive);

  const [workerFilter, setWorkerFilter] = useState<Set<number>>(new Set());
  const [streamFilter, setStreamFilter] = useState<"stdout" | "stderr" | "both">("both");
  const [searchTerm, setSearchTerm] = useState("");
  const [autoScroll, setAutoScroll] = useState(true);
  const [paused, setPaused] = useState(false);
  const [frozenLines, setFrozenLines] = useState<HandlerLogLine[] | null>(null);

  // Combined source: tail snapshot (lines on disk at mount) + live delta.
  const allLines = useMemo(() => {
    if (!tail.data) return live.lines;
    return [...tail.data, ...live.lines];
  }, [tail.data, live.lines]);

  // Pause: freeze the visible source; new live lines accumulate but stay hidden
  // until Resume. frozenLines === null means "not paused / use live source".
  useEffect(() => {
    if (paused && frozenLines === null) {
      setFrozenLines(allLines);
    }
    if (!paused) {
      setFrozenLines(null);
    }
  }, [paused, allLines, frozenLines]);

  const visibleSource = paused && frozenLines !== null ? frozenLines : allLines;

  const availableWorkers = useMemo(() => {
    const set = new Set<number>();
    visibleSource.forEach((l) => set.add(l.worker_id));
    return Array.from(set).sort((a, b) => a - b);
  }, [visibleSource]);

  const filtered = useMemo(() => {
    return visibleSource.filter((l) => {
      if (workerFilter.size > 0 && !workerFilter.has(l.worker_id)) return false;
      if (streamFilter !== "both" && l.stream !== streamFilter) return false;
      if (searchTerm && !l.line.toLowerCase().includes(searchTerm.toLowerCase())) return false;
      return true;
    });
  }, [visibleSource, workerFilter, streamFilter, searchTerm]);

  const onReveal = () => {
    if (logFilePath) {
      shellOpen(logFilePath).catch(() => {});
    }
    // TODO(T8.5): if logFilePath is absent, plumb handler_log_path via
    // AttemptDetail.paths once T9 adds that field to the Rust type.
  };

  if (tail.isLoading) {
    return <div className="p-6 text-muted-foreground">Loading logs…</div>;
  }
  if (tail.isError) {
    return (
      <div className="p-6 text-red-300">
        Failed to load logs: {uiErrorMessage(tail.error)}
      </div>
    );
  }

  // "No output yet": live attempt with no lines at all (not a filter issue)
  // "No log file": finished attempt with no lines (predates Plan 9 capture)
  // "No lines match": there are lines but filters removed everything
  const hasNoLines = allLines.length === 0;
  const isFilteredEmpty = !hasNoLines && filtered.length === 0;

  return (
    <div className="flex flex-col h-full min-h-[400px]">
      <LogsToolbar
        workerFilter={workerFilter}
        setWorkerFilter={setWorkerFilter}
        availableWorkers={availableWorkers}
        streamFilter={streamFilter}
        setStreamFilter={setStreamFilter}
        searchTerm={searchTerm}
        setSearchTerm={setSearchTerm}
        autoScroll={autoScroll}
        setAutoScroll={setAutoScroll}
        paused={paused}
        onTogglePaused={() => setPaused((v) => !v)}
        onReveal={onReveal}
        isLive={isLive}
      />

      {live.dropped > 0 && (
        <div className="border-b border-red-500/30 bg-red-500/10 p-2 text-xs text-red-200">
          ⚠ {live.dropped} log line{live.dropped === 1 ? "" : "s"} dropped during
          high-throughput period.{" "}
          <button onClick={onReveal} className="text-blue-400 hover:underline">
            Reveal log file
          </button>{" "}
          for complete capture.
        </div>
      )}

      {hasNoLines ? (
        <div className="flex-1 flex items-center justify-center text-muted-foreground text-sm">
          {isLive
            ? "Handler has not produced any output yet."
            : "No log file. This attempt predates Plan 9 log capture."}
        </div>
      ) : isFilteredEmpty ? (
        <div className="flex-1 flex items-center justify-center text-muted-foreground text-sm">
          No lines match the current filters.
        </div>
      ) : (
        <LogsVirtualList lines={filtered} autoScroll={autoScroll && !paused} />
      )}
    </div>
  );
}
