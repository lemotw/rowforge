import { CheckCircle2, AlertTriangle, AlertOctagon } from "lucide-react";
import type { ManifestError, ManifestReport, ManifestWarning } from "@/ipc/types";

export function ManifestReportView({ report }: { report: ManifestReport }) {
  // Success state: parsed manifest, no errors, no warnings.
  if (report.errors.length === 0 && report.warnings.length === 0 && report.manifest) {
    return (
      <div className="flex items-center gap-2 rounded border border-green-500/30 bg-green-500/10 p-3 text-sm">
        <CheckCircle2 className="h-4 w-4 text-green-400" />
        <span>
          Manifest valid
          {report.manifest.version && (
            <span className="ml-2 rounded bg-zinc-700/40 px-1.5 py-0.5 font-mono text-xs">
              v{report.manifest.version}
            </span>
          )}
          {report.manifest.language && (
            <span className="ml-1 rounded bg-zinc-700/40 px-1.5 py-0.5 font-mono text-xs">
              {report.manifest.language}
            </span>
          )}
        </span>
      </div>
    );
  }

  // Edge case: no errors, no warnings, no manifest — invalid combination, render nothing.
  if (report.errors.length === 0 && report.warnings.length === 0) {
    return null;
  }

  return (
    <div className="space-y-2">
      {report.errors.length > 0 && (
        <div className="rounded border border-red-500/40 bg-red-500/10 p-3">
          <div className="mb-1 flex items-center gap-2 text-sm font-medium text-red-300">
            <AlertOctagon className="h-4 w-4" />
            {report.errors.length} error{report.errors.length === 1 ? "" : "s"}
          </div>
          <ul className="space-y-1 text-sm">
            {report.errors.map((e, i) => (
              <li key={i} className="font-mono text-xs text-red-200">
                {formatError(e)}
              </li>
            ))}
          </ul>
        </div>
      )}
      {report.warnings.length > 0 && (
        <div className="rounded border border-amber-500/40 bg-amber-500/10 p-3">
          <div className="mb-1 flex items-center gap-2 text-sm font-medium text-amber-300">
            <AlertTriangle className="h-4 w-4" />
            {report.warnings.length} warning{report.warnings.length === 1 ? "" : "s"}
          </div>
          <ul className="space-y-1 text-sm">
            {report.warnings.map((w, i) => (
              <li key={i} className="font-mono text-xs text-amber-200">
                {formatWarning(w)}
              </li>
            ))}
          </ul>
          {report.manifest?.version && (
            <div className="mt-2 text-xs text-zinc-400">
              Parsed manifest:{" "}
              <span className="rounded bg-zinc-700/40 px-1.5 py-0.5 font-mono">
                v{report.manifest.version}
              </span>
              {report.manifest.language && (
                <span className="ml-1 rounded bg-zinc-700/40 px-1.5 py-0.5 font-mono">
                  {report.manifest.language}
                </span>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function formatError(e: ManifestError): string {
  switch (e.kind) {
    case "manifest_missing":
      return `manifest.toml not found at ${e.path}`;
    case "parse_failed":
      return `TOML parse failed: ${e.message}`;
    case "missing_required":
      return `Required field missing: '${e.field}'`;
    case "shell_parse_failed":
      return `Shell parse failed for '${e.field}': ${e.message}`;
  }
}

function formatWarning(w: ManifestWarning): string {
  switch (w.kind) {
    case "path_lookup_failed":
      return `'${w.token}' (from ${w.field}) not found on PATH — may still work on a different machine`;
  }
}
