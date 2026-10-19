import { BookOpen, ExternalLink, ShieldAlert } from "lucide-react";
import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { ErrorNotice } from "@/components/ErrorNotice";
import { Button } from "@/components/ui/button";
import type { AssistantActionOutcome, AssistantEvidence, AssistantRecommendation, AssistantTurn, CardAction, PrerequisiteReport } from "@/domain/types";
import { useAction } from "@/hooks/useAction";
import { useBackend } from "@/infra/backendContext";

const ACTION_LABEL: Record<CardAction["kind"], string> = { view_details: "View Details", check_prerequisites: "Check Prerequisites", install: "Install…", prepare_run: "Prepare Scan" };

function Tag({ children, tone = "plain" }: { children: React.ReactNode; tone?: "plain" | "model" | "verified" }) {
  const cls = tone === "model" ? "border-amber-500/50 text-amber-700 dark:text-amber-300" : tone === "verified" ? "border-emerald-600/50 text-emerald-700 dark:text-emerald-300" : "border-border text-muted-foreground";
  return <span className={`rounded border px-1.5 py-0.5 text-[11px] ${cls}`}>{children}</span>;
}

function EvidenceLink({ conversationId, messageId, e }: { conversationId: string; messageId: string; e: AssistantEvidence }) {
  const backend = useBackend();
  const { run } = useAction();
  if (!e.url) return <span title={e.detail ?? undefined}>{e.id}: {e.title}</span>;
  return (
    <button type="button" className="inline-flex items-center gap-1 text-primary hover:underline" onClick={() => void run(() => backend.assistantOpenEvidence(conversationId, messageId, e.id))}>
      {e.id}: {e.title}
      <ExternalLink size={11} aria-hidden />
      <span className="sr-only">(opens in your browser)</span>
    </button>
  );
}

function Prerequisites({ report }: { report: PrerequisiteReport }) {
  return (
    <div role="status" aria-label={`Prerequisites for ${report.tool_name}`} className="mt-2 rounded-md border border-border p-2 text-xs">
      <p className="font-medium">{report.ready ? `${report.tool_name} can be installed on this computer.` : report.unsupported_reason ? `${report.tool_name} cannot be installed here: ${report.unsupported_reason}` : `${report.tool_name} is missing: ${report.missing.join(", ")}`}</p>
      <ul className="mt-1 list-disc pl-4">
        {report.items.map((i) => (
          <li key={i.name}>
            {i.name}: {i.satisfied === true ? "present" : i.satisfied === false ? "missing" : "could not be checked"} — {i.reason}
            {i.requires_admin ? " (needs administrator rights)" : ""}
            {i.requires_restart ? " (needs a restart)" : ""}
          </li>
        ))}
        {report.items.length === 0 ? <li>No prerequisites.</li> : null}
      </ul>
      <p className="mt-1 text-muted-foreground">Administrator rights to install: {report.administrator_rights}. This only checked; nothing was changed.</p>
    </div>
  );
}

function RecommendationCard({ r, conversationId, messageId, turn }: { r: AssistantRecommendation; conversationId: string; messageId: string; turn: AssistantTurn }) {
  const backend = useBackend();
  const navigate = useNavigate();
  const { busy, error, run } = useAction();
  const [outcome, setOutcome] = useState<AssistantActionOutcome | null>(null);
  const docEvidence = turn.evidence.find((e) => r.evidence.includes(e.id) && e.kind === "catalog_record");
  const act = async (a: CardAction) => {
    if (a.kind === "view_details" || a.kind === "install") navigate(`/catalog/${r.tool_id}`);
    else if (a.kind === "prepare_run") navigate(`/run/${r.tool_id}${a.profile_id ? `?profile=${encodeURIComponent(a.profile_id)}` : ""}`);
    else {
      const o = await run(() => backend.assistantRunAction({ action: "check_prerequisites", tool_id: r.tool_id }));
      if (o) setOutcome(o);
    }
  };
  return (
    <article aria-label={`Recommendation: ${r.tool_name}`} className="rounded-lg border border-border p-3">
      <div className="flex flex-wrap items-center gap-2">
        <h3 className="text-base font-semibold">{r.tool_name}</h3>
        <Tag>{r.role === "primary" ? "Recommended first" : "Alternative"}</Tag>
        <Tag tone="verified">{r.status.label}{r.status.version ? ` · ${r.status.version}` : ""}</Tag>
        <Tag>License {r.license}</Tag>
      </div>
      <p className="mt-1 text-muted-foreground">{r.summary}</p>
      {r.reasons.length ? (
        <div className="mt-2">
          <p className="text-xs font-medium">Why <Tag tone="model">written by the AI model</Tag></p>
          <ul className="list-disc pl-5">{r.reasons.map((x, i) => <li key={i}>{x}</li>)}</ul>
        </div>
      ) : null}
      <div className="mt-2">
        <p className="text-xs font-medium">Prerequisites <Tag tone="verified">from the Vixalune catalog</Tag></p>
        <ul className="list-disc pl-5">
          {r.verified_prerequisites.map((p) => <li key={p.name}>{p.name}: {p.reason}{p.requires_admin ? " (needs administrator rights)" : ""}{p.requires_restart ? " (may need a restart)" : ""}</li>)}
          {r.verified_prerequisites.length === 0 ? <li>None beyond Windows itself.</li> : null}
        </ul>
        {r.model_prerequisites.length ? <ul className="mt-1 list-disc pl-5 text-muted-foreground">{r.model_prerequisites.map((x, i) => <li key={i}>{x} <Tag tone="model">AI</Tag></li>)}</ul> : null}
      </div>
      {r.verified_limitations.length || r.model_limitations.length ? (
        <div className="mt-2">
          <p className="text-xs font-medium">Limitations</p>
          <ul className="list-disc pl-5">
            {r.verified_limitations.map((x, i) => <li key={`v${i}`}>{x}</li>)}
            {r.model_limitations.map((x, i) => <li key={`m${i}`} className="text-muted-foreground">{x} <Tag tone="model">AI</Tag></li>)}
          </ul>
        </div>
      ) : null}
      <p className="mt-2 flex items-center gap-2 text-xs text-muted-foreground">
        <BookOpen size={12} aria-hidden />
        {docEvidence ? <EvidenceLink conversationId={conversationId} messageId={messageId} e={{ ...docEvidence, title: "Official documentation" }} /> : "Documentation: see View Details"}
        <span>· catalog facts verified {r.provenance.verified_on} ({r.provenance.publisher})</span>
      </p>
      <div className="mt-3 flex flex-wrap gap-2">
        {r.actions.map((a) => (
          <span key={a.kind} title={a.enabled ? undefined : (a.reason ?? undefined)}>
            <Button variant={a.kind === "prepare_run" || a.kind === "install" ? "default" : "outline"} size="sm" disabled={!a.enabled || busy} onClick={() => void act(a)} aria-label={`${ACTION_LABEL[a.kind]} — ${r.tool_name}`}>
              {ACTION_LABEL[a.kind]}
            </Button>
          </span>
        ))}
      </div>
      {r.actions.filter((a) => !a.enabled && a.reason).length ? <p className="mt-1 text-xs text-muted-foreground">{r.actions.filter((a) => !a.enabled && a.reason).map((a) => `${ACTION_LABEL[a.kind]}: ${a.reason}`).join(" ")}</p> : null}
      {outcome?.kind === "prerequisites" ? <Prerequisites report={outcome.report} /> : null}
      {error ? <div className="mt-2"><ErrorNotice error={error} /></div> : null}
    </article>
  );
}

function Vulnerabilities({ turn, conversationId, messageId, onConsent }: { turn: AssistantTurn; conversationId: string; messageId: string; onConsent?: () => void }) {
  const v = turn.vulnerability;
  if (!v) return null;
  const evFor = (id: string) => turn.evidence.find((e) => e.title.endsWith(` ${id}`) || e.title === id);
  return (
    <section aria-label="Vulnerability findings" className="rounded-lg border border-border p-3">
      <h3 className="text-base font-semibold">
        {v.package ? `Known vulnerabilities: ${v.package.name} ${v.package.version} (${v.package.ecosystem})` : "Findings"} <Tag tone="verified">from {v.lookup?.source ?? "scanner output"}, not written by the model</Tag>
      </h3>
      {v.lookup ? (
        <>
          <p className="text-xs text-muted-foreground">Asked {v.lookup.source} at {v.lookup.queried_at}. {v.lookup.records.length} record(s){v.lookup.truncated ? " (more exist)" : ""}.</p>
          {v.lookup.records.length === 0 ? <p className="mt-2 font-medium">No known vulnerability was found in this database. That is not proof that the package is safe.</p> : null}
          <ul className="mt-2 space-y-2">
            {v.lookup.records.map((r) => {
              const e = evFor(r.id);
              return (
                <li key={r.id} className="rounded border border-border p-2">
                  <p className="font-medium">
                    {e ? <EvidenceLink conversationId={conversationId} messageId={messageId} e={{ ...e, title: r.id }} /> : r.id}
                    {r.aliases.length ? <span className="font-normal text-muted-foreground"> ({r.aliases.join(", ")})</span> : null}
                  </p>
                  <p>{r.summary}</p>
                  <dl className="mt-1 grid gap-x-4 text-xs text-muted-foreground sm:grid-cols-2">
                    <div>Fixed in: {r.fixed_versions.length ? r.fixed_versions.join(", ") : "no fixed version listed"}</div>
                    <div>Published: {r.published ?? "not stated"} · Modified: {r.modified ?? "not stated"}</div>
                    <div className="sm:col-span-2">Affected: {r.affected.length ? r.affected.map((a) => `${a.introduced ?? "?"} → ${a.fixed ? `before ${a.fixed}` : a.last_affected ? `through ${a.last_affected}` : "no end listed"}`).join("; ") : "not stated"}</div>
                    {r.severity.length ? <div className="sm:col-span-2">Severity: {r.severity.map((s) => `${s.kind} ${s.score}`).join("; ")}</div> : null}
                  </dl>
                  <p className="mt-1"><Tag>database match</Tag></p>
                </li>
              );
            })}
          </ul>
        </>
      ) : null}
      {v.scanner_ids.length ? (
        <div className="mt-2">
          <p className="font-medium">Identifiers in the scanner output <Tag>scanner finding</Tag></p>
          <p className="text-xs text-muted-foreground">{v.scanner_note}</p>
          <p>{v.scanner_ids.join(", ")}</p>
        </div>
      ) : null}
      <ul className="mt-2 list-disc pl-5 text-xs text-muted-foreground">
        {[...v.classes, ...v.not_assessed, ...(v.lookup?.coverage_notes ?? [])].map((c, i) => <li key={i}>{c}</li>)}
      </ul>
      {onConsent ? null : null}
    </section>
  );
}

/** A validated answer of the tool-aware assistant. Model-written text is labelled; facts come from the application's records. */
export function TurnView({ turn, conversationId, messageId, onAllowLookups }: { turn: AssistantTurn; conversationId: string; messageId: string; onAllowLookups?: () => void }) {
  return (
    <div className="space-y-3">
      {turn.explanation ? (
        <section aria-label={turn.model_generated ? "Model-generated explanation" : "Explanation"}>
          {turn.model_generated ? <p className="mb-1"><Tag tone="model">Explanation written by the AI model — not a verified finding</Tag></p> : null}
          <p className="whitespace-pre-wrap">{turn.explanation}</p>
        </section>
      ) : null}
      {turn.clarifications.length ? (
        <section aria-label="Questions" className="rounded-md border border-border bg-muted p-3">
          <p className="font-medium">I need to know:</p>
          <ul className="list-disc pl-5">{turn.clarifications.map((c, i) => <li key={i}>{c.question} <span className="text-muted-foreground">— {c.why}</span></li>)}</ul>
        </section>
      ) : null}
      {turn.consent_request ? (
        <section aria-label="Permission needed" className="rounded-md border border-amber-500/50 p-3">
          <p className="flex items-center gap-2 font-medium"><ShieldAlert size={14} aria-hidden /> Permission needed</p>
          <p>{turn.consent_request.text}</p>
          {onAllowLookups ? <Button className="mt-2" size="sm" onClick={onAllowLookups}>Allow lookups and ask again</Button> : null}
        </section>
      ) : null}
      {turn.recommendations.map((r) => <RecommendationCard key={r.tool_id} r={r} conversationId={conversationId} messageId={messageId} turn={turn} />)}
      {turn.no_suitable_tool ? (
        <section aria-label="No suitable catalog tool" className="rounded-md border border-border p-3">
          <p className="font-medium">No tool in the Vixalune catalog fits this.</p>
          <p>{turn.no_suitable_tool.explanation}</p>
          {turn.no_suitable_tool.documented_alternatives.length ? (
            <>
              <p className="mt-1 text-xs font-medium">Documented elsewhere (not part of Vixalune, cannot be installed or run here):</p>
              <ul className="list-disc pl-5">{turn.no_suitable_tool.documented_alternatives.map((a) => <li key={a.name}>{a.name}: {a.why}</li>)}</ul>
            </>
          ) : null}
        </section>
      ) : null}
      <Vulnerabilities turn={turn} conversationId={conversationId} messageId={messageId} />
      {turn.finding_notes.length ? (
        <section aria-label="Explanations of findings">
          <p className="text-xs font-medium">Explanations <Tag tone="model">written by the AI model</Tag> (only for identifiers in the data above)</p>
          <ul className="list-disc pl-5">
            {turn.finding_notes.map((f) => (
              <li key={f.vulnerability_id}>
                <strong>{f.vulnerability_id}</strong>: {f.explanation}{" "}
                {f.evidence.map((id) => turn.evidence.find((e) => e.id === id)).filter((e): e is AssistantEvidence => !!e).map((e) => <span key={e.id} className="mr-1 text-xs"><EvidenceLink conversationId={conversationId} messageId={messageId} e={e} /></span>)}
              </li>
            ))}
          </ul>
        </section>
      ) : null}
      {turn.next_steps.length ? (
        <section aria-label="Suggested next steps">
          <p className="text-xs font-medium">Next steps <Tag tone="model">suggested by the AI model</Tag></p>
          <ul className="list-disc pl-5">{turn.next_steps.map((s, i) => <li key={i}>{s}</li>)}</ul>
        </section>
      ) : null}
      {turn.proposed_actions.length ? (
        <section aria-label="Suggested actions">
          <p className="text-xs font-medium">Actions the assistant suggested (none has run; each needs you)</p>
          <ul className="list-disc pl-5 text-muted-foreground">{turn.proposed_actions.map((a, i) => <li key={i}>{a.summary}</li>)}</ul>
        </section>
      ) : null}
      {turn.limitations.length ? <ul aria-label="Limitations" className="list-disc pl-5 text-xs text-muted-foreground">{turn.limitations.map((l, i) => <li key={i}>{l}</li>)}</ul> : null}
      {turn.warnings.length ? (
        <section aria-label="Removed or refused" className="rounded-md border border-amber-500/50 p-2 text-xs">
          <p className="font-medium">Vixalune removed or refused some of what the model said:</p>
          <ul className="list-disc pl-5">{turn.warnings.map((w, i) => <li key={i}>{w}</li>)}</ul>
        </section>
      ) : null}
      {turn.evidence.length ? (
        <section aria-label="Sources" className="text-xs">
          <p className="font-medium">Sources</p>
          <ul className="list-disc pl-5">{turn.evidence.map((e) => <li key={e.id}><EvidenceLink conversationId={conversationId} messageId={messageId} e={e} />{e.as_of ? <span className="text-muted-foreground"> · as of {e.as_of}</span> : null}</li>)}</ul>
        </section>
      ) : null}
    </div>
  );
}
