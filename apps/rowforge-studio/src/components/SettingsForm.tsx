import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ipc } from "@/ipc/client";
import { uiErrorMessage, type Settings } from "@/ipc/types";
import { WorkspaceSwitchButton } from "@/components/WorkspaceSwitchButton";

/**
 * Plan 6 T11. Controlled form bound to Settings shape. Tracks dirty
 * state per field; surfaces a "Will apply on next workspace open"
 * banner when max_concurrent_runs has diverged from the loaded value
 * (since that field is only consumed at workspace_open time, not at
 * settings_save).
 *
 * WorkspaceSwitchButton (T12) handles the workspace_root field;
 * this form just shows the current root as read-only text.
 */
export function SettingsForm() {
  const qc = useQueryClient();
  const loaded = useQuery({
    queryKey: ["settings"],
    queryFn: () => ipc.workspace_settings_load(),
  });
  const save = useMutation({
    mutationFn: (settings: Settings) =>
      ipc.workspace_settings_save({ settings }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["settings"] });
      // Without an explicit confirmation, users can't tell whether
      // Save actually wrote — the form just bounces back from
      // "Saving…" to "Save". Toast is the cheapest unambiguous
      // feedback (sonner already mounted in App.tsx for Plan 5
      // ExportDialog).
      toast.success("Settings saved");
    },
  });

  const [form, setForm] = useState<Settings | null>(null);
  // Seed once when the query first resolves; keep `form` independent
  // so user edits aren't clobbered by background refetches.
  useEffect(() => {
    if (loaded.data && form === null) setForm(loaded.data);
  }, [loaded.data, form]);

  if (loaded.isLoading || form === null) {
    return <div className="text-muted-foreground">Loading…</div>;
  }
  if (loaded.isError) {
    return (
      <div className="text-red-300">
        Failed to load settings: {uiErrorMessage(loaded.error)}
      </div>
    );
  }

  const original = loaded.data!;
  const mcrDirty = form.max_concurrent_runs !== original.max_concurrent_runs;

  return (
    <div className="space-y-6">
      <Section title="Workspace">
        <div className="space-y-2">
          <div className="font-mono text-sm">{form.workspace_root ?? "—"}</div>
          <WorkspaceSwitchButton />
        </div>
      </Section>

      <Section title="Concurrency">
        <Field label="Default workers" htmlFor="default-workers">
          <Input
            id="default-workers"
            type="number"
            min={1}
            value={form.default_workers ?? ""}
            onChange={(e) =>
              setForm({
                ...form,
                default_workers:
                  e.target.value === ""
                    ? null
                    : Math.max(1, parseInt(e.target.value, 10) || 1),
              })
            }
          />
        </Field>
        <Field label="Max concurrent runs" htmlFor="max-concurrent-runs">
          <Input
            id="max-concurrent-runs"
            type="number"
            min={1}
            value={form.max_concurrent_runs ?? ""}
            onChange={(e) =>
              setForm({
                ...form,
                max_concurrent_runs:
                  e.target.value === ""
                    ? null
                    : Math.max(1, parseInt(e.target.value, 10) || 1),
              })
            }
          />
        </Field>
        {mcrDirty && (
          <div className="rounded border border-blue-500/30 bg-blue-500/10 p-2 text-xs">
            ℹ Changes to max concurrent runs apply on next workspace open.
          </div>
        )}
      </Section>

      <Section title="Telemetry">
        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            checked={form.telemetry_opt_in}
            onChange={(e) =>
              setForm({ ...form, telemetry_opt_in: e.target.checked })
            }
          />
          Opt in to anonymous usage metrics
        </label>
      </Section>

      {save.isError && (
        <div className="rounded border border-red-500/40 bg-red-500/10 p-2 text-sm text-red-200">
          Save failed: {uiErrorMessage(save.error)}
        </div>
      )}

      <div className="flex justify-end gap-2">
        <Button
          variant="ghost"
          onClick={() => setForm(original)}
          disabled={save.isPending}
        >
          Cancel
        </Button>
        <Button onClick={() => save.mutate(form)} disabled={save.isPending}>
          {save.isPending ? "Saving…" : "Save"}
        </Button>
      </div>
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="rounded-lg border border-zinc-700 p-4">
      <div className="mb-3 text-sm font-medium uppercase text-muted-foreground">
        {title}
      </div>
      <div className="space-y-3">{children}</div>
    </div>
  );
}

function Field({
  label,
  htmlFor,
  children,
}: {
  label: string;
  htmlFor?: string;
  children: React.ReactNode;
}) {
  return (
    <div>
      <label htmlFor={htmlFor} className="mb-1 block text-sm">
        {label}
      </label>
      {children}
    </div>
  );
}
