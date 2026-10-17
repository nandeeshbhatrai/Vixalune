import { HashRouter, Route, Routes } from "react-router-dom";
import { AppShell } from "@/components/AppShell";
import { ActivityPage } from "@/pages/ActivityPage";
import { Catalog } from "@/pages/Catalog";
import { Dashboard } from "@/pages/Dashboard";
import { InstalledTools } from "@/pages/InstalledTools";
import { Jobs } from "@/pages/Jobs";
import { Labs } from "@/pages/Labs";
import { SettingsPage } from "@/pages/SettingsPage";
import { ToolDetail } from "@/pages/ToolDetail";
import { BackendProvider, selectBackend } from "@/infra/backendContext";

export function AppRoutes() {
  return (
    <Routes>
      <Route element={<AppShell />}>
        <Route index element={<Dashboard />} />
        <Route path="catalog" element={<Catalog />} />
        <Route path="catalog/:id" element={<ToolDetail />} />
        <Route path="installed" element={<InstalledTools />} />
        <Route path="jobs" element={<Jobs />} />
        <Route path="labs" element={<Labs />} />
        <Route path="activity" element={<ActivityPage />} />
        <Route path="settings" element={<SettingsPage />} />
        <Route path="*" element={<p>Page not found.</p>} />
      </Route>
    </Routes>
  );
}

const backend = selectBackend();

export function App() {
  return (
    <BackendProvider backend={backend}>
      <HashRouter>
        <AppRoutes />
      </HashRouter>
    </BackendProvider>
  );
}
