import { useCallback, useState } from "react";
import type { AppErrorDto } from "@/domain/types";
import { toAppError } from "./useAsync";

/** Runs one backend action at a time and exposes its busy/error state for the UI. */
export function useAction() {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<AppErrorDto | null>(null);

  const run = useCallback(async <T,>(fn: () => Promise<T>): Promise<T | undefined> => {
    setBusy(true);
    setError(null);
    try {
      return await fn();
    } catch (e) {
      setError(toAppError(e));
      return undefined;
    } finally {
      setBusy(false);
    }
  }, []);

  return { busy, error, run, clearError: () => setError(null) };
}
