import { AlertTriangle } from "lucide-react";
import type { AppErrorDto } from "@/domain/types";
import { Button } from "@/components/ui/button";

/** Error message plus the recovery instruction every backend error carries. */
export function ErrorNotice({ error, onRetry }: { error: AppErrorDto; onRetry?: () => void }) {
  return (
    <div role="alert" className="flex gap-3 rounded-lg border border-destructive/40 bg-destructive/10 p-3">
      <AlertTriangle size={16} className="mt-0.5 shrink-0 text-destructive" aria-hidden />
      <div className="min-w-0 flex-1">
        <p className="font-medium text-destructive">{error.message}</p>
        <p className="mt-0.5 text-muted-foreground">{error.recovery}</p>
        {onRetry ? (
          <Button className="mt-2" variant="outline" size="sm" onClick={onRetry}>
            Try again
          </Button>
        ) : null}
      </div>
    </div>
  );
}
