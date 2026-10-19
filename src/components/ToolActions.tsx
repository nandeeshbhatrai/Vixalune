import { CheckCircle2, CircleHelp, XCircle } from "lucide-react";
import { useState } from "react";
import { Link } from "react-router-dom";
import { AsyncView } from "@/components/AsyncView";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { ErrorNotice } from "@/components/ErrorNotice";
import { JobProgress } from "@/components/JobProgress";
import { LaunchDialog } from "@/components/LaunchDialog";
import { Button } from "@/components/ui/button";
import { actionsFor, type ToolAction } from "@/domain/toolStatus";
import type { CatalogEntry, InstallPlan } from "@/domain/types";
import { useAction } from "@/hooks/useAction";
import { useAsync } from "@/hooks/useAsync";
import { useWorkspace } from "@/hooks/useWorkspace";
import { useBackend } from "@/infra/backendContext";
import { describeInstall, ELEVATION_LABEL, INTEGRITY_LABEL, METHOD_LABEL, PREREQ_LABEL } from "@/presentation/labels";

const LABEL: Record<ToolAction, string> = { install: "Install", launch: "Launch", verify: "Verify", uninstall: "Uninstall" };

function PlanDetails({ plan }: { plan: InstallPlan }) {
  const size =
    plan.approx_download_mb || plan.approx_disk_mb
      ? [plan.approx_download_mb ? `about ${plan.approx_download_mb} MB download` : null, plan.approx_disk_mb ? `about ${plan.approx_disk_mb} MB on disk` : null].filter(Boolean).join(", ")
      : "not recorded for this tool";
  return (
    <div className="space-y-3 text-sm">
      <dl className="space-y-1.5">
        <div>
          <dt className="inline text-muted-foreground">Method: </dt>
          <dd className="inline">{describeInstall(plan.method)}</dd>
        </div>
        <div>
          <dt className="inline text-muted-foreground">Administrator rights: </dt>
          <dd className="inline">{ELEVATION_LABEL[plan.elevation]}</dd>
        </div>
        <div>
          <dt className="inline text-muted-foreground">Restart: </dt>
          <dd className="inline">{plan.may_require_restart ? "may be needed" : "not expected"}</dd>
        </div>
        <div>
          <dt className="inline text-muted-foreground">Internet: </dt>
          <dd className="inline">{plan.network_required ? "needed to download the tool" : "not needed"}</dd>
        </div>
        <div>
          <dt className="inline text-muted-foreground">Storage: </dt>
          <dd className="inline">{size}</dd>
        </div>
        <div>
          <dt className="inline text-muted-foreground">Integrity: </dt>
          <dd className="inline">{INTEGRITY_LABEL[plan.integrity]}</dd>
        </div>
      </dl>
      {plan.prerequisites.length > 0 ? (
        <div>
          <p className="mb-1 text-xs font-medium uppercase tracking-wider text-muted-foreground">Needs</p>
          <ul className="space-y-1">
            {plan.prerequisites.map(({ spec, satisfied }) => {
              const Icon = satisfied === true ? CheckCircle2 : satisfied === false ? XCircle : CircleHelp;
              const tone = satisfied === true ? "text-success" : satisfied === false ? "text-destructive" : "text-muted-foreground";
              return (
                <li key={spec.kind} className="flex gap-2">
                  <Icon size={14} className={`mt-0.5 shrink-0 ${tone}`} aria-hidden />
                  <span>
                    <span className="font-medium">{PREREQ_LABEL[spec.kind]}</span>{" "}
                    <span className="text-muted-foreground">
                      ({satisfied === true ? "found" : satisfied === false ? "missing" : "could not be checked"}
                      {spec.requires_admin ? ", needs administrator rights" : ""}
                      {spec.requires_restart ? ", needs a restart" : ""}
                      {spec.approx_size_mb ? `, about ${spec.approx_size_mb} MB` : ""})
                    </span>
                    <span className="block text-muted-foreground">{spec.reason}</span>
                  </span>
                </li>
              );
            })}
          </ul>
        </div>
      ) : null}
      {plan.notes.map((n) => (
        <p key={n} role="note" className="text-xs text-warning">
          {n}
        </p>
      ))}
      <p className="text-xs text-muted-foreground">
        Installing accepts the package&apos;s license on your behalf (license: <span className="font-mono">{plan.license}</span>). Review it first if you are unsure.
      </p>
    </div>
  );
}

/**
 * Install / launch / verify / uninstall controls. Availability comes from `actionsFor`
 * (backend capabilities + status); unavailable actions are disabled and explain why.
 * Install shows the real plan (prerequisites, privileges, size) and always asks for confirmation.
 */
export function ToolActions({ entry }: { entry: CatalogEntry }) {
  const m = entry.manifest;
  const backend = useBackend();
  const ws = useWorkspace();
  const { busy, error, run, clearError } = useAction();
  const [confirm, setConfirm] = useState<"install" | "uninstall" | null>(null);
  const [elevationOk, setElevationOk] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [launching, setLaunching] = useState(false);

  const caps = ws.core.status === "ready" ? ws.core.data.caps : null;
  const status = ws.statusOf(m.id);
  const missing = ws.missingFor(m.id);
  const activeJob = ws.activeJobFor(m.id);
  const hasAdapter = caps ? m.install.some((i) => caps.install_methods.includes(i.method)) : true;
  const blocked = hasAdapter ? undefined : `Installing this tool needs the ${m.install.map((i) => METHOD_LABEL[i.method]).join(" or ")} installer, which this version of Vixalune does not include yet.`;
  const hasProfiles = m.launch_profiles.length > 0;
  const actions = (activeJob ? [] : actionsFor(status, caps, missing, blocked)).map((a) =>
    a.action === "launch" && !hasProfiles ? { ...a, enabled: false, reason: "This tool has no launch profile yet." } : a,
  );

  const [plan, retryPlan] = useAsync<InstallPlan | null>(
    () => (confirm === "install" ? backend.getInstallPlan(m.id) : Promise.resolve(null)),
    [backend, m.id, confirm],
  );
  const planBlocker =
    confirm === "install" && plan.status === "ready" && plan.data
      ? (plan.data.unsupported_reason ??
        (plan.data.missing_prerequisites.length > 0 ? `Install first: ${plan.data.missing_prerequisites.map((p) => PREREQ_LABEL[p]).join(", ")}.` : null))
      : null;
  const elevation = plan.status === "ready" && plan.data ? plan.data.elevation : "none";

  const perform = async (action: ToolAction) => {
    setNotice(null);
    const message = await run(async () => {
      switch (action) {
        case "install":
          await backend.installTool(m.id, elevationOk);
          return elevationOk || elevation === "none" ? "Installation started. Progress is shown below and under Running Jobs." : "Installation queued. Approve it under Running Jobs to continue.";
        case "uninstall":
          await backend.uninstallTool(m.id, false);
          return "Removal started. Progress is shown below and under Running Jobs.";
        case "verify":
          await backend.verifyTool(m.id);
          return "Verification started.";
        case "launch":
          return "";
      }
    });
    if (message) {
      setConfirm(null);
      setNotice(message);
      ws.refreshAll();
    }
  };

  const onClick = (a: ToolAction) => {
    clearError();
    setElevationOk(false);
    if (a === "install" || a === "uninstall") setConfirm(a);
    else if (a === "launch") setLaunching(true);
    else void perform(a);
  };

  const hints = [...new Set(actions.filter((a) => !a.enabled && a.reason).map((a) => `${LABEL[a.action]}: ${a.reason}`))];

  return (
    <div className="space-y-2">
      {activeJob ? (
        <div className="space-y-2">
          <JobProgress job={activeJob} />
          <div className="flex items-center gap-3">
            {activeJob.phase === "awaiting_consent" ? (
              <Button size="sm" onClick={() => void run(() => backend.approveJob(activeJob.id)).then(() => ws.reloadJobs())}>
                Approve and continue
              </Button>
            ) : null}
            <Link to="/jobs" className="text-primary hover:underline">
              Open Running Jobs
            </Link>
          </div>
          {error ? <ErrorNotice error={error} /> : null}
        </div>
      ) : (
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
        </div>
      )}
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
      {error && !confirm && !activeJob ? <ErrorNotice error={error} /> : null}

      <LaunchDialog
        entry={entry}
        open={launching}
        onOpenChange={setLaunching}
        onStarted={(msg) => {
          setNotice(msg);
          ws.refreshAll();
        }}
      />

      <ConfirmDialog
        open={confirm === "install"}
        onOpenChange={(o) => !o && setConfirm(null)}
        title={`Install ${m.name}?`}
        description="This is what will happen on your computer."
        confirmLabel="Install"
        busy={busy}
        confirmDisabled={plan.status !== "ready" || !plan.data || planBlocker !== null || (elevation === "required" && !elevationOk)}
        onConfirm={() => void perform("install")}
      >
        <AsyncView state={plan} retry={retryPlan} label="Checking what the install needs">
          {(p) =>
            p ? (
              <>
                <PlanDetails plan={p} />
                {planBlocker ? (
                  <div role="alert" className="mt-3 rounded-md border border-destructive/40 bg-destructive/10 p-2.5 text-sm text-destructive">
                    {planBlocker}
                  </div>
                ) : null}
                {p.elevation !== "none" && !planBlocker ? (
                  <label className="mt-3 flex items-start gap-2 text-sm">
                    <input type="checkbox" className="mt-1" checked={elevationOk} onChange={(e) => setElevationOk(e.target.checked)} />
                    <span>
                      Allow Windows to ask for administrator approval (UAC) for this install only.
                      {p.elevation === "possible" ? " If you leave this off, the install waits for your approval in Running Jobs." : ""}
                    </span>
                  </label>
                ) : null}
              </>
            ) : null
          }
        </AsyncView>
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


