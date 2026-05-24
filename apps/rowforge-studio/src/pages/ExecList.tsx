import { useNavigate } from "react-router-dom";
import { useWorkspace, useExecList } from "@/ipc/queries";
import { AppShell } from "@/layout/AppShell";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, Thead, Tr, Th, Td } from "@/components/ui/table";
import { uiErrorMessage } from "@/ipc/types";

export function ExecListPage() {
  const navigate = useNavigate();
  const ws = useWorkspace();
  const list = useExecList(!!ws.data);

  const workspace = ws.data ?? null;

  return (
    <AppShell workspace={workspace}>
      <div className="p-6">
        <div className="mb-4 flex items-center justify-between">
          <h1 className="text-lg font-medium">Executions</h1>
          <Button onClick={() => navigate("/new")} variant="outline" size="sm">
            New execution
          </Button>
        </div>

        {list.isLoading && (
          <div className="space-y-2">
            <Skeleton className="h-9 w-full" />
            <Skeleton className="h-9 w-full" />
            <Skeleton className="h-9 w-full" />
          </div>
        )}

        {list.isError && (
          <div className="rounded border border-red-500/40 bg-red-500/10 p-3 text-sm text-red-300">
            {uiErrorMessage(list.error)}
          </div>
        )}

        {list.data && list.data.length === 0 && (
          <div className="flex flex-col items-center gap-3 rounded-lg border border-dashed border-border p-10 text-center">
            <div className="text-sm text-muted-foreground">No executions yet</div>
            <Button onClick={() => navigate("/new")}>New execution</Button>
          </div>
        )}

        {list.data && list.data.length > 0 && (
          <Table>
            <Thead>
              <Tr>
                <Th>Name</Th>
                <Th>Created</Th>
                <Th>Rows</Th>
                <Th>Attempts</Th>
              </Tr>
            </Thead>
            <tbody>
              {list.data.map((e) => (
                <Tr key={e.id} className="cursor-pointer" onClick={() => navigate(`/exec/${e.id}`)}>
                  <Td className="font-mono">{e.name || "—"}</Td>
                  <Td className="font-mono">
                    {new Date(e.created_at).toISOString().replace("T", " ").slice(0, 16)}
                  </Td>
                  <Td className="text-right">{e.input_rows ?? "—"}</Td>
                  <Td className="text-right">{e.attempts_count}</Td>
                </Tr>
              ))}
            </tbody>
          </Table>
        )}
      </div>
    </AppShell>
  );
}
