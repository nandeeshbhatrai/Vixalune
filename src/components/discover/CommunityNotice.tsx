import { ShieldAlert } from "lucide-react";
import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { ErrorNotice } from "@/components/ErrorNotice";
import { Button } from "@/components/ui/button";
import type { CatalogEntry } from "@/domain/types";
import { useAction } from "@/hooks/useAction";
import { useWorkspace } from "@/hooks/useWorkspace";
import { useBackend } from "@/infra/backendContext";

/** Shown on the detail page of an entry added from Discover. */
export function CommunityNotice({ entry }: { entry: CatalogEntry }) {
  const backend = useBackend();
  const ws = useWorkspace();
  const navigate = useNavigate();
  const { busy, error, run } = useAction();
  const [confirm, setConfirm] = useState(false);
  if (entry.trust === "bundled") return null;
  const remove = async () => {
    const ok = await run(async () => {
      await backend.communityRemove(entry.manifest.id);
      return true;
    });
    if (ok) {
      setConfirm(false);
      ws.refreshAll();
      navigate("/catalog");
    }
  };
  return (
    <section aria-label="Community entry" className="rounded-md border border-amber-500/60 p-3 text-sm">
      <p className="flex items-center gap-2 font-medium">
        <ShieldAlert size={14} aria-hidden /> Community entry — not reviewed by Vixalune
      </p>
      <p className="mt-1 text-muted-foreground">
        You added this package from Discover. Vixalune can install, verify and uninstall it, but has not checked the software, has no guided run profiles for it, and the AI Assistant will not recommend it. Install only software from publishers you trust.
      </p>
      <Button className="mt-2" variant="outline" size="sm" onClick={() => setConfirm(true)}>
        Remove from catalog
      </Button>
      <ConfirmDialog
        open={confirm}
        onOpenChange={setConfirm}
        title={`Remove ${entry.manifest.name} from your catalog?`}
        description="This only removes the catalog entry. If the software is installed, uninstall it first; it cannot be removed while it is installed."
        confirmLabel="Remove"
        destructive
        busy={busy}
        onConfirm={() => void remove()}
      >
        {error ? <ErrorNotice error={error} /> : null}
      </ConfirmDialog>
    </section>
  );
}
