import { ShieldAlert, ShieldCheck } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import type { RunPreview, ScopeReport } from "@/domain/types";
import { LOCALITY_LABEL, NETWORK_ACTIVITY_LABEL, OUTPUT_FORMAT_LABEL } from "@/presentation/labels";

function ContainerSummary({ preview }: { preview: RunPreview }) {
  const c = preview.container;
  if (!c) return null;
  return (
    <dl className="space-y-1">
      <div>
        <dt className="inline text-muted-foreground">Image: </dt>
        <dd className="inline break-all font-mono text-xs">{c.image}</dd>
      </div>
      <div>
        <dt className="inline text-muted-foreground">Runs as: </dt>
        <dd className="inline">non-root user {c.user}, no extra capabilities{c.read_only_rootfs ? ", read-only root filesystem" : ""}</dd>
      </div>
      <div>
        <dt className="inline text-muted-foreground">Container network: </dt>
        <dd className="inline font-medium">{c.network === "none" ? "none (no internet access)" : "ALLOWED (internet and local network)"}</dd>
      </div>
      <div>
        <dt className="inline text-muted-foreground">Folders: </dt>
        <dd className="inline">{c.mounts.length === 0 ? "none" : c.mounts.map((m) => `${m.host} → ${m.container} (${m.read_only ? "read-only" : "READ-WRITE"})`).join("; ")}</dd>
      </div>
      <div>
        <dt className="inline text-muted-foreground">Ports: </dt>
        <dd className="inline">{c.ports.length === 0 ? "none" : c.ports.map(([h, p]) => `127.0.0.1:${h} → ${p}`).join(", ")}</dd>
      </div>
      <div>
        <dt className="inline text-muted-foreground">Limits: </dt>
        <dd className="inline">{c.limits}</dd>
      </div>
      {c.warnings.map((w) => (
        <p key={w} role="note" className="text-xs text-warning">
          {w}
        </p>
      ))}
    </dl>
  );
}

export function ScopeSummary({ scope }: { scope: ScopeReport }) {
  const ok = scope.verdict === "allowed";
  const Icon = ok ? ShieldCheck : ShieldAlert;
  return (
    <section aria-label="Target and scope" className="space-y-2 rounded-md border border-border bg-card p-3">
      <h3 className="flex items-center gap-2 text-sm font-semibold">
        <Icon size={15} aria-hidden className={ok ? "text-success" : "text-warning"} />
        Target and scope
      </h3>
      <ul className="space-y-2">
        {scope.targets.map((t) => (
          <li key={t.parameter} className="text-sm">
            <div className="flex flex-wrap items-center gap-2">
              <span className="font-mono text-xs">{t.canonical}</span>
              <Badge tone={t.verdict === "allowed" ? "success" : t.verdict === "blocked" ? "danger" : "warning"}>{LOCALITY_LABEL[t.locality]}</Badge>
            </div>
            {t.addresses.length > 0 && t.addresses.join(", ") !== t.canonical ? <p className="text-xs text-muted-foreground">Resolves to: {t.addresses.join(", ")}</p> : null}
            {t.reasons.map((r) => (
              <p key={r} className="text-xs text-muted-foreground">
                {r}
              </p>
            ))}
          </li>
        ))}
      </ul>
      {scope.redirects.length > 0 ? (
        <div className="text-xs">
          <p className="font-medium">Redirects found (plain http only):</p>
          <ul className="list-disc pl-5 text-muted-foreground">
            {scope.redirects.map((r) => (
              <li key={`${r.from}${r.to}`}>
                {r.from} → {r.to} ({LOCALITY_LABEL[r.locality]})
              </li>
            ))}
          </ul>
        </div>
      ) : null}
      <p className="text-xs text-muted-foreground">
        Your lab: this computer{scope.lab_targets.length > 0 ? `, ${scope.lab_targets.join(", ")}` : ""}. https redirects cannot be checked. This check is a safety net, not proof that you are authorised to test the target.
      </p>
    </section>
  );
}

/** Everything the user should see before pressing Run: the exact command, who it talks to, limits and what to expect. */
export function PreviewPanel({ preview }: { preview: RunPreview }) {
  return (
    <div className="space-y-3" aria-label="What will run">
      <section className="space-y-2 rounded-md border border-border bg-background p-3 text-sm">
        <h3 className="text-sm font-semibold">Command</h3>
        <p className="break-all font-mono text-xs" aria-label="Command preview">
          {preview.command.display}
        </p>
        <dl className="space-y-1">
          <div>
            <dt className="inline text-muted-foreground">Network use: </dt>
            <dd className="inline">{NETWORK_ACTIVITY_LABEL[preview.network_activity]}</dd>
          </div>
          <div>
            <dt className="inline text-muted-foreground">Time limit: </dt>
            <dd className="inline">{preview.timeout_secs} seconds</dd>
          </div>
          <div>
            <dt className="inline text-muted-foreground">Output: </dt>
            <dd className="inline">{OUTPUT_FORMAT_LABEL[preview.output_format]}</dd>
          </div>
          <div>
            <dt className="inline text-muted-foreground">Starts in: </dt>
            <dd className="inline">{preview.working_dir}</dd>
          </div>
          {preview.expected_output ? (
            <div>
              <dt className="inline text-muted-foreground">What to expect: </dt>
              <dd className="inline">{preview.expected_output}</dd>
            </div>
          ) : null}
        </dl>
        <ContainerSummary preview={preview} />
        {preview.notes.map((n) => (
          <p key={n} role="note" className="text-xs text-muted-foreground">
            {n}
          </p>
        ))}
      </section>
      {preview.scope ? <ScopeSummary scope={preview.scope} /> : null}
    </div>
  );
}
