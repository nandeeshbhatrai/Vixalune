import { Link } from "react-router-dom";
import { Badge } from "@/components/ui/badge";
import { StatusBadge } from "@/components/StatusBadge";
import { ToolIcon } from "@/components/ToolIcon";
import type { ToolStatus } from "@/domain/toolStatus";
import type { CatalogEntry } from "@/domain/types";
import { CATEGORY_LABEL, RUNTIME_LABEL } from "@/presentation/labels";

export function ToolCard({ entry, status }: { entry: CatalogEntry; status: ToolStatus }) {
  const m = entry.manifest;
  return (
    <Link
      to={`/catalog/${m.id}`}
      aria-label={`${m.name}, ${CATEGORY_LABEL[m.category]}`}
      className="flex h-full flex-col gap-3 rounded-lg border border-border bg-card p-4 transition-colors hover:border-primary/60"
    >
      <div className="flex items-start gap-3">
        <ToolIcon category={m.category} />
        <div className="min-w-0 flex-1">
          <h3 className="truncate font-semibold">{m.name}</h3>
          <p className="text-xs text-muted-foreground">{CATEGORY_LABEL[m.category]}</p>
        </div>
        <StatusBadge status={status} />
      </div>
      <p className="flex-1 text-muted-foreground">{m.summary}</p>
      {entry.trust !== "bundled" ? <p className="text-xs font-medium text-amber-700 dark:text-amber-300">Community, not reviewed</p> : null}
      <div className="flex flex-wrap gap-1.5" aria-label="Platforms">
        {m.platforms.map((p) => (
          <Badge key={p}>{RUNTIME_LABEL[p]}</Badge>
        ))}
      </div>
    </Link>
  );
}
