import { Header } from "./Header";
import { Sidebar } from "./Sidebar";
import type { Workspace } from "@/ipc/types";
import type { Crumb } from "@/components/Breadcrumb";

export function AppShell({
  workspace,
  crumbs,
  children,
}: {
  workspace: Workspace | null;
  crumbs?: Crumb[];
  children: React.ReactNode;
}) {
  return (
    <div className="grid h-screen grid-cols-[auto_1fr] grid-rows-[auto_1fr]">
      <div className="col-span-2">
        <Header workspace={workspace} crumbs={crumbs} />
      </div>
      <Sidebar />
      <main className="overflow-auto">{children}</main>
    </div>
  );
}
