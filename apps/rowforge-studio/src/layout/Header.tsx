import { useState } from "react";
import type { Workspace } from "@/ipc/types";
import { Breadcrumb, type Crumb } from "@/components/Breadcrumb";
import { WorkspaceMenu } from "@/components/WorkspaceMenu";

export function Header({
  workspace,
  crumbs,
}: {
  workspace: Workspace | null;
  crumbs?: Crumb[];
}) {
  const [menuOpen, setMenuOpen] = useState(false);
  return (
    <header className="flex h-12 items-center gap-4 border-b border-border px-4 text-sm">
      <button
        className="font-mono text-muted-foreground underline decoration-dashed underline-offset-4 hover:text-foreground disabled:no-underline disabled:opacity-60"
        onClick={() => setMenuOpen(true)}
        disabled={!workspace}
      >
        {workspace?.root ?? "—"}
      </button>
      {workspace && (
        <span className="text-xs text-muted-foreground/70">
          schema v{workspace.schema_version}
        </span>
      )}
      {crumbs && crumbs.length > 0 && (
        <div className="ml-4 border-l border-border pl-4">
          <Breadcrumb crumbs={crumbs} />
        </div>
      )}
      {workspace && (
        <WorkspaceMenu workspaceRoot={workspace.root} open={menuOpen} onOpenChange={setMenuOpen} />
      )}
    </header>
  );
}
