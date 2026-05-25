import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { Button } from "@/components/ui/button";
import { ipc } from "@/ipc/client";
import { uiErrorMessage } from "@/ipc/types";

/**
 * Plan 6 T12. Self-contained workspace switcher on the Settings page.
 *
 * - Polls `run_active` every 2s while mounted, so the disabled state
 *   reflects reality without needing a manual refresh.
 * - Disabled (+ amber warning text) when active_runs.length > 0:
 *   switching with live runs would orphan their pipeline tasks and
 *   leave the new core unable to subscribe to their broadcast.
 * - On click: directory picker → save settings → workspace_open →
 *   navigate to "/" so the user lands on the new workspace's exec
 *   list. All three steps run sequentially; failures surface in red.
 */
export function WorkspaceSwitchButton() {
  const navigate = useNavigate();
  const qc = useQueryClient();
  const [error, setError] = useState<string | null>(null);

  const active = useQuery({
    queryKey: ["run_active"],
    queryFn: () => ipc.run_active(),
    refetchInterval: 2000,
  });
  const activeCount = active.data?.length ?? 0;
  const disabled = activeCount > 0;

  const switchMut = useMutation({
    mutationFn: async (newRoot: string) => {
      const prev = await ipc.workspace_settings_load();
      await ipc.workspace_settings_save({
        settings: { ...prev, workspace_root: newRoot },
      });
      await ipc.workspace_open({ path: newRoot });
    },
    onSuccess: () => {
      // Bust everything — the new workspace has different execs,
      // active runs, settings, …
      qc.invalidateQueries();
      navigate("/");
    },
    onError: (e) => setError(uiErrorMessage(e)),
  });

  const onClick = async () => {
    setError(null);
    if (disabled) return;
    const picked = await openDialog({ directory: true, multiple: false });
    if (typeof picked !== "string") return;  // user cancelled
    switchMut.mutate(picked);
  };

  const tooltip = disabled
    ? `Cancel ${activeCount} active run${activeCount === 1 ? "" : "s"} first`
    : "Open a different workspace";

  return (
    <div className="space-y-1">
      <Button
        onClick={onClick}
        disabled={disabled || switchMut.isPending}
        title={tooltip}
        variant="outline"
      >
        {switchMut.isPending ? "Switching…" : "Switch workspace…"}
      </Button>
      {disabled && (
        <div className="text-xs text-amber-300">
          ⚠ {activeCount} active run{activeCount === 1 ? "" : "s"} — cancel to switch
        </div>
      )}
      {error && (
        <div className="text-xs text-red-300">Switch failed: {error}</div>
      )}
    </div>
  );
}
