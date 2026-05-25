import { useEffect, useState } from "react";
import { Navigate, useNavigate, useParams, useSearchParams } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { ipc } from "@/ipc/client";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { open as shellOpen } from "@tauri-apps/plugin-shell";
import { AppShell } from "@/layout/AppShell";
import { useAttemptDetail, useExecDetail, useWorkspace } from "@/ipc/queries";
import { uiErrorMessage } from "@/ipc/types";
import { ErrorsByCodeList } from "@/components/ErrorsByCodeList";
import { FailedRowsTable } from "@/components/FailedRowsTable";
import { ProgressRegion } from "@/components/ProgressRegion";
import { EventTail } from "@/components/EventTail";
import { PhaseChipBar } from "@/components/PhaseChipBar";
import { LifecycleBanners } from "@/components/LifecycleBanner";
import { CancelDialog } from "@/components/CancelDialog";
import { useRun } from "@/ipc/use-run";
import { AttemptLogsTab } from "@/pages/AttemptLogsTab";

export function AttemptDetailPage() {
  const { id, aid } = useParams<{ id: string; aid: string }>();
  const [searchParams] = useSearchParams();
  const runHandle = searchParams.get("run");
  const ws = useWorkspace();
  const detail = useAttemptDetail(id ?? null, aid ?? null);
  // Fetch ExecDetail so the CancelDialog destructive-confirm token can be
  // derived from the human-readable exec name (spec §7.2 #6), not the
  // attempt ulid. Already cached if Exec page was visited.
  const exec = useExecDetail(id ?? null);
  const liveState = useRun(runHandle);
  const navigate = useNavigate();

  // If the user lands on this page without `?run=` in the URL, see whether
  // there's still a live session for this attempt — if so, offer to attach.
  // Skip the query if `?run=` is already present (we're already live) or
  // if attempt id is missing.
  // Skip when ?run= is already in the URL (we're attaching to a known
  // handle), or when attempt_show says this attempt is terminal (no
  // session can possibly be active — polling would just hit None forever).
  const skipActiveHandlePoll = !!runHandle || detail.data?.is_terminal === true;
  const activeHandle = useQuery({
    queryKey: ["attempt_active_handle", aid],
    queryFn: () => ipc.attempt_active_handle({ attemptId: aid! }),
    enabled: !!aid && !skipActiveHandlePoll,
    // Re-poll every 2s while no handle resolved, so users notice when a
    // run starts (e.g. from CLI in another terminal) without manual refresh.
    refetchInterval: skipActiveHandlePoll ? false : 2000,
  });

  // Compute whether we should treat this view as terminal. Three signals:
  // 1. attempt_show says is_terminal (sqlite-backed truth)
  // 2. liveState got a Done / Aborted / Crashed event (terminal observed live)
  // 3. liveState.phantomBootstrap (snapshot returned UnknownHandle — run was
  //    gone from the registry by the time we asked; happens for fast runs)
  const liveTerminal =
    liveState.status === "done" ||
    liveState.status === "aborted" ||
    liveState.status === "crashed";
  const isTerminal =
    detail.data?.is_terminal === true ||
    liveTerminal ||
    liveState.phantomBootstrap;

  // When we detect the run is done but attempt_show still says it's running
  // (race: AttemptDetail mounted between run start and finish), force a
  // refetch so the static panels reflect the final stats.
  useEffect(() => {
    if (
      (liveTerminal || liveState.phantomBootstrap) &&
      detail.data &&
      !detail.data.is_terminal
    ) {
      detail.refetch();
    }
  }, [liveTerminal, liveState.phantomBootstrap, detail.data?.is_terminal]);

  // Tab selection is controlled so we can auto-switch in two directions:
  // 1. terminal observed mid-page → switch from Live to Summary
  // 2. runHandle appears (Watch-live click added ?run=<h> to the URL while
  //    we were on Summary) → switch to Live
  // defaultValue alone wouldn't re-evaluate after initial mount.
  const [tab, setTab] = useState<string>(runHandle ? "live" : "summary");
  useEffect(() => {
    if (isTerminal && tab === "live") setTab("summary");
  }, [isTerminal]);
  useEffect(() => {
    if (runHandle && !isTerminal && tab !== "live") setTab("live");
  }, [runHandle, isTerminal]);

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

            {/* Live mode header — only while the run is in flight */}
            {runHandle && !isTerminal && (
              <div className="mb-4 flex items-center justify-between">
                <PhaseChipBar current={liveState.phase} />
                <CancelDialog
                  handle={runHandle}
                  status={liveState.status}
                  execName={exec.data?.summary.name ?? id ?? ""}
                />
              </div>
            )}

            {runHandle && !isTerminal && <LifecycleBanners banners={liveState.banners} />}

            {/* Fast-run notice — run finished before listener attached */}
            {runHandle && liveState.phantomBootstrap && (
              <div className="mb-4 rounded border border-blue-500/40 bg-blue-500/10 p-3 text-sm text-blue-200">
                ✓ Run completed before live updates could attach.
                Showing final results from the Summary tab.
              </div>
            )}

            {/* Live-run available banner — re-attach to a live session that
                we landed on without ?run= in the URL */}
            {!runHandle && activeHandle.data && (
              <div className="mb-4 flex items-center justify-between rounded border border-emerald-500/40 bg-emerald-500/10 p-3 text-sm text-emerald-200">
                <span>● Live run in progress for this attempt.</span>
                <button
                  onClick={() =>
                    navigate(
                      `/exec/${id}/attempt/${aid}?run=${activeHandle.data}`,
                      { replace: true }
                    )
                  }
                  className="rounded bg-emerald-500/20 px-2 py-0.5 underline"
                >
                  Watch live →
                </button>
              </div>
            )}

            {/* Stale banner — only when no runHandle, no active session, AND attempt is non-terminal */}
            {!runHandle && !activeHandle.data && !detail.data.is_terminal && (
              <div className="mb-4 rounded border border-amber-500/40 bg-amber-500/10 p-3 text-sm text-amber-200">
                ⚠ This attempt may still be running. Snapshot may be stale.{" "}
                <button onClick={() => detail.refetch()} className="underline ml-1">
                  Refresh manually
                </button>
              </div>
            )}

            <Tabs value={tab} onValueChange={setTab}>
              <TabsList>
                {runHandle && !isTerminal && <TabsTrigger value="live">Live</TabsTrigger>}
                <TabsTrigger value="summary">Summary</TabsTrigger>
                <TabsTrigger value="failed">Failed rows</TabsTrigger>
                <TabsTrigger value="errors">Errors by code</TabsTrigger>
                <TabsTrigger value="logs">Logs</TabsTrigger>
                <TabsTrigger value="artifacts">Artifacts</TabsTrigger>
              </TabsList>

              {runHandle && !isTerminal && (
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

              <TabsContent value="logs">
                <AttemptLogsTab
                  execId={id!}
                  attemptId={aid!}
                  isLive={!isTerminal}
                  logFilePath={
                    workspace
                      ? `${workspace.root}/executions/${id}/attempts/${aid}/handler_log.log`
                      : undefined
                  }
                />
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

