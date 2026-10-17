import { SearchX } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { AsyncView } from "@/components/AsyncView";
import { EmptyState } from "@/components/EmptyState";
import { PageHeader } from "@/components/PageHeader";
import { ToolCard } from "@/components/ToolCard";
import { Button } from "@/components/ui/button";
import { EMPTY_FILTERS, filterAndSort, hasActiveFilters, type CatalogFilters, type SortKey } from "@/domain/catalog";
import { ALL_STATUSES } from "@/domain/toolStatus";
import { useWorkspace } from "@/hooks/useWorkspace";
import { CATEGORY_LABEL, METHOD_LABEL, RUNTIME_LABEL, STATUS_META } from "@/presentation/labels";

const selectClass = "h-8 rounded-md border border-border bg-card px-2 text-sm";

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="flex flex-col gap-1 text-xs text-muted-foreground">
      {label}
      {children}
    </label>
  );
}

export function Catalog() {
  const ws = useWorkspace();
  const [filters, setFilters] = useState<CatalogFilters>(EMPTY_FILTERS);
  const [sort, setSort] = useState<SortKey>("name_asc");
  const searchRef = useRef<HTMLInputElement>(null);
  const set = <K extends keyof CatalogFilters>(k: K, v: CatalogFilters[K]) => setFilters((f) => ({ ...f, [k]: v }));

  // "/" jumps to search, unless the user is already typing somewhere.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement;
      if (e.key === "/" && !["INPUT", "SELECT", "TEXTAREA"].includes(t.tagName)) {
        e.preventDefault();
        searchRef.current?.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const tools = ws.core.status === "ready" ? ws.core.data.tools : null;
  const results = useMemo(() => (tools ? filterAndSort(tools, ws.statusOf, filters, sort) : []), [tools, ws, filters, sort]);

  return (
    <div className="space-y-4">
      <PageHeader title="Tool Catalog" subtitle="Browse open-source security tools. Press / to search." />

      <form role="search" aria-label="Filter tools" className="flex flex-wrap items-end gap-3" onSubmit={(e) => e.preventDefault()}>
        <Field label="Search">
          <input
            ref={searchRef}
            type="search"
            aria-label="Search tools"
            placeholder="Name or description"
            value={filters.text}
            onChange={(e) => set("text", e.target.value)}
            className={`${selectClass} w-56 text-foreground`}
          />
        </Field>
        <Field label="Category">
          <select aria-label="Category" className={selectClass} value={filters.category} onChange={(e) => set("category", e.target.value as CatalogFilters["category"])}>
            <option value="all">All categories</option>
            {Object.entries(CATEGORY_LABEL).map(([v, l]) => (
              <option key={v} value={v}>
                {l}
              </option>
            ))}
          </select>
        </Field>
        <Field label="Platform">
          <select aria-label="Platform" className={selectClass} value={filters.platform} onChange={(e) => set("platform", e.target.value as CatalogFilters["platform"])}>
            <option value="all">All platforms</option>
            {Object.entries(RUNTIME_LABEL).map(([v, l]) => (
              <option key={v} value={v}>
                {l}
              </option>
            ))}
          </select>
        </Field>
        <Field label="Execution method">
          <select aria-label="Execution method" className={selectClass} value={filters.method} onChange={(e) => set("method", e.target.value as CatalogFilters["method"])}>
            <option value="all">All methods</option>
            {Object.entries(METHOD_LABEL).map(([v, l]) => (
              <option key={v} value={v}>
                {l}
              </option>
            ))}
          </select>
        </Field>
        <Field label="Status">
          <select aria-label="Status" className={selectClass} value={filters.status} onChange={(e) => set("status", e.target.value as CatalogFilters["status"])}>
            <option value="all">Any status</option>
            {ALL_STATUSES.map((s) => (
              <option key={s} value={s}>
                {STATUS_META[s].label}
              </option>
            ))}
          </select>
        </Field>
        <Field label="Sort by">
          <select aria-label="Sort by" className={selectClass} value={sort} onChange={(e) => setSort(e.target.value as SortKey)}>
            <option value="name_asc">Name (A to Z)</option>
            <option value="name_desc">Name (Z to A)</option>
            <option value="category">Category</option>
          </select>
        </Field>
        {hasActiveFilters(filters) ? (
          <Button variant="ghost" size="sm" onClick={() => setFilters(EMPTY_FILTERS)}>
            Clear filters
          </Button>
        ) : null}
      </form>

      <AsyncView state={ws.core} retry={ws.refreshAll} label="Loading catalog">
        {(data) => (
          <>
            <p role="status" className="text-xs text-muted-foreground">
              {results.length} of {data.tools.length} tools
            </p>
            {data.tools.length === 0 ? (
              <EmptyState icon={SearchX} title="The catalog is empty">
                No tool definitions were found. Reinstall Vixalune or check the manifests folder.
              </EmptyState>
            ) : results.length === 0 ? (
              <EmptyState icon={SearchX} title="No tools match your filters">
                <Button variant="outline" size="sm" onClick={() => setFilters(EMPTY_FILTERS)}>
                  Clear filters
                </Button>
              </EmptyState>
            ) : (
              <ul className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
                {results.map((entry) => (
                  <li key={entry.manifest.id}>
                    <ToolCard entry={entry} status={ws.statusOf(entry.manifest.id)} />
                  </li>
                ))}
              </ul>
            )}
          </>
        )}
      </AsyncView>
    </div>
  );
}
