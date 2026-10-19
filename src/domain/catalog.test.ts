import { DEMO_TOOLS } from "@/infra/demoBackend";
import { EMPTY_FILTERS, filterAndSort, hasActiveFilters } from "./catalog";
import type { ToolStatus } from "./toolStatus";

const status = (id: string): ToolStatus => (id === "nmap" ? "installed" : id === "zap" ? "verification_failed" : "ready_to_install");
const names = (f: Partial<typeof EMPTY_FILTERS>, sort: Parameters<typeof filterAndSort>[3] = "name_asc") =>
  filterAndSort(DEMO_TOOLS, status, { ...EMPTY_FILTERS, ...f }, sort).map((e) => e.manifest.id);

describe("filterAndSort", () => {
  it("returns everything sorted by name by default", () => {
    expect(names({})).toEqual(["nmap", "zap", "semgrep", "trivy", "wireshark", "nikto"].sort((a, b) => nameOf(a).localeCompare(nameOf(b))));
  });

  it("searches name, summary and description case-insensitively", () => {
    expect(names({ text: "WIRESHARK" })).toEqual(["wireshark"]);
    expect(names({ text: "port scanner" })).toEqual(["nmap"]);
    expect(names({ text: "hosts and services" })).toEqual(["nmap"]); // description only
    expect(names({ text: "no-such-tool" })).toEqual([]);
  });

  it("filters by category, platform, execution method and status", () => {
    expect(names({ category: "vulnerability" })).toEqual(["semgrep", "trivy"]);
    expect(names({ platform: "container" })).toEqual(["semgrep"]);
    expect(names({ method: "container_image" })).toEqual(["semgrep"]);
    expect(names({ status: "verification_failed" })).toEqual(["zap"]);
    expect(names({ status: "installed", category: "reconnaissance" })).toEqual(["nmap"]);
    expect(names({ status: "installed", category: "network" })).toEqual([]);
  });

  it("sorts by name descending and by category", () => {
    expect(names({}, "name_desc")[0]).toBe("wireshark"); // "OWASP ZAP" sorts under O
    const byCategory = filterAndSort(DEMO_TOOLS, status, EMPTY_FILTERS, "category").map((e) => e.manifest.category);
    expect(byCategory).toEqual([...byCategory].sort());
  });

  it("reports active filters", () => {
    expect(hasActiveFilters(EMPTY_FILTERS)).toBe(false);
    expect(hasActiveFilters({ ...EMPTY_FILTERS, text: " " })).toBe(false);
    expect(hasActiveFilters({ ...EMPTY_FILTERS, status: "error" })).toBe(true);
  });
});

function nameOf(id: string) {
  return DEMO_TOOLS.find((t) => t.manifest.id === id)!.manifest.name;
}

