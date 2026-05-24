import { useState } from "react";
import { open as shellOpen } from "@tauri-apps/plugin-shell";
import { Button } from "@/components/ui/button";
import { Table, Thead, Tr, Th, Td } from "@/components/ui/table";
import { useFailedPage } from "@/ipc/queries";
import { uiErrorMessage } from "@/ipc/types";
import type { FailedRow } from "@/ipc/types";
import { RowHistoryDrawer } from "./RowHistoryDrawer";

const PAGE_LIMIT = 100;

export function FailedRowsTable({
  executionId,
  attemptId,
  pathsOutcomes,
}: {
  executionId: string;
  attemptId: string;
  pathsOutcomes: string;
}) {
  const [offset, setOffset] = useState(0);
  const [historySeq, setHistorySeq] = useState<number | null>(null);
  const [expanded, setExpanded] = useState<Set<number>>(new Set());

  const q = useFailedPage({
    execution_id: executionId,
    attempt_id: attemptId,
    offset,
    limit: PAGE_LIMIT,
    error_code_filter: null,
  });

  const toggle = (seq: number) =>
    setExpanded((s) => {
      const n = new Set(s);
      if (n.has(seq)) {
        n.delete(seq);
      } else {
        n.add(seq);
      }
      return n;
    });

  return (
    <div>
      <div className="mb-2 flex justify-end gap-2">
        <Button size="sm" variant="ghost" onClick={() => shellOpen(pathsOutcomes)}>
          Reveal outcomes.jsonl
        </Button>
      </div>

      {q.isError && <div className="text-red-300">{uiErrorMessage(q.error)}</div>}

      <Table>
        <Thead>
          <Tr>
            <Th>seq</Th>
            <Th>kind</Th>
            <Th>error_code</Th>
            <Th>message</Th>
            <Th className="text-right">dur_ms</Th>
            <Th></Th>
          </Tr>
        </Thead>
        <tbody>
          {q.data?.rows.map((r: FailedRow) => (
            <FailedRowItem
              key={r.seq}
              row={r}
              expanded={expanded.has(r.seq)}
              onToggle={() => toggle(r.seq)}
              onHistory={() => setHistorySeq(r.seq)}
            />
          ))}
        </tbody>
      </Table>

      <div className="mt-3 flex items-center justify-between">
        <span className="text-sm text-muted-foreground">
          Showing {offset + 1}–{offset + (q.data?.rows.length ?? 0)} of unknown
        </span>
        {q.data?.next_offset != null && (
          <Button
            size="sm"
            variant="outline"
            onClick={() => setOffset(q.data!.next_offset!)}
          >
            Load more
          </Button>
        )}
      </div>

      <RowHistoryDrawer
        executionId={executionId}
        seq={historySeq}
        onClose={() => setHistorySeq(null)}
      />
    </div>
  );
}

function FailedRowItem({
  row,
  expanded,
  onToggle,
  onHistory,
}: {
  row: FailedRow;
  expanded: boolean;
  onToggle: () => void;
  onHistory: () => void;
}) {
  return (
    <>
      <Tr>
        <Td className="font-mono">{row.seq}</Td>
        <Td><KindChip kind={row.kind} /></Td>
        <Td><span className="font-mono text-xs">{row.error_code ?? "—"}</span></Td>
        <Td className="max-w-md truncate" title={row.message ?? ""}>
          {row.message}
        </Td>
        <Td className="text-right tabular-nums">{row.dur_ms}</Td>
        <Td>
          <button onClick={onToggle} className="text-xs text-primary hover:underline">
            {expanded ? "hide" : "raw"}
          </button>
          <button onClick={onHistory} className="ml-2 text-xs text-primary hover:underline">
            history
          </button>
        </Td>
      </Tr>
      {expanded && (
        <Tr>
          <Td colSpan={6} className="bg-neutral-900/50 p-4">
            <pre className="overflow-auto text-xs">
              {JSON.stringify(row.raw_record, null, 2)}
            </pre>
          </Td>
        </Tr>
      )}
    </>
  );
}

function KindChip({ kind }: { kind: string }) {
  const tone =
    kind === "error" ? "text-red-400" :
    kind === "crash" ? "text-red-500" :
    "text-amber-400";
  return <span className={tone}>● {kind}</span>;
}
