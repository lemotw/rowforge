import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { useQueryClient } from "@tanstack/react-query";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { open as shellOpen } from "@tauri-apps/plugin-shell";
import { useOpenWorkspace } from "@/ipc/queries";

export function WorkspaceMenu({
  workspaceRoot,
  open,
  onOpenChange,
}: {
  workspaceRoot: string;
  open: boolean;
  onOpenChange: (b: boolean) => void;
}) {
  const qc = useQueryClient();
  const openMut = useOpenWorkspace();

  const reveal = () => {
    shellOpen(workspaceRoot);
    onOpenChange(false);
  };

  const switchWs = async () => {
    const picked = await openDialog({ directory: true, multiple: false });
    if (typeof picked !== "string") return;
    openMut.mutate(picked, {
      onSuccess: () => {
        onOpenChange(false);
        window.location.hash = "/";
      },
    });
  };

  const reload = () => {
    qc.invalidateQueries();
    onOpenChange(false);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Workspace</DialogTitle>
        </DialogHeader>
        <div className="my-2 font-mono text-sm text-muted-foreground">{workspaceRoot}</div>
        <div className="flex flex-col gap-2">
          <Button variant="outline" onClick={reveal}>Reveal in Finder</Button>
          <Button variant="outline" onClick={reload}>Reload data</Button>
          <Button variant="outline" onClick={switchWs}>Switch workspace…</Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
