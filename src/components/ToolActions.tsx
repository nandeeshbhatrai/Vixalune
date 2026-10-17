import { useState } from "react";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { ErrorNotice } from "@/components/ErrorNotice";
import { actionsFor, type ToolAction } from "@/domain/toolStatus";
import type { CatalogEntry } from "@/domain/types";
import { useAction } from "@/hooks/useAction";
import { useWorkspace } from "@/hooks/useWorkspace";
import { useBackend } from "@/infra/backendContext";
import { describeInstall, PREREQ_LABEL } from "@/presentation/labels";

const LABEL: Record<ToolAction, string> = { install: "Install", launch: "Launch", verify: "Verify", uninstall: "Uninstall" };

/**
 * Install / launch / verify / uninstall controls. Availability comes from `actionsFor`
 * (backend capabilities + status); unavailable actions are disabled and explain why.
 * Install and uninstall always ask for confirmation first.
 */
export function ToolActions({ entry }: { entry: CatalogEntry }) {
  const m = entry.manifest;
  const backend = useBackend();
  const ws = useWorkspace();
  const { busy, error, run, clearError } = useAction();
  const [confirm, setConfirm] = useState<"install" | "uninstall" | null>(null);
  const [elevationOk, setElevationOk] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);

  const caps = ws.core.status === "ready" ? ws.core.data.caps : null;
  const status = ws.statusOf(m.id);
  const missing = ws.missingFor(m.id);
  const actions = actionsFor(status, caps, missing);

  const perform = async (action: ToolAction) => {
    setNotice(null);
    const ok = await run(async () => {
      switch (action) {
        case "install":
          await backend.installTool(m.id, elevationOk);
          return "Installation requested. Follow progress under Running Jobs.";
        case "uninstall":
          await backend.uninstallTool(m.id, false);
          return "Removal requested. Follow progress under Running Jobs.";
        case "verify":
          await backend.verifyTool(m.id);
          return "Verification finished.";
        case "launch":
          await backend.launchTool(m.id);
          return `${m.name} was launched.`;
      }
    });
    setConfirm(null);
    if (ok) {
      setNotice(ok);
      ws.refreshAll();
    }
  };

  const onClick = (a: ToolAction) => {
    clearError();
    setElevationOk(false);
    if (a === "install" || a === "uninstall") setConfirm(a);
    else void perform(a);
  };

  const hints = [...new Set(actions.filter((a) => !a.enabled && a.reason).map((a) => `${LABEL[a.action]}: ${a.reason}`))];

  return (
    <div className="space-y-2">
      <div className="flex flex-wrap gap-2" role="group" aria-label={`Actions for ${m.name}`}>
        {actions.map((a) => (
          <Button
            key={a.action}
            variant={a.action === "install" || a.action === "launch" ? "default" : "outline"}
            disabled={!a.enabled || busy}
            title={a.reason}
            onClick={() => onClick(a.action)}
          >
            {status === "error" && a.action === "install" ? "Retry install" : LABEL[a.action]}
          </Button>
        ))}
        {actions.length === 0 ? <span className="text-muted-foreground">An operation is in progress. See Running Jobs.</span> : null}
      </div>
      {hints.map((h) => (
        <p key={h} className="text-xs text-muted-foreground">
          {h}
        </p>
      ))}
      {notice ? (
        <p role="status" className="text-xs text-success">
          {notice}
        </p>
      ) : null}
      {error && !confirm ? <ErrorNotice error={error} /> : null}

      <ConfirmDialog
        open={confirm === "install"}
        onOpenChange={(o) => !o && setConfirm(null)}
        title={`Install ${m.name}?`}
        description="Review what will happen before continuing."
        confirmLabel="Install"
        busy={busy}
        confirmDisabled={m.requires_elevation && !elevationOk}
        onConfirm={() => void perform("install")}
      >
        <dl className="space-y-1.5 text-sm">
          <div>
            <dt className="inline text-muted-foreground">Method: </dt>
            <dd className="inline">{m.install[0] ? describeInstall(m.install[0]) : "none"}</dd>
          </div>
          <div>
            <dt className="inline text-muted-foreground">Prerequisites: </dt>
            <dd className="inline">{m.prerequisites.length ? m.prerequisites.map((p) => PREREQ_LABEL[p]).join(", ") : "none"}</dd>
          </div>
        </dl>
        {m.requires_elevation ? (
          <label className="mt-3 flex items-start gap-2 text-sm">
            <input type="checkbox" className="mt-1" checked={elevationOk} onChange={(e) => setElevationOk(e.target.checked)} />
            <span>This installer needs administrator rights. Allow Windows to ask for your approval (UAC) for this install only.</span>
          </label>
        ) : null}
        {error ? (
          <div className="mt-3">
            <ErrorNotice error={error} />
          </div>
        ) : null}
      </ConfirmDialog>

      <ConfirmDialog
        open={confirm === "uninstall"}
        onOpenChange={(o) => !o && setConfirm(null)}
        title={`Uninstall ${m.name}?`}
        description="The tool will be removed from this computer. Your own files are not touched."
        confirmLabel="Uninstall"
        destructive
        busy={busy}
        onConfirm={() => void perform("uninstall")}
      >
        {error ? <ErrorNotice error={error} /> : null}
      </ConfirmDialog>
    </div>
  );
}
