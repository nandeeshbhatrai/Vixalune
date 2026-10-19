import { CheckCircle2, ShieldCheck } from "lucide-react";
import { useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import { DemoTag } from "@/components/DemoTag";
import { ErrorNotice } from "@/components/ErrorNotice";
import { PageHeader } from "@/components/PageHeader";
import { fieldClass } from "@/components/run/ParamFields";
import { StatusBadge } from "@/components/StatusBadge";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardTitle } from "@/components/ui/card";
import type { CapabilityReadiness, StorageInfo } from "@/domain/types";
import { useAction } from "@/hooks/useAction";
import { useAsync } from "@/hooks/useAsync";
import { useWorkspace } from "@/hooks/useWorkspace";
import { useBackend } from "@/infra/backendContext";
import { CATEGORY_LABEL, READINESS_META } from "@/presentation/labels";
import { markSetupSeen } from "@/pages/SetupWizard";

const STEPS = ["Welcome", "Your computer", "Where things are stored", "Permissions and safety", "The catalog", "Your first tools"] as const;

function Readiness({ title, r, optional }: { title: string; r: CapabilityReadiness | undefined; optional?: boolean }) {
  const meta = r ? READINESS_META[r.state] : null;
  return (
    <li className="rounded-md border border-border bg-card p-3">
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-medium">{title}</span>
        {optional ? <Badge>Optional</Badge> : null}
        {meta ? <Badge tone={meta.tone}>{meta.label}</Badge> : <Badge>Checking…</Badge>}
      </div>
      {r ? <p className="mt-1 text-sm text-muted-foreground">{r.summary}</p> : null}
      {r && r.missing.length > 0 ? <p className="text-xs text-muted-foreground">Missing: {r.missing.join(", ")}</p> : null}
      {r && r.blockers.length > 0 ? <p className="text-xs text-destructive">{r.blockers.join(" ")}</p> : null}
    </li>
  );
}

function StorageForm({ info, onSaved }: { info: StorageInfo; onSaved: (i: StorageInfo) => void }) {
  const backend = useBackend();
  const { busy, error, run, clearError } = useAction();
  const [tools, setTools] = useState(info.tools_dir);
  const [labs, setLabs] = useState(info.labs_dir);
  const changed = tools !== info.tools_dir || labs !== info.labs_dir;
  const save = async () => {
    const next = await run(() => backend.updateStorage({ tools_dir: tools.trim(), labs_dir: labs.trim() }));
    if (next) onSaved(next);
  };
  return (
    <div className="space-y-3">
      <dl className="space-y-1 text-sm">
        <div>
          <dt className="inline text-muted-foreground">Settings, database, logs and run folders: </dt>
          <dd className="inline break-all font-mono text-xs">{info.data_dir}</dd>
        </div>
        <p className="text-xs text-muted-foreground">
          This folder holds your data and is kept separate from the program files, so updating or removing Vixalune does not delete it unless you ask.
          {info.data_dir_overridden ? " It was chosen with the VIXALUNE_DATA_DIR environment variable." : ""}
        </p>
      </dl>
      <label className="block">
        <span className="mb-1 block text-xs text-muted-foreground">Tools folder (for tools Vixalune keeps itself)</span>
        <input
          type="text"
          aria-label="Tools folder"
          className={fieldClass}
          spellCheck={false}
          value={tools}
          onChange={(e) => {
            setTools(e.target.value);
            clearError();
          }}
        />
      </label>
      <label className="block">
        <span className="mb-1 block text-xs text-muted-foreground">Practice labs folder (sample code you can open and edit)</span>
        <input
          type="text"
          aria-label="Practice labs folder"
          className={fieldClass}
          spellCheck={false}
          value={labs}
          onChange={(e) => {
            setLabs(e.target.value);
            clearError();
          }}
        />
        <span className="mt-0.5 block text-xs text-muted-foreground">Choose a folder outside AppData: scanners and containers refuse to read inside it.</span>
      </label>
      <div className="flex items-center gap-2">
        <Button variant="outline" disabled={!changed || busy} onClick={() => void save()}>
          Save locations
        </Button>
        {info.restart_required ? (
          <span role="status" className="text-xs text-warning">
            Saved. The new locations are used the next time you start Vixalune.
          </span>
        ) : null}
      </div>
      {error ? <ErrorNotice error={error} /> : null}
    </div>
  );
}

/**
 * First-run guide: what Vixalune is, whether this computer is ready, where data lives, what the permission prompts and
 * scope safeguards mean, the catalog, and a short list of tools to start with. Every step can be skipped and Vixalune is
 * useful without WSL or Docker.
 */
export function Welcome() {
  const backend = useBackend();
  const ws = useWorkspace();
  const navigate = useNavigate();
  const [step, setStep] = useState(0);
  const [storage, retryStorage] = useAsync(() => backend.getStorageInfo(), [backend]);
  const [saved, setSaved] = useState<StorageInfo | null>(null);

  const finish = (to: string) => {
    markSetupSeen();
    navigate(to);
  };

  const tools = ws.core.status === "ready" ? ws.core.data.tools : [];
  const starters = tools.filter((t) => t.manifest.recommended_for_beginners && t.manifest.install.some((i) => i.method === "winget")).slice(0, 3);
  const r = ws.platform?.readiness;

  return (
    <div className="space-y-5">
      <PageHeader title="Welcome to Vixalune" subtitle="A short tour. Nothing is installed or changed on your computer unless you confirm it." actions={<DemoTag />} />

      <ol className="flex flex-wrap gap-2 text-xs" aria-label="Welcome steps">
        {STEPS.map((s, i) => (
          <li key={s} aria-current={i === step ? "step" : undefined}>
            <button type="button" onClick={() => setStep(i)} className={`rounded-md px-2.5 py-1 ${i === step ? "bg-primary text-primary-foreground" : "bg-muted text-muted-foreground hover:text-foreground"}`}>
              {i + 1}. {s}
            </button>
          </li>
        ))}
      </ol>

      {step === 0 ? (
        <section aria-label="Welcome" className="space-y-3">
          <Card>
            <h2 className="text-lg font-semibold">What Vixalune is</h2>
            <p className="mt-1">
              Vixalune is a workbench for learning security testing. It helps you install a small, curated set of open-source security tools, run them with guarded settings against practice targets on your own computer, and understand the results.
            </p>
            <p className="mt-2 text-muted-foreground">It is for authorized testing and learning only: your own computers, the built-in practice labs, and systems you have written permission to test.</p>
          </Card>
          <Card>
            <CardTitle>Where it works</CardTitle>
            <ul className="list-disc space-y-1 pl-5">
              <li>Windows 11, or Windows 10 version 22H2, 64-bit (x64).</li>
              <li>
                <strong>Windows tools</strong> (Nmap, OWASP ZAP, Trivy) need only Windows and the Windows Package Manager (winget).
              </li>
              <li>
                <strong>Linux tools</strong> need WSL 2 and <strong>container tools</strong> (Semgrep) need Docker Desktop. Both are optional.
              </li>
            </ul>
            <p className="mt-2 text-sm text-muted-foreground">
              Without WSL or Docker you can still browse the catalog, work through the tutorials, start the practice labs and run the Windows tools. Missing pieces are explained where they matter, never hidden.
            </p>
          </Card>
        </section>
      ) : null}

      {step === 1 ? (
        <section aria-label="Your computer" className="space-y-3">
          <h2 className="text-lg font-semibold">Is your computer ready?</h2>
          <p className="text-muted-foreground">This only looks; it changes nothing.</p>
          {ws.envError ? <ErrorNotice error={ws.envError} onRetry={ws.recheckEnvironment} /> : null}
          <ul className="space-y-2" aria-label="Readiness">
            <Readiness title="Windows tools (winget)" r={r?.windows_tools} />
            <Readiness title="Linux tools (WSL 2)" r={r?.linux_tools_wsl} optional />
            <Readiness title="Container tools (Docker)" r={r?.containers} optional />
          </ul>
          <p className="text-sm text-muted-foreground">
            WSL 2 and containers are not required to start. If you want them, the{" "}
            <Link to="/setup" onClick={markSetupSeen} className="text-primary hover:underline">
              Setup guide
            </Link>{" "}
            explains each prerequisite, shows the exact command, and asks before changing anything. It never restarts Windows without asking.
          </p>
        </section>
      ) : null}

      {step === 2 ? (
        <section aria-label="Where things are stored" className="space-y-3">
          <h2 className="text-lg font-semibold">Where things are stored</h2>
          {storage.status === "loading" ? <p role="status">Loading…</p> : null}
          {storage.status === "error" ? <ErrorNotice error={storage.error} onRetry={retryStorage} /> : null}
          {storage.status === "ready" ? (
            <Card>
              <StorageForm info={saved ?? storage.data} onSaved={setSaved} />
            </Card>
          ) : null}
        </section>
      ) : null}

      {step === 3 ? (
        <section aria-label="Permissions and safety" className="space-y-3">
          <h2 className="text-lg font-semibold">Permission prompts and safeguards</h2>
          <Card>
            <CardTitle>Permission prompts you may see</CardTitle>
            <ul className="list-disc space-y-1 pl-5">
              <li>Vixalune runs as a normal user and does not ask for administrator rights itself.</li>
              <li>Windows may show its own administrator prompt (UAC) when a tool&apos;s installer needs it, or for the single step that installs WSL. You see the exact command first, and you can decline.</li>
              <li>The Windows Package Manager (winget) accepts a package&apos;s license on your behalf during an install; the install dialog says so and links the license.</li>
              <li>Vixalune never changes security settings, bypasses organisation policies, or restarts Windows without asking.</li>
            </ul>
          </Card>
          <Card>
            <CardTitle className="flex items-center gap-2">
              <ShieldCheck size={14} aria-hidden /> Network-scope safeguards
            </CardTitle>
            <ul className="list-disc space-y-1 pl-5">
              <li>Every run starts from a safe default: this computer and the practice lab.</li>
              <li>Before a run you see the target, where it really points, and whether that is inside your lab.</li>
              <li>A private-network target needs a tick box; a public address needs you to type its name. Reserved and unspecified addresses are never allowed.</li>
              <li>Web addresses that redirect somewhere outside your lab are stopped. You can list networks you manage in Settings.</li>
            </ul>
            <p className="mt-2 text-xs text-muted-foreground">These checks prevent mistakes. They cannot tell whether you are allowed to test a target; that is always your responsibility.</p>
          </Card>
        </section>
      ) : null}

      {step === 4 ? (
        <section aria-label="The catalog" className="space-y-3">
          <h2 className="text-lg font-semibold">The curated catalog</h2>
          <p className="text-muted-foreground">Every tool definition ships with Vixalune and is checked when the app starts. Nothing is downloaded to build this list, so it works offline.</p>
          <ul className="grid gap-2 sm:grid-cols-2" aria-label="Catalog preview">
            {tools.map((t) => (
              <li key={t.manifest.id} className="rounded-md border border-border bg-card p-3">
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <Link to={`/catalog/${t.manifest.id}`} onClick={markSetupSeen} className="font-medium text-primary hover:underline">
                    {t.manifest.name}
                  </Link>
                  <StatusBadge status={ws.statusOf(t.manifest.id)} />
                </div>
                <p className="text-xs text-muted-foreground">{CATEGORY_LABEL[t.manifest.category]}</p>
                <p className="mt-1 text-sm">{t.manifest.summary}</p>
              </li>
            ))}
          </ul>
        </section>
      ) : null}

      {step === 5 ? (
        <section aria-label="Your first tools" className="space-y-3">
          <h2 className="text-lg font-semibold">Start with one or two tools</h2>
          <p className="text-muted-foreground">These install through winget and need no extra runtime. Each Install button opens the tool&apos;s page, where you review exactly what will happen before confirming.</p>
          {starters.length === 0 ? (
            <p>No starter tools are available in this catalog. Browse the full catalog instead.</p>
          ) : (
            <ul className="space-y-2" aria-label="Starter tools">
              {starters.map((t) => {
                const status = ws.statusOf(t.manifest.id);
                return (
                  <li key={t.manifest.id} className="flex flex-wrap items-center justify-between gap-2 rounded-md border border-border bg-card p-3">
                    <div>
                      <p className="font-medium">{t.manifest.name}</p>
                      <p className="text-sm text-muted-foreground">{t.manifest.summary}</p>
                    </div>
                    <div className="flex items-center gap-2">
                      <StatusBadge status={status} />
                      {status === "installed" ? (
                        <span className="flex items-center gap-1 text-success">
                          <CheckCircle2 size={14} aria-hidden /> Ready
                        </span>
                      ) : (
                        <Button size="sm" onClick={() => finish(`/catalog/${t.manifest.id}`)}>
                          Install {t.manifest.name}…
                        </Button>
                      )}
                    </div>
                  </li>
                );
              })}
            </ul>
          )}
          <div className="flex flex-wrap gap-2 pt-2">
            <Button onClick={() => finish("/")}>Finish and go to the dashboard</Button>
            <Button variant="outline" onClick={() => finish("/learn")}>
              Open the tutorials
            </Button>
          </div>
        </section>
      ) : null}

      <div className="flex items-center justify-between">
        <Button variant="outline" onClick={() => setStep((s) => Math.max(0, s - 1))} disabled={step === 0}>
          Back
        </Button>
        <div className="flex items-center gap-3">
          <button type="button" className="text-sm text-muted-foreground underline hover:text-foreground" onClick={() => finish("/")}>
            Skip the tour
          </button>
          {step < STEPS.length - 1 ? <Button onClick={() => setStep((s) => Math.min(STEPS.length - 1, s + 1))}>Next</Button> : null}
        </div>
      </div>
    </div>
  );
}
