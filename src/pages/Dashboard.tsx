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
import { useAsync } from "@/hooks/useAsync";
import { useWorkspace } from "@/hooks/useWorkspace";
import { useBackend } from "@/infra/backendContext";
import { formatTime, isActivePhase, READINESS_META } from "@/presentation/labels";

const INSTALL_KINDS = new Set(["install_started", "install_succeeded", "install_failed", "uninstalled", "verified"]);

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
  const [activity, retryActivity] = useAsync(() => backend.recentActivity(30), [backend]);

  const installedCount = installed.status === "ready" && installed.data ? String(installed.data.length) : "—";
  const activeJobs = caps?.jobs && ws.jobs.status === "ready" ? String(ws.jobs.data.filter((j) => isActivePhase(j.phase)).length) : "—";

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
          ) : ws.platform ? (
            <div className="space-y-3">
              <ul className="space-y-2" aria-label="Environment checklist">
                {(
                  [
                    ["Windows tools (winget)", ws.platform.readiness.windows_tools],
                    ["Linux tools (WSL 2)", ws.platform.readiness.linux_tools_wsl],
                    ["Container tools (Docker)", ws.platform.readiness.containers],
                  ] as const
                ).map(([name, r]) => (
                  <li key={name} className="flex items-start gap-2">
                    <Badge tone={READINESS_META[r.state].tone}>{READINESS_META[r.state].label}</Badge>
                    <span className="min-w-0 flex-1">
                      <span className="font-medium">{name}</span>
                      <span className="block text-muted-foreground">{r.summary}</span>
                      {(r.blockers[0] ?? r.missing[0]) ? <span className="block text-xs text-muted-foreground">{r.blockers[0] ?? `Missing: ${r.missing[0]}`}</span> : null}
                    </span>
                  </li>
                ))}
              </ul>
              <div className="flex gap-2">
                <Button size="sm" asChild>
                  <Link to="/setup">{ws.platform.setup_actions.length > 0 ? "Open Setup" : "Review setup"}</Link>
                </Button>
                <Button variant="outline" size="sm" onClick={ws.recheckEnvironment}>
                  Re-check
                </Button>
              </div>
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


