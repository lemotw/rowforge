import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  rowCount: number;
  handlerDir: string;
  sourceAttemptId: string;
  onConfirm: () => void;
  isPending: boolean;
}

export function RerunFailedDialog({
  open,
  onOpenChange,
  rowCount,
  handlerDir,
  sourceAttemptId,
  onConfirm,
  isPending,
}: Props) {
  const rowLabel = rowCount === 1 ? "row" : "rows";

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>
            Re-run {rowCount} failed {rowLabel}?
          </DialogTitle>
        </DialogHeader>

        <div className="space-y-3 text-sm">
          <p className="text-muted-foreground">
            A new attempt will be created on this execution targeting only{" "}
            {rowCount} {rowLabel}. The same handler will be used as the source
            attempt.
          </p>

          <div className="rounded border border-border bg-neutral-900 p-3 space-y-1 font-mono text-xs">
            <div className="flex gap-2">
              <span className="text-muted-foreground shrink-0">Handler:</span>
              <span className="truncate">{handlerDir}</span>
            </div>
            <div className="flex gap-2">
              <span className="text-muted-foreground shrink-0">Source attempt:</span>
              <span className="truncate">{sourceAttemptId}</span>
            </div>
          </div>
        </div>

        <div className="mt-4 flex justify-end gap-2">
          <Button
            variant="outline"
            onClick={() => onOpenChange(false)}
            disabled={isPending}
          >
            Cancel
          </Button>
          <Button onClick={onConfirm} disabled={isPending}>
            {isPending ? "Starting…" : `Re-run ${rowCount} ${rowLabel}`}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
