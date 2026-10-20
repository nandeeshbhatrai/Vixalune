import { Search, ShieldAlert } from "lucide-react";
import { useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { EmptyState } from "@/components/EmptyState";
import { ErrorNotice } from "@/components/ErrorNotice";
import { fieldClass } from "@/components/run/ParamFields";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import type { DiscoveredDetails, DiscoverResult } from "@/domain/types";
import { useAction } from "@/hooks/useAction";
import { useWorkspace } from "@/hooks/useWorkspace";
import { useBackend } from "@/infra/backendContext";

/**
 * Search the Windows Package Manager for software beyond the reviewed catalog. Results are not reviewed by Vixalune: adding one
 * creates a labelled catalog entry that can only install and verify that single package, and installing it still asks first.
 */
export function DiscoverPanel() {
  const backend = useBackend();
  const ws = useWorkspace();
  const navigate = useNavigate();
  const search = useAction();
  const detail = useAction();
  const add = useAction();
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<DiscoverResult[] | null>(null);
  const [asked, setAsked] = useState("");
  const [open, setOpen] = useState<DiscoveredDetails | null>(null);

  const doSearch = async () => {
    const q = query.trim();
    if (q.length < 2) return;
    setResults(null);
    const r = await search.run(() => backend.discoverSearch(q));
    if (r) {
      setResults(r);
      setAsked(q);
    }
  };

  const showDetails = async (id: string) => {
    const d = await detail.run(() => backend.discoverDetails(id));
    if (d) setOpen(d);
  };

  const confirmAdd = async () => {
    if (!open) return;
    const entry = await add.run(() => backend.discoverAdd(open.id));
    if (entry) {
      setOpen(null);
      ws.refreshAll();
      navigate(`/catalog/${entry.manifest.id}`);
    }
  };

  return (
    <div className="space-y-4">
      <Card role="note" aria-label="About Discover">
        <p className="flex items-center gap-2 font-medium">
          <ShieldAlert size={14} aria-hidden /> These packages are not reviewed by Vixalune
        </p>
        <ul className="mt-1 list-disc space-y-1 pl-5 text-muted-foreground">
          <li>Search uses the Windows Package Manager (winget) community repository, so your search text is sent to Microsoft's package service.</li>
          <li>Adding a package creates a catalog entry labelled <strong>Community, not reviewed</strong>. Adding installs nothing.</li>
          <li>Installing is a separate step with its own confirmation. It runs the publisher's installer, which may ask for administrator rights.</li>
          <li>Vixalune can install, verify and uninstall these packages. It has no guided run profiles for them and the AI Assistant does not recommend them. Only install software from publishers you trust.</li>
        </ul>
      </Card>

      <form
        role="search"
        aria-label="Package search"
        className="flex flex-wrap items-end gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          void doSearch();
        }}
      >
        <div>
          <label htmlFor="discover-q" className="mb-1 block text-xs text-muted-foreground">
            Search the package repository
          </label>
          <input id="discover-q" className={`${fieldClass} w-72`} value={query} maxLength={64} placeholder="For example: packet capture, nmap, forensics" onChange={(e) => setQuery(e.target.value)} />
        </div>
        <Button type="submit" disabled={search.busy || query.trim().length < 2}>
          <Search size={14} aria-hidden /> Search
        </Button>
      </form>

      {search.busy ? <p role="status" className="text-muted-foreground">Searching…</p> : null}
      {search.error ? <ErrorNotice error={search.error} /> : null}
      {detail.error ? <ErrorNotice error={detail.error} /> : null}

      {results && results.length === 0 ? (
        <EmptyState icon={Search} title={`Nothing found for “${asked}”`}>
          Try another word, or part of a name. Only packages from the winget community repository are searched.
        </EmptyState>
      ) : null}
      {results && results.length > 0 ? (
        <section aria-label="Search results">
          <p role="status" className="mb-2 text-xs text-muted-foreground">
            {results.length} result{results.length === 1 ? "" : "s"} for “{asked}” (not reviewed)
          </p>
          <ul className="space-y-2">
            {results.map((r) => (
              <li key={r.package.id} className="flex flex-wrap items-center gap-3 rounded-lg border border-border bg-card p-3">
                <div className="min-w-0 flex-1">
                  <p className="font-medium">{r.package.name}</p>
                  <p className="text-xs text-muted-foreground">
                    {r.package.id} · version {r.package.version}
                    {r.package.matched ? ` · matched ${r.package.matched}` : ""}
                  </p>
                </div>
                {r.in_catalog ? (
                  <Link className="text-primary underline" to={`/catalog/${r.in_catalog}`}>
                    {r.in_catalog_is_community ? "Already added from Discover" : "In the reviewed catalog"}
                  </Link>
                ) : (
                  <Button variant="outline" size="sm" disabled={detail.busy} onClick={() => void showDetails(r.package.id)} aria-label={`Details of ${r.package.name}`}>
                    Details…
                  </Button>
                )}
              </li>
            ))}
          </ul>
        </section>
      ) : null}

      <ConfirmDialog
        open={open !== null}
        onOpenChange={(o) => !o && setOpen(null)}
        title={open ? `Add ${open.name} to your catalog?` : ""}
        description="Adding creates a catalog entry and installs nothing. You will confirm the installation separately."
        confirmLabel="Add to my catalog"
        busy={add.busy}
        onConfirm={() => void confirmAdd()}
      >
        {open ? (
          <div className="space-y-2 text-sm">
            <p role="alert" className="rounded-md border border-amber-500/60 p-2">
              Not reviewed by Vixalune. Check the publisher and licence yourself before installing.
            </p>
            <dl className="grid gap-x-4 gap-y-1 sm:grid-cols-2">
              <div><dt className="text-xs text-muted-foreground">Package id</dt><dd className="font-mono text-xs">{open.id}</dd></div>
              <div><dt className="text-xs text-muted-foreground">Version (the one that would be installed)</dt><dd>{open.version}</dd></div>
              <div><dt className="text-xs text-muted-foreground">Publisher</dt><dd>{open.publisher || "not stated"}</dd></div>
              <div><dt className="text-xs text-muted-foreground">Licence</dt><dd>{open.license || "not stated"}</dd></div>
              <div><dt className="text-xs text-muted-foreground">Homepage</dt><dd className="break-all">{open.homepage || "not stated"}</dd></div>
              <div><dt className="text-xs text-muted-foreground">Installer type</dt><dd>{open.installer_type || "not stated"}</dd></div>
              <div className="sm:col-span-2"><dt className="text-xs text-muted-foreground">Installer SHA-256 published by the repository</dt><dd className="break-all font-mono text-xs">{open.installer_sha256 || "not stated"}</dd></div>
            </dl>
            {open.description ? <p className="text-muted-foreground">{open.description}</p> : null}
            {add.error ? <ErrorNotice error={add.error} /> : null}
          </div>
        ) : null}
      </ConfirmDialog>
    </div>
  );
}
