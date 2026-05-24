import { Sheet, SheetContent, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { Skeleton } from "@/components/ui/skeleton";
import { useRowHistory } from "@/ipc/queries";
import { uiErrorMessage } from "@/ipc/types";

export function RowHistoryDrawer({
  executionId,
  seq,
  onClose,
}: {
  executionId: string;
  seq: number | null;
  onClose: () => void;
}) {
  const q = useRowHistory(executionId, seq);

  return (
    <Sheet open={seq !== null} onOpenChange={(o) => { if (!o) onClose(); }}>
      <SheetContent>
        <SheetHeader>
          <SheetTitle>Row history · seq {seq}</SheetTitle>
        </SheetHeader>
        {q.isLoading && <Skeleton className="h-32 w-full" />}
        {q.isError && <div className="text-red-300">{uiErrorMessage(q.error)}</div>}
        {q.data && (
          <div className="mt-4 space-y-2 text-sm">
            {q.data.resolved_at && (
              <div className="text-emerald-400">
                ✓ resolved at attempt {q.data.resolved_at}
              </div>
            )}
            {q.data.rows.length === 0 ? (
              <div className="text-muted-foreground">
                No prior failed attempts for this row.
              </div>
            ) : (
              <ul className="space-y-1 font-mono text-xs">
                {q.data.rows.map(([att, kind, code], i) => (
                  <li key={i}>
                    attempt {att}: <span className="text-red-400">{kind}</span>
                    {code && <> · {code}</>}
                  </li>
                ))}
              </ul>
            )}
          </div>
        )}
      </SheetContent>
    </Sheet>
  );
}
