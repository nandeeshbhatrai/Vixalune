import { Briefcase } from "lucide-react";
import { useEffect, useState } from "react";
import { AsyncView } from "@/components/AsyncView";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { DemoTag } from "@/components/DemoTag";
import { EmptyState } from "@/components/EmptyState";
import { ErrorNotice } from "@/components/ErrorNotice";
import { PageHeader } from "@/components/PageHeader";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dialog } from "@/components/ui/dialog";
import type { Job } from "@/domain/types";
import { useAction } from "@/hooks/useAction";
import { toAppError, useAsync } from "@/hooks/useAsync";
import { useWorkspace } from "@/hooks/useWorkspace";
import { useBackend } from "@/infra/backendContext";
import { formatDuration, formatTime, JOB_KIND_LABEL, JOB_STATE_META } from "@/presentation/labels";

function useNow(active: boolean) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [active]);
  return now;
}

function OutputDialog({ job, name, onClose }: { job: Job; name: string; onClose: () => void }) {
  const backend = useBackend();
  const [state, retry] = useAsync(() => backend.getJobOutput(job.id), [backend, job.id]);
  return (
    <Dialog open onOpenChange={(o) => !o && onClose()} title={`${JOB_KIND_LABEL[job.kind]} output: ${name}`} wide>
      <AsyncView state={state} retry={retry} label="Loading output">
        {(lines) =>
          lines.length === 0 ? (
            <p className="text-muted-foreground">This job has not produced any output.</p>
          ) : (
            <pre aria-label="Job output" className="max-h-80 overflow-auto rounded-md bg-background p-3 font-mono text-xs">
              {lines.join("\n")}
            </pre>
          )
        }
      </AsyncView>
    </Dialog>
  );
}

export function Jobs() {
  const backend = useBackend();
  const ws = useWorkspace();
  const caps = ws.core.status === "ready" ? ws.core.data.caps : null;
  const [state, retry] = useAsync(() => backend.listJobs(), [backend]);
  const [viewing, setViewing] = useState<Job | null>(null);
  const [cancelling, setCancelling] = useState<Job | null>(null);
  const { busy, error, run, clearError } = useAction();
  const hasRunning = state.status === "ready" && state.data.some((j) => j.state === "running");
  const now = useNow(hasRunning);

  const nameOf = (id: string) => (ws.core.status === "ready" ? ws.core.data.tools.find((t) => t.manifest.id === id)?.manifest.name : undefined) ?? id;

  const confirmCancel = async () => {
    if (!cancelling) return;
    const ok = await run(async () => {
      await backend.cancelJob(cancelling.id);
      return true;
    });
    if (ok) {
      setCancelling(null);
      retry();
    }
  };

  return (
    <div className="space-y-4">
      <PageHeader
        title="Running Jobs"
        subtitle="Installations, verifications and tool runs."
        actions={
          <div className="flex items-center gap-2">
            <DemoTag />
            <Button variant="outline" size="sm" onClick={retry}>
              Refresh
            </Button>
          </div>
        }
      />
      <AsyncView state={state} retry={retry} label="Loading jobs">
        {(jobs) =>
          jobs.length === 0 ? (
            <EmptyState icon={Briefcase} title="No jobs yet">
              {caps && !caps.jobs
                ? "Running installs and tools is not available in this version of Vixalune yet."
                : "Jobs appear here when you install, verify or run a tool."}
            </EmptyState>
          ) : (
            <div className="overflow-x-auto rounded-lg border border-border">
              <table className="w-full text-left">
                <caption className="sr-only">Jobs</caption>
                <thead className="bg-muted text-xs uppercase tracking-wider text-muted-foreground">
                  <tr>
                    <th scope="col" className="px-3 py-2">Tool</th>
                    <th scope="col" className="px-3 py-2">Job</th>
                    <th scope="col" className="px-3 py-2">Started</th>
                    <th scope="col" className="px-3 py-2">State</th>
                    <th scope="col" className="px-3 py-2">Elapsed</th>
                    <th scope="col" className="px-3 py-2">Exit status</th>
                    <th scope="col" className="px-3 py-2"><span className="sr-only">Actions</span></th>
                  </tr>
                </thead>
                <tbody>
                  {jobs.map((j) => {
                    const end = j.finished_at ? new Date(j.finished_at).getTime() : now;
                    const meta = JOB_STATE_META[j.state];
                    const active = j.state === "running" || j.state === "queued";
                    return (
                      <tr key={j.id} className="border-t border-border">
                        <td className="px-3 py-2 font-medium">{nameOf(j.tool_id)}</td>
                        <td className="px-3 py-2">{JOB_KIND_LABEL[j.kind]}</td>
                        <td className="px-3 py-2">{formatTime(j.started_at)}</td>
                        <td className="px-3 py-2"><Badge tone={meta.tone}>{meta.label}</Badge></td>
                        <td className="px-3 py-2 tabular-nums">{formatDuration(end - new Date(j.started_at).getTime())}</td>
                        <td className="px-3 py-2 tabular-nums">{j.exit_code ?? "—"}</td>
                        <td className="px-3 py-2">
                          <div className="flex justify-end gap-2">
                            <Button variant="outline" size="sm" onClick={() => setViewing(j)} aria-label={`View output for ${nameOf(j.tool_id)} ${JOB_KIND_LABEL[j.kind]}`}>
                              View output
                            </Button>
                            {active ? (
                              <Button
                                variant="outline"
                                size="sm"
                                onClick={() => {
                                  clearError();
                                  setCancelling(j);
                                }}
                                aria-label={`Cancel ${nameOf(j.tool_id)} ${JOB_KIND_LABEL[j.kind]}`}
                              >
                                Cancel
                              </Button>
                            ) : null}
                          </div>
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          )
        }
      </AsyncView>

      {viewing ? <OutputDialog job={viewing} name={nameOf(viewing.tool_id)} onClose={() => setViewing(null)} /> : null}
      <ConfirmDialog
        open={cancelling !== null}
        onOpenChange={(o) => !o && setCancelling(null)}
        title="Cancel this job?"
        description={cancelling ? `${JOB_KIND_LABEL[cancelling.kind]} of ${nameOf(cancelling.tool_id)} will be stopped.` : undefined}
        confirmLabel="Cancel job"
        destructive
        busy={busy}
        onConfirm={() => void confirmCancel()}
      >
        {error ? <ErrorNotice error={toAppError(error)} /> : null}
      </ConfirmDialog>
    </div>
  );
}
