import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { AppRoutes } from "@/App";
import type { Backend } from "@/domain/backend";
import { BackendProvider } from "@/infra/backendContext";
import { mockBackend } from "@/infra/mockBackend";

function renderAt(path: string, backend: Backend = mockBackend) {
  return render(
    <BackendProvider backend={backend}>
      <MemoryRouter initialEntries={[path]}>
        <AppRoutes />
      </MemoryRouter>
    </BackendProvider>,
  );
}

describe("dashboard", () => {
  it("shows environment readiness and setup issues with recovery text", async () => {
    renderAt("/");
    expect(await screen.findByText("WSL 2 was not found")).toBeInTheDocument();
    expect(screen.getByText(/wsl --install/)).toBeInTheDocument();
  });

  it("shows a recoverable error when the backend fails", async () => {
    const failing: Backend = {
      ...mockBackend,
      detectEnvironment: () => Promise.reject({ code: "environment", message: "probe failed", recovery: "Reinstall winget." }),
    };
    renderAt("/", failing);
    expect(await screen.findByRole("alert")).toHaveTextContent("probe failed");
    expect(screen.getByRole("alert")).toHaveTextContent("Reinstall winget.");
  });
});

describe("catalog", () => {
  it("lists tools and filters by search text", async () => {
    renderAt("/catalog");
    expect(await screen.findByText("Nmap")).toBeInTheDocument();
    await userEvent.type(screen.getByLabelText("Search tools"), "wire");
    await waitFor(() => expect(screen.queryByText("Nmap")).not.toBeInTheDocument());
    expect(screen.getByText("Wireshark")).toBeInTheDocument();
  });

  it("opens tool details", async () => {
    renderAt("/catalog/nmap");
    expect(await screen.findByRole("heading", { name: "Nmap" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Install" })).toBeDisabled();
  });
});
