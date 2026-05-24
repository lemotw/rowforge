import { useEffect, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useNavigate } from "react-router-dom";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Play, Settings2 } from "lucide-react";
import { useExecRollup, useRunStart } from "@/ipc/queries";
import { uiErrorMessage } from "@/ipc/types";
import type { ExecutionId } from "@/ipc/types";

const LS_HANDLER_DIR = "studio.lastHandlerDir";

/**
 * Run launcher.
 *
 * Two modes:
 * - **Quick Run** (primary button): picks handler dir → starts a full run.
 *   Same behaviour as the Plan 4 minimal launcher.
 * - **Options** (settings icon): opens an inline panel for sample size
 *   (`row_limit`), worker count, and dry-run flag. Submit from here uses
 *   those values + handler dir.
 *
 * After successful start, navigates to the new attempt's Live tab
 * (Plan 5 T15).
 */
export function RunButton({
  executionId,
  lastHandlerDir,
}: {
  executionId: ExecutionId;
  lastHandlerDir?: string | null;
}) {
  const runMut = useRunStart();
  const navigate = useNavigate();
  const [error, setError] = useState<string | null>(null);

  const [optionsOpen, setOptionsOpen] = useState(false);
  // Seed from prop, then localStorage, then null. localStorage persists the
  // last picked dir across sessions so users don't re-pick every time.
  const [handlerDir, setHandlerDir] = useState<string | null>(() => {
    if (lastHandlerDir) return lastHandlerDir;
    try {
      return localStorage.getItem(LS_HANDLER_DIR);
    } catch {
      return null;
    }
  });
  const [sample, setSample] = useState<string>("");
  const [workers, setWorkers] = useState<string>("");
  const [dryRun, setDryRun] = useState<boolean>(false);
  const [skipAttempted, setSkipAttempted] = useState<boolean>(false);

  // Mirror handlerDir changes into localStorage.
  useEffect(() => {
    try {
      if (handlerDir) localStorage.setItem(LS_HANDLER_DIR, handlerDir);
    } catch { /* ignore quota / privacy mode */ }
  }, [handlerDir]);

  // Rollup tells us how many rows are NeverAttempted (= remaining when
  // skip_attempted is on). Lazy: only fetched once the options panel opens.
  const rollup = useExecRollup(executionId, optionsOpen);
  const totalRows = rollup.data
    ? rollup.data.resolved +
      rollup.data.failed_last +
      rollup.data.crashed_last +
      rollup.data.cancelled_last +
      rollup.data.too_large +
      rollup.data.never_attempted
    : null;
  const remaining = rollup.data?.never_attempted ?? null;
  const attemptedCount =
    totalRows !== null && remaining !== null ? totalRows - remaining : null;

  // Preview of what the next Start run will dispatch given current options.
  const sampleN = sample.trim() === "" ? null : Math.max(0, parseInt(sample, 10) || 0);
  const willDispatch = (() => {
    if (skipAttempted) {
      if (remaining === null) return null;
      return sampleN !== null ? Math.min(sampleN, remaining) : remaining;
    }
    if (totalRows === null) return null;
    return sampleN !== null ? Math.min(sampleN, totalRows) : totalRows;
  })();

  const pickHandlerDir = async () => {
    const p = await openDialog({ directory: true, multiple: false });
    if (typeof p === "string") setHandlerDir(p);
  };

  const fireRun = (dir: string) => {
    setError(null);
    const rowLimit = sample.trim() === "" ? null : Math.max(1, parseInt(sample, 10) || 0);
    const w = workers.trim() === "" ? null : Math.max(1, parseInt(workers, 10) || 0);
    runMut.mutate(
      {
        executionId,
        handlerDir: dir,
        rowLimit,
        workers: w,
        dryRun: dryRun || null,
        skipAttempted: skipAttempted || null,
      },
      {
        onSuccess: (started) => {
          setOptionsOpen(false);
          navigate(
            `/exec/${executionId}/attempt/${started.attempt_id}?run=${started.handle}`
          );
        },
        onError: (e) => {
          setError(uiErrorMessage(e));
        },
      }
    );
  };

  const handleQuickRun = async () => {
    let dir = handlerDir;
    if (!dir) {
      const picked = await openDialog({ directory: true, multiple: false });
      if (typeof picked !== "string") return;
      dir = picked;
      setHandlerDir(picked);
    }
    fireRun(dir);
  };

  return (
    <div className="relative">
      <div className="flex items-center gap-1">
        <Button onClick={handleQuickRun} disabled={runMut.isPending} size="sm">
          <Play className="h-3 w-3" />
          {runMut.isPending ? "Starting…" : "Run"}
        </Button>
        <Button
          onClick={() => setOptionsOpen((v) => !v)}
          disabled={runMut.isPending}
          size="sm"
          variant="outline"
          aria-label="Run options"
          title="Run with sample / workers / dry-run"
        >
          <Settings2 className="h-3 w-3" />
        </Button>

        {error && (
          <span className="text-xs text-red-300" title={error}>
            {error.slice(0, 80)}
          </span>
        )}
      </div>

      {optionsOpen && (
        <div className="absolute right-0 z-30 mt-1 w-80 rounded-lg border border-zinc-700 bg-zinc-900 p-3 shadow-xl">
          <div className="mb-2 flex items-baseline justify-between">
            <span className="text-xs font-medium uppercase text-muted-foreground">
              Run options
            </span>
            {totalRows !== null && (
              <span className="font-mono text-xs text-muted-foreground">
                {totalRows} rows · {attemptedCount} attempted · {remaining} fresh
              </span>
            )}
          </div>

          <div className="space-y-3">
            <div>
              <label className="mb-1 block text-xs">Handler directory</label>
              <div className="flex gap-1">
                <Input
                  value={handlerDir ?? ""}
                  placeholder="not selected"
                  readOnly
                  className="text-xs"
                />
                <Button onClick={pickHandlerDir} variant="outline" size="sm">
                  Pick…
                </Button>
              </div>
            </div>

            <div>
              <label className="mb-1 block text-xs">
                Sample first N rows{" "}
                <span className="text-muted-foreground">(blank = all)</span>
              </label>
              <Input
                type="number"
                min={1}
                value={sample}
                onChange={(e) => setSample(e.target.value)}
                placeholder="e.g. 10"
              />
            </div>

            <div>
              <label className="mb-1 block text-xs">
                Workers{" "}
                <span className="text-muted-foreground">(blank = manifest default)</span>
              </label>
              <Input
                type="number"
                min={1}
                value={workers}
                onChange={(e) => setWorkers(e.target.value)}
                placeholder="e.g. 4"
              />
            </div>

            <label className="flex items-center gap-2 text-xs">
              <input
                type="checkbox"
                checked={skipAttempted}
                onChange={(e) => setSkipAttempted(e.target.checked)}
              />
              Skip rows already attempted (sample fresh rows across runs)
            </label>

            <label className="flex items-center gap-2 text-xs">
              <input
                type="checkbox"
                checked={dryRun}
                onChange={(e) => setDryRun(e.target.checked)}
              />
              Dry run (handler sees <code>meta.dry_run = true</code>)
            </label>

            {willDispatch !== null && (
              <div className="rounded border border-blue-500/30 bg-blue-500/10 p-2 text-xs">
                Will dispatch{" "}
                <span className="font-mono font-medium">{willDispatch}</span>
                {" "}row{willDispatch === 1 ? "" : "s"} in this run
                {skipAttempted && (
                  <span className="text-muted-foreground">
                    {" "}(skipping {attemptedCount} already-attempted)
                  </span>
                )}
                {willDispatch === 0 && (
                  <div className="mt-1 text-amber-300">
                    Nothing to dispatch — all rows have been attempted.
                    {skipAttempted && " Uncheck 'Skip rows already attempted' to re-run."}
                  </div>
                )}
              </div>
            )}

            <div className="flex justify-end gap-2 pt-1">
              <Button
                variant="ghost"
                size="sm"
                onClick={() => setOptionsOpen(false)}
              >
                Cancel
              </Button>
              <Button
                size="sm"
                disabled={
                  !handlerDir ||
                  runMut.isPending ||
                  willDispatch === 0
                }
                onClick={() => handlerDir && fireRun(handlerDir)}
              >
                {runMut.isPending ? "Starting…" : "Start run"}
              </Button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
