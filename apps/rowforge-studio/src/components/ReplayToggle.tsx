import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useNavigate, useParams } from "react-router-dom";
import { Button } from "@/components/ui/button";
import { Play } from "lucide-react";
import { uiErrorMessage } from "@/ipc/types";
import type { ExecutionId, AttemptId, RunHandle } from "@/ipc/types";

const SPEEDS: Array<{ label: string; value: number }> = [
  { label: "1×", value: 1 },
  { label: "5×", value: 5 },
  { label: "10×", value: 10 },
];

export function ReplayToggle({
  executionId,
  attemptId,
}: {
  executionId: ExecutionId;
  attemptId: AttemptId;
}) {
  const navigate = useNavigate();
  const params = useParams<{ id: string; aid: string }>();
  const [speed, setSpeed] = useState(1);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const startReplay = async () => {
    setBusy(true);
    setError(null);
    try {
      const handle = await invoke<RunHandle>("attempt_replay_start", {
        executionId,
        attemptId,
        speed,
      });
      // Navigate to same page with ?run=<handle> to engage Live tab.
      navigate(`/exec/${params.id}/attempt/${params.aid}?run=${handle}`, {
        replace: true,
      });
    } catch (e) {
      setError(uiErrorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex items-center gap-2">
      <div className="flex rounded border border-border bg-neutral-900">
        {SPEEDS.map((s) => (
          <button
            key={s.value}
            onClick={() => setSpeed(s.value)}
            className={`px-2 py-1 text-xs ${
              s.value === speed ? "bg-primary/20 text-foreground" : "text-muted-foreground hover:bg-muted"
            }`}
          >
            {s.label}
          </button>
        ))}
      </div>
      <Button onClick={startReplay} disabled={busy} size="sm" variant="outline">
        <Play className="h-3 w-3" />
        {busy ? "Starting…" : "Replay"}
      </Button>
      {error && <span className="text-xs text-red-300">{error}</span>}
    </div>
  );
}
