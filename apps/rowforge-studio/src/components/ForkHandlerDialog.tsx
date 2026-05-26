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
import { useHandlerFork } from "@/ipc/use-handlers";
import { uiErrorMessage } from "@/ipc/types";

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  sourceName: string;
}

const NAME_RE = /^[a-z0-9][a-z0-9-]*$/;

export function ForkHandlerDialog({ open, onOpenChange, sourceName }: Props) {
  const navigate = useNavigate();
  const fork = useHandlerFork();
  const [newName, setNewName] = useState(`${sourceName}-fork`);

  // Reset to <sourceName>-fork on every (re)open.
  const forkReset = fork.reset;
  useEffect(() => {
    if (open) {
      setNewName(`${sourceName}-fork`);
      forkReset();
    }
  }, [open, sourceName, forkReset]);

  const nameError =
    newName === ""
      ? "Name is required"
      : !NAME_RE.test(newName)
        ? "Lowercase letters, numbers, and hyphens; must start with a letter or number"
        : newName === sourceName
          ? "Name must differ from source"
          : null;
  const canSubmit = nameError === null && !fork.isPending;

  const handleSubmit = () => {
    if (!canSubmit) return;
    fork.mutate(
      { sourceName, newName },
      {
        onSuccess: () => {
          toast.success(`Handler forked to "${newName}"`);
          onOpenChange(false);
          navigate(`/handlers/${newName}`);
        },
      },
    );
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Fork handler "{sourceName}"</DialogTitle>
        </DialogHeader>

        <div className="space-y-4">
          <p className="text-sm text-muted-foreground">
            Copies all files from "{sourceName}" into a new handler dir,
            updating the manifest's name field to match.
          </p>

          <div className="rounded border border-yellow-500/40 bg-yellow-500/10 p-2 text-sm text-yellow-200">
            ⚠ Comments in rowforge.yaml will not survive the fork (serde
            round-trip).
          </div>

          <div>
            <label htmlFor="fork-name-input" className="mb-1 block text-sm font-medium">
              New name
            </label>
            <Input
              id="fork-name-input"
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              autoFocus
            />
            {nameError && (
              <div className="mt-1 text-xs text-red-300">{nameError}</div>
            )}
          </div>

          {fork.isError && (
            <div className="rounded border border-red-500/40 bg-red-500/10 p-2 text-sm text-red-200">
              {uiErrorMessage(fork.error)}
            </div>
          )}
        </div>

        <div className="mt-4 flex justify-end gap-2">
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button onClick={handleSubmit} disabled={!canSubmit}>
            {fork.isPending ? "Forking…" : "Fork"}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
