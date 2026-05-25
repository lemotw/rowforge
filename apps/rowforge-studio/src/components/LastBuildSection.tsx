import { useState } from "react";
import type { BuildOutcome } from "@/ipc/types";

interface Props {
  last_build: BuildOutcome | null;
  pending: boolean;
}

export function LastBuildSection({ last_build, pending }: Props) {
  const [open, setOpen] = useState(false);

  if (pending) {
    return (
      <section className="space-y-2">
        <h2 className="text-sm font-medium uppercase text-muted-foreground">Last build</h2>
        <div className="flex items-center gap-2 text-sm text-muted-foreground">
          <span className="inline-block h-3 w-3 animate-spin rounded-full border-2 border-zinc-400 border-t-transparent" />
          Building…
        </div>
      </section>
    );
  }
  if (!last_build) return null;

  const success = last_build.exit_code === 0;
  const durationMs =
    new Date(last_build.finished_at).getTime() -
    new Date(last_build.started_at).getTime();
  const badgeCls = success
    ? "bg-green-500/15 text-green-300 border-green-500/30"
    : "bg-red-500/15 text-red-300 border-red-500/30";

  return (
    <section className="space-y-2">
      <h2 className="text-sm font-medium uppercase text-muted-foreground">Last build</h2>
      <div className="flex items-center gap-3">
        <span className={`inline-block rounded px-2 py-0.5 text-xs border ${badgeCls}`}>
          {success ? "success" : "failed"}
        </span>
        <span className="text-sm text-muted-foreground">
          exit {last_build.exit_code} · {durationMs} ms · {new Date(last_build.finished_at).toLocaleTimeString()}
        </span>
      </div>
      <button
        onClick={() => setOpen((v) => !v)}
        className="text-xs text-blue-400 hover:underline"
      >
        {open ? "Hide output ▴" : "Show output ▾"}
      </button>
      {open && (
        <pre className="max-h-64 overflow-auto rounded border border-zinc-700 bg-zinc-900 p-2 text-xs font-mono whitespace-pre-wrap">
          {last_build.stdout}
          {last_build.stderr && "\n--- stderr ---\n"}
          {last_build.stderr}
        </pre>
      )}
    </section>
  );
}
