import { History } from "lucide-react";
import { AsyncView } from "@/components/AsyncView";
import { EmptyState } from "@/components/EmptyState";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import type { AsyncState } from "@/hooks/useAsync";
import type { RunRecord } from "@/domain/types";
import { formatDuration, formatTime, RUN_OUTCOME_META } from "@/presentation/labels";

/** Recent runs: what ran, against what, and how it ended. "Findings" and "failed" are visibly different. */
export function RunHistory({ runs, retry, selectedId, onView }: { runs: AsyncState<RunRecord[]>; retry: () => void; selectedId: string | null; onView: (id: string) => void }) {
  return (
    <section aria-label="Recent runs" className="space-y-2">
      <h2 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">Recent runs</h2>
      <AsyncView state={runs} retry={retry} label="Loading run history">
        {(list) =>
          list.length === 0 ? (
            <EmptyState icon={History} title="No runs yet">
              Runs you start here are listed with their result. History is kept only as long as your retention settings allow.
            </EmptyState>
          ) : (
            <div className="overflow-x-auto rounded-lg border border-border">
              <table className="w-full text-left text-sm">
                <caption className="sr-only">Recent runs</caption>
                <thead className="bg-muted text-xs uppercase tracking-wider text-muted-foreground">
                  <tr>
                    <th scope="col" className="px-3 py-2">Run</th>
                    <th scope="col" className="px-3 py-2">Started</th>
                    <th scope="col" className="px-3 py-2">Result</th>
                    <th scope="col" className="px-3 py-2">Target</th>
                    <th scope="col" className="px-3 py-2">Exit</th>
                    <th scope="col" className="px-3 py-2">Time</th>
                    <th scope="col" className="px-3 py-2"><span className="sr-only">Actions</span></th>
                  </tr>
                </thead>
                <tbody>
                  {list.map((r) => {
                    const meta = r.outcome ? RUN_OUTCOME_META[r.outcome] : null;
                    return (
                      <tr key={r.id} className={`border-t border-border align-top ${r.id === selectedId ? "bg-muted/50" : ""}`}>
                        <td className="px-3 py-2">
                          <div className="font-medium">{r.tool_name}</div>
                          <div className="text-xs text-muted-foreground">{r.profile_label}</div>
                        </td>
                        <td className="px-3 py-2">{formatTime(r.started_at)}</td>
                        <td className="px-3 py-2">{meta ? <Badge tone={meta.tone}>{meta.label}</Badge> : <Badge tone="info">Running</Badge>}</td>
                        <td className="px-3 py-2 text-xs">{r.targets.length > 0 ? r.targets.join(", ") : "—"}</td>
                        <td className="px-3 py-2 tabular-nums">{r.exit_code ?? "—"}</td>
                        <td className="px-3 py-2 tabular-nums">{r.duration_ms != null ? formatDuration(r.duration_ms) : "—"}</td>
                        <td className="px-3 py-2">
                          <Button variant="outline" size="sm" onClick={() => onView(r.id)} aria-label={`View ${r.tool_name} run from ${formatTime(r.started_at)}`}>
                            View
                          </Button>
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
    </section>
  );
}
