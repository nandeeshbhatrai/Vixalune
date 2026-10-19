import { AsyncView } from "@/components/AsyncView";
import { PageHeader } from "@/components/PageHeader";
import { AboutAndUpdates } from "@/components/AboutAndUpdates";
import { ScopeAndRetention } from "@/components/settings/ScopeAndRetention";
import { Card, CardTitle } from "@/components/ui/card";
import { useAsync } from "@/hooks/useAsync";
import { useTheme, type ThemeChoice } from "@/hooks/useTheme";
import { useBackend } from "@/infra/backendContext";

const THEMES: { value: ThemeChoice; label: string }[] = [
  { value: "system", label: "Match system" },
  { value: "dark", label: "Dark" },
  { value: "light", label: "Light" },
];

export function SettingsPage() {
  const backend = useBackend();
  const [theme, setTheme] = useTheme();
  const [info, retry] = useAsync(() => backend.getAppInfo(), [backend]);

  return (
    <div className="space-y-4">
      <PageHeader title="Settings" />
      <Card>
        <CardTitle>Appearance</CardTitle>
        <fieldset>
          <legend className="sr-only">Theme</legend>
          <div className="flex gap-4">
            {THEMES.map((t) => (
              <label key={t.value} className="flex items-center gap-2">
                <input type="radio" name="theme" checked={theme === t.value} onChange={() => setTheme(t.value)} />
                {t.label}
              </label>
            ))}
          </div>
        </fieldset>
      </Card>
      <ScopeAndRetention />
      <AsyncView state={info} retry={retry} label="Loading app info">
        {(i) => <AboutAndUpdates info={i} />}
      </AsyncView>
      <Card>
        <CardTitle>About this installation</CardTitle>
        <AsyncView state={info} retry={retry} label="Loading app info">
          {(i) => (
            <dl className="space-y-2">
              <div>
                <dt className="text-xs text-muted-foreground">Version</dt>
                <dd>{i.version}</dd>
              </div>
              <div>
                <dt className="text-xs text-muted-foreground">Data folder</dt>
                <dd className="break-all font-mono text-xs">{i.data_dir}</dd>
              </div>
              {i.rejected_manifests.length > 0 ? (
                <div>
                  <dt className="text-xs text-muted-foreground">Catalog files that were rejected</dt>
                  <dd>
                    <ul className="list-disc pl-5 text-destructive">
                      {i.rejected_manifests.map((r) => (
                        <li key={r}>{r}</li>
                      ))}
                    </ul>
                  </dd>
                </div>
              ) : null}
            </dl>
          )}
        </AsyncView>
      </Card>
      <p className="text-muted-foreground">
        Storage locations, runtime paths and resource limits cannot be changed from the interface yet.
      </p>
    </div>
  );
}
