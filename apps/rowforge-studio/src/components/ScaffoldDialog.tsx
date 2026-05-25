import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { toast } from "sonner";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useHandlerScaffold } from "@/ipc/use-handlers";
import { uiErrorMessage, type ScaffoldTemplate } from "@/ipc/types";

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

const NAME_RE = /^[a-z0-9][a-z0-9-]*$/;
const PRIMARY_FIELD_RE = /^[a-zA-Z_][a-zA-Z0-9_]*$/;

const TEMPLATES: { value: ScaffoldTemplate; label: string; hint: string }[] = [
  {
    value: "go_stdio",
    label: "Go (row mode)",
    hint: "Minimal row handler reading stdin, writing stdout",
  },
  {
    value: "go_batch",
    label: "Go (batch mode)",
    hint: "Batch handler with batch_size: 5",
  },
  {
    value: "empty",
    label: "Empty",
    hint: "Minimal skeleton: rowforge.yaml + empty handler.go",
  },
];

export function ScaffoldDialog({ open, onOpenChange }: Props) {
  const navigate = useNavigate();
  const scaffold = useHandlerScaffold();

  const [name, setName] = useState("");
  const [template, setTemplate] = useState<ScaffoldTemplate>("go_stdio");
  const [primaryField, setPrimaryField] = useState("id");

  const nameError =
    name === ""
      ? null
      : !NAME_RE.test(name)
        ? "Lowercase letters, numbers, and hyphens; must start with a letter or number"
        : null;

  const primaryError =
    primaryField === ""
      ? "Primary field is required"
      : !PRIMARY_FIELD_RE.test(primaryField)
        ? "Must be a valid identifier: letters, digits, underscores; cannot start with a digit"
        : null;

  const canSubmit =
    name !== "" && nameError === null && primaryError === null && !scaffold.isPending;

  // Reset state when the dialog closes.
  const handleOpenChange = (next: boolean) => {
    if (!next) {
      setName("");
      setTemplate("go_stdio");
      setPrimaryField("id");
      scaffold.reset();
    }
    onOpenChange(next);
  };

  const handleSubmit = () => {
    if (!canSubmit) return;
    scaffold.mutate(
      { name, template, primary_field: primaryField },
      {
        onSuccess: (createdName) => {
          toast.success(`Handler "${createdName}" created`);
          handleOpenChange(false);
          navigate(`/handlers/${createdName}`);
        },
      },
    );
  };

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Create new handler</DialogTitle>
          <p className="text-sm text-muted-foreground">
            Scaffold a new handler in this workspace's handlers/ directory.
          </p>
        </DialogHeader>

        <div className="space-y-4">
          <Field label="Name" htmlFor="scaffold-name" error={nameError}>
            <Input
              id="scaffold-name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="my-handler"
              autoFocus
            />
            <div className="mt-1 text-xs text-muted-foreground">
              Lowercase letters, numbers, hyphens; must start with a letter or number
            </div>
          </Field>

          <Field label="Template" htmlFor="">
            <div className="space-y-2">
              {TEMPLATES.map((t) => (
                <label
                  key={t.value}
                  className={`flex cursor-pointer items-start gap-3 rounded border p-3 ${
                    template === t.value
                      ? "border-blue-500 bg-blue-500/5"
                      : "border-zinc-700 hover:border-zinc-600"
                  }`}
                >
                  <input
                    type="radio"
                    name="scaffold-template"
                    value={t.value}
                    checked={template === t.value}
                    onChange={() => setTemplate(t.value)}
                    className="mt-1"
                    aria-label={t.label}
                  />
                  <div className="text-sm">
                    <div className="font-medium">{t.label}</div>
                    <div className="text-muted-foreground">{t.hint}</div>
                  </div>
                </label>
              ))}
            </div>
          </Field>

          <Field
            label="Primary field"
            htmlFor="scaffold-primary"
            error={primaryError}
            helper="The CSV column this handler will process. Must match a column in your input."
          >
            <Input
              id="scaffold-primary"
              value={primaryField}
              onChange={(e) => setPrimaryField(e.target.value)}
              placeholder="id"
            />
          </Field>

          {scaffold.isError && (
            <div className="rounded border border-red-500/40 bg-red-500/10 p-2 text-sm text-red-200">
              {uiErrorMessage(scaffold.error)}
            </div>
          )}
        </div>

        <div className="flex justify-end gap-2">
          <Button variant="outline" onClick={() => handleOpenChange(false)}>
            Cancel
          </Button>
          <Button onClick={handleSubmit} disabled={!canSubmit}>
            {scaffold.isPending ? "Creating…" : "Create"}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function Field({
  label,
  htmlFor,
  error,
  helper,
  children,
}: {
  label: string;
  htmlFor?: string;
  error?: string | null;
  helper?: string;
  children: React.ReactNode;
}) {
  return (
    <div>
      <label htmlFor={htmlFor || undefined} className="mb-1 block text-sm font-medium">
        {label}
      </label>
      {children}
      {helper && !error && (
        <div className="mt-1 text-xs text-muted-foreground">{helper}</div>
      )}
      {error && <div className="mt-1 text-xs text-red-300">{error}</div>}
    </div>
  );
}
