import { Link, Navigate, useParams } from "react-router-dom";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Table, Thead, Tr, Th, Td } from "@/components/ui/table";
import { Skeleton } from "@/components/ui/skeleton";
import { AppShell } from "@/layout/AppShell";
import { useExecDetail, useWorkspace } from "@/ipc/queries";
import { RollupCard } from "@/components/RollupCard";
import { uiErrorMessage } from "@/ipc/types";

export function ExecDetailPage() {
  const { id } = useParams<{ id: string }>();
  const ws = useWorkspace();
  const detail = useExecDetail(id ?? null);

  if (ws.data === null && !ws.isLoading) return <Navigate to="/" replace />;

  const workspace = ws.data ?? null;
  const crumbs = [
    { label: "Executions", to: "/" },
    { label: detail.data?.summary.name || id || "...", mono: true },
  ];

  return (
    <AppShell workspace={workspace} crumbs={crumbs}>
      <div className="p-6">
        {detail.isLoading && <Skeleton className="h-32 w-full" />}
        {detail.isError && (
          <div className="text-red-300">{uiErrorMessage(detail.error)}</div>
        )}
        {detail.data && (
          <>
            <header className="mb-6">
              <h1 className="text-xl font-medium">{detail.data.summary.name || "(unnamed)"}</h1>
              <div className="mt-1 font-mono text-xs text-muted-foreground">
                id: {detail.data.summary.id} · input: {detail.data.input_path_snapshot} ({detail.data.summary.input_rows ?? "?"} rows)
              </div>
            </header>

            <Tabs defaultValue="attempts">
              <TabsList>
                <TabsTrigger value="attempts">Attempts ({detail.data.attempts.length})</TabsTrigger>
                <TabsTrigger value="rollup">Rollup</TabsTrigger>
                <TabsTrigger value="bindings">Bindings</TabsTrigger>
              </TabsList>

              <TabsContent value="attempts">
                {detail.data.attempts.length === 0 ? (
                  <div className="rounded-lg border border-dashed p-10 text-center text-muted-foreground">
                    This execution has never been run.
                  </div>
                ) : (
                  <Table>
                    <Thead>
                      <Tr><Th>#</Th><Th>State</Th><Th>Started</Th><Th>Run type</Th><Th></Th></Tr>
                    </Thead>
                    <tbody>
                      {detail.data.attempts.map((a, i) => (
                        <Tr key={a.id}>
                          <Td>{i + 1}</Td>
                          <Td><StateChip state={a.state} /></Td>
                          <Td className="font-mono">
                            {new Date(a.started_at).toISOString().replace("T", " ").slice(0, 16)}
                          </Td>
                          <Td>{a.run_type}</Td>
                          <Td>
                            <Link to={`/exec/${id}/attempt/${a.id}`} className="text-primary hover:underline">
                              open ⏵
                            </Link>
                          </Td>
                        </Tr>
                      ))}
                    </tbody>
                  </Table>
                )}
              </TabsContent>

              <TabsContent value="rollup">
                <RollupCard executionId={id!} />
              </TabsContent>

              <TabsContent value="bindings">
                <pre className="rounded-lg border border-border bg-neutral-900 p-4 text-xs">
{JSON.stringify({
  handler_binding: detail.data.handler_binding,
  field_mapping: detail.data.field_mapping,
  config_overrides: detail.data.config_overrides,
}, null, 2)}
                </pre>
              </TabsContent>
            </Tabs>
          </>
        )}
      </div>
    </AppShell>
  );
}

function StateChip({ state }: { state: string }) {
  const tone =
    state === "done" || state === "completed" ? "text-emerald-400" :
    state === "aborted" ? "text-neutral-400" :
    state === "crashed" ? "text-red-400" :
    state === "running" ? "text-emerald-300" :
    "text-blue-300";
  return <span className={tone}>● {state}</span>;
}
