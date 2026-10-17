import { History } from "lucide-react";
import { AsyncView } from "@/components/AsyncView";
import { DemoTag } from "@/components/DemoTag";
import { EmptyState } from "@/components/EmptyState";
import { PageHeader } from "@/components/PageHeader";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useAsync } from "@/hooks/useAsync";
import { useBackend } from "@/infra/backendContext";
import { formatTime } from "@/presentation/labels";

export function ActivityPage() {
  const backend = useBackend();
  const [state, retry] = useAsync(() => backend.recentActivity(100), [backend]);
  return (
    <div className="space-y-4">
      <PageHeader
        title="Activity"
        subtitle="A local record of what Vixalune did. Secrets are masked before anything is stored."
        actions={
          <div className="flex items-center gap-2">
            <DemoTag />
            <Button variant="outline" size="sm" onClick={retry}>
              Refresh
            </Button>
          </div>
        }
      />
      <AsyncView state={state} retry={retry} label="Loading activity">
        {(events) =>
          events.length === 0 ? (
            <EmptyState icon={History} title="No activity yet" />
          ) : (
            <ol className="divide-y divide-border rounded-lg border border-border bg-card">
              {events.map((e, i) => (
                <li key={`${e.at}-${i}`} className="flex flex-wrap items-center gap-3 px-3 py-2">
                  <time dateTime={e.at} className="w-44 shrink-0 text-xs text-muted-foreground">
                    {formatTime(e.at)}
                  </time>
                  <Badge tone={e.kind.endsWith("failed") ? "danger" : "neutral"}>{e.kind.replaceAll("_", " ")}</Badge>
                  <span className="min-w-0 flex-1">{e.summary}</span>
                </li>
              ))}
            </ol>
          )
        }
      </AsyncView>
    </div>
  );
}
