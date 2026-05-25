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
import { useHandlerDelete } from "@/ipc/use-handlers";
import { uiErrorMessage } from "@/ipc/types";

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  name: string;
}

export function DeleteHandlerDialog({ open, onOpenChange, name }: Props) {
  const navigate = useNavigate();
  const del = useHandlerDelete();
  const [token, setToken] = useState("");

  // Reset on close.
  const delReset = del.reset;
  useEffect(() => {
    if (!open) {
      setToken("");
      delReset();
    }
  }, [open, delReset]);

  const matches = token === name;

  const handleSubmit = () => {
    if (!matches) return;
    del.mutate(
      { name },
      {
        onSuccess: () => {
          toast.success(`Handler "${name}" deleted`);
          onOpenChange(false);
          navigate("/handlers");
        },
      },
    );
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Delete handler "{name}"?</DialogTitle>
        </DialogHeader>

        <div className="space-y-4">
          <p className="text-sm text-muted-foreground">
            This permanently deletes the handler directory and all its source
            files. Past execution rows referencing this handler will keep
            pointing to the now-missing directory (lazy semantics).
          </p>

          <div>
            <label htmlFor="delete-token" className="mb-1 block text-sm">
              Type{" "}
              <span className="font-mono font-semibold">{name}</span> to confirm
            </label>
            <Input
              id="delete-token"
              value={token}
              onChange={(e) => setToken(e.target.value)}
              autoFocus
            />
          </div>

          {del.isError && (
            <div className="rounded border border-red-500/40 bg-red-500/10 p-2 text-sm text-red-200">
              {uiErrorMessage(del.error)}
            </div>
          )}
        </div>

        <div className="mt-4 flex justify-end gap-2">
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button
            variant="outline"
            className="bg-red-500/10 text-red-200 border-red-500/40 hover:bg-red-500/20"
            onClick={handleSubmit}
            disabled={!matches || del.isPending}
          >
            {del.isPending ? "Deleting…" : "Delete"}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
