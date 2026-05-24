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
 * Plan 4: pick handler dir → start_run. Plan 5 adds full launcher
 * (retry-failed / sample / dry-run / config overrides).
 *
 * After start_run returns the RunHandle, we need to navigate to the
 * NEW attempt's page so the Live tab can subscribe. The new attempt
 * id is not in the start_run response (would require a follow-up
 * query). For Plan 4 we navigate back to `/exec/:id` and let the
 * Attempts table refresh; clicking the new attempt picks up the
 * ?run query param if we cache the handle in URL state.
 *
 * Simpler Plan 4 flow: after start_run, navigate to
 * `/exec/:id?pending_run=<handle>` and let the ExecDetail surface a
 * link or toast pointing to the latest attempt. Or simplest of all:
 * just stay on ExecDetail and let the user click into the new attempt
 * row when it appears.
 *
 * We pick: navigate back to /exec/:id (refetches the exec_show data
 * via TanStack Query invalidation in useRunStart), and let the
 * user click into the new attempt to see the Live tab. The Run
 * button itself stores the handle in component state for a quick
 * "Open Live" link.
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
  const [activeHandle, setActiveHandle] = useState<string | null>(null);

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
        onSuccess: (handle) => {
          setActiveHandle(handle);
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

      {activeHandle && (
        <Button
          variant="outline"
          size="sm"
          onClick={() => {
            // Navigate to the attempt with ?run= so Live tab activates.
            // We don't know the attempt_id yet without a re-query;
            // for Plan 4, navigate back to /exec/:id which will
            // refetch and show the new attempt row.
            navigate(`/exec/${executionId}`);
            // The handle is captured in URL by the user clicking the
            // new attempt row; for now we keep it accessible here.
            setActiveHandle(null);
          }}
        >
          ✓ Started (refresh)
        </Button>
      )}

      {error && (
        <span className="text-xs text-red-300" title={error}>
          {error.slice(0, 80)}
        </span>
      )}
    </div>
  );
}
