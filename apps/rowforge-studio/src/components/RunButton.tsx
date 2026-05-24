import { useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useNavigate } from "react-router-dom";
import { Button } from "@/components/ui/button";
import { Play } from "lucide-react";
import { useRunStart } from "@/ipc/queries";
import { uiErrorMessage } from "@/ipc/types";
import type { ExecutionId } from "@/ipc/types";

/**
 * Minimal "Run" launcher.
 *
 * Plan 4: pick handler dir → start_run.
 * Plan 5 (Task 8): start_run returns RunStartedHandle { handle, attempt_id }.
 * Plan 5 (Task 15): on success, auto-navigate to the new attempt's Live tab
 *   so the user lands on live progress without manually clicking the row.
 *   Closes the Plan 4 known limitation.
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

  const handleRun = async () => {
    setError(null);
    // 1. Determine handler dir.
    let dir = lastHandlerDir;
    if (!dir) {
      const picked = await openDialog({ directory: true, multiple: false });
      if (typeof picked !== "string") return; // user cancelled
      dir = picked;
    }

    // 2. Start the run.
    runMut.mutate(
      { executionId, handlerDir: dir },
      {
        onSuccess: (started) => {
          // Plan 5 T15: navigate directly to the new attempt's Live tab.
          // RunStartedHandle carries attempt_id (Task 8) so no follow-up
          // query is needed — closes the Plan 4 known limitation.
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

  return (
    <div className="flex items-center gap-2">
      <Button
        onClick={handleRun}
        disabled={runMut.isPending}
        size="sm"
      >
        <Play className="h-3 w-3" />
        {runMut.isPending ? "Starting…" : "Run"}
      </Button>

      {error && (
        <span className="text-xs text-red-300" title={error}>
          {error.slice(0, 80)}
        </span>
      )}
    </div>
  );
}
