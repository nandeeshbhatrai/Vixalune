import { Activity, Briefcase, FlaskConical, GraduationCap, LayoutDashboard, ListChecks, Play, Settings, ShieldCheck, Wrench, type LucideIcon } from "lucide-react";
import { Navigate, NavLink, Outlet, useLocation } from "react-router-dom";
import { StartupNotices } from "@/components/StartupNotices";
import { setupSeen } from "@/pages/SetupWizard";
import { useAsync } from "@/hooks/useAsync";
import { useBackend } from "@/infra/backendContext";
import { WorkspaceProvider } from "@/hooks/useWorkspace";
import { cn } from "@/lib/utils";

const NAV: { to: string; label: string; icon: LucideIcon; end?: boolean }[] = [
  { to: "/", label: "Dashboard", icon: LayoutDashboard, end: true },
  { to: "/catalog", label: "Tool Catalog", icon: Wrench },
  { to: "/installed", label: "Installed Tools", icon: ListChecks },
  { to: "/run", label: "Run a Tool", icon: Play },
  { to: "/jobs", label: "Running Jobs", icon: Briefcase },
  { to: "/learn", label: "Learn", icon: GraduationCap },
  { to: "/labs", label: "Learning Labs", icon: FlaskConical },
  { to: "/activity", label: "Activity", icon: Activity },
  { to: "/setup", label: "Setup", icon: ShieldCheck },
  { to: "/settings", label: "Settings", icon: Settings },
];

function AppVersion() {
  const backend = useBackend();
  const [info] = useAsync(() => backend.getAppInfo(), [backend]);
  return info.status === "ready" ? <p className="px-3 pb-2 text-xs text-muted-foreground">Version {info.data.version}</p> : null;
}

export function AppShell({ firstRunRedirect = false }: { firstRunRedirect?: boolean }) {
  const backend = useBackend();
  const location = useLocation();
  // First launch: open the welcome tour once. It never changes anything by itself.
  if (firstRunRedirect && location.pathname === "/" && !setupSeen()) return <Navigate to="/welcome" replace />;
  return (
    <WorkspaceProvider>
      <a
        href="#main"
        className="sr-only focus:not-sr-only focus:fixed focus:left-2 focus:top-2 focus:z-50 focus:rounded focus:bg-primary focus:px-3 focus:py-1.5 focus:text-primary-foreground"
      >
        Skip to content
      </a>
      <div className="flex h-screen">
        <nav aria-label="Main" className="flex w-52 shrink-0 flex-col gap-0.5 border-r border-border bg-card p-2">
          <div className="px-3 pb-4 pt-3 text-base font-semibold tracking-tight">
            <span className="text-primary">▲</span> Vixalune
          </div>
          {NAV.map(({ to, label, icon: Icon, end }) => (
            <NavLink
              key={to}
              to={to}
              end={end}
              className={({ isActive }) =>
                cn(
                  "flex items-center gap-2.5 rounded-md px-3 py-1.5 text-muted-foreground hover:bg-muted hover:text-foreground",
                  isActive && "bg-muted font-medium text-foreground",
                )
              }
            >
              <Icon size={15} aria-hidden /> {label}
            </NavLink>
          ))}
          <p className="mt-auto px-3 pb-1 text-xs text-muted-foreground">For authorized testing and learning only.</p>
          <AppVersion />
        </nav>
        <div className="flex min-w-0 flex-1 flex-col">
          {backend.isDemo ? (
            <div role="note" className="border-b border-warning/40 bg-warning/15 px-4 py-1.5 text-xs text-warning">
              <strong>Demonstration data.</strong> Everything shown comes from a built-in demo, not from this computer.
            </div>
          ) : null}
          <StartupNotices />
          <main id="main" tabIndex={-1} className="flex-1 overflow-y-auto p-6">
            <div className="mx-auto max-w-5xl">
              <Outlet />
            </div>
          </main>
        </div>
      </div>
    </WorkspaceProvider>
  );
}

