import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { AppShell } from "@/layout/AppShell";
import { useStartExec } from "@/ipc/use-start-exec";
import { useWorkspace } from "@/ipc/queries";
import { uiErrorMessage } from "@/ipc/types";

const NAME_RX = /^[a-z0-9_-]{1,64}$/;

function detectFormat(p: string): "csv" | "jsonl" | "ndjson" | null {
  const ext = p.toLowerCase().split(".").pop();
  if (ext === "csv" || ext === "jsonl" || ext === "ndjson") return ext as "csv" | "jsonl" | "ndjson";
  return null;
}

/**
 * Single-step New Execution form.
 *
 * Per the data model (rowforge-core spec part 2): handler is bound at
 * attempt-time, not at exec-creation. The exec just owns the input rows
 * and identity; each Run picks its own handler via RunButton on ExecDetail.
 *
 * Previously this was a 2-step wizard that also picked a handler dir
 * (for inline manifest validation + optional "Start run immediately").
 * Removed because forcing handler selection here misleads users into
 * thinking the handler is exec-scoped.
 */
export function NewExecutionWizardPage() {
  const navigate = useNavigate();
  const ws = useWorkspace();
  const startExec = useStartExec();

  const [name, setName] = useState("");
  const [inputPath, setInputPath] = useState<string | null>(null);

  const detectedFormat = inputPath ? detectFormat(inputPath) : null;
  const formValid = NAME_RX.test(name) && !!inputPath && detectedFormat !== null;

  const pickInput = async () => {
    const p = await openDialog({
      filters: [{ name: "Input", extensions: ["csv", "jsonl", "ndjson"] }],
    });
    if (typeof p === "string") setInputPath(p);
  };

  const onSubmit = async () => {
    if (!inputPath) return;
    try {
      const id = await startExec.mutateAsync({
        input_path: inputPath,
        name,
        csv_id: null,
        pinned_handler_instance: null,
      });
      navigate(`/exec/${id}`);
    } catch (e) {
      // Error surfaces via startExec.error / mutation state below.
      console.error("wizard submit failed:", e);
    }
  };

  return (
    <AppShell
      workspace={ws.data ?? null}
      crumbs={[{ label: "Executions", to: "/" }, { label: "New execution" }]}
    >
      <div className="mx-auto max-w-2xl p-6">
        <h1 className="mb-4 text-xl font-medium">New execution</h1>
        <p className="mb-4 text-sm text-muted-foreground">
          Pick the input file and give the execution a name. The handler is
          chosen per Run on the next page, not here.
        </p>

        <div className="space-y-4">
          <div>
            <label htmlFor="exec-name" className="mb-1 block text-sm font-medium">
              Name
            </label>
            <Input
              id="exec-name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="my-exec-2026-05"
            />
            {name && !NAME_RX.test(name) && (
              <div className="mt-1 text-xs text-red-300">
                must match [a-z0-9_-]+ and be ≤ 64 chars
              </div>
            )}
          </div>

          <div>
            <label className="mb-1 block text-sm font-medium">Input file</label>
            <div className="flex gap-2">
              <Input value={inputPath ?? ""} placeholder="not selected" readOnly />
              <Button onClick={pickInput} variant="outline">Pick…</Button>
            </div>
            {detectedFormat && (
              <span className="mt-1 inline-block rounded bg-zinc-700/40 px-1.5 py-0.5 font-mono text-xs">
                {detectedFormat}
              </span>
            )}
          </div>

          {startExec.isError && (
            <div className="rounded border border-red-500/40 bg-red-500/10 p-3 text-sm text-red-200">
              {uiErrorMessage(startExec.error)}
            </div>
          )}

          <div className="flex justify-between pt-4">
            <Button variant="ghost" onClick={() => navigate("/")}>Cancel</Button>
            <Button onClick={onSubmit} disabled={!formValid || startExec.isPending}>
              {startExec.isPending ? "Creating…" : "Create execution"}
            </Button>
          </div>
        </div>
      </div>
    </AppShell>
  );
}
