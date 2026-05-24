import { cn } from "@/lib/utils";
import type { Phase } from "@/ipc/types";

const PHASES: Phase[] = [
  "initializing",
  "snapshotting",
  "starting",
  "running",
  "cancelling",
  "persisting",
];

const PHASE_LABEL: Record<Phase, string> = {
  initializing: "Init",
  snapshotting: "Snap",
  starting: "Start",
  running: "Run",
  cancelling: "Cancel",
  persisting: "Persist",
};

export function PhaseChipBar({ current }: { current: Phase | null }) {
  const currentIdx = current ? PHASES.indexOf(current) : -1;

  return (
    <div className="flex items-center gap-2 text-xs">
      <span className="text-muted-foreground">Phase:</span>
      {PHASES.map((phase, idx) => {
        const isCurrent = idx === currentIdx;
        const isPast = currentIdx >= 0 && idx < currentIdx;
        return (
          <Chip
            key={phase}
            label={PHASE_LABEL[phase]}
            current={isCurrent}
            past={isPast}
          />
        );
      })}
    </div>
  );
}

function Chip({
  label,
  current,
  past,
}: {
  label: string;
  current: boolean;
  past: boolean;
}) {
  return (
    <span
      className={cn(
        "rounded border px-2 py-0.5",
        current && "border-emerald-500 bg-emerald-500/10 text-emerald-300",
        past && "border-border text-muted-foreground",
        !current && !past && "border-border/50 text-muted-foreground/60",
      )}
    >
      {current && "● "}
      {past && "✓ "}
      {label}
    </span>
  );
}
