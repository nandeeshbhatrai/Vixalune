import type { ReactNode } from "react";
import type { AsyncState } from "@/hooks/useAsync";
import { Button } from "@/components/ui/button";

/** Renders loading / error (with recovery instructions) / ready states uniformly. */
export function AsyncView<T>({
  state,
  retry,
  children,
}: {
  state: AsyncState<T>;
  retry: () => void;
  children: (data: T) => ReactNode;
}) {
  if (state.status === "loading") return <p className="text-muted-foreground">Loading…</p>;
  if (state.status === "error") {
    return (
      <div role="alert" className="rounded-lg border border-destructive/40 bg-destructive/10 p-4">
        <p className="font-medium text-destructive">{state.error.message}</p>
        <p className="mt-1 text-sm">{state.error.recovery}</p>
        <Button className="mt-3" variant="outline" size="sm" onClick={retry}>
          Try again
        </Button>
      </div>
    );
  }
  return <>{children(state.data)}</>;
}
