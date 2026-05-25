import { useMemo, useState, useEffect } from "react";
import { useNavigate } from "react-router-dom";
import { listen } from "@tauri-apps/api/event";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { useWorkspace, useExecList, useExecutionDeleteBulk } from "@/ipc/queries";
import { AppShell } from "@/layout/AppShell";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, Thead, Tr, Th, Td } from "@/components/ui/table";
import { uiErrorMessage } from "@/ipc/types";
import { formatBytes } from "@/lib/format";
import { DeleteExecutionsDialog } from "@/components/DeleteExecutionsDialog";
import type { ExecDeleteFailure } from "@/ipc/types";

export function ExecListPage() {
  const navigate = useNavigate();
  const qc = useQueryClient();
  const ws = useWorkspace();
  const list = useExecList(!!ws.data);
  const bulkMutation = useExecutionDeleteBulk();

  const [selectMode, setSelectMode] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [bulkAlert, setBulkAlert] = useState<ExecDeleteFailure[] | null>(null);

  const workspace = ws.data ?? null;

  // Active exec IDs: derived from exec list items whose last_attempt_state is "running".
  // run_active returns RunHandle[] (opaque strings), not exec IDs, so we use the
  // last_attempt_state field on the ExecSummary as the authoritative active-run signal.
  const activeExecIds = useMemo(
    () =>
      new Set(
        (list.data ?? [])
          .filter((e) => e.last_attempt_state === "running")
          .map((e) => e.id),
      ),
    [list.data],
  );

  // Listen for exec_list:refresh event emitted by Tauri after deletion commands.
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    listen("exec_list:refresh", () => {
      qc.invalidateQueries({ queryKey: ["exec_list"] });
    }).then((u) => {
      unlisten = u;
    });
    return () => {
      unlisten?.();
    };
  }, [qc]);

  const selectedExecs = useMemo(
    () => (list.data ?? []).filter((e) => selectedIds.has(e.id)),
    [list.data, selectedIds],
  );

  const toggleSelect = (id: string) => {
    if (activeExecIds.has(id)) return;
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const exitSelectMode = () => {
    setSelectMode(false);
    setSelectedIds(new Set());
  };

  const handleConfirm = () => {
    bulkMutation.mutate(
      { execIds: Array.from(selectedIds) },
      {
        onSuccess: (result) => {
          setConfirmOpen(false);
          if (result.failed.length === 0) {
            toast.success(
              `${result.deleted.length} execution${result.deleted.length === 1 ? "" : "s"} deleted`,
            );
            exitSelectMode();
            setBulkAlert(null);
          } else if (result.deleted.length > 0) {
            toast.warning(`${result.deleted.length} deleted, ${result.failed.length} failed`);
            exitSelectMode();
            setBulkAlert(result.failed);
          } else {
            toast.error(
              `All ${result.failed.length} deletion${result.failed.length === 1 ? "" : "s"} failed`,
            );
            // Stay in select mode so user can adjust
            setBulkAlert(result.failed);
          }
        },
        onError: (err) => {
          setConfirmOpen(false);
          toast.error(uiErrorMessage(err));
        },
      },
    );
  };

  return (
    <AppShell workspace={workspace}>
      <div className="p-6 space-y-4">
        {/* header */}
        <div className="flex items-center justify-between">
          <h1 className="text-lg font-medium">Executions</h1>
          <div className="flex gap-2">
            {!selectMode ? (
              <>
                <Button onClick={() => setSelectMode(true)} variant="outline" size="sm">
                  Select
                </Button>
                <Button onClick={() => navigate("/new")} variant="outline" size="sm">
                  New execution
                </Button>
              </>
            ) : (
              <>
                <Button
                  variant="outline"
                  size="sm"
                  disabled={selectedIds.size === 0 || bulkMutation.isPending}
                  onClick={() => setConfirmOpen(true)}
                  className="bg-red-500/10 text-red-200 border-red-500/40 hover:bg-red-500/20"
                >
                  Delete {selectedIds.size} execution{selectedIds.size === 1 ? "" : "s"}
                </Button>
                <Button variant="ghost" size="sm" onClick={exitSelectMode}>
                  Cancel
                </Button>
              </>
            )}
          </div>
        </div>

        {/* bulk-fail alert */}
        {bulkAlert && bulkAlert.length > 0 && (
          <div className="rounded border border-yellow-500/30 bg-yellow-500/10 p-3 text-sm text-yellow-200 flex items-start gap-3">
            <div className="flex-1">
              <div className="font-medium mb-1">
                ⚠ {bulkAlert.length} deletion{bulkAlert.length === 1 ? "" : "s"} failed:
              </div>
              <ul className="text-xs space-y-0.5">
                {bulkAlert.map((f) => (
                  <li key={f.exec_id} className="font-mono">
                    • {f.exec_id.slice(0, 12)}…: {f.reason}
                  </li>
                ))}
              </ul>
            </div>
            <Button variant="ghost" size="sm" onClick={() => setBulkAlert(null)}>
              Dismiss
            </Button>
          </div>
        )}

        {/* loading state */}
        {list.isLoading && (
          <div className="space-y-2">
            <Skeleton className="h-9 w-full" />
            <Skeleton className="h-9 w-full" />
            <Skeleton className="h-9 w-full" />
          </div>
        )}

        {/* error state */}
        {list.isError && (
          <div className="rounded border border-red-500/40 bg-red-500/10 p-3 text-sm text-red-300">
            {uiErrorMessage(list.error)}
          </div>
        )}

        {/* empty state */}
        {list.data && list.data.length === 0 && (
          <div className="flex flex-col items-center gap-3 rounded-lg border border-dashed border-border p-10 text-center">
            <div className="text-sm text-muted-foreground">No executions yet</div>
            <Button onClick={() => navigate("/new")}>New execution</Button>
          </div>
        )}

        {/* table */}
        {list.data && list.data.length > 0 && (
          <Table>
            <Thead>
              <Tr>
                {selectMode && <Th className="w-8"></Th>}
                <Th>Name</Th>
                <Th className="text-right">Rows</Th>
                <Th className="text-right">Attempts</Th>
                <Th className="text-right">Size</Th>
                <Th>Created</Th>
              </Tr>
            </Thead>
            <tbody>
              {list.data.map((e) => {
                const isActive = activeExecIds.has(e.id);
                const isSelected = selectedIds.has(e.id);
                return (
                  <Tr
                    key={e.id}
                    className={selectMode && !isActive ? "cursor-pointer" : selectMode ? "" : "cursor-pointer"}
                    onClick={() => {
                      if (selectMode) toggleSelect(e.id);
                      else navigate(`/exec/${e.id}`);
                    }}
                  >
                    {selectMode && (
                      <Td>
                        <input
                          type="checkbox"
                          checked={isSelected}
                          disabled={isActive}
                          title={isActive ? "Cancel active run first" : undefined}
                          onChange={() => {}}
                          onClick={(ev) => {
                            ev.stopPropagation();
                            toggleSelect(e.id);
                          }}
                        />
                      </Td>
                    )}
                    <Td className="font-mono" title={e.id}>
                      {e.name || "—"}
                    </Td>
                    <Td className="text-right">{e.input_rows ?? "—"}</Td>
                    <Td className="text-right">{e.attempts_count}</Td>
                    <Td className="text-right">{formatBytes(e.size_bytes)}</Td>
                    <Td className="font-mono">
                      {new Date(e.created_at).toISOString().replace("T", " ").slice(0, 16)}
                    </Td>
                  </Tr>
                );
              })}
            </tbody>
          </Table>
        )}

        <DeleteExecutionsDialog
          open={confirmOpen}
          onOpenChange={setConfirmOpen}
          selected={selectedExecs}
          onConfirm={handleConfirm}
          isPending={bulkMutation.isPending}
        />
      </div>
    </AppShell>
  );
}
