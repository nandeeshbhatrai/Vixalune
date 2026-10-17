import { ArrowLeft } from "lucide-react";
import { Link, useParams } from "react-router-dom";
import { AsyncView } from "@/components/AsyncView";
import { ExternalLink } from "@/components/ExternalLink";
import { StatusBadge } from "@/components/StatusBadge";
import { ToolActions } from "@/components/ToolActions";
import { ToolIcon } from "@/components/ToolIcon";
import { Badge } from "@/components/ui/badge";
import { Card, CardTitle } from "@/components/ui/card";
import { useAsync } from "@/hooks/useAsync";
import { useWorkspace } from "@/hooks/useWorkspace";
import { useBackend } from "@/infra/backendContext";
import { CATEGORY_LABEL, describeInstall, MAINTENANCE_LABEL, METHOD_LABEL, PREREQ_LABEL, RUNTIME_LABEL } from "@/presentation/labels";

export function ToolDetail() {
  const { id = "" } = useParams();
  const backend = useBackend();
  const ws = useWorkspace();
  const [state, retry] = useAsync(() => backend.getTool(id), [backend, id]);

  return (
    <div className="space-y-4">
      <Link to="/catalog" className="inline-flex items-center gap-1 text-muted-foreground hover:text-foreground">
        <ArrowLeft size={14} aria-hidden /> Back to catalog
      </Link>
      <AsyncView state={state} retry={retry} label="Loading tool">
        {(entry) => {
          const m = entry.manifest;
          const missing = ws.missingFor(m.id);
          const envAvailable = (p: string) => ws.env?.components.some((c) => c.component === p && c.state === "available");
          return (
            <>
              <div className="flex items-start gap-4">
                <ToolIcon category={m.category} size={48} />
                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <h1 className="text-xl font-semibold tracking-tight">{m.name}</h1>
                    <StatusBadge status={ws.statusOf(m.id)} />
                    <Badge>{CATEGORY_LABEL[m.category]}</Badge>
                  </div>
                  <p className="mt-1 text-muted-foreground">{m.summary}</p>
                </div>
              </div>

              <Card>
                <ToolActions entry={entry} />
              </Card>

              <p>{m.description}</p>

              <div className="grid gap-4 md:grid-cols-2">
                <Card>
                  <CardTitle>Requirements</CardTitle>
                  <dl className="space-y-2.5">
                    <div>
                      <dt className="text-xs text-muted-foreground">Runs on</dt>
                      <dd className="mt-1 flex flex-wrap gap-1.5">
                        {m.platforms.map((p) => (
                          <Badge key={p} tone="info">
                            {RUNTIME_LABEL[p]}
                          </Badge>
                        ))}
                      </dd>
                    </div>
                    <div>
                      <dt className="text-xs text-muted-foreground">Prerequisites</dt>
                      <dd className="mt-1 flex flex-wrap gap-1.5">
                        {m.prerequisites.length === 0 ? (
                          <span>None</span>
                        ) : (
                          m.prerequisites.map((p) => (
                            <Badge key={p} tone={envAvailable(p) ? "success" : missing ? "warning" : "neutral"}>
                              {PREREQ_LABEL[p]}
                              {ws.env ? (envAvailable(p) ? " · found" : " · missing") : " · not checked"}
                            </Badge>
                          ))
                        )}
                      </dd>
                    </div>
                    <div>
                      <dt className="text-xs text-muted-foreground">How it is installed</dt>
                      <dd>
                        {m.install.map((i) => (
                          <span key={describeInstall(i)} className="block" title={METHOD_LABEL[i.method]}>
                            {describeInstall(i)}
                          </span>
                        ))}
                      </dd>
                    </div>
                    <div>
                      <dt className="text-xs text-muted-foreground">Administrator rights</dt>
                      <dd>{m.requires_elevation ? "Needed for installation. You will be asked to approve." : "Not needed"}</dd>
                    </div>
                  </dl>
                </Card>

                <Card>
                  <CardTitle>About</CardTitle>
                  <dl className="space-y-2.5">
                    <div>
                      <dt className="text-xs text-muted-foreground">License</dt>
                      <dd className="font-mono text-xs">{m.license}</dd>
                    </div>
                    <div>
                      <dt className="text-xs text-muted-foreground">Maintenance</dt>
                      <dd>{m.maintenance ? `${MAINTENANCE_LABEL[m.maintenance]} (as recorded by the catalog)` : "Not recorded"}</dd>
                    </div>
                    <div>
                      <dt className="text-xs text-muted-foreground">Catalog record</dt>
                      <dd>{entry.trust === "bundled" ? "Bundled with Vixalune" : "Unsigned remote record"}</dd>
                    </div>
                    <div>
                      <dt className="text-xs text-muted-foreground">Links</dt>
                      <dd className="flex flex-wrap gap-x-4 gap-y-1">
                        <ExternalLink toolId={m.id} kind="homepage">
                          Website
                        </ExternalLink>
                        <ExternalLink toolId={m.id} kind="documentation">
                          Official documentation
                        </ExternalLink>
                        {m.source_repository ? (
                          <ExternalLink toolId={m.id} kind="source_repository">
                            Source repository
                          </ExternalLink>
                        ) : null}
                      </dd>
                    </div>
                  </dl>
                </Card>
              </div>
              <p className="text-xs text-muted-foreground">Use security tools only on systems you own or have written permission to test.</p>
            </>
          );
        }}
      </AsyncView>
    </div>
  );
}
