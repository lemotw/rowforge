import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { formatBytes } from "@/lib/format";
import type { ExecSummary } from "@/ipc/types";

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  selected: ExecSummary[];
  onConfirm: () => void;
  isPending: boolean;
}

const MAX_LISTED = 10;

export function DeleteExecutionsDialog({
  open,
  onOpenChange,
  selected,
  onConfirm,
  isPending,
}: Props) {
  const total = selected.reduce((sum, e) => sum + (e.size_bytes ?? 0), 0);
  const listed = selected.slice(0, MAX_LISTED);
  const remaining = selected.length - listed.length;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>
            Delete {selected.length} execution
            {selected.length === 1 ? "" : "s"}?
          </DialogTitle>
        </DialogHeader>

        <div className="space-y-3 text-sm">
          <p className="text-muted-foreground">
            This permanently deletes the selected executions and all their
            attempt data (outcomes, handler logs, exports, etc.). Total:{" "}
            <span className="font-mono">{formatBytes(total)}</span>. This
            cannot be undone.
          </p>

          <ul className="max-h-64 overflow-auto rounded border border-zinc-700 p-2 font-mono text-xs">
            {listed.map((e) => (
              <li key={e.id} className="flex justify-between gap-2 py-0.5">
                <span className="truncate">{e.name ?? "—"}</span>
                <span className="text-muted-foreground shrink-0">
                  {formatBytes(e.size_bytes)}
                </span>
              </li>
            ))}
            {remaining > 0 && (
              <li className="text-muted-foreground py-0.5">
                … and {remaining} more
              </li>
            )}
          </ul>
        </div>

        <div className="mt-4 flex justify-end gap-2">
          <Button
            variant="outline"
            onClick={() => onOpenChange(false)}
            disabled={isPending}
          >
            Cancel
          </Button>
          <Button
            variant="outline"
            onClick={onConfirm}
            disabled={isPending}
            className="bg-red-500/10 text-red-200 border-red-500/40 hover:bg-red-500/20"
          >
            {isPending
              ? "Deleting…"
              : `Delete ${selected.length} execution${selected.length === 1 ? "" : "s"}`}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
