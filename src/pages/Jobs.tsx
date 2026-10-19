import { Briefcase } from "lucide-react";
import { useEffect, useState } from "react";
import { AsyncView } from "@/components/AsyncView";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { DemoTag } from "@/components/DemoTag";
import { EmptyState } from "@/components/EmptyState";
import { ErrorNotice } from "@/components/ErrorNotice";
import { JobProgress } from "@/components/JobProgress";
import { PageHeader } from "@/components/PageHeader";
import { Button } from "@/components/ui/button";
import { Dialog } from "@/components/ui/dialog";
import type { Job } from "@/domain/types";
import { useAction } from "@/hooks/useAction";
import { useAsync } from "@/hooks/useAsync";
import { useWorkspace } from "@/hooks/useWorkspace";
import { useBackend } from "@/infra/backendContext";
import { formatDuration, formatTime, isActivePhase, JOB_KIND_LABEL } from "@/presentation/labels";

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
  const [state, retry] = useAsync(() => backend.getJobOutput(job.id), [backend, job.id, job.phase, job.updated_at]);
  return (
    <Dialog open onOpenChange={(o) => !o && onClose()} title={`${JOB_KIND_LABEL[job.kind]}: ${name}`} wide>
      <div className="space-y-3">
        <JobProgress job={job} />
        {job.error ? (
          <div role="alert" className="rounded-md border border-destructive/40 bg-destructive/10 p-3">
            <p className="font-medium text-destructive">{job.error.message}</p>
            <p className="mt-0.5 text-muted-foreground">{job.error.recovery}</p>
            {job.error.detail ? <p className="mt-1 break-words font-mono text-xs text-muted-foreground">{job.error.detail}</p> : null}
            <p className="mt-1 text-xs text-muted-foreground">
              Error code: <span className="font-mono">{job.error.code}</span>
              {typeof job.exit_code === "number" ? (
                <>
                  {" "}
                  · exit code <span className="font-mono">{job.exit_code}</span>
                </>
              ) : null}
            </p>
          </div>
        ) : null}
        <AsyncView state={state} retry={retry} label="Loading output">
          {(lines) =>
            lines.length === 0 ? (
              <p className="text-muted-foreground">This job has not produced any output.</p>
            ) : (
              <pre aria-label="Job output" className="max-h-72 overflow-auto rounded-md bg-background p-3 font-mono text-xs">
                {lines.join("\n")}
              </pre>
            )
          }
        </AsyncView>
      </div>
    </Dialog>
  );
}

export function Jobs() {
  const backend = useBackend();
  const ws = useWorkspace();
  const caps = ws.core.status === "ready" ? ws.core.data.caps : null;
  const [viewingId, setViewingId] = useState<string | null>(null);
  const [cancelling, setCancelling] = useState<Job | null>(null);
  const { busy, error, run, clearError } = useAction();
  const approve = useAction();
  const hasActive = ws.jobs.status === "ready" && ws.jobs.data.some((j) => isActivePhase(j.phase));
  const now = useNow(hasActive);

  const nameOf = (id: string) => (ws.core.status === "ready" ? ws.core.data.tools.find((t) => t.manifest.id === id)?.manifest.name : undefined) ?? id;

  const confirmCancel = async () => {
    if (!cancelling) return;
    const ok = await run(async () => {
      await backend.cancelJob(cancelling.id);
      return true;
    });
    if (ok) {
      setCancelling(null);
      ws.reloadJobs();
    }
  };

  const viewing = ws.jobs.status === "ready" ? ws.jobs.data.find((j) => j.id === viewingId) : undefined;

  return (
    <div className="space-y-4">
      <PageHeader
        title="Running Jobs"
        subtitle="Installations, verifications and tool runs. Updates every moment while something is running."
        actions={
          <div className="flex items-center gap-2">
            <DemoTag />
            <Button variant="outline" size="sm" onClick={ws.reloadJobs}>
              Refresh
            </Button>
          </div>
        }
      />
      {approve.error ? <ErrorNotice error={approve.error} /> : null}
      <AsyncView state={ws.jobs} retry={ws.reloadJobs} label="Loading jobs">
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
                    const active = isActivePhase(j.phase);
                    const name = nameOf(j.tool_id);
                    return (
                      <tr key={j.id} className="border-t border-border align-top">
                        <td className="px-3 py-2 font-medium">{name}</td>
                        <td className="px-3 py-2">{JOB_KIND_LABEL[j.kind]}</td>
                        <td className="px-3 py-2">{formatTime(j.started_at)}</td>
                        <td className="min-w-48 px-3 py-2">
                          <JobProgress job={j} />
                          {j.error ? <p className="mt-1 text-xs text-destructive">{j.error.message}</p> : null}
                        </td>
                        <td className="px-3 py-2 tabular-nums">{formatDuration(end - new Date(j.started_at).getTime())}</td>
                        <td className="px-3 py-2 tabular-nums">{j.exit_code ?? "—"}</td>
                        <td className="px-3 py-2">
                          <div className="flex justify-end gap-2">
                            {j.phase === "awaiting_consent" ? (
                              <Button
                                size="sm"
                                disabled={approve.busy}
                                onClick={() => void approve.run(() => backend.approveJob(j.id)).then(() => ws.reloadJobs())}
                                aria-label={`Approve ${name} ${JOB_KIND_LABEL[j.kind]}`}
                              >
                                Approve
                              </Button>
                            ) : null}
                            <Button variant="outline" size="sm" onClick={() => setViewingId(j.id)} aria-label={`View output for ${name} ${JOB_KIND_LABEL[j.kind]}`}>
                              View output
                            </Button>
                            {active && j.kind !== "setup" && j.phase !== "cancelling" && j.phase !== "rolling_back" ? (
                              <Button
                                variant="outline"
                                size="sm"
                                onClick={() => {
                                  clearError();
                                  setCancelling(j);
                                }}
                                aria-label={`Cancel ${name} ${JOB_KIND_LABEL[j.kind]}`}
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

      {viewing ? <OutputDialog job={viewing} name={nameOf(viewing.tool_id)} onClose={() => setViewingId(null)} /> : null}
      <ConfirmDialog
        open={cancelling !== null}
        onOpenChange={(o) => !o && setCancelling(null)}
        title="Cancel this job?"
        description={
          cancelling
            ? `${JOB_KIND_LABEL[cancelling.kind]} of ${nameOf(cancelling.tool_id)} will be stopped.${cancelling.kind === "install" ? " A partly finished installation is removed again." : ""}`
            : undefined
        }
        confirmLabel="Cancel job"
        destructive
        busy={busy}
        onConfirm={() => void confirmCancel()}
      >
        {error ? <ErrorNotice error={error} /> : null}
      </ConfirmDialog>
    </div>
  );
}

