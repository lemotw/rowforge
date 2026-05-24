import { useEffect, useState } from "react";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { useRunCancel } from "@/ipc/queries";
import type { RunHandle, RunStatus } from "@/ipc/types";

const FORCE_KILL_THRESHOLD_MS = 10_000;

export function CancelDialog({
  handle,
  status,
  execName,
}: {
  handle: RunHandle;
  status: RunStatus;
  execName: string;
}) {
  const [softConfirmOpen, setSoftConfirmOpen] = useState(false);
  const [hardConfirmOpen, setHardConfirmOpen] = useState(false);
  const cancelMut = useRunCancel();
  const isCancelling = status === "cancelling";

  return (
    <>
      {!isCancelling ? (
        <Button variant="outline" size="sm" onClick={() => setSoftConfirmOpen(true)}>
          Cancel
        </Button>
      ) : (
        <CancellingBanner
          handle={handle}
          execName={execName}
          onForceKill={() => setHardConfirmOpen(true)}
        />
      )}

      <SoftConfirmDialog
        open={softConfirmOpen}
        onOpenChange={setSoftConfirmOpen}
        onConfirm={() => {
          cancelMut.mutate({ handle, mode: "soft" });
          setSoftConfirmOpen(false);
        }}
      />

      <HardConfirmDialog
        open={hardConfirmOpen}
        onOpenChange={setHardConfirmOpen}
        execName={execName}
        onConfirm={() => {
          cancelMut.mutate({ handle, mode: "hard" });
          setHardConfirmOpen(false);
        }}
      />
    </>
  );
}

function SoftConfirmDialog({
  open,
  onOpenChange,
  onConfirm,
}: {
  open: boolean;
  onOpenChange: (v: boolean) => void;
  onConfirm: () => void;
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Soft cancel?</DialogTitle>
        </DialogHeader>
        <p className="text-sm text-muted-foreground">
          In-flight rows will finish, then the run will abort cleanly.
        </p>
        <div className="mt-4 flex justify-end gap-2">
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Keep running
          </Button>
          <Button onClick={onConfirm}>Soft cancel</Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function CancellingBanner({
  handle: _handle,
  execName: _execName,
  onForceKill,
}: {
  handle: RunHandle;
  execName: string;
  onForceKill: () => void;
}) {
  const [elapsed, setElapsed] = useState(0);
  useEffect(() => {
    const started = Date.now();
    const id = setInterval(() => setElapsed(Date.now() - started), 250);
    return () => clearInterval(id);
  }, []);

  const showForceKill = elapsed >= FORCE_KILL_THRESHOLD_MS;

  return (
    <div
      className={cn(
        "flex items-center gap-3 rounded border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-sm text-amber-200",
      )}
    >
      <span>⏳ Cancelling…</span>
      <span className="text-xs text-amber-300/60 tabular-nums">
        {Math.floor(elapsed / 1000)}s
      </span>
      {showForceKill && (
        <Button
          size="sm"
          variant="outline"
          className="ml-auto border-red-500/60 text-red-300 hover:bg-red-500/10"
          onClick={onForceKill}
        >
          Force kill
        </Button>
      )}
    </div>
  );
}

function HardConfirmDialog({
  open,
  onOpenChange,
  execName,
  onConfirm,
}: {
  open: boolean;
  onOpenChange: (v: boolean) => void;
  execName: string;
  onConfirm: () => void;
}) {
  const [typed, setTyped] = useState("");
  const requiredToken = execName.slice(0, 4).toLowerCase();
  const canConfirm = typed.trim().toLowerCase() === requiredToken;

  // Reset typed when dialog closes.
  useEffect(() => {
    if (!open) setTyped("");
  }, [open]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle className="text-red-300">Force-kill workers?</DialogTitle>
        </DialogHeader>
        <div className="space-y-3 text-sm">
          <p className="text-muted-foreground">
            Partial outcomes may be lost. This cannot be undone.
          </p>
          <div>
            <label className="block text-xs text-muted-foreground">
              Type <code className="rounded bg-neutral-800 px-1.5">{requiredToken}</code> (first 4 chars of exec name) to confirm
            </label>
            <input
              autoFocus
              value={typed}
              onChange={(e) => setTyped(e.target.value)}
              className="mt-1 w-full rounded border border-border bg-neutral-950 px-2 py-1 font-mono"
            />
          </div>
        </div>
        <div className="mt-4 flex justify-end gap-2">
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button
            disabled={!canConfirm}
            className={cn(
              "bg-red-500 text-white hover:bg-red-500/90",
              !canConfirm && "opacity-50",
            )}
            onClick={onConfirm}
          >
            Force kill
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
