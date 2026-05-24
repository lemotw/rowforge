import { useEffect, useState } from "react";
import { useSettings, useOpenWorkspace } from "@/ipc/queries";
import { WorkspacePicker } from "./WorkspacePicker";
import { ExecListPage } from "./ExecList";
import { uiErrorMessage } from "@/ipc/types";

type Phase = "loading" | "picker" | "ready" | "error";

export function BootGate() {
  const settings = useSettings();
  const openMut = useOpenWorkspace();
  const [phase, setPhase] = useState<Phase>("loading");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (settings.isLoading) return;
    if (settings.isError) {
      setError(uiErrorMessage(settings.error));
      setPhase("error");
      return;
    }
    const stored = settings.data?.workspace_root ?? null;
    if (stored) {
      openMut.mutate(stored, {
        onSuccess: () => setPhase("ready"),
        onError: (e) => {
          // Stored workspace bad; fall back to picker.
          console.warn("autoload failed:", uiErrorMessage(e));
          setPhase("picker");
        },
      });
    } else {
      setPhase("picker");
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settings.isLoading, settings.isError]);

  if (phase === "loading") {
    return (
      <div className="grid h-screen place-items-center text-muted-foreground">
        Loading…
      </div>
    );
  }
  if (phase === "error") {
    return (
      <div className="grid h-screen place-items-center text-red-400">
        {error ?? "unknown error"}
      </div>
    );
  }
  if (phase === "picker") {
    return <WorkspacePicker onPicked={() => setPhase("ready")} />;
  }
  return <ExecListPage />;
}
