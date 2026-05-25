import { Navigate } from "react-router-dom";
import { AppShell } from "@/layout/AppShell";
import { SettingsForm } from "@/components/SettingsForm";
import { useWorkspace } from "@/ipc/queries";

export function SettingsPage() {
  const ws = useWorkspace();
  if (ws.data === null && !ws.isLoading) return <Navigate to="/" replace />;
  return (
    <AppShell
      workspace={ws.data ?? null}
      crumbs={[{ label: "Settings" }]}
    >
      <div className="mx-auto max-w-2xl p-6">
        <h1 className="mb-4 text-xl font-medium">Settings</h1>
        <SettingsForm />
      </div>
    </AppShell>
  );
}
