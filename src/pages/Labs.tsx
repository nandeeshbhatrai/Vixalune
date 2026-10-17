import { FlaskConical } from "lucide-react";
import { AsyncView } from "@/components/AsyncView";
import { DemoTag } from "@/components/DemoTag";
import { EmptyState } from "@/components/EmptyState";
import { PageHeader } from "@/components/PageHeader";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { useAsync } from "@/hooks/useAsync";
import { useWorkspace } from "@/hooks/useWorkspace";
import { useBackend } from "@/infra/backendContext";

export function Labs() {
  const backend = useBackend();
  const ws = useWorkspace();
  const labsAvailable = ws.core.status === "ready" && ws.core.data.caps.labs;
  const [state, retry] = useAsync(() => backend.listLabs(), [backend]);

  return (
    <div className="space-y-4">
      <PageHeader
        title="Learning Labs"
        subtitle="Practice on intentionally vulnerable targets running only on your own computer."
        actions={<DemoTag />}
      />
      <AsyncView state={state} retry={retry} label="Loading labs">
        {(labs) =>
          labs.length === 0 ? (
            <EmptyState icon={FlaskConical} title="No labs available yet">
              Guided tutorials and local lab environments arrive in a later version of Vixalune. Labs will run in containers reachable only from this computer.
            </EmptyState>
          ) : (
            <ul className="grid gap-3 sm:grid-cols-2">
              {labs.map((lab) => (
                <li key={lab.id}>
                  <Card className="flex h-full flex-col gap-2">
                    <h2 className="font-semibold">{lab.name}</h2>
                    <p className="flex-1 text-muted-foreground">{lab.description}</p>
                    <div className="flex items-center justify-between">
                      <Badge>Requires Docker</Badge>
                      <Button disabled title={labsAvailable ? undefined : "Starting labs is not available yet."}>
                        Start lab
                      </Button>
                    </div>
                    {labsAvailable ? null : <p className="text-xs text-muted-foreground">Starting labs is not available in this version yet.</p>}
                  </Card>
                </li>
              ))}
            </ul>
          )
        }
      </AsyncView>
    </div>
  );
}
