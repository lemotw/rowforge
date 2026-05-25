import { useEffect } from "react";
import { Link, useParams } from "react-router-dom";
import { useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { Button } from "@/components/ui/button";
import { useWorkspace } from "@/ipc/queries";
import { AppShell } from "@/layout/AppShell";
import {
  useHandlerShow,
  useHandlerOpenEditor,
  useHandlerReveal,
} from "@/ipc/use-handlers";
import {
  uiErrorMessage,
  type ManifestStatus,
  type HandlerDetail,
  type SourceFileSummary,
} from "@/ipc/types";

export function HandlerDetailPage() {
  const { name = "" } = useParams<{ name: string }>();
  const qc = useQueryClient();
  const ws = useWorkspace();
  const { data, isLoading, isError, error } = useHandlerShow(name);
  const openEditor = useHandlerOpenEditor();
  const reveal = useHandlerReveal();

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    listen("handlers:list", () => {
      qc.invalidateQueries({ queryKey: ["handler_show", name] });
      qc.invalidateQueries({ queryKey: ["handler_list"] });
    }).then((u) => {
      unlisten = u;
    });
    return () => {
      unlisten?.();
    };
  }, [qc, name]);

  const workspace = ws.data ?? null;

  if (isLoading) {
    return (
      <AppShell workspace={workspace}>
        <div className="p-6 text-muted-foreground">Loading handler…</div>
      </AppShell>
    );
  }

  if (isError) {
    const errAny = error as { kind?: string } | undefined;
    if (errAny?.kind === "handler_not_found") {
      return (
        <AppShell workspace={workspace}>
          <div className="p-6 space-y-4">
            <div className="text-red-300">
              Handler "{name}" not found. It may have been deleted or renamed.
            </div>
            <Link to="/handlers" className="text-blue-400 hover:underline">
              ← Back to handlers
            </Link>
          </div>
        </AppShell>
      );
    }
    return (
      <AppShell workspace={workspace}>
        <div className="p-6 text-red-300">
          Failed to load handler: {uiErrorMessage(error)}
        </div>
      </AppShell>
    );
  }

  if (!data) return null;

  return (
    <AppShell workspace={workspace}>
      <div className="p-6 space-y-6">
        <DetailHeader
          detail={data}
          onOpenEditor={() => openEditor.mutate({ name })}
          onReveal={() => reveal.mutate({ name })}
        />
        <ManifestSection detail={data} />
        <SourceFilesSection detail={data} />
      </div>
    </AppShell>
  );
}

function DetailHeader({
  detail,
  onOpenEditor,
  onReveal,
}: {
  detail: HandlerDetail;
  onOpenEditor: () => void;
  onReveal: () => void;
}) {
  return (
    <div className="space-y-3">
      <Link
        to="/handlers"
        className="text-sm text-muted-foreground hover:underline"
      >
        ← Handlers
      </Link>
      <div className="flex items-start justify-between">
        <div>
          <h1 className="font-mono text-2xl font-semibold">
            {detail.summary.name}
          </h1>
          <div className="text-sm text-muted-foreground mt-1">
            {detail.summary.path}
          </div>
        </div>
        <div className="flex gap-2 flex-wrap justify-end">
          <Button onClick={onOpenEditor}>Open in editor</Button>
          <Button variant="outline" onClick={onReveal}>
            Reveal
          </Button>
          <Button
            variant="outline"
            onClick={() =>
              // TODO(T14): open RenameDialog
              console.warn("RenameDialog not yet wired (Plan 7 T14)")
            }
          >
            Rename…
          </Button>
          <Button
            variant="outline"
            onClick={() =>
              // TODO(T14): open DeleteDialog
              console.warn("DeleteDialog not yet wired (Plan 7 T14)")
            }
          >
            Delete…
          </Button>
        </div>
      </div>
    </div>
  );
}

function ManifestSection({ detail }: { detail: HandlerDetail }) {
  const { summary, manifest, manifest_errors, manifest_warnings } = detail;
  return (
    <Section title="Manifest">
      <div className="flex items-center gap-3 mb-3">
        <StatusBadge status={summary.manifest_status} />
        {summary.version && (
          <span className="text-sm text-muted-foreground">
            v{summary.version}
          </span>
        )}
        {summary.language && (
          <span className="text-sm text-muted-foreground">
            {summary.language}
          </span>
        )}
      </div>

      {summary.manifest_status === "missing" && (
        <div className="text-sm text-muted-foreground">
          No rowforge.yaml in this handler directory.
        </div>
      )}

      {summary.manifest_status === "invalid" && manifest_errors.length > 0 && (
        <div className="space-y-1">
          <div className="text-sm font-medium text-red-300">Errors</div>
          <ul className="rounded border border-red-500/30 bg-red-500/10 p-3 text-sm text-red-200 list-disc pl-6">
            {manifest_errors.map((e, i) => (
              <li key={i}>
                {typeof e === "string"
                  ? e
                  : (e as { message?: string }).message ?? JSON.stringify(e)}
              </li>
            ))}
          </ul>
        </div>
      )}

      {summary.manifest_status === "valid" && manifest && (
        <ManifestSummary manifest={manifest} />
      )}

      {manifest_warnings.length > 0 && (
        <div className="space-y-1 mt-3">
          <div className="text-sm font-medium text-yellow-300">Warnings</div>
          <ul className="rounded border border-yellow-500/30 bg-yellow-500/10 p-3 text-sm text-yellow-200 list-disc pl-6">
            {manifest_warnings.map((w, i) => (
              <li key={i}>
                {typeof w === "string"
                  ? w
                  : (w as { message?: string }).message ?? JSON.stringify(w)}
              </li>
            ))}
          </ul>
        </div>
      )}
    </Section>
  );
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function ManifestSummary({ manifest }: { manifest: any }) {
  // Render whichever fields exist. Be defensive — the rowforge-core Manifest
  // shape may vary (kind/entry/primary_field etc.) and differs from the
  // exec-side Manifest declared in types.ts. Type-erased locally to avoid noise.
  const rows: Array<[string, string]> = [];

  if (manifest.kind) rows.push(["kind", String(manifest.kind)]);
  if (manifest.primary_field)
    rows.push(["primary_field", String(manifest.primary_field)]);
  if (manifest.batch_size != null)
    rows.push(["batch_size", String(manifest.batch_size)]);
  if (manifest.row_timeout)
    rows.push(["row_timeout", String(manifest.row_timeout)]);

  // Handle both rowforge-core entry shape (entry.cmd array) and
  // exec-side shape (entry_cmd array).
  if (manifest.entry?.cmd) {
    const cmdStr = Array.isArray(manifest.entry.cmd)
      ? manifest.entry.cmd.join(" ")
      : String(manifest.entry.cmd);
    rows.push(["entry.cmd", cmdStr]);
  } else if (manifest.entry_cmd) {
    const cmdStr = Array.isArray(manifest.entry_cmd)
      ? manifest.entry_cmd.join(" ")
      : String(manifest.entry_cmd);
    rows.push(["entry_cmd", cmdStr]);
  }

  if (manifest.fixtures) rows.push(["fixtures", String(manifest.fixtures)]);
  if (manifest.version) rows.push(["version", String(manifest.version)]);
  if (manifest.language) rows.push(["language", String(manifest.language)]);
  if (manifest.name) rows.push(["name", String(manifest.name)]);

  if (rows.length === 0) {
    return (
      <div className="text-sm text-muted-foreground">
        Manifest loaded (no displayable fields).
      </div>
    );
  }

  return (
    <div className="rounded border border-zinc-700 divide-y divide-zinc-800">
      {rows.map(([k, v]) => (
        <div
          key={k}
          className="grid grid-cols-[180px_1fr] gap-3 p-2 text-sm"
        >
          <div className="text-muted-foreground">{k}</div>
          <div className="font-mono break-all">{v}</div>
        </div>
      ))}
    </div>
  );
}

function SourceFilesSection({ detail }: { detail: HandlerDetail }) {
  const { source_files, has_fixtures_dir } = detail;
  return (
    <Section title={`Files (${source_files.length})`}>
      {has_fixtures_dir && (
        <div className="text-xs text-muted-foreground mb-2">
          fixtures/ directory present
        </div>
      )}
      {source_files.length === 0 ? (
        <div className="text-sm text-muted-foreground">No files.</div>
      ) : (
        <div className="rounded border border-zinc-700 divide-y divide-zinc-800">
          {source_files.map((f) => (
            <FileRow key={f.name} f={f} />
          ))}
        </div>
      )}
    </Section>
  );
}

function FileRow({ f }: { f: SourceFileSummary }) {
  return (
    <div className="grid grid-cols-[1fr_120px] gap-3 p-2 text-sm">
      <div className="font-mono">
        {f.is_directory ? `${f.name}/` : f.name}
      </div>
      <div className="text-right text-muted-foreground">
        {f.is_directory ? "—" : formatBytes(f.size_bytes)}
      </div>
    </div>
  );
}

function StatusBadge({ status }: { status: ManifestStatus }) {
  const cls =
    status === "valid"
      ? "bg-green-500/15 text-green-300 border-green-500/30"
      : status === "invalid"
        ? "bg-yellow-500/15 text-yellow-300 border-yellow-500/30"
        : "bg-red-500/15 text-red-300 border-red-500/30";
  return (
    <span className={`inline-block rounded px-2 py-0.5 text-xs border ${cls}`}>
      {status}
    </span>
  );
}

function Section({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <div className="space-y-2">
      <h2 className="text-sm font-medium uppercase text-muted-foreground">
        {title}
      </h2>
      {children}
    </div>
  );
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}
