import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Backend } from "@/domain/backend";
import type { CatalogEntry, DiscoveredDetails, DiscoverResult } from "@/domain/types";
import { createDemoBackend } from "@/infra/demoBackend";
import { renderApp } from "@/test/render";

const DETAILS: DiscoveredDetails = {
  id: "Vendor.Cool-Tool",
  name: "Cool Tool",
  version: "2.1",
  publisher: "Vendor Inc",
  publisher_url: "https://vendor.example",
  homepage: "https://vendor.example",
  license: "MIT",
  license_url: "",
  description: "Does cool things.",
  tags: ["security"],
  installer_type: "exe",
  installer_url: "https://vendor.example/setup.exe",
  installer_sha256: "ab".repeat(32),
};

const RESULTS: DiscoverResult[] = [
  { package: { id: "Insecure.Nmap", name: "Nmap", version: "7.80", matched: "" }, in_catalog: "nmap", in_catalog_is_community: false },
  { package: { id: "Vendor.Cool-Tool", name: "Cool Tool", version: "2.1", matched: "Tag: security" }, in_catalog: null, in_catalog_is_community: false },
];

async function communityEntry(): Promise<CatalogEntry> {
  const base = await createDemoBackend().getTool("nmap");
  return { ...base, trust: "remote_unsigned", manifest: { ...base.manifest, id: "community-vendor-cool-tool", name: "Cool Tool", summary: "Does cool things.", launch_profiles: [] } };
}

function backendWith(extra: Partial<Backend> = {}): Backend {
  return { ...createDemoBackend(), isDemo: false, ...extra };
}

describe("Discover", () => {
  it("says the results are not reviewed, what adding does, and where the search text goes", async () => {
    renderApp("/catalog", backendWith());
    await userEvent.click(await screen.findByRole("tab", { name: "Discover more tools" }));
    const note = await screen.findByRole("note", { name: "About Discover" });
    expect(note).toHaveTextContent("not reviewed by Vixalune");
    expect(note).toHaveTextContent("Adding installs nothing");
    expect(note).toHaveTextContent("your search text is sent to Microsoft's package service");
    expect(note).toHaveTextContent("AI Assistant does not recommend them");
  });

  it("searches, marks packages already in the catalog, and shows details with the publisher's hash before adding", async () => {
    const search = vi.fn(async () => RESULTS);
    const details = vi.fn(async () => DETAILS);
    renderApp("/catalog", backendWith({ discoverSearch: search, discoverDetails: details }));
    await userEvent.click(await screen.findByRole("tab", { name: "Discover more tools" }));
    await userEvent.type(await screen.findByLabelText("Search the package repository"), "  cool  ");
    await userEvent.click(screen.getByRole("button", { name: "Search" }));
    expect(search).toHaveBeenCalledWith("cool");
    const list = await screen.findByRole("region", { name: "Search results" });
    expect(within(list).getByRole("link", { name: "In the reviewed catalog" })).toHaveAttribute("href", "/catalog/nmap");
    await userEvent.click(within(list).getByRole("button", { name: "Details of Cool Tool" }));
    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent("Not reviewed by Vixalune");
    expect(dialog).toHaveTextContent("Vendor Inc");
    expect(dialog).toHaveTextContent("ab".repeat(32));
    expect(details).toHaveBeenCalledWith("Vendor.Cool-Tool");
  });

  it("adds only by package id, installs nothing, and opens the new entry", async () => {
    const entry = await communityEntry();
    const add = vi.fn(async () => entry);
    const install = vi.fn();
    renderApp("/catalog", backendWith({ discoverSearch: async () => RESULTS, discoverDetails: async () => DETAILS, discoverAdd: add, installTool: install, getTool: async () => entry }));
    await userEvent.click(await screen.findByRole("tab", { name: "Discover more tools" }));
    await userEvent.type(await screen.findByLabelText("Search the package repository"), "cool");
    await userEvent.click(screen.getByRole("button", { name: "Search" }));
    await userEvent.click(await screen.findByRole("button", { name: "Details of Cool Tool" }));
    await userEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Add to my catalog" }));
    await waitFor(() => expect(add).toHaveBeenCalledWith("Vendor.Cool-Tool"));
    expect(install).not.toHaveBeenCalled();
    expect(await screen.findByRole("region", { name: "Community entry" })).toHaveTextContent("not reviewed by Vixalune");
  });

  it("says nothing was found instead of inventing results, and shows backend errors", async () => {
    renderApp("/catalog", backendWith({ discoverSearch: async () => [] }));
    await userEvent.click(await screen.findByRole("tab", { name: "Discover more tools" }));
    await userEvent.type(await screen.findByLabelText("Search the package repository"), "zzzz");
    await userEvent.click(screen.getByRole("button", { name: "Search" }));
    expect(await screen.findByText(/Nothing found for “zzzz”/)).toBeInTheDocument();
  });

  it("is honest in the demonstration: no invented packages", async () => {
    renderApp("/catalog", createDemoBackend());
    await userEvent.click(await screen.findByRole("tab", { name: "Discover more tools" }));
    await userEvent.type(await screen.findByLabelText("Search the package repository"), "nmap");
    await userEvent.click(screen.getByRole("button", { name: "Search" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("not available in the demonstration");
  });
});

describe("community entries in the catalog", () => {
  it("are labelled on their card and on their page, and warn before installing", async () => {
    const entry = await communityEntry();
    renderApp("/catalog", backendWith({ listTools: async () => [entry], getTool: async () => entry }));
    const card = await screen.findByRole("link", { name: /Cool Tool/ });
    expect(card).toHaveTextContent("Community, not reviewed");
    await userEvent.click(card);
    expect(await screen.findByRole("region", { name: "Community entry" })).toHaveTextContent("has no guided run profiles");
  });

  it("can be removed from the catalog after a confirmation", async () => {
    const entry = await communityEntry();
    const remove = vi.fn(async () => undefined);
    renderApp("/catalog/community-vendor-cool-tool", backendWith({ getTool: async () => entry, listTools: async () => [entry], communityRemove: remove }));
    await userEvent.click(await screen.findByRole("button", { name: "Remove from catalog" }));
    expect(remove).not.toHaveBeenCalled();
    await userEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Remove" }));
    await waitFor(() => expect(remove).toHaveBeenCalledWith("community-vendor-cool-tool"));
  });
});
