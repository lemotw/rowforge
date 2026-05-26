import { useEffect, useMemo, useState } from "react";
import { open as dialogOpen } from "@tauri-apps/plugin-dialog";
import { Button } from "@/components/ui/button";
import {
  useHandlerSmokeRun,
  useHandlerSmokeLoadFixtures,
} from "@/ipc/use-handlers";
import { uiErrorMessage, type SmokeOutcome } from "@/ipc/types";

type Source = "paste" | "fixtures" | "synthetic";

interface Props {
  handlerName: string;
  defaultRows: number;
}

export function SmokeSection({ handlerName, defaultRows }: Props) {
  const smoke = useHandlerSmokeRun();
  const loadFixtures = useHandlerSmokeLoadFixtures();

  const [source, setSource] = useState<Source>("paste");
  const [pasted, setPasted] = useState("");
  const [fixturePath, setFixturePath] = useState<string | null>(null);
  const [loadedRows, setLoadedRows] = useState<Record<string, unknown>[] | null>(null);
  const [rowCount, setRowCount] = useState(defaultRows);

  // Parse pasted JSON lines.
  const parsedPaste = useMemo(() => {
    if (source !== "paste") return { rows: [], error: null as string | null };
    const lines = pasted.split("\n").map((l) => l.trim()).filter(Boolean);
    const rows: Record<string, unknown>[] = [];
    for (let i = 0; i < lines.length; i++) {
      try {
        const v = JSON.parse(lines[i]);
        if (typeof v !== "object" || v === null || Array.isArray(v)) {
          return { rows: [], error: `line ${i + 1}: not a JSON object` };
        }
        rows.push(v as Record<string, unknown>);
      } catch (e) {
        return {
          rows: [],
          error: `line ${i + 1}: ${(e as Error).message}`,
        };
      }
    }
    return { rows, error: null };
  }, [source, pasted]);

  // Reset loaded rows when source changes away from fixtures.
  useEffect(() => {
    if (source !== "fixtures") {
      setLoadedRows(null);
      setFixturePath(null);
    }
  }, [source]);

  const availableRows: Record<string, unknown>[] = useMemo(() => {
    if (source === "paste") return parsedPaste.rows;
    if (source === "fixtures") return loadedRows ?? [];
    return [{ row: 1 }];
  }, [source, parsedPaste.rows, loadedRows]);

  const effectiveRows = availableRows.slice(0, Math.min(rowCount, 100));
  const canRun =
    effectiveRows.length > 0 &&
    parsedPaste.error == null &&
    !smoke.isPending &&
    !loadFixtures.isPending;

  const pickFixture = async () => {
    const path = await dialogOpen({ directory: false, multiple: false });
    if (typeof path !== "string") return;
    setFixturePath(path);
    loadFixtures.mutate(
      { path, limit: 100 },
      {
        onSuccess: (rows) => setLoadedRows(rows),
      },
    );
  };

  const runSmoke = () => {
    smoke.mutate({
      handler_name: handlerName,
      rows: effectiveRows,
    });
  };

  return (
    <div className="space-y-3">
      <h2 className="text-sm font-medium uppercase text-muted-foreground">
        Smoke test
      </h2>

      <div className="rounded border border-zinc-700 p-4 space-y-4">
        <div className="flex gap-4 text-sm">
          {(["paste", "fixtures", "synthetic"] as const).map((s) => (
            <label key={s} className="flex items-center gap-2 cursor-pointer">
              <input
                type="radio"
                name="smoke-source"
                checked={source === s}
                onChange={() => setSource(s)}
              />
              <span>
                {s === "paste"
                  ? "Paste JSON"
                  : s === "fixtures"
                    ? "Fixtures…"
                    : "One synthetic row"}
              </span>
            </label>
          ))}
        </div>

        {source === "paste" && (
          <div className="space-y-1">
            <textarea
              value={pasted}
              onChange={(e) => setPasted(e.target.value)}
              placeholder={`{"id":"1","email":"a@example.com"}\n{"id":"2","email":"b@example.com"}`}
              className="w-full h-32 rounded border border-zinc-700 bg-zinc-900 p-2 font-mono text-xs"
            />
            {parsedPaste.error ? (
              <div className="text-xs text-red-300">{parsedPaste.error}</div>
            ) : (
              <div className="text-xs text-muted-foreground">
                {parsedPaste.rows.length} row
                {parsedPaste.rows.length === 1 ? "" : "s"} parsed
              </div>
            )}
          </div>
        )}

        {source === "fixtures" && (
          <div className="space-y-2">
            <Button onClick={pickFixture} variant="outline" size="sm">
              {fixturePath ? "Change…" : "Pick file…"}
            </Button>
            {fixturePath && (
              <code
                className="block break-all rounded border border-zinc-700 bg-zinc-900 px-2 py-1 text-xs text-muted-foreground"
                title={fixturePath}
              >
                {fixturePath}
              </code>
            )}
            {loadFixtures.isPending && (
              <div className="text-xs text-muted-foreground">Loading…</div>
            )}
            {loadFixtures.isError && (
              <div className="text-xs text-red-300">
                {uiErrorMessage(loadFixtures.error)}
              </div>
            )}
            {loadedRows && (
              <div className="text-xs text-muted-foreground">
                {loadedRows.length} row{loadedRows.length === 1 ? "" : "s"}{" "}
                loaded — keys:{" "}
                {Object.keys(loadedRows[0] ?? {}).slice(0, 4).join(", ") ||
                  "(empty)"}
              </div>
            )}
          </div>
        )}

        {source === "synthetic" && (
          <div className="text-xs text-muted-foreground">
            Dispatches a single row{" "}
            <code className="font-mono">{"{ \"row\": 1 }"}</code> — useful for
            verifying the binary starts at all.
          </div>
        )}

        <div className="flex items-center gap-3">
          <label className="text-sm">Rows to run:</label>
          <input
            type="number"
            min={1}
            max={100}
            value={rowCount}
            onChange={(e) =>
              setRowCount(
                Math.max(1, Math.min(100, parseInt(e.target.value, 10) || 1)),
              )
            }
            className="w-20 rounded border border-zinc-700 bg-zinc-900 px-2 py-1 text-sm"
          />
          <span className="text-xs text-muted-foreground">(max 100)</span>
          <div className="flex-1" />
          <Button onClick={runSmoke} disabled={!canRun}>
            {smoke.isPending ? "Running…" : "Run smoke test"}
          </Button>
        </div>

        {smoke.isError && (
          <div className="rounded border border-red-500/40 bg-red-500/10 p-2 text-sm text-red-200">
            {uiErrorMessage(smoke.error)}
          </div>
        )}

        {smoke.data && <SmokeResults result={smoke.data} />}
      </div>
    </div>
  );
}

function SmokeResults({
  result,
}: {
  result: import("@/ipc/types").SmokeRunResult;
}) {
  const counts = result.outcomes.reduce(
    (acc, o) => {
      acc[o.status] = (acc[o.status] ?? 0) + 1;
      return acc;
    },
    {} as Record<string, number>,
  );

  return (
    <div className="space-y-3">
      <div className="text-xs text-muted-foreground">
        Outcomes ({result.outcomes.length})
        {counts.success ? ` · ✓ ${counts.success} success` : ""}
        {counts.error ? ` · ✗ ${counts.error} error` : ""}
        {counts.crash ? ` · ⚠ ${counts.crash} crash` : ""}
        {" · "}
        {result.elapsed_ms} ms
        {result.exit_code != null && ` · exit ${result.exit_code}`}
      </div>
      <div className="overflow-x-auto rounded border border-zinc-700">
        <table className="w-full text-xs">
          <thead className="bg-zinc-900">
            <tr>
              <Th>seq</Th>
              <Th>status</Th>
              <Th>message</Th>
              <Th>dur_ms</Th>
              <Th>data</Th>
            </tr>
          </thead>
          <tbody>
            {result.outcomes.map((o) => (
              <OutcomeRow key={o.seq} o={o} />
            ))}
          </tbody>
        </table>
      </div>
      {result.stderr_tail && (
        <details>
          <summary className="text-xs text-muted-foreground cursor-pointer">
            stderr tail ({result.stderr_tail.length} B)
          </summary>
          <pre className="mt-2 max-h-64 overflow-auto rounded border border-zinc-700 bg-zinc-900 p-2 text-xs whitespace-pre-wrap">
            {result.stderr_tail}
          </pre>
        </details>
      )}
    </div>
  );
}

function OutcomeRow({ o }: { o: SmokeOutcome }) {
  const statusColor =
    o.status === "success"
      ? "text-green-300"
      : o.status === "error"
        ? "text-yellow-300"
        : "text-red-300";
  const dataPreview = o.data
    ? JSON.stringify(o.data).slice(0, 80)
    : "—";
  return (
    <tr className="border-t border-zinc-800">
      <Td>{o.seq}</Td>
      <Td>
        <span className={statusColor}>{o.status}</span>
      </Td>
      <Td>{o.message ?? (o.code ? `${o.code}` : "—")}</Td>
      <Td>{o.dur_ms}</Td>
      <Td>
        <code
          className="font-mono break-all"
          title={o.data ? JSON.stringify(o.data) : undefined}
        >
          {dataPreview}
        </code>
      </Td>
    </tr>
  );
}

function Th({ children }: { children: React.ReactNode }) {
  return <th className="px-2 py-1 text-left font-medium">{children}</th>;
}

function Td({ children }: { children: React.ReactNode }) {
  return <td className="px-2 py-1 align-top">{children}</td>;
}
