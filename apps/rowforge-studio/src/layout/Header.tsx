import type { Workspace } from "@/ipc/types";

export function Header({ workspace }: { workspace: Workspace | null }) {
  return (
    <header className="flex h-12 items-center border-b border-border px-4 text-sm">
      <span className="font-mono text-muted-foreground">
        {workspace?.root ?? "—"}
      </span>
      <span className="ml-2 text-xs text-muted-foreground/70">
        {workspace ? `schema v${workspace.schema_version}` : ""}
      </span>
    </header>
  );
}
