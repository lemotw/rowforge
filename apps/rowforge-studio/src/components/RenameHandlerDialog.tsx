import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { toast } from "sonner";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useHandlerRename } from "@/ipc/use-handlers";
import { uiErrorMessage } from "@/ipc/types";

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  oldName: string;
}

const NAME_RE = /^[a-z0-9][a-z0-9-]*$/;

export function RenameHandlerDialog({ open, onOpenChange, oldName }: Props) {
  const navigate = useNavigate();
  const rename = useHandlerRename();
  const [next, setNext] = useState(oldName);

  // Reset to current name on every (re)open.
  const renameReset = rename.reset;
  useEffect(() => {
    if (open) {
      setNext(oldName);
      renameReset();
    }
  }, [open, oldName, renameReset]);

  const nameError =
    next === ""
      ? "Name is required"
      : !NAME_RE.test(next)
        ? "Lowercase letters, numbers, and hyphens; must start with a letter or number"
        : null;
  const isUnchanged = next === oldName;
  const canSubmit = nameError === null && !isUnchanged && !rename.isPending;

  const handleSubmit = () => {
    if (!canSubmit) return;
    rename.mutate(
      { old: oldName, new: next },
      {
        onSuccess: () => {
          toast.success(`Handler renamed to "${next}"`);
          onOpenChange(false);
          navigate(`/handlers/${next}`);
        },
      },
    );
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Rename handler "{oldName}"</DialogTitle>
        </DialogHeader>

        <div className="space-y-4">
          <div>
            <label htmlFor="rename-input" className="mb-1 block text-sm font-medium">
              New name
            </label>
            <Input
              id="rename-input"
              value={next}
              onChange={(e) => setNext(e.target.value)}
              autoFocus
            />
            {nameError && (
              <div className="mt-1 text-xs text-red-300">{nameError}</div>
            )}
          </div>

          <p className="text-xs text-muted-foreground">
            Lazy rename: existing execution rows keep their original handler
            reference. New runs will use the new name.
          </p>

          {rename.isError && (
            <div className="rounded border border-red-500/40 bg-red-500/10 p-2 text-sm text-red-200">
              {uiErrorMessage(rename.error)}
            </div>
          )}
        </div>

        <div className="mt-4 flex justify-end gap-2">
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button onClick={handleSubmit} disabled={!canSubmit}>
            {rename.isPending ? "Renaming…" : "Rename"}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
