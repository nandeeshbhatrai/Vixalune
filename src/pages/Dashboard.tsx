import { CheckCircle2, CircleHelp, MinusCircle, XCircle } from "lucide-react";
import { Link } from "react-router-dom";
import { AsyncView } from "@/components/AsyncView";
import { DemoTag } from "@/components/DemoTag";
import { ErrorNotice } from "@/components/ErrorNotice";
import { PageHeader } from "@/components/PageHeader";
import { StatusBadge } from "@/components/StatusBadge";
import { ToolIcon } from "@/components/ToolIcon";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardTitle } from "@/components/ui/card";
import type { ComponentStatus } from "@/domain/types";
import { useAsync } from "@/hooks/useAsync";
import { useWorkspace } from "@/hooks/useWorkspace";
import { useBackend } from "@/infra/backendContext";
import { formatTime, PREREQ_LABEL } from "@/presentation/labels";

const INSTALL_KINDS = new Set(["install_started", "install_succeeded", "install_failed", "uninstalled", "verified"]);

function ComponentRow({ c }: { c: ComponentStatus }) {
  const Icon = c.state === "available" ? CheckCircle2 : c.state === "missing" ? MinusCircle : CircleHelp;
  const tone = c.state === "available" ? "text-success" : c.state === "missing" ? "text-warning" : "text-muted-foreground";
  return (
    <li className="flex items-center gap-2">
      <Icon size={16} className={tone} aria-hidden />
      <span>{PREREQ_LABEL[c.component]}</span>
      <span className="text-muted-foreground">
        {c.state === "available" ? (c.version ? `found (${c.version})` : "found") : c.state === "missing" ? "not found" : `could not check: ${c.reason}`}
      </span>
    </li>
  );
}

function Stat({ label, value, note }: { label: string; value: string; note?: string }) {
  return (
    <Card>
      <div className="flex items-center justify-between">
        <p className="text-xs uppercase tracking-wider text-muted-foreground">{label}</p>
        <DemoTag />
      </div>
      <p className="mt-1 text-2xl font-semibold tabular-nums">{value}</p>
      {note ? <p className="mt-0.5 text-xs text-muted-foreground">{note}</p> : null}
    </Card>
  );
}

export function Dashboard() {
  const backend = useBackend();
  const ws = useWorkspace();
  const caps = ws.core.status === "ready" ? ws.core.data.caps : null;
  const [installed] = useAsync(() => (caps?.installed_detection ? backend.listInstalledTools() : Promise.resolve(null)), [backend, caps]);
  const [jobs] = useAsync(() => (caps?.jobs ? backend.listJobs() : Promise.resolve(null)), [backend, caps]);
  const [activity, retryActivity] = useAsync(() => backend.recentActivity(30), [backend]);

  const installedCount = installed.status === "ready" && installed.data ? String(installed.data.length) : "—";
  const activeJobs =
    jobs.status === "ready" && jobs.data ? String(jobs.data.filter((j) => j.state === "running" || j.state === "queued").length) : "—";

  return (
    <div className="space-y-5">
      <PageHeader
        title="Dashboard"
        subtitle="Your environment and tools at a glance."
        actions={
          <Button asChild>
            <Link to="/catalog">Browse tools</Link>
          </Button>
        }
      />

      {ws.core.status === "error" ? <ErrorNotice error={ws.core.error} onRetry={ws.refreshAll} /> : null}

      <div className="grid gap-4 sm:grid-cols-3">
        <Stat
          label="Installed tools"
          value={installedCount}
          note={installedCount === "—" ? "Installed-tool detection is not available yet." : undefined}
        />
        <Stat label="Catalog tools" value={ws.core.status === "ready" ? String(ws.core.data.tools.length) : "—"} />
        <Stat label="Active jobs" value={activeJobs} note={activeJobs === "—" ? "Job tracking is not available yet." : undefined} />
      </div>

      <div className="grid gap-4 lg:grid-cols-2">
        <Card>
          <div className="flex items-center justify-between">
            <CardTitle>Environment readiness</CardTitle>
            <DemoTag />
          </div>
          {ws.envLoading ? (
            <p role="status" className="text-muted-foreground">
              Checking your environment…
            </p>
          ) : ws.envError ? (
            <ErrorNotice error={ws.envError} onRetry={ws.recheckEnvironment} />
          ) : ws.env ? (
            <div className="space-y-3">
              <ul className="space-y-1.5" aria-label="Environment checklist">
                <li className="flex items-center gap-2">
                  {ws.env.supported_os ? <CheckCircle2 size={16} className="text-success" aria-hidden /> : <XCircle size={16} className="text-destructive" aria-hidden />}
                  <span>Windows</span>
                  <span className="text-muted-foreground">{ws.env.supported_os ? "supported" : "unsupported operating system"}</span>
                </li>
                {ws.env.components.map((c) => (
                  <ComponentRow key={c.component} c={c} />
                ))}
              </ul>
              {ws.env.issues.length > 0 ? (
                <section aria-label="Setup issues" className="space-y-2">
                  {ws.env.issues.map((i) => (
                    <div key={i.title} className="rounded-md bg-muted p-2.5">
                      <p className="font-medium">{i.title}</p>
                      <p className="text-muted-foreground">{i.recovery}</p>
                    </div>
                  ))}
                </section>
              ) : (
                <p className="text-muted-foreground">No setup issues found.</p>
              )}
              <Button variant="outline" size="sm" onClick={ws.recheckEnvironment}>
                Re-check
              </Button>
            </div>
          ) : null}
        </Card>

        <Card>
          <div className="flex items-center justify-between">
            <CardTitle>Recent installations and errors</CardTitle>
            <DemoTag />
          </div>
          <AsyncView state={activity} retry={retryActivity} label="Loading activity">
            {(events) => {
              const relevant = events.filter((e) => INSTALL_KINDS.has(e.kind) || e.kind.endsWith("failed")).slice(0, 6);
              return relevant.length === 0 ? (
                <p className="text-muted-foreground">No installations or errors recorded yet.</p>
              ) : (
                <ul className="space-y-2">
                  {relevant.map((e, i) => (
                    <li key={`${e.at}-${i}`} className="flex items-start gap-2">
                      <Badge tone={e.kind.endsWith("failed") ? "danger" : e.kind === "install_succeeded" ? "success" : "neutral"}>
                        {e.kind.replaceAll("_", " ")}
                      </Badge>
                      <span className="min-w-0 flex-1">
                        {e.summary}
                        <span className="block text-xs text-muted-foreground">{formatTime(e.at)}</span>
                      </span>
                    </li>
                  ))}
                </ul>
              );
            }}
          </AsyncView>
        </Card>
      </div>

      <Card>
        <CardTitle>Recommended for beginners</CardTitle>
        {ws.core.status !== "ready" ? (
          <p className="text-muted-foreground">Loading the catalog…</p>
        ) : (
          <ul className="grid gap-3 sm:grid-cols-3">
            {ws.core.data.tools
              .filter((t) => t.manifest.recommended_for_beginners)
              .map(({ manifest: m }) => (
                <li key={m.id}>
                  <Link to={`/catalog/${m.id}`} className="flex h-full gap-3 rounded-md border border-border p-3 hover:border-primary/60">
                    <ToolIcon category={m.category} size={32} />
                    <span className="min-w-0 flex-1">
                      <span className="block font-medium">{m.name}</span>
                      <span className="block text-xs text-muted-foreground">{m.summary}</span>
                      <span className="mt-1.5 inline-block">
                        <StatusBadge status={ws.statusOf(m.id)} />
                      </span>
                    </span>
                  </Link>
                </li>
              ))}
          </ul>
        )}
      </Card>
    </div>
  );
}
