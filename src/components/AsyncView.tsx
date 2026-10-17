import type { ReactNode } from "react";
import type { AsyncState } from "@/hooks/useAsync";
import { ErrorNotice } from "./ErrorNotice";

export function LoadingState({ label = "Loading" }: { label?: string }) {
  return (
    <div role="status" aria-live="polite" className="space-y-2">
      <span className="sr-only">{label}…</span>
      {[0, 1, 2].map((i) => (
        <div key={i} aria-hidden className="h-10 animate-pulse rounded-md bg-muted" />
      ))}
    </div>
  );
}

/** Renders loading / error (with recovery instructions) / ready states uniformly. */
export function AsyncView<T>({
  state,
  retry,
  label,
  children,
}: {
  state: AsyncState<T>;
  retry: () => void;
  label?: string;
  children: (data: T) => ReactNode;
}) {
  if (state.status === "loading") return <LoadingState label={label} />;
  if (state.status === "error") return <ErrorNotice error={state.error} onRetry={retry} />;
  return <>{children(state.data)}</>;
}
