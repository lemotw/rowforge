import { useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { open as shellOpen } from "@tauri-apps/plugin-shell";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useExport } from "@/ipc/use-export";
import { uiErrorMessage, type ExportFormat, type ExecutionId, type UiError } from "@/ipc/types";

export function ExportDialog({
  open,
  execId,
  onClose,
}: {
  open: boolean;
  execId: ExecutionId;
  onClose: () => void;
}) {
  const [outputDir, setOutputDir] = useState<string | null>(null);
  const [format, setFormat] = useState<ExportFormat>("csv");
  const [requireComplete, setRequireComplete] = useState(false);
  const exportMut = useExport();

  if (!open) return null;

  const pickDir = async () => {
    const p = await openDialog({ directory: true });
    if (typeof p === "string") setOutputDir(p);
  };

  const onSubmit = async () => {
    const toastId = toast.loading("Exporting…");
    try {
      const report = await exportMut.mutateAsync({
        id: execId,
        opts: { output_dir: outputDir, format, require_complete: requireComplete },
      });
      toast.dismiss(toastId);
      toast.success(
        `Exported ${report.success_count + report.failed_count} rows to ${report.output_dir}`,
        {
          action: {
            label: "Reveal",
            onClick: () => { shellOpen(report.output_dir); },
          },
        },
      );
      onClose();
    } catch (e) {
      toast.dismiss(toastId);
      // Surface ExportIncomplete with the specific missing-row count;
      // fall back to the generic uiErrorMessage formatter otherwise.
      const err = e as UiError | null;
      if (err && err.kind === "export_incomplete") {
        toast.error(
          `Export incomplete: ${err.message.missing_count} rows unresolved — uncheck 'Require complete' or finish the run first.`
        );
      } else {
        toast.error(`Export failed: ${uiErrorMessage(e)}`);
      }
    }
  };

  // Use a native dialog-style overlay since shadcn Dialog may not be in this codebase.
  return (
    <div
      role="dialog"
      aria-modal="true"
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50"
      onClick={onClose}
    >
      <div
        className="w-full max-w-md rounded-lg border border-zinc-700 bg-zinc-900 p-6 shadow-xl"
        onClick={(e) => e.stopPropagation()}
      >
        <h2 className="mb-4 text-lg font-medium">Export execution</h2>

        <div className="space-y-4">
          <div>
            <label className="mb-1 block text-sm font-medium">Output directory</label>
            <div className="flex gap-2">
              <Input
                value={outputDir ?? ""}
                placeholder="default: <workspace>/exports/…"
                readOnly
              />
              <Button onClick={pickDir} variant="outline">Pick…</Button>
            </div>
          </div>

          <fieldset>
            <legend className="mb-1 block text-sm font-medium">Format</legend>
            <div className="flex gap-4">
              {(["csv", "jsonl", "both"] as const).map((f) => (
                <label key={f} className="flex items-center gap-1.5 text-sm">
                  <input
                    type="radio"
                    name="format"
                    aria-label={f}
                    checked={format === f}
                    onChange={() => setFormat(f)}
                  />
                  {f}
                </label>
              ))}
            </div>
          </fieldset>

          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              aria-label="require complete"
              checked={requireComplete}
              onChange={(e) => setRequireComplete(e.target.checked)}
            />
            Require complete (refuse if any rows are unresolved)
          </label>
        </div>

        <div className="mt-6 flex justify-end gap-2">
          <Button variant="ghost" onClick={onClose}>Cancel</Button>
          <Button onClick={onSubmit} disabled={exportMut.isPending}>
            {exportMut.isPending ? "Exporting…" : "Export"}
          </Button>
        </div>
      </div>
    </div>
  );
}
