import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";

export function EmptyState({ icon: Icon, title, children }: { icon: LucideIcon; title: string; children?: ReactNode }) {
  return (
    <div className="flex flex-col items-center rounded-lg border border-dashed border-border px-6 py-10 text-center">
      <Icon size={24} className="text-muted-foreground" aria-hidden />
      <p className="mt-3 font-medium">{title}</p>
      {children ? <div className="mt-1 max-w-md text-muted-foreground">{children}</div> : null}
    </div>
  );
}
