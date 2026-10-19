import { Copy, FolderOpen, Save, Square } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { ErrorNotice } from "@/components/ErrorNotice";
import { JobProgress } from "@/components/JobProgress";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import type { AppErrorDto, Job, OutputRow, RunRecord } from "@/domain/types";
import { useAction } from "@/hooks/useAction";
import { toAppError } from "@/hooks/useAsync";
import { useBackend } from "@/infra/backendContext";
import { formatDuration, isActivePhase, RUN_OUTCOME_META } from "@/presentation/labels";

const POLL_MS = 500;

/**
 * Live view of one run: streamed output (polled from the backend), outcome, exit code and duration, search, copy,
 * save and cancel. "Completed with findings" is shown as a result, never as an error.
 */
export function OutputPanel({ jobId, onFinished, pollMs = POLL_MS }: { jobId: string; onFinished?: () => void; pollMs?: number }) {
  const backend = useBackend();
  const [job, setJob] = useState<Job | null>(null);
  const [run, setRun] = useState<RunRecord | null>(null);
  const [rows, setRows] = useState<OutputRow[]>([]);
  const [loadError, setLoadError] = useState<AppErrorDto | null>(null);
  const [query, setQuery] = useState("");
  const [confirmCancel, setConfirmCancel] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const last = useRef(0);
  const finishedFor = useRef<string | null>(null);
  // The caller passes a new function on every render; keep the latest one without restarting polling.
  const onFinishedRef = useRef(onFinished);
  onFinishedRef.current = onFinished;
  const act = useAction();

  // Reset when another run is shown.
  useEffect(() => {
    setJob(null);
    setRun(null);
    setRows([]);
    setLoadError(null);
    setQuery("");
    setNotice(null);
    last.current = 0;
  }, [jobId]);

  const refresh = useCallback(async (): Promise<boolean> => {
    try {
      const [j, fresh] = await Promise.all([backend.getJob(jobId), backend.getRunOutput(jobId, last.current)]);
      const newer = fresh.filter((r) => r.seq > last.current);
      if (newer.length > 0) {
        last.current = newer[newer.length - 1]!.seq;
        setRows((r) => [...r, ...newer]);
      }
      setJob(j);
      setLoadError(null);
      if (!isActivePhase(j.phase)) {
        setRun(await backend.getRun(jobId).catch(() => null));
        return true;
      }
      return false;
    } catch (e) {
      setLoadError(toAppError(e));
      return true; // do not hammer a failing backend
    }
  }, [backend, jobId]);

  useEffect(() => {
    let live = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const tick = async () => {
      const done = await refresh();
      if (!live) return;
      if (done) {
        if (finishedFor.current !== jobId) {
          finishedFor.current = jobId;
          onFinishedRef.current?.();
        }
      } else {
        timer = setTimeout(() => void tick(), pollMs);
      }
    };
    void tick();
    return () => {
      live = false;
      if (timer) clearTimeout(timer);
    };
  }, [refresh, pollMs, jobId]);

  const q = query.trim().toLowerCase();
  const visible = useMemo(() => (q ? rows.filter((r) => r.text.toLowerCase().includes(q)) : rows), [rows, q]);
  const active = job ? isActivePhase(job.phase) : true;
  const outcome = run?.outcome ? RUN_OUTCOME_META[run.outcome] : null;

  const copy = async () => {
    setNotice(null);
    const ok = await act.run(async () => {
      await navigator.clipboard.writeText(visible.map((r) => r.text).join("\n"));
      return true;
    });
    if (ok) setNotice(q ? `Copied ${visible.length} matching line(s).` : `Copied ${visible.length} line(s).`);
  };

  const save = async () => {
    setNotice(null);
    const path = await act.run(() => backend.saveRunOutput(jobId));
    if (path) {
      setNotice(`Saved to ${path}`);
      setRun((r) => (r ? { ...r, result_path: path } : r));
    }
  };

  const cancel = async () => {
    const ok = await act.run(async () => {
      await backend.cancelJob(jobId);
      return true;
    });
    if (ok) {
      setConfirmCancel(false);
      void refresh();
    }
  };

  return (
    <section aria-label="Run output" className="space-y-3 rounded-lg border border-border bg-card p-4">
      {loadError ? <ErrorNotice error={loadError} onRetry={() => void refresh()} /> : null}
      {job ? (
        <>
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="min-w-0 space-y-1.5">
              <JobProgress job={job} />
              {outcome ? (
                <div className="flex flex-wrap items-center gap-2" role="status">
                  <Badge tone={outcome.tone}>{outcome.label}</Badge>
                  <span className="text-xs text-muted-foreground">{outcome.explanation}</span>
                </div>
              ) : null}
            </div>
            <div className="flex flex-wrap gap-2">
              {active && job.phase !== "cancelling" ? (
                <Button variant="outline" size="sm" onClick={() => setConfirmCancel(true)}>
                  <Square size={13} aria-hidden /> Cancel run
                </Button>
              ) : null}
            </div>
          </div>
          {job.error ? (
            <div role="alert" className="rounded-md border border-destructive/40 bg-destructive/10 p-3">
              <p className="font-medium text-destructive">{job.error.message}</p>
              <p className="mt-0.5 text-muted-foreground">{job.error.recovery}</p>
              {job.error.detail ? <p className="mt-1 break-words font-mono text-xs text-muted-foreground">{job.error.detail}</p> : null}
            </div>
          ) : null}
          <dl className="grid gap-x-6 gap-y-1 text-sm sm:grid-cols-2">
            <div>
              <dt className="inline text-muted-foreground">Exit code: </dt>
              <dd className="inline tabular-nums">{typeof job.exit_code === "number" ? job.exit_code : active ? "still running" : "none"}</dd>
            </div>
            <div>
              <dt className="inline text-muted-foreground">Duration: </dt>
              <dd className="inline tabular-nums">{run?.duration_ms != null ? formatDuration(run.duration_ms) : active ? "running…" : "—"}</dd>
            </div>
            {run ? (
              <>
                <div>
                  <dt className="inline text-muted-foreground">Scope: </dt>
                  <dd className="inline">{run.scope}</dd>
                </div>
                <div>
                  <dt className="inline text-muted-foreground">Tool version: </dt>
                  <dd className="inline">{run.tool_version ?? "unknown"}</dd>
                </div>
              </>
            ) : null}
          </dl>
        </>
      ) : (
        <p role="status" className="text-sm text-muted-foreground">
          Starting…
        </p>
      )}

      <div className="flex flex-wrap items-center gap-2">
        <label className="flex items-center gap-2 text-sm">
          <span className="sr-only">Search output</span>
          <input type="search" placeholder="Search output" aria-label="Search output" className="h-8 w-56 rounded-md border border-border bg-background px-2 text-sm" value={query} onChange={(e) => setQuery(e.target.value)} />
        </label>
        <span className="text-xs text-muted-foreground" aria-live="polite">
          {q ? `${visible.length} of ${rows.length} lines match` : `${rows.length} line${rows.length === 1 ? "" : "s"}`}
        </span>
        <div className="ml-auto flex flex-wrap gap-2">
          <Button variant="outline" size="sm" disabled={visible.length === 0 || act.busy} onClick={() => void copy()}>
            <Copy size={13} aria-hidden /> Copy
          </Button>
          <Button variant="outline" size="sm" disabled={rows.length === 0 || active || act.busy} onClick={() => void save()} title={active ? "Save is available when the run has finished." : undefined}>
            <Save size={13} aria-hidden /> Save output…
          </Button>
          {run?.run_dir && !active ? (
            <Button variant="outline" size="sm" disabled={act.busy} onClick={() => void act.run(() => backend.openRunFolder(jobId))}>
              <FolderOpen size={13} aria-hidden /> Open run folder
            </Button>
          ) : null}
        </div>
      </div>
      {notice ? (
        <p role="status" className="break-all text-xs text-success">
          {notice}
        </p>
      ) : null}
      {act.error ? <ErrorNotice error={act.error} /> : null}

      {rows.length === 0 ? (
        <p className="text-sm text-muted-foreground">{active ? "Waiting for the tool to print something…" : "The tool did not print anything."}</p>
      ) : (
        <pre aria-label="Tool output" className="max-h-96 overflow-auto rounded-md bg-background p-3 font-mono text-xs">
          {visible.map((r) => (
            <span key={r.seq} className={r.stream === "stderr" ? "block text-warning" : "block"}>
              {r.text || " "}
            </span>
          ))}
        </pre>
      )}

      <ConfirmDialog
        open={confirmCancel}
        onOpenChange={setConfirmCancel}
        title="Cancel this run?"
        description="The tool and everything it started will be stopped. Output printed so far is kept."
        confirmLabel="Cancel run"
        destructive
        busy={act.busy}
        onConfirm={() => void cancel()}
      />
    </section>
  );
}
