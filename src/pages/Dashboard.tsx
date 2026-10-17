import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardTitle } from "@/components/ui/card";
import { AsyncView } from "@/components/AsyncView";
import { useAsync } from "@/hooks/useAsync";
import { useBackend } from "@/infra/backendContext";

const COMPONENT_LABEL = { winget: "winget", wsl2: "WSL 2", docker: "Docker" } as const;

export function Dashboard() {
  const backend = useBackend();
  const [env, recheck] = useAsync(() => backend.detectEnvironment(), [backend]);
  const [tools] = useAsync(() => backend.listTools(), [backend]);
  const [activity] = useAsync(() => backend.recentActivity(8), [backend]);

  return (
    <div className="mx-auto max-w-4xl space-y-6">
      <h1 className="text-2xl font-semibold">Dashboard</h1>

      <Card>
        <CardTitle>Environment readiness</CardTitle>
        <AsyncView state={env} retry={recheck}>
          {(r) => (
            <div className="space-y-4">
              <ul className="flex flex-wrap gap-3">
                {r.components.map((c) => (
                  <li key={c.component} className="flex items-center gap-2">
                    {COMPONENT_LABEL[c.component]}
                    <Badge variant={c.state === "available" ? "success" : c.state === "missing" ? "warning" : "default"}>{c.state}</Badge>
                  </li>
                ))}
              </ul>
              <section aria-label="Setup issues">
                <h3 className="mb-2 text-sm font-medium">Setup issues</h3>
                {r.issues.length === 0 ? (
                  <p className="text-sm text-muted-foreground">No issues found.</p>
                ) : (
                  <ul className="space-y-2">
                    {r.issues.map((i) => (
                      <li key={i.title} className="rounded-md bg-muted p-3 text-sm">
                        <p className="font-medium">{i.title}</p>
                        <p className="text-muted-foreground">{i.recovery}</p>
                      </li>
                    ))}
                  </ul>
                )}
              </section>
              <Button variant="outline" size="sm" onClick={recheck}>
                Re-check
              </Button>
            </div>
          )}
        </AsyncView>
      </Card>

      <div className="grid gap-6 md:grid-cols-2">
        <Card>
          <CardTitle>Installed tools</CardTitle>
          <AsyncView state={tools} retry={() => undefined}>
            {(t) => (
              <p className="text-sm text-muted-foreground">
                0 installed · {t.length} available in the catalog. Installing arrives in a later phase.
              </p>
            )}
          </AsyncView>
        </Card>
        <Card>
          <CardTitle>Recent activity</CardTitle>
          <AsyncView state={activity} retry={() => undefined}>
            {(events) =>
              events.length === 0 ? (
                <p className="text-sm text-muted-foreground">Nothing yet.</p>
              ) : (
                <ul className="space-y-1 text-sm">
                  {events.map((e, i) => (
                    <li key={`${e.at}-${i}`}>
                      <span className="text-muted-foreground">{e.at.slice(0, 19).replace("T", " ")}</span> {e.summary}
                    </li>
                  ))}
                </ul>
              )
            }
          </AsyncView>
        </Card>
      </div>
    </div>
  );
}
