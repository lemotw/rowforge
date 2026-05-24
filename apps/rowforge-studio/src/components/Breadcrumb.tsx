import { Link } from "react-router-dom";
import { ChevronRight } from "lucide-react";

export interface Crumb { label: string; to?: string; mono?: boolean }

export function Breadcrumb({ crumbs }: { crumbs: Crumb[] }) {
  return (
    <nav className="flex items-center gap-1 text-sm text-muted-foreground">
      {crumbs.map((c, i) => (
        <span key={i} className="flex items-center gap-1">
          {i > 0 && <ChevronRight className="h-3 w-3" />}
          {c.to ? (
            <Link to={c.to} className={c.mono ? "font-mono hover:text-foreground" : "hover:text-foreground"}>
              {c.label}
            </Link>
          ) : (
            <span className={c.mono ? "font-mono text-foreground" : "text-foreground"}>{c.label}</span>
          )}
        </span>
      ))}
    </nav>
  );
}
