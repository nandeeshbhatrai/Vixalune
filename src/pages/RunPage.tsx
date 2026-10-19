import { BookOpen, Play } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Link, useNavigate, useParams, useSearchParams } from "react-router-dom";
import { AsyncView } from "@/components/AsyncView";
import { ErrorNotice } from "@/components/ErrorNotice";
import { PageHeader } from "@/components/PageHeader";
import { DemoTag } from "@/components/DemoTag";
import { OutputPanel } from "@/components/run/OutputPanel";
import { fieldClass, ParamField } from "@/components/run/ParamFields";
import { PreviewPanel } from "@/components/run/PreviewPanel";
import { RunHistory } from "@/components/run/RunHistory";
import { Button } from "@/components/ui/button";
import { Card, CardTitle } from "@/components/ui/card";
import type { AppErrorDto, CatalogEntry, ParamValue, RunApprovals, RunPreview, RunSpec } from "@/domain/types";
import { toAppError, useAsync } from "@/hooks/useAsync";
import { useWorkspace } from "@/hooks/useWorkspace";
import { useBackend } from "@/infra/backendContext";
import { STATUS_META } from "@/presentation/labels";

/** The addresses the user is shown when confirming; the backend compares them with a fresh resolution at run time. */
function addressesOf(preview: RunPreview | null): string[] {
  if (!preview?.scope) return [];
  return [...new Set(preview.scope.targets.flatMap((t) => t.addresses))].sort();
}

function RunForm({ entry, onStarted }: { entry: CatalogEntry; onStarted: (jobId: string) => void }) {
  const m = entry.manifest;
  const backend = useBackend();
  const ws = useWorkspace();
  const profiles = m.launch_profiles;
  const [search] = useSearchParams();
  const requested = search.get("profile");
  const [profileId, setProfileId] = useState(profiles.some((p) => p.id === requested) ? (requested as string) : (profiles[0]?.id ?? ""));
  const profile: RunSpec | undefined = profiles.find((p) => p.id === profileId) ?? profiles[0];

  const [values, setValues] = useState<Record<string, ParamValue>>({});
  const [mounts, setMounts] = useState<Record<string, string>>({});
  const [ports, setPorts] = useState<Record<number, string>>({});
  const [network, setNetwork] = useState(false);
  const [preview, setPreview] = useState<RunPreview | null>(null);
  const [reviewed, setReviewed] = useState(false);
  const [scopeAck, setScopeAck] = useState(false);
  const [typed, setTyped] = useState("");
  const [error, setError] = useState<AppErrorDto | null>(null);
  const [busy, setBusy] = useState(false);

  const [defaults] = useAsync(() => (profile ? backend.getLaunchDefaults(m.id, profile.id) : Promise.resolve({ parameters: {}, mounts: {} })), [backend, m.id, profile?.id]);

  const resetReview = () => {
    setPreview(null);
    setReviewed(false);
    setScopeAck(false);
    setTyped("");
    setError(null);
  };

  // New profile or fresh defaults: start the form from the safe defaults (lab placeholders already resolved).
  useEffect(() => {
    if (defaults.status !== "ready") return;
    setValues(defaults.data.parameters);
    setMounts(defaults.data.mounts);
    setPorts({});
    setNetwork(false);
    resetReview();
  }, [defaults]);

  const approvals: RunApprovals = useMemo(() => {
    const need = preview?.scope?.confirmation;
    return {
      network: profile?.needs_network ? network : false,
      mounts: Object.fromEntries((profile?.mounts ?? []).map((n) => [n, mounts[n] ?? ""]).filter(([, v]) => v !== "")),
      ports: Object.fromEntries((profile?.ports ?? []).map((p) => [p, Number(ports[p])]).filter(([, v]) => Number.isFinite(v) && Number(v) > 0)),
      scope: need ? { acknowledged: scopeAck, typed: need.level === "type_target" ? typed : null, addresses: addressesOf(preview) } : null,
    };
  }, [profile, network, mounts, ports, preview, scopeAck, typed]);

  const effective = useMemo(() => {
    const v: Record<string, ParamValue> = {};
    for (const p of profile?.parameters ?? []) {
      const given = values[p.name];
      if (given !== undefined && given !== "") v[p.name] = given;
    }
    return v;
  }, [profile, values]);

  const review = async () => {
    if (!profile) return;
    setBusy(true);
    setError(null);
    try {
      setPreview(await backend.previewLaunch(m.id, profile.id, effective, { ...approvals, scope: null }));
      setReviewed(false);
      setScopeAck(false);
      setTyped("");
    } catch (e) {
      setPreview(null);
      setError(toAppError(e));
    } finally {
      setBusy(false);
    }
  };

  const confirmation = preview?.scope?.confirmation ?? null;
  const typedOk = confirmation?.level !== "type_target" || typed.trim().toLowerCase() === (confirmation.phrase ?? "").toLowerCase();
  const confirmed = !confirmation || (scopeAck && typedOk);
  const canRun = !!preview && reviewed && confirmed && !busy;

  const run = async () => {
    if (!profile || !preview) return;
    setBusy(true);
    setError(null);
    try {
      const id = await backend.launchTool(m.id, profile.id, effective, approvals);
      onStarted(id);
      ws.reloadJobs();
      resetReview();
    } catch (e) {
      setError(toAppError(e));
    } finally {
      setBusy(false);
    }
  };

  if (!profile) return <p className="text-muted-foreground">This tool has no launch profile yet.</p>;
  const status = ws.statusOf(m.id);

  return (
    <div className="space-y-4">
      {status !== "installed" ? (
        <p role="note" className="rounded-md border border-warning/40 bg-warning/10 p-2.5 text-sm text-warning">
          {m.name} does not show as installed ({STATUS_META[status].label.toLowerCase()}). You can review a run, but starting it needs the tool.{" "}
          <Link to={`/catalog/${m.id}`} className="underline">
            Open the tool page
          </Link>
        </p>
      ) : null}
      {profiles.length > 1 ? (
        <label className="block">
          <span className="mb-1 block text-xs text-muted-foreground">What to run</span>
          <select
            className={fieldClass}
            value={profile.id}
            onChange={(e) => {
              setProfileId(e.target.value);
            }}
          >
            {profiles.map((p) => (
              <option key={p.id} value={p.id}>
                {p.label}
              </option>
            ))}
          </select>
        </label>
      ) : (
        <p className="font-medium">{profile.label}</p>
      )}
      {profile.description ? <p className="text-muted-foreground">{profile.description}</p> : null}

      {defaults.status === "error" ? <ErrorNotice error={defaults.error} /> : null}
      {profile.parameters.map((def) => (
        <ParamField
          key={`${profile.id}-${def.name}`}
          def={def}
          value={values[def.name]}
          onChange={(v) => {
            setValues((s) => ({ ...s, [def.name]: v }));
            resetReview();
          }}
        />
      ))}

      {profile.mounts.map((name) => {
        const spec = m.container?.mounts.find((x) => x.name === name);
        return (
          <label key={name} className="block">
            <span className="mb-1 block text-xs text-muted-foreground">
              Folder to share ({name}). It appears inside the container as {spec?.container_path} ({spec?.read_only === false ? "read-write" : "read-only"})
            </span>
            <input
              type="text"
              aria-label={`Folder for ${name}`}
              className={fieldClass}
              placeholder="D:\labs\my-project"
              value={mounts[name] ?? ""}
              onChange={(e) => {
                setMounts((s) => ({ ...s, [name]: e.target.value }));
                resetReview();
              }}
            />
            <span className="mt-0.5 block text-xs text-muted-foreground">Only this folder is shared. Drives, Windows, Program Files, your whole profile and credential folders are refused.</span>
          </label>
        );
      })}
      {profile.ports.map((p) => (
        <label key={p} className="block">
          <span className="mb-1 block text-xs text-muted-foreground">Publish container port {p} on 127.0.0.1 (this computer only) as host port</span>
          <input
            type="number"
            aria-label={`Host port for ${p}`}
            className={fieldClass}
            min={1024}
            max={65535}
            value={ports[p] ?? ""}
            onChange={(e) => {
              setPorts((s) => ({ ...s, [p]: e.target.value }));
              resetReview();
            }}
          />
        </label>
      ))}
      {profile.needs_network ? (
        <label className="flex items-start gap-2 text-sm">
          <input
            type="checkbox"
            className="mt-1"
            checked={network}
            onChange={(e) => {
              setNetwork(e.target.checked);
              resetReview();
            }}
          />
          <span>Allow this run to use the network. Without it the tool cannot start (it needs to download something).</span>
        </label>
      ) : null}

      <div className="flex gap-2">
        <Button variant="outline" onClick={() => void review()} disabled={busy || defaults.status === "loading"}>
          Review what will run
        </Button>
      </div>
      {error ? <ErrorNotice error={error} /> : null}

      {preview ? (
        <div className="space-y-3">
          <PreviewPanel preview={preview} />
          {confirmation ? (
            <div role="group" aria-label="Confirm the target" className="space-y-2 rounded-md border border-warning/40 bg-warning/10 p-3 text-sm">
              <p className="font-medium text-warning">This target is outside your lab.</p>
              <label className="flex items-start gap-2">
                <input type="checkbox" className="mt-1" checked={scopeAck} onChange={(e) => setScopeAck(e.target.checked)} />
                <span>I own this target or have written permission to test it, and I understand this run sends traffic to it.</span>
              </label>
              {confirmation.level === "type_target" ? (
                <label className="block">
                  <span className="mb-1 block text-xs text-muted-foreground">
                    Type <span className="font-mono">{confirmation.phrase}</span> to confirm
                  </span>
                  <input type="text" aria-label="Type the target to confirm" spellCheck={false} autoComplete="off" className={fieldClass} value={typed} onChange={(e) => setTyped(e.target.value)} />
                </label>
              ) : null}
            </div>
          ) : null}
          <label className="flex items-start gap-2 text-sm">
            <input type="checkbox" className="mt-1" checked={reviewed} onChange={(e) => setReviewed(e.target.checked)} />
            <span>I have reviewed the command and target above and want to run exactly this.</span>
          </label>
          <Button disabled={!canRun} onClick={() => void run()}>
            <Play size={14} aria-hidden /> {busy ? "Starting…" : "Run"}
          </Button>
        </div>
      ) : null}
    </div>
  );
}

export function RunPage() {
  const backend = useBackend();
  const ws = useWorkspace();
  const navigate = useNavigate();
  const { toolId } = useParams();
  const [activeRun, setActiveRun] = useState<string | null>(null);
  const [runs, reloadRuns] = useAsync(() => backend.listRuns(30), [backend]);
  const reloadHistory = useCallback(() => reloadRuns(), [reloadRuns]);

  return (
    <div className="space-y-4">
      <PageHeader title="Run a Tool" subtitle="Choose a tool and a safe, ready-made way to run it. You see exactly what will run before anything starts." actions={<DemoTag />} />
      <AsyncView state={ws.core} retry={ws.refreshAll} label="Loading tools">
        {({ tools }) => {
          const runnable = tools.filter((t) => t.manifest.launch_profiles.length > 0);
          const selected = runnable.find((t) => t.manifest.id === toolId) ?? runnable[0];
          if (!selected) return <p className="text-muted-foreground">No tool in the catalog has a launch profile yet.</p>;
          return (
            <>
              <Card className="space-y-3">
                <CardTitle>Tool</CardTitle>
                <div className="flex flex-wrap items-end gap-3">
                  <label className="min-w-56">
                    <span className="mb-1 block text-xs text-muted-foreground">Tool</span>
                    <select
                      className={fieldClass}
                      value={selected.manifest.id}
                      onChange={(e) => {
                        setActiveRun(null);
                        navigate(`/run/${e.target.value}`);
                      }}
                    >
                      {runnable.map((t) => (
                        <option key={t.manifest.id} value={t.manifest.id}>
                          {t.manifest.name}
                        </option>
                      ))}
                    </select>
                  </label>
                  <Link to={`/learn/${selected.manifest.id}`} className="inline-flex items-center gap-1.5 text-primary hover:underline">
                    <BookOpen size={14} aria-hidden /> Learn {selected.manifest.name}
                  </Link>
                </div>
                <RunForm
                  key={selected.manifest.id}
                  entry={selected}
                  onStarted={(id) => {
                    setActiveRun(id);
                    reloadHistory();
                  }}
                />
              </Card>
              {activeRun ? <OutputPanel jobId={activeRun} onFinished={reloadHistory} /> : null}
            </>
          );
        }}
      </AsyncView>
      <RunHistory runs={runs} retry={reloadRuns} selectedId={activeRun} onView={setActiveRun} />
    </div>
  );
}
