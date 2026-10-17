import { ExternalLink as Icon } from "lucide-react";
import type { ReactNode } from "react";
import type { LinkKind } from "@/domain/types";
import { useAction } from "@/hooks/useAction";
import { useBackend } from "@/infra/backendContext";
import { ErrorNotice } from "./ErrorNotice";

/**
 * Opens a catalog link through the backend, which resolves the URL from the validated manifest.
 * The UI never navigates the webview to an external address itself.
 */
export function ExternalLink({ toolId, kind, children }: { toolId: string; kind: LinkKind; children: ReactNode }) {
  const backend = useBackend();
  const { error, run } = useAction();
  return (
    <span className="inline-flex flex-col">
      <button
        type="button"
        onClick={() => void run(() => backend.openLink(toolId, kind))}
        className="inline-flex items-center gap-1 text-primary hover:underline"
      >
        {children}
        <Icon size={12} aria-hidden />
        <span className="sr-only">(opens in your browser)</span>
      </button>
      {error ? <ErrorNotice error={error} /> : null}
    </span>
  );
}
