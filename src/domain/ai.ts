import type { AiConnectionState, AiProviderView, AiUsage } from "./types";

/** Cost exactly as the provider reported it; "not reported" otherwise. Vixalune never estimates a price. */
export function formatCost(usage: AiUsage | null | undefined): string {
  if (!usage || usage.cost_usd == null) return "cost not reported by the provider";
  const c = usage.cost_usd;
  return `$${c < 0.01 ? c.toFixed(6) : c.toFixed(4)} (reported by the provider)`;
}

export function formatTokens(usage: AiUsage | null | undefined): string | null {
  if (!usage) return null;
  const parts: string[] = [];
  if (usage.input_tokens != null) parts.push(`${usage.input_tokens} in`);
  if (usage.output_tokens != null) parts.push(`${usage.output_tokens} out`);
  if (!parts.length && usage.total_tokens != null) parts.push(`${usage.total_tokens} total`);
  return parts.length ? `${parts.join(" / ")} tokens` : null;
}

export function connectionLabel(state: AiConnectionState): string {
  switch (state) {
    case "verified":
      return "Verified";
    case "key_accepted":
      return "Key accepted, model not verified";
    case "failed":
      return "Failed";
    default:
      return "Not tested";
  }
}

/** Models the user may pick: those the provider listed or a test request verified. Nothing else is offered. */
export function usableModels(p: AiProviderView, listed: string[]): string[] {
  return Array.from(new Set([...listed, ...p.status.verified_models])).sort();
}
