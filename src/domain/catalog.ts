import type { ToolStatus } from "./toolStatus";
import type { CatalogEntry, Category, InstallMethod, RuntimeKind } from "./types";

export type SortKey = "name_asc" | "name_desc" | "category";

export interface CatalogFilters {
  text: string;
  category: Category | "all";
  platform: RuntimeKind | "all";
  method: InstallMethod | "all";
  status: ToolStatus | "all";
}

export const EMPTY_FILTERS: CatalogFilters = { text: "", category: "all", platform: "all", method: "all", status: "all" };

export function hasActiveFilters(f: CatalogFilters): boolean {
  return f.text.trim() !== "" || f.category !== "all" || f.platform !== "all" || f.method !== "all" || f.status !== "all";
}

/** Pure search + filter + sort over catalog entries. Search covers name, summary and description. */
export function filterAndSort(
  entries: CatalogEntry[],
  statusOf: (toolId: string) => ToolStatus,
  filters: CatalogFilters,
  sort: SortKey,
): CatalogEntry[] {
  const needle = filters.text.trim().toLowerCase();
  const out = entries.filter(({ manifest: m }) => {
    if (needle && !`${m.name} ${m.summary} ${m.description}`.toLowerCase().includes(needle)) return false;
    if (filters.category !== "all" && m.category !== filters.category) return false;
    if (filters.platform !== "all" && !m.platforms.includes(filters.platform)) return false;
    if (filters.method !== "all" && !m.install.some((i) => i.method === filters.method)) return false;
    if (filters.status !== "all" && statusOf(m.id) !== filters.status) return false;
    return true;
  });
  const byName = (a: CatalogEntry, b: CatalogEntry) => a.manifest.name.localeCompare(b.manifest.name, undefined, { sensitivity: "base" });
  out.sort((a, b) => {
    switch (sort) {
      case "name_desc":
        return -byName(a, b);
      case "category":
        return a.manifest.category.localeCompare(b.manifest.category) || byName(a, b);
      default:
        return byName(a, b);
    }
  });
  return out;
}
