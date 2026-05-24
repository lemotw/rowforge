import { open } from "@tauri-apps/plugin-dialog";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { useOpenWorkspace } from "@/ipc/queries";
import { uiErrorMessage } from "@/ipc/types";
import { Inbox } from "lucide-react";

export function WorkspacePicker({ onPicked }: { onPicked: () => void }) {
  const openMut = useOpenWorkspace();

  const pickFolder = async () => {
    const selected = await open({ directory: true, multiple: false });
    if (typeof selected !== "string") return;
    openMut.mutate(selected, { onSuccess: onPicked });
  };

  const useDefault = () => {
    openMut.mutate(null, { onSuccess: onPicked });
  };

  return (
    <div className="grid h-screen place-items-center">
      <Card className="flex w-[480px] flex-col items-center gap-6 p-10">
        <Inbox className="h-12 w-12 text-muted-foreground" />
        <div className="text-center">
          <h1 className="text-xl font-medium">No workspace yet</h1>
          <p className="mt-2 text-sm text-muted-foreground">
            rowforge stores executions and per-row outcomes on disk. Pick
            an existing workspace or create one at <code>~/.rowforge</code>.
          </p>
        </div>
        <div className="flex w-full flex-col gap-2">
          <Button onClick={pickFolder} disabled={openMut.isPending}>
            Open folder…
          </Button>
          <Button onClick={useDefault} variant="outline" disabled={openMut.isPending}>
            Use ~/.rowforge
          </Button>
        </div>
        {openMut.isError && (
          <p className="text-sm text-red-400">{uiErrorMessage(openMut.error)}</p>
        )}
      </Card>
    </div>
  );
}
