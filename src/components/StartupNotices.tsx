import { TriangleAlert } from "lucide-react";
import { useBackend } from "@/infra/backendContext";
import { useAsync } from "@/hooks/useAsync";

/** Things that happened while starting that the user should know about, for example a damaged database that was replaced. */
export function StartupNotices() {
  const backend = useBackend();
  const [info] = useAsync(() => backend.getAppInfo(), [backend]);
  if (info.status !== "ready") return null;
  const notices = [...(info.data.startup_notices ?? []), ...info.data.rejected_manifests.map((m) => `A catalog file was rejected and is not shown: ${m}`)];
  if (notices.length === 0) return null;
  return (
    <div role="alert" aria-label="Start-up notices" className="border-b border-warning/40 bg-warning/10 px-4 py-2 text-sm text-warning">
      <p className="flex items-center gap-2 font-medium">
        <TriangleAlert size={14} aria-hidden /> Vixalune started with problems
      </p>
      <ul className="mt-1 list-disc space-y-0.5 pl-6">
        {notices.map((n) => (
          <li key={n} className="break-words">
            {n}
          </li>
        ))}
      </ul>
    </div>
  );
}
