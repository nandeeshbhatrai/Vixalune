import { PackageOpen } from "lucide-react";
import { Link } from "react-router-dom";
import { AsyncView } from "@/components/AsyncView";
import { DemoTag } from "@/components/DemoTag";
import { EmptyState } from "@/components/EmptyState";
import { PageHeader } from "@/components/PageHeader";
import { StatusBadge } from "@/components/StatusBadge";
import { ToolActions } from "@/components/ToolActions";
import { ToolIcon } from "@/components/ToolIcon";
import { Badge } from "@/components/ui/badge";
import { Card } from "@/components/ui/card";
import { deriveStatus } from "@/domain/toolStatus";
import { useAsync } from "@/hooks/useAsync";
import { useWorkspace } from "@/hooks/useWorkspace";
import { useBackend } from "@/infra/backendContext";
import { formatTime, RUNTIME_LABEL, VERIFICATION_LEVEL_LABEL, VERIFICATION_OUTCOME_META } from "@/presentation/labels";

export function InstalledTools() {
  const backend = useBackend();
  const ws = useWorkspace();
  const detection = ws.core.status === "ready" ? ws.core.data.caps.installed_detection : null;
  const [state, retry] = useAsync(() => backend.listInstalledTools(), [backend, ws.core]);

  return (
    <div className="space-y-4">
      <PageHeader title="Installed Tools" subtitle="Versions, locations and the result of the last verification." actions={<DemoTag />} />
      <AsyncView state={state} retry={retry} label="Loading installed tools">
        {(items) => {
          if (items.length === 0) {
            return detection === false ? (
              <EmptyState icon={PackageOpen} title="Installed-tool detection is not available yet">
                Vixalune cannot yet tell which tools are on this computer, so this list is empty. It does not mean nothing is installed.
              </EmptyState>
            ) : (
              <EmptyState icon={PackageOpen} title="No tools installed">
                <Link to="/catalog" className="text-primary hover:underline">
                  Browse the catalog
                </Link>{" "}
                to install your first tool.
              </EmptyState>
            );
          }
          return (
            <ul className="space-y-3">
              {items.map((it) => {
                const entry = ws.core.status === "ready" ? ws.core.data.tools.find((t) => t.manifest.id === it.tool_id) : undefined;
                const status = deriveStatus(it.state, ws.missingFor(it.tool_id));
                return (
                  <li key={it.tool_id}>
                    <Card className="space-y-3">
                      <div className="flex items-center gap-3">
                        {entry ? <ToolIcon category={entry.manifest.category} /> : null}
                        <div className="min-w-0 flex-1">
                          <Link to={`/catalog/${it.tool_id}`} className="font-semibold hover:underline">
                            {entry?.manifest.name ?? it.tool_id}
                          </Link>
                          <p className="text-xs text-muted-foreground">Version {it.version ?? "unknown"}</p>
                        </div>
                        <StatusBadge status={status} />
                      </div>
                      <dl className="grid gap-x-6 gap-y-1 text-sm sm:grid-cols-3">
                        <div>
                          <dt className="text-xs text-muted-foreground">Location</dt>
                          <dd className="break-all font-mono text-xs">{it.location ?? "unknown"}</dd>
                        </div>
                        <div>
                          <dt className="text-xs text-muted-foreground">Execution environment</dt>
                          <dd>{RUNTIME_LABEL[it.runtime]}</dd>
                        </div>
                        <div>
                          <dt className="text-xs text-muted-foreground">Last verification</dt>
                          <dd>
                            {it.last_verification ? (
                              <>
                                <Badge tone={VERIFICATION_OUTCOME_META[it.last_verification.outcome].tone}>
                                  {VERIFICATION_OUTCOME_META[it.last_verification.outcome].label}
                                </Badge>{" "}
                                <span className="text-xs text-muted-foreground">{formatTime(it.last_verification.at)}</span>
                                <span className="block text-xs text-muted-foreground">{VERIFICATION_LEVEL_LABEL[it.last_verification.level]}</span>
                                <span className="block text-xs text-muted-foreground">{it.last_verification.detail}</span>
                              </>
                            ) : (
                              "Never verified by Vixalune"
                            )}
                          </dd>
                        </div>
                      </dl>
                      {entry ? <ToolActions entry={entry} /> : null}
                    </Card>
                  </li>
                );
              })}
            </ul>
          );
        }}
      </AsyncView>
    </div>
  );
}

