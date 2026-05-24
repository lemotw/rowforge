import { cn } from "@/lib/utils";
import { AlertOctagon, Hourglass, Info } from "lucide-react";
import type { LifecycleBanner } from "@/ipc/run-state";

export function LifecycleBanners({ banners }: { banners: LifecycleBanner[] }) {
  if (banners.length === 0) return null;
  return (
    <div className="flex flex-col gap-2">
      {banners.map((b) => (
        <BannerItem key={b.id} banner={b} />
      ))}
    </div>
  );
}

function BannerItem({ banner }: { banner: LifecycleBanner }) {
  const { tone, icon } = bannerStyle(banner.kind);
  return (
    <div
      className={cn(
        "flex items-start gap-2 rounded border p-2 text-sm",
        tone,
      )}
    >
      {icon}
      <div className="flex-1">
        <div>{banner.message}</div>
        {banner.stderr_tail && (
          <details className="mt-1 text-xs">
            <summary className="cursor-pointer text-muted-foreground">
              stderr tail ({banner.stderr_tail.split("\n").length} lines)
            </summary>
            <pre className="mt-1 max-h-40 overflow-auto rounded bg-neutral-950 p-2 font-mono text-[10px]">
              {banner.stderr_tail}
            </pre>
          </details>
        )}
      </div>
    </div>
  );
}

function bannerStyle(kind: LifecycleBanner["kind"]) {
  switch (kind) {
    case "worker_crashed":
      return {
        tone: "border-red-500/40 bg-red-500/10 text-red-300",
        icon: <AlertOctagon className="h-4 w-4 mt-0.5" />,
      };
    case "stall_warning":
      return {
        tone: "border-amber-500/40 bg-amber-500/10 text-amber-200",
        icon: <Hourglass className="h-4 w-4 mt-0.5" />,
      };
    case "pipeline_warning":
      return {
        tone: "border-blue-500/40 bg-blue-500/10 text-blue-200",
        icon: <Info className="h-4 w-4 mt-0.5" />,
      };
  }
}
