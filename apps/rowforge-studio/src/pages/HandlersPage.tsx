import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { FileCode2 } from "lucide-react";
import { AppShell } from "@/layout/AppShell";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, Thead, Tr, Th, Td } from "@/components/ui/table";
import { ScaffoldDialog } from "@/components/ScaffoldDialog";
import { useWorkspace } from "@/ipc/queries";
import {
  useHandlerList,
  useHandlerOpenEditor,
  useHandlerReveal,
} from "@/ipc/use-handlers";
import { uiErrorMessage, type HandlerSummary, type ManifestStatus } from "@/ipc/types";

export function HandlersPage() {
  const navigate = useNavigate();
  const qc = useQueryClient();
  const ws = useWorkspace();
  const { data, isLoading, isError, error } = useHandlerList();
  const [scaffoldOpen, setScaffoldOpen] = useState(false);

  // Coarse refresh on backend mutation event (T9).
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    listen("handlers:list", () => {
      qc.invalidateQueries({ queryKey: ["handler_list"] });
    }).then((u) => {
      unlisten = u;
    });
    return () => {
      unlisten?.();
    };
  }, [qc]);

  const workspace = ws.data ?? null;

  return (
    <AppShell workspace={workspace}>
      <div className="p-6">
        <div className="mb-4 flex items-center justify-between">
          <h1 className="text-lg font-medium">Handlers</h1>
          <Button
            variant="outline"
            size="sm"
            onClick={() => setScaffoldOpen(true)}
          >
            <FileCode2 className="mr-1.5 h-4 w-4" />
            New Handler
          </Button>
        </div>

        {isLoading && (
          <div className="space-y-2">
            <Skeleton className="h-9 w-full" />
            <Skeleton className="h-9 w-full" />
            <Skeleton className="h-9 w-full" />
          </div>
        )}

        {isError && (
          <div className="rounded border border-red-500/40 bg-red-500/10 p-3 text-sm text-red-300">
            Failed to load handlers: {uiErrorMessage(error)}
          </div>
        )}

        {data && data.length === 0 && (
          <div className="flex flex-col items-center gap-3 rounded-lg border border-dashed border-border p-10 text-center">
            <div className="text-sm text-muted-foreground">
              No handlers in this workspace yet. Click "New Handler" to scaffold one.
            </div>
          </div>
        )}

        {data && data.length > 0 && (
          <Table>
            <Thead>
              <Tr>
                <Th>Name</Th>
                <Th>Status</Th>
                <Th>Version</Th>
                <Th>Language</Th>
                <Th>Modified</Th>
                <Th>Actions</Th>
              </Tr>
            </Thead>
            <tbody>
              {data.map((h) => (
                <HandlerRow
                  key={h.name}
                  h={h}
                  onOpen={() => navigate(`/handlers/${h.name}`)}
                />
              ))}
            </tbody>
          </Table>
        )}
      </div>

      <ScaffoldDialog open={scaffoldOpen} onOpenChange={setScaffoldOpen} />
    </AppShell>
  );
}

function HandlerRow({ h, onOpen }: { h: HandlerSummary; onOpen: () => void }) {
  const openEditor = useHandlerOpenEditor();
  const reveal = useHandlerReveal();

  return (
    <Tr className="cursor-pointer" onClick={onOpen}>
      <Td className="font-mono font-semibold">{h.name}</Td>
      <Td>
        <StatusBadge status={h.manifest_status} />
      </Td>
      <Td className="text-muted-foreground">{h.version ?? "—"}</Td>
      <Td className="text-muted-foreground">{h.language ?? "—"}</Td>
      <Td className="text-muted-foreground">{formatRelative(h.last_modified)}</Td>
      <Td>
        <div
          className="flex gap-2"
          onClick={(e) => e.stopPropagation()}
        >
          <Button
            size="sm"
            variant="outline"
            onClick={() => openEditor.mutate({ name: h.name })}
          >
            Edit
          </Button>
          <Button
            size="sm"
            variant="outline"
            onClick={() => reveal.mutate({ name: h.name })}
          >
            Reveal
          </Button>
        </div>
      </Td>
    </Tr>
  );
}

function StatusBadge({ status }: { status: ManifestStatus }) {
  const cls =
    status === "valid"
      ? "bg-green-500/15 text-green-300 border-green-500/30"
      : status === "invalid"
        ? "bg-yellow-500/15 text-yellow-300 border-yellow-500/30"
        : "bg-red-500/15 text-red-300 border-red-500/30";
  return (
    <span className={`inline-block rounded px-2 py-0.5 text-xs border ${cls}`}>
      {status}
    </span>
  );
}

function formatRelative(iso: string): string {
  const then = new Date(iso).getTime();
  const now = Date.now();
  const sec = Math.max(0, Math.floor((now - then) / 1000));
  if (sec < 60) return `${sec}s ago`;
  if (sec < 3600) return `${Math.floor(sec / 60)}m ago`;
  if (sec < 86400) return `${Math.floor(sec / 3600)}h ago`;
  return new Date(iso).toLocaleDateString();
}
