import { useMemo, useState } from "react";
import { ErrorNotice } from "@/components/ErrorNotice";
import { Button } from "@/components/ui/button";
import { Dialog } from "@/components/ui/dialog";
import type { AppErrorDto, CatalogEntry, ParameterDef, ParamValue, RunApprovals, RunPreview } from "@/domain/types";
import { toAppError } from "@/hooks/useAsync";
import { useBackend } from "@/infra/backendContext";

const field = "h-8 w-full rounded-md border border-border bg-card px-2 text-sm text-foreground";

function ParamField({ def, value, onChange }: { def: ParameterDef; value: ParamValue | undefined; onChange: (v: ParamValue) => void }) {
  const id = `param-${def.name}`;
  const label = (
    <span className="mb-1 block text-xs text-muted-foreground">
      {def.label}
      {def.required ? " (required)" : ""}
    </span>
  );
  const k = def.kind;
  if (k.type === "boolean") {
    return (
      <label className="flex items-center gap-2 text-sm">
        <input type="checkbox" checked={value === true} onChange={(e) => onChange(e.target.checked)} />
        {def.label}
      </label>
    );
  }
  if (k.type === "choice") {
    return (
      <label htmlFor={id}>
        {label}
        <select id={id} className={field} value={String(value ?? "")} onChange={(e) => onChange(e.target.value)}>
          {k.options.map((o) => (
            <option key={o}>{o}</option>
          ))}
        </select>
      </label>
    );
  }
  if (k.type === "integer") {
    return (
      <label htmlFor={id}>
        {label}
        <input id={id} type="number" className={field} min={k.min} max={k.max} value={String(value ?? "")} onChange={(e) => onChange(Number(e.target.value))} />
      </label>
    );
  }
  return (
    <label htmlFor={id}>
      {label}
      <input id={id} type="text" className={field} value={String(value ?? "")} onChange={(e) => onChange(e.target.value)} />
      {def.description ? <span className="mt-0.5 block text-xs text-muted-foreground">{def.description}</span> : null}
    </label>
  );
}

function Summary({ preview }: { preview: RunPreview }) {
  const c = preview.container;
  if (!c) return <p className="font-mono text-xs">{preview.command.display}</p>;
  return (
    <div className="space-y-2 rounded-md border border-border bg-background p-3 text-sm" aria-label="What will run">
      <dl className="space-y-1">
        <div>
          <dt className="inline text-muted-foreground">Image: </dt>
          <dd className="inline break-all font-mono text-xs">{c.image}</dd>
        </div>
        <div>
          <dt className="inline text-muted-foreground">Command: </dt>
          <dd className="inline font-mono text-xs">{c.command}</dd>
        </div>
        <div>
          <dt className="inline text-muted-foreground">Runs as: </dt>
          <dd className="inline">non-root user {c.user}, no extra capabilities{c.read_only_rootfs ? ", read-only root filesystem" : ""}</dd>
        </div>
        <div>
          <dt className="inline text-muted-foreground">Network: </dt>
          <dd className="inline font-medium">{c.network === "none" ? "none (no internet access)" : "ALLOWED (internet and local network)"}</dd>
        </div>
        <div>
          <dt className="inline text-muted-foreground">Folders: </dt>
          <dd className="inline">
            {c.mounts.length === 0
              ? "none"
              : c.mounts.map((m) => `${m.host} → ${m.container} (${m.read_only ? "read-only" : "READ-WRITE"})`).join("; ")}
          </dd>
        </div>
        <div>
          <dt className="inline text-muted-foreground">Ports: </dt>
          <dd className="inline">{c.ports.length === 0 ? "none" : c.ports.map(([h, p]) => `127.0.0.1:${h} → ${p}`).join(", ")}</dd>
        </div>
        <div>
          <dt className="inline text-muted-foreground">Limits: </dt>
          <dd className="inline">{c.limits}</dd>
        </div>
      </dl>
      {c.warnings.map((w) => (
        <p key={w} role="note" className="text-xs text-warning">
          {w}
        </p>
      ))}
    </div>
  );
}

/**
 * Review-and-approve dialog for launching a tool. Nothing starts until the user has reviewed the exact
 * container image, folders, network and ports and ticked the approval; any edit invalidates the review.
 */
export function LaunchDialog({
  entry,
  open,
  onOpenChange,
  onStarted,
}: {
  entry: CatalogEntry;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onStarted: (message: string) => void;
}) {
  const m = entry.manifest;
  const backend = useBackend();
  const profiles = m.launch_profiles;
  const [profileId, setProfileId] = useState(profiles[0]?.id ?? "");
  const profile = profiles.find((p) => p.id === profileId);
  const [values, setValues] = useState<Record<string, ParamValue>>({});
  const [mounts, setMounts] = useState<Record<string, string>>({});
  const [ports, setPorts] = useState<Record<number, string>>({});
  const [network, setNetwork] = useState(false);
  const [approved, setApproved] = useState(false);
  const [preview, setPreview] = useState<RunPreview | null>(null);
  const [error, setError] = useState<AppErrorDto | null>(null);
  const [busy, setBusy] = useState(false);

  const effectiveValues = useMemo(() => {
    const v: Record<string, ParamValue> = {};
    for (const p of profile?.parameters ?? []) {
      const given = values[p.name] ?? p.default ?? undefined;
      if (given !== undefined && given !== "") v[p.name] = given;
    }
    return v;
  }, [profile, values]);

  const approvals: RunApprovals = useMemo(
    () => ({
      network: profile?.needs_network ? network : false,
      mounts: Object.fromEntries((profile?.mounts ?? []).map((n) => [n, mounts[n] ?? ""]).filter(([, v]) => v !== "")),
      ports: Object.fromEntries((profile?.ports ?? []).map((p) => [p, Number(ports[p])]).filter(([, v]) => Number.isFinite(v) && Number(v) > 0)),
    }),
    [profile, network, mounts, ports],
  );

  const invalidate = () => {
    setPreview(null);
    setApproved(false);
    setError(null);
  };

  const review = async () => {
    if (!profile) return;
    setBusy(true);
    setError(null);
    try {
      setPreview(await backend.previewLaunch(m.id, profile.id, effectiveValues, approvals));
    } catch (e) {
      setPreview(null);
      setError(toAppError(e));
    } finally {
      setBusy(false);
    }
  };

  const run = async () => {
    if (!profile) return;
    setBusy(true);
    setError(null);
    try {
      await backend.launchTool(m.id, profile.id, effectiveValues, approvals);
      onStarted(`${m.name} was started. Output is shown under Running Jobs.`);
      onOpenChange(false);
    } catch (e) {
      setError(toAppError(e));
    } finally {
      setBusy(false);
    }
  };

  const notContainer = profile && profile.runtime !== "container";

  return (
    <Dialog open={open} onOpenChange={onOpenChange} title={`Launch ${m.name}`} description="Review exactly what will run, then approve it." wide>
      {!profile ? (
        <p className="text-muted-foreground">This tool has no launch profile yet.</p>
      ) : (
        <div className="space-y-4">
          {profiles.length > 1 ? (
            <label>
              <span className="mb-1 block text-xs text-muted-foreground">What to run</span>
              <select
                className={field}
                value={profileId}
                onChange={(e) => {
                  setProfileId(e.target.value);
                  invalidate();
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

          {notContainer ? (
            <p role="note" className="text-sm text-muted-foreground">
              Launching Windows and WSL tools from Vixalune is not available yet. Only container tools can be launched in this version.
            </p>
          ) : (
            <>
              {profile.parameters.map((def) => (
                <ParamField
                  key={def.name}
                  def={def}
                  value={values[def.name] ?? def.default ?? undefined}
                  onChange={(v) => {
                    setValues((s) => ({ ...s, [def.name]: v }));
                    invalidate();
                  }}
                />
              ))}
              {profile.mounts.map((name) => {
                const spec = m.container?.mounts.find((x) => x.name === name);
                return (
                  <label key={name} className="block">
                    <span className="mb-1 block text-xs text-muted-foreground">
                      Folder to share ({name}) — appears inside the container as {spec?.container_path} ({spec?.read_only === false ? "read-write" : "read-only"})
                    </span>
                    <input
                      type="text"
                      aria-label={`Folder for ${name}`}
                      className={field}
                      placeholder="D:\labs\my-project"
                      value={mounts[name] ?? ""}
                      onChange={(e) => {
                        setMounts((s) => ({ ...s, [name]: e.target.value }));
                        invalidate();
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
                    className={field}
                    min={1024}
                    max={65535}
                    value={ports[p] ?? ""}
                    onChange={(e) => {
                      setPorts((s) => ({ ...s, [p]: e.target.value }));
                      invalidate();
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
                      invalidate();
                    }}
                  />
                  <span>Allow this run to use the network. Without it the tool cannot start (it needs to download something).</span>
                </label>
              ) : (
                <p className="text-xs text-muted-foreground">This run has no network access.</p>
              )}
              <div className="flex gap-2">
                <Button variant="outline" onClick={() => void review()} disabled={busy}>
                  Review what will run
                </Button>
              </div>
              {preview ? (
                <>
                  <Summary preview={preview} />
                  <label className="flex items-start gap-2 text-sm">
                    <input type="checkbox" className="mt-1" checked={approved} onChange={(e) => setApproved(e.target.checked)} />
                    <span>I have reviewed this and approve exactly what is listed above.</span>
                  </label>
                </>
              ) : null}
            </>
          )}
          {error ? <ErrorNotice error={error} /> : null}
          <div className="flex justify-end gap-2">
            <Button variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button disabled={!preview || !approved || busy || !!notContainer} onClick={() => void run()}>
              {busy ? "Working…" : "Run"}
            </Button>
          </div>
        </div>
      )}
    </Dialog>
  );
}
