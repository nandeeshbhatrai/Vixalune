import { Card } from "@/components/ui/card";

function Placeholder({ title, children }: { title: string; children: string }) {
  return (
    <div className="mx-auto max-w-4xl space-y-6">
      <h1 className="text-2xl font-semibold">{title}</h1>
      <Card>
        <p className="text-sm text-muted-foreground">{children}</p>
      </Card>
    </div>
  );
}

export const Learn = () => (
  <Placeholder title="Learn">Tutorials and authorized local lab exercises arrive in a later phase.</Placeholder>
);

export const SettingsPage = () => (
  <Placeholder title="Settings">Storage locations, runtime paths and resource limits arrive in a later phase.</Placeholder>
);
