import { useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useNavigate } from "react-router-dom";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Play, Settings2 } from "lucide-react";
import { useRunStart } from "@/ipc/queries";
import { uiErrorMessage } from "@/ipc/types";
import type { ExecutionId } from "@/ipc/types";

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
  const [handlerDir, setHandlerDir] = useState<string | null>(lastHandlerDir ?? null);
  const [sample, setSample] = useState<string>("");
  const [workers, setWorkers] = useState<string>("");
  const [dryRun, setDryRun] = useState<boolean>(false);

  const pickHandlerDir = async () => {
    const p = await openDialog({ directory: true, multiple: false });
    if (typeof p === "string") setHandlerDir(p);
  };

  const fireRun = (dir: string) => {
    setError(null);
    const rowLimit = sample.trim() === "" ? null : Math.max(1, parseInt(sample, 10) || 0);
    const w = workers.trim() === "" ? null : Math.max(1, parseInt(workers, 10) || 0);
    runMut.mutate(
      { executionId, handlerDir: dir, rowLimit, workers: w, dryRun: dryRun || null },
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
        <div className="absolute right-0 z-30 mt-1 w-72 rounded-lg border border-zinc-700 bg-zinc-900 p-3 shadow-xl">
          <div className="mb-2 text-xs font-medium uppercase text-muted-foreground">
            Run options
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
                checked={dryRun}
                onChange={(e) => setDryRun(e.target.checked)}
              />
              Dry run (handler sees <code>meta.dry_run = true</code>)
            </label>

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
                disabled={!handlerDir || runMut.isPending}
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
