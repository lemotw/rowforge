import { useWorkspace, useExecList } from "@/ipc/queries";
import { AppShell } from "@/layout/AppShell";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, Thead, Tr, Th, Td } from "@/components/ui/table";
import { uiErrorMessage } from "@/ipc/types";

export function ExecListPage() {
  const ws = useWorkspace();
  const list = useExecList(!!ws.data);

  const workspace = ws.data ?? null;

  return (
    <AppShell workspace={workspace}>
      <div className="p-6">
        <div className="mb-4 flex items-center justify-between">
          <h1 className="text-lg font-medium">Executions</h1>
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
          <div className="rounded-lg border border-dashed border-border p-10 text-center text-sm text-muted-foreground">
            No executions yet. Create one with{" "}
            <code>rowforge exec start</code> in a terminal.
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
                <Tr key={e.id}>
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
