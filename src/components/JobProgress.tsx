import { Badge } from "@/components/ui/badge";
import type { Job } from "@/domain/types";
import { JOB_KIND_LABEL, JOB_PHASE_META } from "@/presentation/labels";

/** Live phase, latest message and (when the tool reports it) a percentage bar for one job. */
export function JobProgress({ job }: { job: Job }) {
  const meta = JOB_PHASE_META[job.phase];
  return (
    <div className="space-y-1.5" aria-live="polite">
      <div className="flex flex-wrap items-center gap-2">
        <Badge tone={meta.tone}>{meta.label}</Badge>
        <span className="text-muted-foreground">{JOB_KIND_LABEL[job.kind]}</span>
      </div>
      {job.message ? <p className="break-words text-xs text-muted-foreground">{job.message}</p> : null}
      {typeof job.percent === "number" ? (
        <div
          role="progressbar"
          aria-label={`${JOB_KIND_LABEL[job.kind]} progress`}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={job.percent}
          className="h-1.5 w-full overflow-hidden rounded bg-muted"
        >
          <div className="h-full bg-primary transition-all" style={{ width: `${job.percent}%` }} />
        </div>
      ) : null}
    </div>
  );
}
