import { cn } from "@/lib/utils";
import { Activity, Settings as SettingsIcon } from "lucide-react";

export function Sidebar() {
  return (
    <aside className="w-44 border-r border-border bg-neutral-900">
      <nav className="flex flex-col gap-1 p-3 text-sm">
        <div className="px-2 pb-1 pt-2 text-xs uppercase text-muted-foreground">
          Workspace
        </div>
        <SideLink icon={<Activity className="h-4 w-4" />} label="Executions" active />
        <SideLink icon={<SettingsIcon className="h-4 w-4" />} label="Settings" disabled />

        <div className="mt-4 px-2 pb-1 text-xs uppercase text-muted-foreground">
          Authoring
        </div>
        <SideLink label="Handlers" disabled hint="Coming soon" />
      </nav>
    </aside>
  );
}

function SideLink({
  icon,
  label,
  active,
  disabled,
  hint,
}: {
  icon?: React.ReactNode;
  label: string;
  active?: boolean;
  disabled?: boolean;
  hint?: string;
}) {
  return (
    <div
      className={cn(
        "flex items-center gap-2 rounded px-2 py-1.5",
        active && "bg-primary/20 text-foreground",
        !active && !disabled && "text-muted-foreground hover:bg-muted/40",
        disabled && "text-muted-foreground/50"
      )}
    >
      {icon}
      <span>{label}</span>
      {hint && <span className="ml-auto text-[10px]">{hint}</span>}
    </div>
  );
}
