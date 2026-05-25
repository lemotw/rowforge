import { useMemo, useState } from "react";
import { Link, Navigate, useParams } from "react-router-dom";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Table, Thead, Tr, Th, Td } from "@/components/ui/table";
import { Skeleton } from "@/components/ui/skeleton";
import { AppShell } from "@/layout/AppShell";
import { useExecDetail, useWorkspace } from "@/ipc/queries";
import { RollupCard } from "@/components/RollupCard";
import { RunButton } from "@/components/RunButton";
import { ExportDialog } from "@/components/ExportDialog";
import { uiErrorMessage } from "@/ipc/types";
import { Button } from "@/components/ui/button";

export function ExecDetailPage() {
  const { id } = useParams<{ id: string }>();
  const ws = useWorkspace();
  const detail = useExecDetail(id ?? null);
  const [exportOpen, setExportOpen] = useState(false);

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
        {detail.isError && (() => {
          const msg = uiErrorMessage(detail.error);
          const isNotFound =
            typeof detail.error === "object" &&
            detail.error !== null &&
            "kind" in detail.error &&
            (detail.error as { kind: string }).kind === "not_found";
          return isNotFound ? (
            <div className="space-y-3">
              <div className="text-red-300">
                This execution has been deleted or is unavailable.
              </div>
              <Link to="/" className="text-blue-400 hover:underline">
                ← Back to executions
              </Link>
            </div>
          ) : (
            <div className="text-red-300">{msg}</div>
          );
        })()}
        {detail.data && (
          <>
            <header className="mb-6 flex items-start justify-between">
              <div>
                <h1 className="text-xl font-medium">{detail.data.summary.name || "(unnamed)"}</h1>
                <div className="mt-1 font-mono text-xs text-muted-foreground">
                  id: {detail.data.summary.id} · input: {detail.data.input_path_snapshot} ({detail.data.summary.input_rows ?? "?"} rows)
                </div>
              </div>
              <div className="flex gap-2">
                <Button onClick={() => setExportOpen(true)} variant="outline">Export</Button>
                <RunButton
                  executionId={id!}
                  lastHandlerDir={detail.data?.summary.last_handler_dir ?? null}
                />
              </div>
              <ExportDialog open={exportOpen} execId={id!} onClose={() => setExportOpen(false)} />
            </header>

            <Tabs defaultValue="attempts">
              <TabsList>
                <TabsTrigger value="attempts">Attempts ({detail.data.attempts.length})</TabsTrigger>
                <TabsTrigger value="rollup">Rollup</TabsTrigger>
                <TabsTrigger value="bindings">Bindings</TabsTrigger>
              </TabsList>

              <TabsContent value="attempts">
                <AttemptsList attempts={detail.data.attempts} execId={id!} />
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

function AttemptsList({
  attempts,
  execId,
}: {
  attempts: import("@/ipc/types").AttemptSummary[];
  execId: string;
}) {
  const [sortDir, setSortDir] = useState<"desc" | "asc">("desc");

  const sorted = useMemo(() => {
    const copy = [...attempts];
    copy.sort((a, b) => {
      const ta = new Date(a.started_at).getTime();
      const tb = new Date(b.started_at).getTime();
      return sortDir === "desc" ? tb - ta : ta - tb;
    });
    return copy;
  }, [attempts, sortDir]);

  if (attempts.length === 0) {
    return (
      <div className="rounded-lg border border-dashed p-10 text-center text-muted-foreground">
        This execution has never been run.
      </div>
    );
  }

  const toggleSort = () => setSortDir((prev) => (prev === "desc" ? "asc" : "desc"));
  const arrow = sortDir === "desc" ? "▼" : "▲";

  return (
    <Table>
      <Thead>
        <Tr>
          <Th>State</Th>
          <Th>Run type</Th>
          <Th>
            <button
              type="button"
              onClick={toggleSort}
              className="inline-flex items-center gap-1 hover:text-foreground"
              aria-label={`Sort by Started ${sortDir === "desc" ? "ascending" : "descending"}`}
            >
              Started <span className="text-muted-foreground text-[10px]">{arrow}</span>
            </button>
          </Th>
          <Th></Th>
        </Tr>
      </Thead>
      <tbody>
        {sorted.map((a) => (
          <Tr key={a.id}>
            <Td>
              <StateChip state={a.state} />
            </Td>
            <Td>{a.run_type}</Td>
            <Td className="font-mono">
              {new Date(a.started_at).toISOString().replace("T", " ").slice(0, 16)}
            </Td>
            <Td>
              <Link to={`/exec/${execId}/attempt/${a.id}`} className="text-primary hover:underline">
                open ⏵
              </Link>
            </Td>
          </Tr>
        ))}
      </tbody>
    </Table>
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
