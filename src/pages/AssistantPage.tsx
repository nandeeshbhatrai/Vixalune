import { Bot, Copy, FileSearch, MessageSquarePlus, RotateCcw, Square, Trash2 } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { Markdown } from "@/components/ai/Markdown";
import { TurnView } from "@/components/ai/TurnView";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { EmptyState } from "@/components/EmptyState";
import { ErrorNotice } from "@/components/ErrorNotice";
import { LoadingState } from "@/components/AsyncView";
import { PageHeader } from "@/components/PageHeader";
import { fieldClass } from "@/components/run/ParamFields";
import { Button } from "@/components/ui/button";
import { formatCost, formatTokens, usableModels } from "@/domain/ai";
import type { AiConversation, AiErrorInfo, AiMessage, AiOverview, AiProviderView, AiStreamEvent, AssistantTurn, RunRecord } from "@/domain/types";
import { toAppError } from "@/hooks/useAsync";
import { useBackend } from "@/infra/backendContext";

interface Live {
  requestId: string;
  messageId: string;
  text: string;
  retrying: string | null;
}

export function AssistantPage() {
  const backend = useBackend();
  const [overview, setOverview] = useState<AiOverview | null>(null);
  const [loadError, setLoadError] = useState<AiErrorInfo | null>(null);
  const [conversations, setConversations] = useState<AiConversation[]>([]);
  const [currentId, setCurrentId] = useState<string | null>(null);
  const [messages, setMessages] = useState<AiMessage[]>([]);
  const [provider, setProvider] = useState("");
  const [model, setModel] = useState("");
  const [models, setModels] = useState<string[]>([]);
  const [draft, setDraft] = useState("");
  const [live, setLive] = useState<Live | null>(null);
  const [busyAsk, setBusyAsk] = useState(false);
  const [error, setError] = useState<AiErrorInfo | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  const [useCatalog, setUseCatalog] = useState(true);
  const [turns, setTurns] = useState<Record<string, AssistantTurn>>({});
  const [explainOpen, setExplainOpen] = useState(false);
  const [runs, setRuns] = useState<RunRecord[]>([]);
  const [runId, setRunId] = useState("");
  const finishedRef = useRef<Set<string>>(new Set());
  const currentRef = useRef<string | null>(null);
  const liveRef = useRef<Live | null>(null);
  currentRef.current = currentId;
  liveRef.current = live;

  const fail = (e: unknown) => setError(toAppError(e) as unknown as AiErrorInfo);

  const loadConversation = useCallback(
    async (id: string) => {
      const d = await backend.aiGetConversation(id);
      setCurrentId(id);
      setMessages(d.messages);
      try {
        const entries = await backend.assistantTurns(id);
        setTurns(Object.fromEntries(entries.map((e) => [e.message_id, e.turn])));
      } catch {
        setTurns({});
      }
    },
    [backend],
  );

  const refreshList = useCallback(async () => {
    setConversations(await backend.aiListConversations());
  }, [backend]);

  useEffect(() => {
    let alive = true;
    (async () => {
      try {
        const o = await backend.aiOverview();
        if (!alive) return;
        setOverview(o);
        const active = o.providers.find((p) => p.active) ?? o.providers.find((p) => p.has_key);
        if (active) {
          setProvider(active.descriptor.id);
          setModel(active.config.default_model ?? "");
        }
        await refreshList();
      } catch (e) {
        if (alive) setLoadError(toAppError(e) as unknown as AiErrorInfo);
      }
    })();
    return () => {
      alive = false;
    };
  }, [backend, refreshList]);

  // Models offered for the chosen provider: listed or verified only.
  useEffect(() => {
    const p = overview?.providers.find((x) => x.descriptor.id === provider);
    if (!p?.has_key) {
      setModels([]);
      return;
    }
    let alive = true;
    backend.aiListModels(provider).then(
      (l) => alive && setModels(usableModels(p, l.models.filter((m) => m.source === "listed").map((m) => m.id))),
      () => alive && setModels(usableModels(p, [])),
    );
    return () => {
      alive = false;
    };
  }, [backend, overview, provider]);

  // Streamed answers.
  useEffect(() => {
    let off: (() => void) | undefined;
    let cancelled = false;
    void backend.onAiEvent((ev: AiStreamEvent) => {
      if (ev.conversation_id !== currentRef.current) {
        if (ev.event === "finished") void refreshList();
        return;
      }
      if (ev.event === "started") setLive({ requestId: ev.request_id, messageId: ev.message_id, text: "", retrying: null });
      else if (ev.event === "delta") setLive((l) => (l && l.requestId === ev.request_id ? { ...l, text: l.text + ev.text, retrying: null } : l));
      else if (ev.event === "retrying") setLive((l) => (l && l.requestId === ev.request_id ? { ...l, retrying: `Retrying in ${ev.wait_secs}s (attempt ${ev.attempt + 1}): ${ev.reason}` } : l));
      else if (ev.event === "finished") {
        finishedRef.current.add(ev.request_id);
        setLive(null);
        void loadConversation(ev.conversation_id);
        void refreshList();
      }
    }).then((u) => {
      if (cancelled) u();
      else off = u;
    });
    return () => {
      cancelled = true;
      off?.();
    };
  }, [backend, loadConversation, refreshList]);

  const providerView: AiProviderView | undefined = overview?.providers.find((p) => p.descriptor.id === provider);
  const streaming = live !== null || busyAsk;

  const newConversation = async () => {
    setError(null);
    try {
      const c = await backend.aiCreateConversation(provider || null, model || null);
      setCurrentId(c.id);
      setMessages([]);
      await refreshList();
      return c;
    } catch (e) {
      fail(e);
    }
  };

  const send = async () => {
    const text = draft.trim();
    if (!text || streaming) return;
    setError(null);
    const conv = currentId ? { id: currentId } : await newConversation();
    if (!conv) return;
    try {
      const input = { conversation_id: conv.id, text, provider, model };
      currentRef.current = conv.id;
      setBusyAsk(true);
      const ack = await (useCatalog ? backend.assistantAsk(input) : backend.aiSendMessage(input));
      setDraft("");
      if (finishedRef.current.has(ack.request_id)) {
        await loadConversation(conv.id);
        await refreshList();
      } else {
        setMessages((m) => [...m, ack.user_message]);
        setLive({ requestId: ack.request_id, messageId: ack.assistant_message_id, text: "", retrying: null });
      }
    } catch (e) {
      fail(e);
    } finally {
      setBusyAsk(false);
    }
  };

  const retry = async () => {
    if (!currentId) return;
    setError(null);
    try {
      const ack = await (useCatalog ? backend.assistantRetry(currentId, provider, model) : backend.aiRetry(currentId, provider, model));
      await loadConversation(currentId);
      if (!finishedRef.current.has(ack.request_id)) setLive({ requestId: ack.request_id, messageId: ack.assistant_message_id, text: "", retrying: null });
    } catch (e) {
      fail(e);
    }
  };

  const allowLookupsAndAsk = async (questionText: string) => {
    setError(null);
    try {
      const o = await backend.aiOverview();
      await backend.aiUpdateSettings({ ...o.settings, allow_vulnerability_lookup: true });
      setOverview(await backend.aiOverview());
      setDraft(questionText);
    } catch (e) {
      fail(e);
    }
  };

  const openExplain = async () => {
    setError(null);
    try {
      const all = await backend.listRuns(30);
      const done = all.filter((r) => r.outcome !== null);
      setRuns(done);
      setRunId(done[0]?.id ?? "");
      setExplainOpen(true);
    } catch (e) {
      fail(e);
    }
  };

  const explain = async () => {
    if (!runId) return;
    setExplainOpen(false);
    setError(null);
    try {
      const conv = currentId ? { id: currentId } : await newConversation();
      if (!conv) return;
      currentRef.current = conv.id;
      setBusyAsk(true);
      const ack = await backend.assistantExplainRun(conv.id, runId, true, provider, model);
      if (finishedRef.current.has(ack.request_id)) await loadConversation(conv.id);
      else {
        await loadConversation(conv.id);
        setLive({ requestId: ack.request_id, messageId: ack.assistant_message_id, text: "", retrying: null });
      }
      await refreshList();
    } catch (e) {
      fail(e);
    } finally {
      setBusyAsk(false);
    }
  };

  const stop = async () => {
    if (!live) return;
    try {
      await backend.aiCancel(live.requestId);
    } catch (e) {
      fail(e);
    }
  };

  const remove = async (id: string) => {
    try {
      await backend.aiDeleteConversation(id);
      setConfirmDelete(null);
      if (id === currentId) {
        setCurrentId(null);
        setMessages([]);
      }
      await refreshList();
    } catch (e) {
      setConfirmDelete(null);
      fail(e);
    }
  };

  const copy = (text: string) => void navigator.clipboard?.writeText(text).catch(() => undefined);

  const assistantBody = (m: AiMessage) => {
    const turn = turns[m.id];
    if (!turn) return <Markdown text={m.content} />;
    const q = messages[messages.findIndex((x) => x.id === m.id) - 1]?.content ?? "";
    return <TurnView turn={turn} conversationId={m.conversation_id} messageId={m.id} onAllowLookups={turn.consent_request ? () => void allowLookupsAndAsk(q) : undefined} />;
  };

  if (loadError) return <ErrorNotice error={loadError} onRetry={() => window.location.reload()} />;
  if (!overview) return <LoadingState label="Loading the assistant" />;

  const configured = overview.providers.filter((p) => p.has_key);
  const last = messages[messages.length - 1];
  const canRetry = !streaming && last && last.role === "assistant" && last.status !== "complete";
  const canSend = !!draft.trim() && !!provider && !!model && !streaming;

  return (
    <div className="flex h-full flex-col">
      <PageHeader title="AI Assistant" subtitle="Ask about security concepts and tool output. The assistant can explain; it cannot run anything." />
      <p role="note" className="mb-3 rounded-md border border-border bg-muted p-2 text-xs">
        What you type here is sent to {providerView?.descriptor.display_name ?? "your chosen provider"} over the internet, and the provider may bill your account. Do not paste secrets, credentials or data you are not allowed to share.
        Vixalune adds nothing else: no keys, environment variables, files or command output.
      </p>
      {configured.length === 0 ? (
        <EmptyState icon={Bot} title="No AI provider is set up">
          Add a provider and an API key in <Link className="text-primary underline" to="/settings/ai">Settings → AI Providers</Link> to start.
        </EmptyState>
      ) : (
        <div className="grid min-h-0 flex-1 gap-4 md:grid-cols-[14rem_1fr]">
          <aside aria-label="Conversations" className="space-y-2">
            <Button className="w-full" onClick={() => void newConversation()} disabled={streaming}>
              <MessageSquarePlus size={14} aria-hidden /> New conversation
            </Button>
            {conversations.length === 0 ? <p className="text-xs text-muted-foreground">No conversations yet.</p> : null}
            <ul className="space-y-1">
              {conversations.map((c) => (
                <li key={c.id} className="flex items-center gap-1">
                  <button type="button" onClick={() => void loadConversation(c.id)} disabled={streaming} aria-current={c.id === currentId} className={`min-w-0 flex-1 truncate rounded px-2 py-1 text-left hover:bg-muted ${c.id === currentId ? "bg-muted font-medium" : ""}`}>
                    {c.title}
                  </button>
                  <button type="button" aria-label={`Delete conversation ${c.title}`} onClick={() => setConfirmDelete(c.id)} disabled={streaming && c.id === currentId} className="rounded p-1 hover:bg-muted">
                    <Trash2 size={13} aria-hidden />
                  </button>
                </li>
              ))}
            </ul>
          </aside>

          <section aria-label="Conversation" className="flex min-h-0 flex-col">
            <div className="mb-2 flex flex-wrap gap-2">
              <div>
                <label htmlFor="as-provider" className="sr-only">Provider</label>
                <select id="as-provider" className={fieldClass} value={provider} disabled={streaming} onChange={(e) => { setProvider(e.target.value); setModel(overview.providers.find((p) => p.descriptor.id === e.target.value)?.config.default_model ?? ""); }}>
                  {configured.map((p) => (
                    <option key={p.descriptor.id} value={p.descriptor.id}>{p.descriptor.display_name}</option>
                  ))}
                </select>
              </div>
              <div>
                <label htmlFor="as-model" className="sr-only">Model</label>
                <select id="as-model" className={fieldClass} value={model} disabled={streaming} onChange={(e) => setModel(e.target.value)}>
                  <option value="">Choose a model</option>
                  {models.map((m) => (
                    <option key={m} value={m}>{m}</option>
                  ))}
                </select>
              </div>
              <label className="flex items-center gap-1.5 self-center text-xs">
                <input type="checkbox" checked={useCatalog} disabled={streaming} onChange={(e) => setUseCatalog(e.target.checked)} />
                Use the tool catalog (recommended)
              </label>
              {useCatalog ? (
                <Button variant="outline" size="sm" disabled={streaming} onClick={() => void openExplain()}>
                  <FileSearch size={13} aria-hidden /> Explain a scan run…
                </Button>
              ) : null}
              {models.length === 0 ? <p className="self-center text-xs text-muted-foreground">No verified model yet — <Link className="text-primary underline" to="/settings/ai">test the connection</Link>.</p> : null}
            </div>

            <div className="min-h-[12rem] flex-1 space-y-3 overflow-y-auto rounded-lg border border-border p-3" aria-live="polite" aria-label="Messages">
              {messages.length === 0 && !live ? <p className="text-muted-foreground">Ask a question to begin.</p> : null}
              {messages.map((m) => (
                <article key={m.id} aria-label={m.role === "user" ? "You" : "Assistant"} className={m.role === "user" ? "ml-8 rounded-md bg-muted p-3" : "mr-8 rounded-md border border-border p-3"}>
                  <p className="mb-1 text-xs font-semibold text-muted-foreground">{m.role === "user" ? "You" : `Assistant${m.model ? ` · ${m.model}` : ""}`}</p>
                  {m.role === "user" ? <p className="whitespace-pre-wrap break-words">{m.content}</p> : m.id === live?.messageId ? null : assistantBody(m)}
                  {m.role === "assistant" && m.id !== live?.messageId ? (
                    <div className="mt-2 flex flex-wrap items-center gap-3 text-xs text-muted-foreground">
                      {m.status !== "complete" ? <span className="font-medium text-destructive">{m.status === "cancelled" ? "Stopped" : m.status === "interrupted" ? "Interrupted — partial answer" : "Failed"}{m.error_message ? `: ${m.error_message}` : ""}</span> : null}
                      {formatTokens(m.usage) ? <span>{formatTokens(m.usage)}</span> : null}
                      <span>{formatCost(m.usage)}</span>
                      {m.content && !turns[m.id] ? <button type="button" className="inline-flex items-center gap-1 hover:underline" onClick={() => copy(m.content)}><Copy size={12} aria-hidden /> Copy response</button> : null}
                    </div>
                  ) : null}
                </article>
              ))}
              {live ? (
                <article aria-label="Assistant" className="mr-8 rounded-md border border-border p-3">
                  <p className="mb-1 text-xs font-semibold text-muted-foreground">Assistant · {model}</p>
                  {live.text ? <Markdown text={live.text} /> : <p role="status" className="text-muted-foreground">{live.retrying ?? (useCatalog ? "Checking the catalog and drafting an answer… the reply is checked before it is shown." : "Waiting for the provider…")}</p>}
                </article>
              ) : null}
            </div>

            {error ? <div className="mt-2"><ErrorNotice error={error} /></div> : null}

            <div className="mt-2 flex items-end gap-2">
              <div className="flex-1">
                <label htmlFor="as-draft" className="sr-only">Message</label>
                <textarea id="as-draft" rows={3} className="w-full rounded-md border border-border bg-card p-2 text-sm" placeholder="Ask a question…" value={draft} maxLength={50000} onChange={(e) => setDraft(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) { e.preventDefault(); void send(); } }} />
              </div>
              {streaming ? (
                <Button variant="destructive" onClick={() => void stop()}><Square size={14} aria-hidden /> Stop generating</Button>
              ) : (
                <>
                  {canRetry ? <Button variant="outline" onClick={() => void retry()}><RotateCcw size={14} aria-hidden /> Retry</Button> : null}
                  <Button onClick={() => void send()} disabled={!canSend}>Send</Button>
                </>
              )}
            </div>
            <p className="mt-1 text-xs text-muted-foreground">Ctrl+Enter sends. Costs shown are only what the provider reports; Vixalune never estimates a price.</p>
          </section>
        </div>
      )}
      <ConfirmDialog
        open={explainOpen}
        onOpenChange={setExplainOpen}
        title="Explain a scan run"
        description={`Part of the stored output of the run you choose (at most 400 lines, with lines that look like credentials withheld) will be sent to ${providerView?.descriptor.display_name ?? "your AI provider"} over the internet, and the provider may bill you. The output is treated as untrusted text. It is not saved in the conversation.`}
        confirmLabel="Send and explain"
        confirmDisabled={!runId}
        onConfirm={() => void explain()}
      >
        {runs.length === 0 ? (
          <p className="text-sm text-muted-foreground">There are no finished runs yet. Run a tool first.</p>
        ) : (
          <div>
            <label htmlFor="explain-run" className="mb-1 block text-sm font-medium">Run</label>
            <select id="explain-run" className={fieldClass} value={runId} onChange={(e) => setRunId(e.target.value)}>
              {runs.map((r) => (
                <option key={r.id} value={r.id}>{r.tool_name} — {r.profile_label} ({r.started_at})</option>
              ))}
            </select>
          </div>
        )}
      </ConfirmDialog>
      <ConfirmDialog open={confirmDelete !== null} onOpenChange={(o) => !o && setConfirmDelete(null)} title="Delete this conversation?" description="It is removed from this computer. It cannot be undone." confirmLabel="Delete" destructive onConfirm={() => confirmDelete && void remove(confirmDelete)} />
    </div>
  );
}
