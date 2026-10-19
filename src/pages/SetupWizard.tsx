import { CheckCircle2, CircleHelp, ExternalLink, XCircle } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { ErrorNotice } from "@/components/ErrorNotice";
import { JobProgress } from "@/components/JobProgress";
import { PageHeader } from "@/components/PageHeader";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardTitle } from "@/components/ui/card";
import { allReady, formatBytes, isBlockedByDependency } from "@/domain/platform";
import type { CapabilityReadiness, Job, PlatformReport, SetupAction, SetupActionId } from "@/domain/types";
import { useAction } from "@/hooks/useAction";
import { useWorkspace } from "@/hooks/useWorkspace";
import { useBackend } from "@/infra/backendContext";
import { isActivePhase, READINESS_META } from "@/presentation/labels";

export const SETUP_SEEN_KEY = "vixalune.setupSeen";

export function markSetupSeen() {
  try {
    localStorage.setItem(SETUP_SEEN_KEY, "1");
  } catch {
    /* storage unavailable: the wizard simply shows again next time */
  }
}

export function setupSeen(): boolean {
  try {
    return localStorage.getItem(SETUP_SEEN_KEY) === "1";
  } catch {
    return true; // never trap the user in a redirect if storage is unavailable
  }
}

const STEPS = ["Check your computer", "What you need", "What is missing", "Set up", "Restart", "Re-check"] as const;
const JOB_TOOL: Partial<Record<SetupActionId, string>> = { install_wsl: "setup-install-wsl", install_distro: "setup-install-distro" };

function ReadinessCard({ title, r, why }: { title: string; r: CapabilityReadiness; why?: string }) {
  const meta = READINESS_META[r.state];
  return (
    <Card>
      <div className="flex items-center justify-between gap-2">
        <h3 className="font-semibold">{title}</h3>
        <Badge tone={meta.tone}>{meta.label}</Badge>
      </div>
      <p className="mt-1 text-muted-foreground">{r.summary}</p>
      {why ? <p className="mt-1 text-xs text-muted-foreground">{why}</p> : null}
      {r.blockers.length > 0 ? (
        <ul className="mt-2 list-disc space-y-0.5 pl-5 text-destructive">
          {r.blockers.map((b) => (
            <li key={b}>{b}</li>
          ))}
        </ul>
      ) : null}
      {r.missing.length > 0 ? (
        <ul className="mt-2 list-disc space-y-0.5 pl-5">
          {r.missing.map((b) => (
            <li key={b}>Missing: {b}</li>
          ))}
        </ul>
      ) : null}
      {r.notes.map((n) => (
        <p key={n} role="note" className="mt-2 text-xs text-muted-foreground">
          {n}
        </p>
      ))}
    </Card>
  );
}

function Fact({ label, value, tone }: { label: string; value: string; tone?: "ok" | "bad" | "unknown" }) {
  const Icon = tone === "ok" ? CheckCircle2 : tone === "bad" ? XCircle : CircleHelp;
  const cls = tone === "ok" ? "text-success" : tone === "bad" ? "text-destructive" : "text-muted-foreground";
  return (
    <li className="flex items-start gap-2">
      <Icon size={15} className={`mt-0.5 shrink-0 ${cls}`} aria-hidden />
      <span>
        <span className="text-muted-foreground">{label}: </span>
        {value}
      </span>
    </li>
  );
}

function CompatibilityStep({ r }: { r: PlatformReport }) {
  const w = r.windows;
  const v = r.virtualization;
  const bool = (b: boolean | null | undefined) => (b == null ? "unknown" : b ? "yes" : "no");
  const tone = (b: boolean | null | undefined) => (b == null ? "unknown" : b ? "ok" : "bad");
  const winget = r.package_managers.find((p) => p.name === "winget");
  const rt = r.containers.runtime;
  return (
    <div className="space-y-3">
      <Card>
        <CardTitle>What was detected</CardTitle>
        <ul className="space-y-1.5" aria-label="Detected facts">
          <Fact label="Windows" value={`build ${w.build ?? "unknown"}${w.display_version ? ` (${w.display_version})` : ""}, ${w.architecture}${w.edition ? `, ${w.edition}` : ""}`} tone={w.build == null ? "unknown" : w.build >= 19045 ? "ok" : "bad"} />
          <Fact label="CPU virtualization enabled" value={bool(v.firmware_enabled)} tone={tone(v.firmware_enabled)} />
          <Fact label="Windows hypervisor running" value={bool(v.hypervisor_present)} tone={tone(v.hypervisor_present)} />
          <Fact label="winget" value={winget ? (winget.version ?? "not found") : "not checked"} tone={winget ? (winget.version ? "ok" : "bad") : "unknown"} />
          <Fact
            label="WSL"
            value={r.wsl.installed == null ? "could not be checked" : r.wsl.installed ? `installed${r.wsl.wsl_version ? ` (${r.wsl.wsl_version})` : ""}, ${r.wsl.distros.filter((d) => !d.internal).length} distribution(s)` : "not installed"}
            tone={tone(r.wsl.installed)}
          />
          <Fact
            label="Container engine"
            value={
              !rt
                ? "no Docker CLI found"
                : rt.daemon.state === "running"
                  ? `running (Docker ${rt.cli_version ?? ""}${rt.daemon.os_type ? `, ${rt.daemon.os_type} containers` : ""})`
                  : rt.daemon.state === "stopped"
                    ? "installed but not running"
                    : "could not be checked"
            }
            tone={!rt ? "bad" : rt.daemon.state === "running" ? "ok" : rt.daemon.state === "stopped" ? "bad" : "unknown"}
          />
          <Fact label="Free disk space" value={`${formatBytes(r.disk.free_bytes)} on ${r.disk.path}`} tone={r.disk.free_bytes == null ? "unknown" : "ok"} />
        </ul>
      </Card>
      <Card>
        <CardTitle>Windows features</CardTitle>
        <ul className="space-y-1.5">
          {r.features.map((f) => (
            <li key={f.id}>
              <span className="font-medium">{f.label}</span>: {f.state}
              <span className="block text-xs text-muted-foreground">{f.basis}</span>
            </li>
          ))}
        </ul>
      </Card>
      {r.system.pending_restart || r.system.proxy || r.system.policy_keys.length > 0 || r.system.is_elevated ? (
        <Card>
          <CardTitle>Worth knowing</CardTitle>
          <ul className="list-disc space-y-1 pl-5">
            {r.system.pending_restart ? <li>Windows has a restart pending.</li> : null}
            {r.system.proxy ? <li>A network proxy is configured ({r.system.proxy}). Downloads may need it; Vixalune does not bypass proxies.</li> : null}
            {r.system.policy_keys.length > 0 ? <li>Policy settings exist on this PC ({r.system.policy_keys.join(", ")}). Your organisation may restrict WSL or containers; Vixalune does not bypass policies.</li> : null}
            {r.system.is_elevated ? <li className="text-warning">Vixalune is running as administrator. It does not need to; run it normally.</li> : null}
          </ul>
        </Card>
      ) : null}
    </div>
  );
}

function ActionCard({ action, all, jobs, onRun }: { action: SetupAction; all: SetupAction[]; jobs: Job[]; onRun: (a: SetupAction) => void }) {
  const backend = useBackend();
  const link = useAction();
  const blocked = isBlockedByDependency(action, all);
  const toolId = JOB_TOOL[action.id];
  const job = toolId ? jobs.find((j) => j.tool_id === toolId) : undefined;
  const active = job && isActivePhase(job.phase);
  return (
    <Card>
      <div className="flex flex-wrap items-center gap-2">
        <h3 className="font-semibold">{action.title}</h3>
        <Badge tone={action.kind === "automatic" ? "info" : "neutral"}>{action.kind === "automatic" ? "Vixalune can do this" : "You do this"}</Badge>
        {action.requires_admin ? <Badge tone="warning">Administrator approval</Badge> : null}
        {action.requires_restart ? <Badge tone="warning">May need a restart</Badge> : null}
      </div>
      <p className="mt-1 text-muted-foreground">{action.why}</p>
      <ul className="mt-2 list-disc space-y-0.5 pl-5 text-sm">
        {action.instructions.map((i) => (
          <li key={i}>{i}</li>
        ))}
      </ul>
      {action.command_preview ? (
        <p className="mt-2 text-xs">
          Exact command: <code className="rounded bg-muted px-1.5 py-0.5 font-mono">{action.command_preview}</code>
        </p>
      ) : null}
      {job ? (
        <div className="mt-2">
          <JobProgress job={job} />
          {job.error ? <ErrorNotice error={{ code: job.error.code, message: job.error.message, recovery: job.error.recovery }} /> : null}
        </div>
      ) : null}
      <div className="mt-3 flex flex-wrap items-center gap-2">
        {action.kind === "automatic" ? (
          <Button size="sm" disabled={blocked || !!active} onClick={() => onRun(action)}>
            {active ? "Running…" : "Review and run…"}
          </Button>
        ) : null}
        {action.official_link ? (
          <Button size="sm" variant="outline" onClick={() => void link.run(() => backend.openSetupLink(action.id))}>
            Official guide <ExternalLink size={12} aria-hidden />
          </Button>
        ) : null}
        {blocked ? <span className="text-xs text-muted-foreground">Finish the step before this one first (and restart Windows if asked).</span> : null}
      </div>
      {link.error ? <ErrorNotice error={link.error} /> : null}
    </Card>
  );
}

export function SetupWizard() {
  const backend = useBackend();
  const ws = useWorkspace();
  const navigate = useNavigate();
  const [step, setStep] = useState(0);
  const [want, setWant] = useState({ wsl: true, containers: true });
  const [confirming, setConfirming] = useState<SetupAction | null>(null);
  const [understood, setUnderstood] = useState(false);
  const [restartDialog, setRestartDialog] = useState(false);
  const [restartScheduled, setRestartScheduled] = useState(false);
  const run = useAction();
  const restart = useAction();

  const r = ws.platform;
  const jobs = ws.jobs.status === "ready" ? ws.jobs.data : [];
  const setupJobs = jobs.filter((j) => j.kind === "setup");
  const activeSetup = setupJobs.some((j) => isActivePhase(j.phase));
  const was = useRef(false);
  // When a setup step finishes, observe the new state instead of trusting the command.
  useEffect(() => {
    if (was.current && !activeSetup) ws.recheckEnvironment();
    was.current = activeSetup;
  }, [activeSetup, ws]);

  const relevant = useMemo(() => {
    if (!r) return [];
    const wslIds: SetupActionId[] = ["install_wsl", "install_distro", "convert_distro_to_wsl2", "enable_virtualization"];
    const conIds: SetupActionId[] = ["install_docker_desktop", "start_docker_desktop", "install_wsl", "enable_virtualization"];
    return r.setup_actions.filter(
      (a) => a.id === "free_disk_space" || a.id === "install_winget" || (want.wsl && wslIds.includes(a.id)) || (want.containers && conIds.includes(a.id)),
    );
  }, [r, want]);

  const restartNeeded = !!r?.system.pending_restart || setupJobs.some((j) => j.phase === "succeeded" && /restart/i.test(j.message));

  if (ws.envLoading || (!r && !ws.envError)) {
    return (
      <div className="space-y-4">
        <PageHeader title="Setup" />
        <p role="status">Checking your computer…</p>
      </div>
    );
  }
  if (!r) {
    return (
      <div className="space-y-4">
        <PageHeader title="Setup" />
        {ws.envError ? <ErrorNotice error={ws.envError} onRetry={ws.recheckEnvironment} /> : null}
      </div>
    );
  }

  const next = () => setStep((s) => Math.min(STEPS.length - 1, s + 1));
  const back = () => setStep((s) => Math.max(0, s - 1));

  const confirmRun = async () => {
    if (!confirming) return;
    const ok = await run.run(async () => {
      await backend.runSetupAction(confirming.id, true);
      return true;
    });
    if (ok) {
      setConfirming(null);
      setUnderstood(false);
      ws.reloadJobs();
    }
  };

  const confirmRestart = async () => {
    const ok = await restart.run(async () => {
      await backend.requestRestart(true, 60);
      return true;
    });
    if (ok) {
      setRestartScheduled(true);
      setRestartDialog(false);
    }
  };

  const done = allReady(r, want);

  return (
    <div className="space-y-5">
      <PageHeader title="Setup" subtitle="Get your computer ready for Windows, Linux (WSL) and container tools. Nothing changes until you confirm." />

      <ol className="flex flex-wrap gap-2 text-xs" aria-label="Setup steps">
        {STEPS.map((s, i) => (
          <li key={s} aria-current={i === step ? "step" : undefined}>
            <button
              type="button"
              onClick={() => setStep(i)}
              className={`rounded-md px-2.5 py-1 ${i === step ? "bg-primary text-primary-foreground" : "bg-muted text-muted-foreground hover:text-foreground"}`}
            >
              {i + 1}. {s}
            </button>
          </li>
        ))}
      </ol>

      {step === 0 ? (
        <section aria-label="Check your computer" className="space-y-3">
          <h2 className="text-lg font-semibold">Check your computer</h2>
          <p className="text-muted-foreground">This only looks; it changes nothing. Detection, readiness and permission to change things are kept separate.</p>
          <CompatibilityStep r={r} />
          <ReadinessCard title="Windows tools (winget)" r={r.readiness.windows_tools} />
        </section>
      ) : null}

      {step === 1 ? (
        <section aria-label="What you need" className="space-y-3">
          <h2 className="text-lg font-semibold">What do you need?</h2>
          <Card>
            <p>
              <strong>Windows tools</strong> work out of the box with winget.
            </p>
          </Card>
          <Card>
            <label className="flex items-start gap-2">
              <input type="checkbox" className="mt-1" checked={want.wsl} onChange={(e) => setWant((w) => ({ ...w, wsl: e.target.checked }))} />
              <span>
                <strong>Linux tools (WSL 2)</strong>
                <span className="block text-muted-foreground">Many security tools are Linux programs. WSL 2 runs a real Linux (Ubuntu) inside Windows. It is convenient, but it is not a sandbox: Linux programs can see your Windows files and network.</span>
              </span>
            </label>
          </Card>
          <Card>
            <label className="flex items-start gap-2">
              <input type="checkbox" className="mt-1" checked={want.containers} onChange={(e) => setWant((w) => ({ ...w, containers: e.target.checked }))} />
              <span>
                <strong>Container tools (Docker)</strong>
                <span className="block text-muted-foreground">Some tools ship as container images. Vixalune runs them without network access, as a non-root user, with only the folder you choose. Containers reduce accidents; they are not a safe place for hostile code (use a separate virtual machine for that).</span>
              </span>
            </label>
          </Card>
        </section>
      ) : null}

      {step === 2 ? (
        <section aria-label="What is missing" className="space-y-3">
          <h2 className="text-lg font-semibold">What is missing</h2>
          <ReadinessCard title="Windows tools" r={r.readiness.windows_tools} />
          {want.wsl ? <ReadinessCard title="Linux tools (WSL 2)" r={r.readiness.linux_tools_wsl} why="Needed for Linux-based tools." /> : null}
          {want.containers ? <ReadinessCard title="Container tools" r={r.readiness.containers} why="Needed for container-based tools and labs." /> : null}
        </section>
      ) : null}

      {step === 3 ? (
        <section aria-label="Set up" className="space-y-3">
          <h2 className="text-lg font-semibold">Set up</h2>
          {relevant.length === 0 ? (
            <Card>
              <p>Nothing needs to be set up for what you chose.</p>
            </Card>
          ) : (
            relevant.map((a) => <ActionCard key={a.id} action={a} all={r.setup_actions} jobs={jobs} onRun={(x) => { run.clearError(); setUnderstood(false); setConfirming(x); }} />)
          )}
          <p className="text-xs text-muted-foreground">Vixalune never installs Docker Desktop, changes BIOS settings, bypasses organisation policies, or restarts Windows without asking you.</p>
        </section>
      ) : null}

      {step === 4 ? (
        <section aria-label="Restart" className="space-y-3">
          <h2 className="text-lg font-semibold">Restart</h2>
          <Card>
            {restartNeeded ? (
              <>
                <p>Windows needs a restart to finish setup. Nothing restarts unless you choose it here.</p>
                {restartScheduled ? (
                  <div className="mt-3 space-y-2">
                    <p role="status" className="text-warning">
                      A restart is scheduled in about a minute. Save your work.
                    </p>
                    <Button
                      variant="outline"
                      onClick={() =>
                        void restart.run(async () => {
                          await backend.cancelRestart();
                          setRestartScheduled(false);
                        })
                      }
                    >
                      Cancel the restart
                    </Button>
                  </div>
                ) : (
                  <div className="mt-3 flex gap-2">
                    <Button onClick={() => setRestartDialog(true)}>Restart Windows…</Button>
                    <Button variant="outline" onClick={next}>
                      Restart later
                    </Button>
                  </div>
                )}
              </>
            ) : (
              <p>No restart is needed right now.</p>
            )}
            {restart.error ? <ErrorNotice error={restart.error} /> : null}
          </Card>
        </section>
      ) : null}

      {step === 5 ? (
        <section aria-label="Re-check" className="space-y-3">
          <h2 className="text-lg font-semibold">Re-check</h2>
          <Button variant="outline" onClick={ws.recheckEnvironment}>
            Re-check my computer
          </Button>
          <ReadinessCard title="Windows tools" r={r.readiness.windows_tools} />
          {want.wsl ? <ReadinessCard title="Linux tools (WSL 2)" r={r.readiness.linux_tools_wsl} /> : null}
          {want.containers ? <ReadinessCard title="Container tools" r={r.readiness.containers} /> : null}
          <p role="status" className={done ? "text-success" : "text-muted-foreground"}>
            {done ? "Everything you chose is ready." : "Some things are still missing. Go back to “Set up” for the remaining steps; a restart may be required first."}
          </p>
          <Button
            onClick={() => {
              markSetupSeen();
              navigate("/");
            }}
          >
            {done ? "Finish" : "Finish for now"}
          </Button>
        </section>
      ) : null}

      <div className="flex justify-between">
        <Button variant="outline" onClick={back} disabled={step === 0}>
          Back
        </Button>
        {step < STEPS.length - 1 ? <Button onClick={next}>Next</Button> : null}
      </div>

      <ConfirmDialog
        open={confirming !== null}
        onOpenChange={(o) => !o && setConfirming(null)}
        title={confirming ? `${confirming.title}?` : ""}
        description="This changes your computer."
        confirmLabel="Run this step"
        busy={run.busy}
        confirmDisabled={!understood}
        onConfirm={() => void confirmRun()}
      >
        {confirming ? (
          <div className="space-y-2 text-sm">
            <p>{confirming.why}</p>
            <p>
              Exact command: <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs">{confirming.command_preview}</code>
            </p>
            <ul className="list-disc pl-5">
              {confirming.requires_admin ? <li>Windows will ask for administrator approval (UAC) for this one command. You can decline.</li> : <li>No administrator rights are needed.</li>}
              {confirming.requires_restart ? <li>Windows may need a restart afterwards. You will be asked separately; it never restarts by itself.</li> : null}
            </ul>
            <label className="flex items-start gap-2">
              <input type="checkbox" className="mt-1" checked={understood} onChange={(e) => setUnderstood(e.target.checked)} />
              <span>I understand and want Vixalune to run this command.</span>
            </label>
            {run.error ? <ErrorNotice error={run.error} /> : null}
          </div>
        ) : null}
      </ConfirmDialog>

      <ConfirmDialog
        open={restartDialog}
        onOpenChange={setRestartDialog}
        title="Restart Windows?"
        description="Windows will restart in about a minute. Unsaved work in other programs will be lost unless you save it first. You can cancel before it happens."
        confirmLabel="Schedule restart"
        busy={restart.busy}
        onConfirm={() => void confirmRestart()}
      />
    </div>
  );
}
