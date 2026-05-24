import { Header } from "./Header";
import { Sidebar } from "./Sidebar";
import type { Workspace } from "@/ipc/types";

export function AppShell({
  workspace,
  children,
}: {
  workspace: Workspace | null;
  children: React.ReactNode;
}) {
  return (
    <div className="grid h-screen grid-cols-[auto_1fr] grid-rows-[auto_1fr]">
      <div className="col-span-2">
        <Header workspace={workspace} />
      </div>
      <Sidebar />
      <main className="overflow-auto">{children}</main>
    </div>
  );
}
