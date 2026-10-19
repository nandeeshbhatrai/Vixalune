import { HashRouter, Route, Routes } from "react-router-dom";
import { AppShell } from "@/components/AppShell";
import { ActivityPage } from "@/pages/ActivityPage";
import { Catalog } from "@/pages/Catalog";
import { Dashboard } from "@/pages/Dashboard";
import { InstalledTools } from "@/pages/InstalledTools";
import { Jobs } from "@/pages/Jobs";
import { Labs } from "@/pages/Labs";
import { LearnIndex, TutorialPage } from "@/pages/Learn";
import { RunPage } from "@/pages/RunPage";
import { SettingsPage } from "@/pages/SettingsPage";
import { SetupWizard } from "@/pages/SetupWizard";
import { ToolDetail } from "@/pages/ToolDetail";
import { Welcome } from "@/pages/Welcome";
import { BackendProvider, selectBackend } from "@/infra/backendContext";

export function AppRoutes({ firstRunRedirect = false }: { firstRunRedirect?: boolean }) {
  return (
    <Routes>
      <Route element={<AppShell firstRunRedirect={firstRunRedirect} />}>
        <Route index element={<Dashboard />} />
        <Route path="catalog" element={<Catalog />} />
        <Route path="catalog/:id" element={<ToolDetail />} />
        <Route path="installed" element={<InstalledTools />} />
        <Route path="jobs" element={<Jobs />} />
        <Route path="run" element={<RunPage />} />
        <Route path="run/:toolId" element={<RunPage />} />
        <Route path="learn" element={<LearnIndex />} />
        <Route path="learn/:toolId" element={<TutorialPage />} />
        <Route path="labs" element={<Labs />} />
        <Route path="welcome" element={<Welcome />} />
        <Route path="setup" element={<SetupWizard />} />
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
        <AppRoutes firstRunRedirect />
      </HashRouter>
    </BackendProvider>
  );
}

