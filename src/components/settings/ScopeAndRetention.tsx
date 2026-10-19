import { useEffect, useState } from "react";
import { AsyncView } from "@/components/AsyncView";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { ErrorNotice } from "@/components/ErrorNotice";
import { fieldClass } from "@/components/run/ParamFields";
import { Button } from "@/components/ui/button";
import { Card, CardTitle } from "@/components/ui/card";
import type { UserSettings } from "@/domain/types";
import { useAction } from "@/hooks/useAction";
import { useAsync } from "@/hooks/useAsync";
import { useBackend } from "@/infra/backendContext";

function Editor({ initial }: { initial: UserSettings }) {
  const backend = useBackend();
  const { busy, error, run, clearError } = useAction();
  const [labs, setLabs] = useState(initial.scope.lab_targets.join("\n"));
  const [history, setHistory] = useState(String(initial.retention.history_days));
  const [output, setOutput] = useState(String(initial.retention.output_days));
  const [notice, setNotice] = useState<string | null>(null);
  const [confirmPurge, setConfirmPurge] = useState(false);

  useEffect(() => {
    setLabs(initial.scope.lab_targets.join("\n"));
    setHistory(String(initial.retention.history_days));
    setOutput(String(initial.retention.output_days));
  }, [initial]);

  const save = async () => {
    setNotice(null);
    const next: UserSettings = {
      scope: {
        lab_targets: labs
          .split(/\r?\n/)
          .map((l) => l.trim())
          .filter(Boolean),
      },
      retention: { history_days: Number(history), output_days: Number(output) },
    };
    const saved = await run(() => backend.updateSettings(next));
    if (saved) setNotice("Settings saved. They apply to the next run.");
  };

  const purge = async () => {
    const r = await run(() => backend.purgeHistoryNow());
    if (r) {
      setConfirmPurge(false);
      setNotice(`Deleted ${r.runs_deleted} run record(s) and ${r.output_lines_deleted} stored output line(s).`);
    }
  };

  return (
    <>
      <Card>
        <CardTitle>Lab list: what counts as your own test targets</CardTitle>
        <p className="mb-2 text-sm text-muted-foreground">
          This computer (127.0.0.1) is always a lab. Add host names, IP addresses or CIDR ranges you manage, one per line, for example <span className="font-mono">192.168.56.0/24</span>. Anything else needs a confirmation before a run, and public
          addresses need you to type the target. This list is a guardrail, not legal authorisation.
        </p>
        <label>
          <span className="sr-only">Lab targets, one per line</span>
          <textarea
            aria-label="Lab targets"
            className="h-28 w-full rounded-md border border-border bg-card p-2 font-mono text-xs"
            spellCheck={false}
            value={labs}
            onChange={(e) => {
              setLabs(e.target.value);
              clearError();
            }}
          />
        </label>
      </Card>
      <Card>
        <CardTitle>Data retention</CardTitle>
        <p className="mb-3 text-sm text-muted-foreground">
          Vixalune keeps a record of each run (command, times, exit status, scope) and its output so you can look back. Output may contain sensitive information, so both expire. Secrets are masked before storage, but you should still treat output as
          sensitive.
        </p>
        <div className="grid gap-3 sm:grid-cols-2">
          <label>
            <span className="mb-1 block text-xs text-muted-foreground">Keep run history for (days)</span>
            <input type="number" min={1} max={3650} className={fieldClass} value={history} onChange={(e) => setHistory(e.target.value)} />
          </label>
          <label>
            <span className="mb-1 block text-xs text-muted-foreground">Keep run output for (days, 0 = delete at the next start)</span>
            <input type="number" min={0} max={3650} className={fieldClass} value={output} onChange={(e) => setOutput(e.target.value)} />
          </label>
        </div>
        <div className="mt-3 flex flex-wrap gap-2">
          <Button disabled={busy} onClick={() => void save()}>
            Save settings
          </Button>
          <Button variant="outline" disabled={busy} onClick={() => setConfirmPurge(true)}>
            Delete history and output now…
          </Button>
        </div>
        {notice ? (
          <p role="status" className="mt-2 text-xs text-success">
            {notice}
          </p>
        ) : null}
        {error ? (
          <div className="mt-2">
            <ErrorNotice error={error} />
          </div>
        ) : null}
        <p className="mt-3 text-xs text-muted-foreground">Files you chose to save with Save output are yours and are never deleted by Vixalune.</p>
      </Card>
      <ConfirmDialog
        open={confirmPurge}
        onOpenChange={setConfirmPurge}
        title="Delete all run history and output?"
        description="Finished runs and their stored output are removed. Files you saved yourself are not touched."
        confirmLabel="Delete"
        destructive
        busy={busy}
        onConfirm={() => void purge()}
      />
    </>
  );
}

export function ScopeAndRetention() {
  const backend = useBackend();
  const [state, retry] = useAsync(() => backend.getSettings(), [backend]);
  return (
    <AsyncView state={state} retry={retry} label="Loading settings">
      {(s) => <Editor initial={s} />}
    </AsyncView>
  );
}
