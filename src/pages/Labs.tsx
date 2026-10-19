import { FlaskConical } from "lucide-react";
import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { AsyncView } from "@/components/AsyncView";
import { DemoTag } from "@/components/DemoTag";
import { EmptyState } from "@/components/EmptyState";
import { ErrorNotice } from "@/components/ErrorNotice";
import { PageHeader } from "@/components/PageHeader";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import type { LabState, LabStatus } from "@/domain/types";
import { useAction } from "@/hooks/useAction";
import { useAsync } from "@/hooks/useAsync";
import { useBackend } from "@/infra/backendContext";

const STATE_LABEL: Record<LabState, { label: string; tone: "neutral" | "info" | "success" | "danger" }> = {
  stopped: { label: "Stopped", tone: "neutral" },
  starting: { label: "Starting", tone: "info" },
  running: { label: "Running", tone: "success" },
  stopping: { label: "Stopping", tone: "info" },
  failed: { label: "Failed", tone: "danger" },
};

function LabCard({ lab, onChange }: { lab: LabStatus; onChange: (labs: LabStatus[]) => void }) {
  const backend = useBackend();
  const { busy, error, run } = useAction();
  const web = lab.kind === "web_app";
  const running = lab.state === "running";
  const meta = STATE_LABEL[lab.state];
  const act = async (fn: () => Promise<LabStatus[]>) => {
    const next = await run(fn);
    if (next) onChange(next);
  };
  return (
    <Card className="flex h-full flex-col gap-3">
      <div className="flex items-start justify-between gap-2">
        <h2 className="font-semibold">{lab.name}</h2>
        <Badge tone={meta.tone}>{web ? meta.label : running ? "Files ready" : "Not created"}</Badge>
      </div>
      <p className="text-muted-foreground">{lab.description}</p>
      {lab.address ? (
        <p className="text-sm">
          Address: <span className="font-mono text-xs">{lab.address}</span>
        </p>
      ) : null}
      {lab.folder ? (
        <p className="text-sm">
          Folder: <span className="break-all font-mono text-xs">{lab.folder}</span>
        </p>
      ) : null}
      <div>
        <p className="mb-1 text-xs font-medium uppercase tracking-wider text-muted-foreground">What is deliberately wrong</p>
        <ul className="list-disc space-y-0.5 pl-5 text-sm">
          {lab.weaknesses.map((w) => (
            <li key={w}>{w}</li>
          ))}
        </ul>
      </div>
      <p role="note" className="text-xs text-warning">
        {lab.warning}
      </p>
      <div className="mt-auto flex flex-wrap items-center gap-2">
        {web ? (
          running ? (
            <Button variant="outline" disabled={busy} onClick={() => void act(() => backend.stopLab(lab.id))}>
              Stop lab
            </Button>
          ) : (
            <Button disabled={busy} onClick={() => void act(() => backend.startLab(lab.id))}>
              Start lab
            </Button>
          )
        ) : (
          <Button variant="outline" disabled={busy} onClick={() => void act(() => backend.startLab(lab.id))}>
            Create or refresh the files
          </Button>
        )}
        <Link to="/learn" className="text-primary hover:underline">
          Tutorials that use this lab
        </Link>
      </div>
      {error ? <ErrorNotice error={error} /> : null}
    </Card>
  );
}

export function Labs() {
  const backend = useBackend();
  const [state, retry] = useAsync(() => backend.listLabs(), [backend]);
  const [override, setOverride] = useState<LabStatus[] | null>(null);
  useEffect(() => setOverride(null), [state]);

  return (
    <div className="space-y-4">
      <PageHeader title="Learning Labs" subtitle="Practice on intentionally vulnerable targets that exist only on your own computer." actions={<DemoTag />} />
      <p role="note" className="rounded-md border border-border bg-card p-3 text-sm text-muted-foreground">
        The practice web application listens on 127.0.0.1 only and serves fixed pages; it never reads your files or contacts other computers. The sample code is never executed. Never point scanners at systems you do not own.
      </p>
      <AsyncView state={state} retry={retry} label="Loading labs">
        {(loaded) => {
          const labs = override ?? loaded;
          return labs.length === 0 ? (
            <EmptyState icon={FlaskConical} title="No labs available">
              The built-in practice labs could not be listed.
            </EmptyState>
          ) : (
            <ul className="grid gap-3 lg:grid-cols-2">
              {labs.map((lab) => (
                <li key={lab.id}>
                  <LabCard lab={lab} onChange={setOverride} />
                </li>
              ))}
            </ul>
          );
        }}
      </AsyncView>
    </div>
  );
}
