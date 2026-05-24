import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { AppShell } from "@/layout/AppShell";
import { ManifestReportView } from "@/components/ManifestReportView";
import { useStartExec } from "@/ipc/use-start-exec";
import { useManifestValidate } from "@/ipc/use-manifest-validate";
import { useWorkspace } from "@/ipc/queries";
import { ipc } from "@/ipc/client";
import { uiErrorMessage } from "@/ipc/types";

const NAME_RX = /^[a-z0-9_-]{1,64}$/;

function detectFormat(p: string): "csv" | "jsonl" | "ndjson" | null {
  const ext = p.toLowerCase().split(".").pop();
  if (ext === "csv" || ext === "jsonl" || ext === "ndjson") return ext as "csv" | "jsonl" | "ndjson";
  return null;
}

export function NewExecutionWizardPage() {
  const navigate = useNavigate();
  const ws = useWorkspace();
  const startExec = useStartExec();
  const validate = useManifestValidate();

  const [step, setStep] = useState<1 | 2>(1);
  const [name, setName] = useState("");
  const [inputPath, setInputPath] = useState<string | null>(null);
  const [handlerDir, setHandlerDir] = useState<string | null>(null);
  const [startImmediately, setStartImmediately] = useState(false);

  const detectedFormat = inputPath ? detectFormat(inputPath) : null;
  const step1Valid = NAME_RX.test(name) && !!inputPath && detectedFormat !== null;
  const validateClean = !!validate.data && validate.data.errors.length === 0;

  const pickInput = async () => {
    const p = await openDialog({
      filters: [{ name: "Input", extensions: ["csv", "jsonl", "ndjson"] }],
    });
    if (typeof p === "string") setInputPath(p);
  };

  const pickHandlerDir = async () => {
    const p = await openDialog({ directory: true });
    if (typeof p === "string") setHandlerDir(p);
  };

  const onValidate = () => {
    if (!handlerDir) return;
    validate.mutate({ type: "path", path: handlerDir });
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
      if (startImmediately && handlerDir) {
        const started = await ipc.run_start({ executionId: id, handlerDir });
        navigate(`/exec/${id}/attempt/${started.attempt_id}?run=${started.handle}`);
      } else {
        navigate(`/exec/${id}`);
      }
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
        <div className="mb-4 flex items-center gap-2 text-sm text-muted-foreground">
          <span className={step === 1 ? "font-medium text-foreground" : ""}>1. Identity + input</span>
          <span>→</span>
          <span className={step === 2 ? "font-medium text-foreground" : ""}>2. Handler</span>
        </div>

        {step === 1 && (
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

            <div className="flex justify-between pt-4">
              <Button variant="ghost" onClick={() => navigate("/")}>Cancel</Button>
              <Button onClick={() => setStep(2)} disabled={!step1Valid}>Next</Button>
            </div>
          </div>
        )}

        {step === 2 && (
          <div className="space-y-4">
            <div>
              <label className="mb-1 block text-sm font-medium">Handler directory</label>
              <div className="flex gap-2">
                <Input value={handlerDir ?? ""} placeholder="not selected" readOnly />
                <Button onClick={pickHandlerDir} variant="outline">Pick…</Button>
                <Button onClick={onValidate} disabled={!handlerDir || validate.isPending}>
                  {validate.isPending ? "Validating…" : "Validate"}
                </Button>
              </div>
            </div>

            {validate.data && <ManifestReportView report={validate.data} />}

            <div className="flex items-center gap-2">
              <input
                type="checkbox"
                id="start-immediately"
                checked={startImmediately}
                onChange={(e) => setStartImmediately(e.target.checked)}
                className="h-4 w-4 rounded border border-zinc-600 bg-zinc-800 accent-blue-500"
              />
              <label htmlFor="start-immediately" className="text-sm">
                Start a run immediately after creation
              </label>
            </div>

            {startExec.isError && (
              <div className="rounded border border-red-500/40 bg-red-500/10 p-3 text-sm text-red-200">
                {uiErrorMessage(startExec.error)}
              </div>
            )}

            <div className="flex justify-between pt-4">
              <Button variant="ghost" onClick={() => setStep(1)}>Back</Button>
              <Button onClick={onSubmit} disabled={!validateClean || startExec.isPending}>
                {startExec.isPending ? "Creating…" : "Create execution"}
              </Button>
            </div>
          </div>
        )}
      </div>
    </AppShell>
  );
}
