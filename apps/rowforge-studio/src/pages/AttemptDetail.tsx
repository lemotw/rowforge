import { Navigate, useParams } from "react-router-dom";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { open as shellOpen } from "@tauri-apps/plugin-shell";
import { AppShell } from "@/layout/AppShell";
import { useAttemptDetail, useWorkspace } from "@/ipc/queries";
import { uiErrorMessage } from "@/ipc/types";
import { ErrorsByCodeList } from "@/components/ErrorsByCodeList";

export function AttemptDetailPage() {
  const { id, aid } = useParams<{ id: string; aid: string }>();
  const ws = useWorkspace();
  const detail = useAttemptDetail(id ?? null, aid ?? null);

  if (ws.data === null && !ws.isLoading) return <Navigate to="/" replace />;
  const workspace = ws.data ?? null;
  const crumbs = [
    { label: "Executions", to: "/" },
    { label: id ?? "...", to: `/exec/${id}`, mono: true },
    { label: `Attempt ${aid}`, mono: true },
  ];

  return (
    <AppShell workspace={workspace} crumbs={crumbs}>
      <div className="p-6">
        {detail.isLoading && <Skeleton className="h-32 w-full" />}
        {detail.isError && <div className="text-red-300">{uiErrorMessage(detail.error)}</div>}
        {detail.data && (
          <>
            <header className="mb-4">
              <h1 className="text-xl font-medium">Attempt {detail.data.id}</h1>
              <div className="mt-1 text-sm text-muted-foreground">
                state: {detail.data.state} · run type: {detail.data.run_type} ·
                started {new Date(detail.data.started_at).toISOString().slice(0, 19)}
              </div>
            </header>

            {!detail.data.is_terminal && (
              <div className="mb-4 rounded border border-amber-500/40 bg-amber-500/10 p-3 text-sm text-amber-200">
                ⚠ This attempt may still be running. Snapshot may be stale.{" "}
                <button onClick={() => detail.refetch()} className="underline">
                  Refresh manually
                </button>{" "}
                · live progress arrives in Plan 4.
              </div>
            )}

            <Tabs defaultValue="summary">
              <TabsList>
                <TabsTrigger value="summary">Summary</TabsTrigger>
                <TabsTrigger value="failed">Failed rows</TabsTrigger>
                <TabsTrigger value="errors">Errors by code</TabsTrigger>
                <TabsTrigger value="artifacts">Artifacts</TabsTrigger>
              </TabsList>

              <TabsContent value="summary">
                <div className="grid grid-cols-3 gap-3">
                  <Stat label="success" value={detail.data.stats.success} tone="text-emerald-400" />
                  <Stat label="failed" value={detail.data.stats.failed} tone="text-red-400" />
                  <Stat label="crashed" value={detail.data.stats.crashed} tone="text-red-500" />
                </div>
              </TabsContent>

              <TabsContent value="failed">
                <FailedRowsPlaceholder />
              </TabsContent>

              <TabsContent value="errors">
                <ErrorsByCodeList data={detail.data.by_error_code} />
              </TabsContent>

              <TabsContent value="artifacts">
                <ul className="space-y-2 text-sm">
                  {Object.entries(detail.data.paths).map(([k, v]) => (
                    <li key={k} className="flex items-center gap-2">
                      <span className="font-mono text-muted-foreground">{k}:</span>
                      <span className="font-mono">{v}</span>
                      <Button size="sm" variant="ghost" onClick={() => shellOpen(v)}>
                        Reveal
                      </Button>
                    </li>
                  ))}
                </ul>
              </TabsContent>
            </Tabs>
          </>
        )}
      </div>
    </AppShell>
  );
}

function Stat({ label, value, tone }: { label: string; value: number; tone: string }) {
  return (
    <div className="rounded-lg border border-border p-4">
      <div className={`text-2xl tabular-nums ${tone}`}>{value}</div>
      <div className="mt-1 text-xs text-muted-foreground">{label}</div>
    </div>
  );
}

function FailedRowsPlaceholder() {
  return (
    <div className="rounded-lg border border-dashed border-border p-10 text-center text-muted-foreground">
      Failed rows table arrives in Task 17.
    </div>
  );
}
