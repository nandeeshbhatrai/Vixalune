import { useState } from "react";
import { Link } from "react-router-dom";
import { ErrorNotice } from "@/components/ErrorNotice";
import { Button } from "@/components/ui/button";
import { Card, CardTitle } from "@/components/ui/card";
import type { AppInfo } from "@/domain/types";
import { useAction } from "@/hooks/useAction";
import { useWorkspace } from "@/hooks/useWorkspace";
import { useBackend } from "@/infra/backendContext";
import releaseNotes from "../../CHANGELOG.md?raw";

/** Version, release notes, how updates work, and what the catalog contains. Updating Vixalune never runs downloaded code. */
export function AboutAndUpdates({ info }: { info: AppInfo }) {
  const backend = useBackend();
  const ws = useWorkspace();
  const { error, run } = useAction();
  const [showNotes, setShowNotes] = useState(false);
  const tools = ws.core.status === "ready" ? ws.core.data.tools : [];
  const updates = tools.filter((t) => ws.statusOf(t.manifest.id) === "update_available");

  return (
    <>
      <Card>
        <CardTitle>Vixalune updates</CardTitle>
        <p>
          You are running <strong>version {info.version}</strong>.
        </p>
        <p className="mt-2 text-sm text-muted-foreground">
          Vixalune does not check for, download or run updates by itself. To update, download the newer installer from the project&apos;s release page, check its SHA-256 against the value published there, and run it: it installs over the current version and keeps your data
          (settings, history, labs). If something goes wrong, your data folder also contains a backup made before any database upgrade.
        </p>
        <div className="mt-3 flex flex-wrap gap-2">
          <Button variant="outline" onClick={() => void run(() => backend.openReleasePage())}>
            Open the release page
          </Button>
          <Button variant="outline" onClick={() => setShowNotes((s) => !s)} aria-expanded={showNotes}>
            {showNotes ? "Hide release notes" : "Show release notes"}
          </Button>
        </div>
        {error ? (
          <div className="mt-2">
            <ErrorNotice error={error} />
          </div>
        ) : null}
        {showNotes ? (
          <pre aria-label="Release notes" className="mt-3 max-h-80 overflow-auto whitespace-pre-wrap rounded-md bg-background p-3 text-xs">
            {releaseNotes}
          </pre>
        ) : null}
      </Card>

      <Card>
        <CardTitle>Catalog and tool updates</CardTitle>
        <p className="text-sm text-muted-foreground">
          The tool catalog ships with Vixalune (bundled, offline). Remote catalogs are disabled. Updates to the tools themselves (Nmap, Trivy and so on) are separate from Vixalune updates and are always started by you from the tool&apos;s page.
        </p>
        <p className="mt-2 text-sm" role="status">
          {updates.length === 0 ? "No tool updates are known right now." : `${updates.length} tool update${updates.length === 1 ? " is" : "s are"} available: ${updates.map((t) => t.manifest.name).join(", ")}. `}
          {updates.length > 0 ? (
            <Link to="/installed" className="text-primary hover:underline">
              Open Installed Tools
            </Link>
          ) : null}
        </p>
        <div className="mt-3 overflow-x-auto rounded-md border border-border">
          <table className="w-full text-left text-xs">
            <caption className="sr-only">Catalog entries</caption>
            <thead className="bg-muted uppercase tracking-wider text-muted-foreground">
              <tr>
                <th scope="col" className="px-3 py-1.5">Tool</th>
                <th scope="col" className="px-3 py-1.5">Definition version</th>
                <th scope="col" className="px-3 py-1.5">Source</th>
                <th scope="col" className="px-3 py-1.5">SHA-256 (start)</th>
              </tr>
            </thead>
            <tbody>
              {tools.map((t) => (
                <tr key={t.manifest.id} className="border-t border-border">
                  <td className="px-3 py-1.5">{t.manifest.name}</td>
                  <td className="px-3 py-1.5 tabular-nums">{t.manifest.manifest_version}</td>
                  <td className="px-3 py-1.5">{t.trust === "bundled" ? "bundled with Vixalune" : "remote (unsigned)"}</td>
                  <td className="px-3 py-1.5 font-mono">{t.digest ? t.digest.slice(0, 12) : "n/a"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </Card>
    </>
  );
}
