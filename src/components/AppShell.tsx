import { BookOpen, LayoutDashboard, Settings, Wrench } from "lucide-react";
import { NavLink, Outlet } from "react-router-dom";
import { cn } from "@/lib/utils";

const NAV = [
  { to: "/", label: "Dashboard", icon: LayoutDashboard, end: true },
  { to: "/catalog", label: "Tool catalog", icon: Wrench, end: false },
  { to: "/learn", label: "Learn", icon: BookOpen, end: false },
  { to: "/settings", label: "Settings", icon: Settings, end: false },
];

export function AppShell() {
  return (
    <div className="flex h-screen">
      <nav aria-label="Main" className="flex w-56 shrink-0 flex-col gap-1 border-r border-border bg-card p-3">
        <div className="px-3 py-4 text-lg font-semibold">Vixalune</div>
        {NAV.map(({ to, label, icon: Icon, end }) => (
          <NavLink
            key={to}
            to={to}
            end={end}
            className={({ isActive }) =>
              cn("flex items-center gap-3 rounded-md px-3 py-2 text-sm hover:bg-muted", isActive && "bg-muted font-medium")
            }
          >
            <Icon size={16} aria-hidden /> {label}
          </NavLink>
        ))}
        <p className="mt-auto px-3 text-xs text-muted-foreground">For authorized testing and learning only.</p>
      </nav>
      <main className="flex-1 overflow-y-auto p-8">
        <Outlet />
      </main>
    </div>
  );
}
