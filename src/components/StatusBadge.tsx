import { Badge } from "@/components/ui/badge";
import type { ToolStatus } from "@/domain/toolStatus";
import { STATUS_META } from "@/presentation/labels";

export function StatusBadge({ status }: { status: ToolStatus }) {
  const meta = STATUS_META[status];
  return (
    <Badge tone={meta.tone} title={meta.description}>
      <span aria-hidden className="size-1.5 rounded-full bg-current" />
      {meta.label}
    </Badge>
  );
}
