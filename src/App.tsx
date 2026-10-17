import { HashRouter, Route, Routes } from "react-router-dom";
import { AppShell } from "@/components/AppShell";
import { Catalog } from "@/pages/Catalog";
import { Dashboard } from "@/pages/Dashboard";
import { Learn, SettingsPage } from "@/pages/Placeholders";
import { ToolDetail } from "@/pages/ToolDetail";

export function AppRoutes() {
  return (
    <Routes>
      <Route element={<AppShell />}>
        <Route index element={<Dashboard />} />
        <Route path="catalog" element={<Catalog />} />
        <Route path="catalog/:id" element={<ToolDetail />} />
        <Route path="learn" element={<Learn />} />
        <Route path="settings" element={<SettingsPage />} />
      </Route>
    </Routes>
  );
}

export function App() {
  return (
    <HashRouter>
      <AppRoutes />
    </HashRouter>
  );
}
