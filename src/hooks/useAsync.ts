import { useCallback, useEffect, useState } from "react";
import type { AppErrorDto } from "@/domain/types";

export type AsyncState<T> =
  | { status: "loading" }
  | { status: "ready"; data: T }
  | { status: "error"; error: AppErrorDto };

export function toAppError(e: unknown): AppErrorDto {
  if (e && typeof e === "object" && "message" in e && "recovery" in e) return e as AppErrorDto;
  return { code: "unknown", message: String(e), recovery: "Restart the app and try again." };
}

export function useAsync<T>(fn: () => Promise<T>, deps: unknown[]): [AsyncState<T>, () => void] {
  const [state, setState] = useState<AsyncState<T>>({ status: "loading" });
  const [tick, setTick] = useState(0);
  const stableFn = useCallback(fn, deps);

  useEffect(() => {
    let live = true;
    setState({ status: "loading" });
    stableFn().then(
      (data) => live && setState({ status: "ready", data }),
      (e) => live && setState({ status: "error", error: toAppError(e) }),
    );
    return () => {
      live = false;
    };
  }, [stableFn, tick]);

  return [state, () => setTick((t) => t + 1)];
}
