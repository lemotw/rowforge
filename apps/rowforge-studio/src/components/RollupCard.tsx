import { useState } from "react";
import { useExecRollup } from "@/ipc/queries";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { uiErrorMessage } from "@/ipc/types";
import { ErrorsByCodeList } from "./ErrorsByCodeList";

export function RollupCard({ executionId }: { executionId: string }) {
  const [enabled, setEnabled] = useState(false);
  const q = useExecRollup(executionId, enabled);
  if (!enabled) {
    return (
      <div className="rounded-lg border border-border p-6">
        <p className="mb-3 text-sm text-muted-foreground">
          Rollup folds outcomes from every attempt; this can take a few seconds.
        </p>
        <Button onClick={() => setEnabled(true)}>Compute rollup</Button>
      </div>
    );
  }
  if (q.isLoading) return <Skeleton className="h-32 w-full" />;
  if (q.isError) return <div className="text-red-300">{uiErrorMessage(q.error)}</div>;
  if (!q.data) return null;
  const r = q.data;
  return (
    <div className="space-y-4">
      <div className="grid grid-cols-6 gap-3">
        <Stat label="resolved" value={r.resolved} tone="text-emerald-400" />
        <Stat label="failed_last" value={r.failed_last} tone="text-red-400" />
        <Stat label="crashed_last" value={r.crashed_last} tone="text-red-500" />
        <Stat label="cancelled_last" value={r.cancelled_last} tone="text-neutral-400" />
        <Stat label="too_large" value={r.too_large} tone="text-amber-400" />
        <Stat label="never_attempted" value={r.never_attempted} tone="text-neutral-500" />
      </div>
      {Object.keys(r.by_error_code).length > 0 && (
        <div className="mt-6">
          <h3 className="mb-2 text-sm font-medium text-muted-foreground">
            By error code
          </h3>
          <ErrorsByCodeList data={r.by_error_code} />
        </div>
      )}
    </div>
  );
}

function Stat({ label, value, tone }: { label: string; value: number; tone: string }) {
  return (
    <div className="rounded-lg border border-border p-4">
      <div className={`text-2xl font-medium tabular-nums ${tone}`}>{value}</div>
      <div className="mt-1 text-xs text-muted-foreground">{label}</div>
    </div>
  );
}
