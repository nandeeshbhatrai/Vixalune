import { Link, useParams } from "react-router-dom";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardTitle } from "@/components/ui/card";
import { AsyncView } from "@/components/AsyncView";
import { useAsync } from "@/hooks/useAsync";
import { useBackend } from "@/infra/backendContext";

export function ToolDetail() {
  const { id = "" } = useParams();
  const backend = useBackend();
  const [state, retry] = useAsync(() => backend.getTool(id), [backend, id]);

  return (
    <div className="mx-auto max-w-4xl space-y-6">
      <Link to="/catalog" className="text-sm text-muted-foreground hover:underline">
        ← Back to catalog
      </Link>
      <AsyncView state={state} retry={retry}>
        {({ manifest: m, trust }) => (
          <>
            <div className="flex items-center gap-3">
              <h1 className="text-2xl font-semibold">{m.name}</h1>
              <Badge>{m.category.replace("_", " ")}</Badge>
              <Badge variant={trust === "bundled" ? "success" : "warning"}>
                {trust === "bundled" ? "Bundled manifest" : "Unsigned remote manifest"}
              </Badge>
            </div>
            <p>{m.description}</p>
            <Card>
              <CardTitle>Details</CardTitle>
              <dl className="grid grid-cols-[10rem_1fr] gap-y-2 text-sm">
                <dt className="text-muted-foreground">License</dt>
                <dd>{m.license}</dd>
                <dt className="text-muted-foreground">Platforms</dt>
                <dd>{m.platforms.join(", ")}</dd>
                <dt className="text-muted-foreground">Prerequisites</dt>
                <dd>{m.prerequisites.length ? m.prerequisites.join(", ") : "none"}</dd>
                <dt className="text-muted-foreground">Install method</dt>
                <dd>{m.install.map((i) => i.method).join(", ")}</dd>
                <dt className="text-muted-foreground">Needs administrator</dt>
                <dd>{m.requires_elevation ? "Yes - you will be asked to approve" : "No"}</dd>
                <dt className="text-muted-foreground">Documentation</dt>
                <dd>{m.documentation}</dd>
              </dl>
            </Card>
            <div className="flex gap-3">
              <Button disabled title="Installing arrives in a later phase">
                Install
              </Button>
              <Button variant="outline" disabled>
                Verify
              </Button>
            </div>
          </>
        )}
      </AsyncView>
    </div>
  );
}
