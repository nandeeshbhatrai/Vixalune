import { Badge } from "@/components/ui/badge";
import { useBackend } from "@/infra/backendContext";

/** Marks a value as demonstration data when the demo backend is active; renders nothing otherwise. */
export function DemoTag() {
  const { isDemo } = useBackend();
  return isDemo ? <Badge tone="warning">Demo data</Badge> : null;
}
