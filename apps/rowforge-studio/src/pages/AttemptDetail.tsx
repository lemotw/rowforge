import { Navigate, useParams, useSearchParams } from "react-router-dom";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { open as shellOpen } from "@tauri-apps/plugin-shell";
import { AppShell } from "@/layout/AppShell";
import { useAttemptDetail, useWorkspace } from "@/ipc/queries";
import { uiErrorMessage } from "@/ipc/types";
import { ErrorsByCodeList } from "@/components/ErrorsByCodeList";
import { FailedRowsTable } from "@/components/FailedRowsTable";
import { ProgressRegion } from "@/components/ProgressRegion";
import { EventTail } from "@/components/EventTail";
import { PhaseChipBar } from "@/components/PhaseChipBar";
import { LifecycleBanners } from "@/components/LifecycleBanner";
import { CancelDialog } from "@/components/CancelDialog";
import { ReplayToggle } from "@/components/ReplayToggle";
import { useRun } from "@/ipc/use-run";

export function AttemptDetailPage() {
  const { id, aid } = useParams<{ id: string; aid: string }>();
  const [searchParams] = useSearchParams();
  const runHandle = searchParams.get("run");
  const ws = useWorkspace();
  const detail = useAttemptDetail(id ?? null, aid ?? null);
  const liveState = useRun(runHandle);

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

            {/* Live mode header — only when ?run= param is present */}
            {runHandle && (
              <div className="mb-4 flex items-center justify-between">
                <PhaseChipBar current={liveState.phase} />
                <CancelDialog
                  handle={runHandle}
                  status={liveState.status}
                  execName={detail.data.id ?? id ?? ""}
                />
              </div>
            )}

            {runHandle && <LifecycleBanners banners={liveState.banners} />}

            {/* Replay toggle — only when terminal and not currently replaying */}
            {detail.data.is_terminal && !runHandle && (
              <div className="mb-4">
                <ReplayToggle executionId={id!} attemptId={aid!} />
              </div>
            )}

            {/* Stale banner — only when no runHandle AND attempt is non-terminal */}
            {!runHandle && !detail.data.is_terminal && (
              <div className="mb-4 rounded border border-amber-500/40 bg-amber-500/10 p-3 text-sm text-amber-200">
                ⚠ This attempt may still be running. Snapshot may be stale.{" "}
                <button onClick={() => detail.refetch()} className="underline ml-1">
                  Refresh manually
                </button>
              </div>
            )}

            <Tabs defaultValue={runHandle ? "live" : "summary"}>
              <TabsList>
                {runHandle && <TabsTrigger value="live">Live</TabsTrigger>}
                <TabsTrigger value="summary">Summary</TabsTrigger>
                <TabsTrigger value="failed">Failed rows</TabsTrigger>
                <TabsTrigger value="errors">Errors by code</TabsTrigger>
                <TabsTrigger value="artifacts">Artifacts</TabsTrigger>
              </TabsList>

              {runHandle && (
                <TabsContent value="live">
                  <div className="space-y-4">
                    <ProgressRegion state={liveState} />
                    <EventTail samples={liveState.recentSamples} />
                  </div>
                </TabsContent>
              )}

              <TabsContent value="summary">
                <div className="grid grid-cols-3 gap-3">
                  <Stat label="success" value={detail.data.stats.success} tone="text-emerald-400" />
                  <Stat label="failed" value={detail.data.stats.failed} tone="text-red-400" />
                  <Stat label="crashed" value={detail.data.stats.crashed} tone="text-red-500" />
                </div>
              </TabsContent>

              <TabsContent value="failed">
                <FailedRowsTable
                  executionId={id!}
                  attemptId={aid!}
                  pathsOutcomes={detail.data.paths.outcomes_jsonl}
                />
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

