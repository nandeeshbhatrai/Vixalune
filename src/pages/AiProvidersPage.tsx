import { useEffect, useState } from "react";
import { AsyncView } from "@/components/AsyncView";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { ErrorNotice } from "@/components/ErrorNotice";
import { PageHeader } from "@/components/PageHeader";
import { fieldClass } from "@/components/run/ParamFields";
import { Button } from "@/components/ui/button";
import { Card, CardTitle } from "@/components/ui/card";
import { connectionLabel, usableModels } from "@/domain/ai";
import type { AiModelList, AiOverview, AiProviderView, AiTestReport } from "@/domain/types";
import { useAction } from "@/hooks/useAction";
import { useAsync } from "@/hooks/useAsync";
import { useBackend } from "@/infra/backendContext";

function ProviderPanel({ view, onChanged }: { view: AiProviderView; onChanged: () => void }) {
  const backend = useBackend();
  const { busy, error, run, clearError } = useAction();
  const [apiKey, setApiKey] = useState("");
  const [label, setLabel] = useState(view.config.label ?? "");
  const [models, setModels] = useState<AiModelList | null>(null);
  const [model, setModel] = useState(view.config.default_model ?? "");
  const [manual, setManual] = useState("");
  const [timeout, setTimeoutSecs] = useState(String(view.config.timeout_secs));
  const [temperature, setTemperature] = useState(view.config.temperature == null ? "" : String(view.config.temperature));
  const [maxTokens, setMaxTokens] = useState(view.config.max_output_tokens == null ? "" : String(view.config.max_output_tokens));
  const [fallbacks, setFallbacks] = useState(view.config.extra.allow_upstream_fallbacks === "true");
  const [report, setReport] = useState<AiTestReport | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const id = view.descriptor.id;
  const caps = view.descriptor.capabilities;

  useEffect(() => {
    setLabel(view.config.label ?? "");
    setModel(view.config.default_model ?? "");
    setReport(null);
    setModels(null);
    setNotice(null);
    if (view.has_key) void backend.aiListModels(id).then(setModels, () => undefined);
  }, [id, view.has_key, backend]); // eslint-disable-line react-hooks/exhaustive-deps

  const listed = models?.models.filter((m) => m.source === "listed").map((m) => m.id) ?? [];
  const choices = usableModels(view, listed);

  const save = async () => {
    setNotice(null);
    const r = await run(() => backend.aiSaveCredential(id, apiKey, label.trim() || null));
    if (r) {
      setApiKey(""); // the key is cleared from the screen as soon as it is stored
      setNotice("Key saved in Windows Credential Manager. Test the connection to confirm it works.");
      onChanged();
    }
  };

  const saveConfig = async () => {
    setNotice(null);
    const extra = { ...view.config.extra };
    if (id === "openrouter") extra.allow_upstream_fallbacks = fallbacks ? "true" : "false";
    const r = await run(() =>
      backend.aiUpdateConfig({
        ...view.config,
        label: label.trim() || null,
        default_model: model || null,
        timeout_secs: Number(timeout),
        temperature: temperature === "" ? null : Number(temperature),
        max_output_tokens: maxTokens === "" ? null : Number(maxTokens),
        extra,
      }),
    );
    if (r) {
      setNotice("Settings saved.");
      onChanged();
    }
  };

  const test = async (m: string | null) => {
    setReport(null);
    const r = await run(() => backend.aiTestConnection(id, m));
    if (r) {
      setReport(r);
      if (r.verified_model) setModel(r.verified_model);
      setModels(await backend.aiListModels(id).catch(() => null));
      onChanged();
    }
  };

  const remove = async () => {
    const r = await run(() => backend.aiDeleteCredential(id));
    if (r) {
      setConfirmDelete(false);
      setNotice("Key removed from Windows Credential Manager.");
      onChanged();
    }
  };

  return (
    <Card aria-label={view.descriptor.display_name}>
      <div className="mb-3 flex flex-wrap items-center gap-2">
        <h2 className="text-base font-semibold">{view.descriptor.display_name}</h2>
        {view.active ? <span className="rounded bg-primary px-2 py-0.5 text-xs text-primary-foreground">Active</span> : null}
        <span className="rounded border border-border px-2 py-0.5 text-xs" data-testid="connection-state">
          {view.has_key ? connectionLabel(view.status.state) : "No key saved"}
        </span>
      </div>
      <p className="text-muted-foreground">{view.descriptor.billing_note}</p>
      <p className="mt-1 text-xs text-muted-foreground">Implementation status: {view.descriptor.verification_note}</p>
      <p className="mt-1 flex gap-3 text-xs">
        <button type="button" className="text-primary hover:underline" onClick={() => void run(() => backend.aiOpenProviderPage(id, "key"))}>
          Get a key (opens your browser)
        </button>
        <button type="button" className="text-primary hover:underline" onClick={() => void run(() => backend.aiOpenProviderPage(id, "docs"))}>
          Provider documentation
        </button>
      </p>

      {view.credential_error ? <div className="mt-3"><ErrorNotice error={view.credential_error} /></div> : null}

      <div className="mt-4 grid gap-3 sm:grid-cols-2">
        <div className="sm:col-span-2">
          <label htmlFor={`key-${id}`} className="mb-1 block font-medium">
            API key
          </label>
          {view.has_key ? (
            <p className="mb-1 text-muted-foreground" data-testid="key-hint">
              Saved key: {view.key_hint ?? "hidden"}
            </p>
          ) : null}
          <input
            id={`key-${id}`}
            type="password"
            className={fieldClass}
            autoComplete="off"
            spellCheck={false}
            value={apiKey}
            placeholder={view.has_key ? "Paste a new key to replace the saved one" : view.descriptor.key_hint || "Paste your API key"}
            onChange={(e) => setApiKey(e.target.value)}
          />
        </div>
        <div>
          <label htmlFor={`label-${id}`} className="mb-1 block font-medium">
            Label (optional)
          </label>
          <input id={`label-${id}`} className={fieldClass} maxLength={60} value={label} onChange={(e) => setLabel(e.target.value)} />
        </div>
        <div className="flex items-end gap-2">
          <Button onClick={() => void save()} disabled={busy || apiKey.trim().length === 0}>
            {view.has_key ? "Replace key" : "Save key"}
          </Button>
          {view.has_key ? (
            <Button variant="outline" onClick={() => setConfirmDelete(true)} disabled={busy}>
              Remove key
            </Button>
          ) : null}
        </div>
      </div>

      {view.has_key ? (
        <>
          <div className="mt-5 grid gap-3 sm:grid-cols-2">
            <div className="sm:col-span-2">
              <label htmlFor={`model-${id}`} className="mb-1 block font-medium">
                Default model
              </label>
              <select id={`model-${id}`} className={fieldClass} value={model} onChange={(e) => setModel(e.target.value)}>
                <option value="">No model selected</option>
                {choices.map((m) => (
                  <option key={m} value={m}>
                    {m}
                    {view.status.verified_models.includes(m) && !listed.includes(m) ? " (verified by a test request)" : ""}
                  </option>
                ))}
              </select>
              <p className="mt-1 text-xs text-muted-foreground">
                {models ? models.note : "Loading models…"} Only models the provider listed, or that a test request verified, can be chosen.
              </p>
            </div>
            <div className="sm:col-span-2">
              <label htmlFor={`manual-${id}`} className="mb-1 block font-medium">
                {caps.model_listing ? "A model that is not in the list" : "Model name (this provider cannot list models)"}
              </label>
              <div className="flex gap-2">
                <input id={`manual-${id}`} className={fieldClass} value={manual} onChange={(e) => setManual(e.target.value)} placeholder="Type the exact model name" />
                <Button variant="outline" disabled={busy || !manual.trim()} onClick={() => void test(manual.trim())}>
                  Verify model
                </Button>
              </div>
            </div>
            <div>
              <label htmlFor={`timeout-${id}`} className="mb-1 block font-medium">
                Request timeout (seconds)
              </label>
              <input id={`timeout-${id}`} type="number" min={5} max={600} className={fieldClass} value={timeout} onChange={(e) => setTimeoutSecs(e.target.value)} />
            </div>
            <div>
              <label htmlFor={`temp-${id}`} className="mb-1 block font-medium">
                Temperature (optional, 0–2)
              </label>
              <input id={`temp-${id}`} type="number" step="0.1" min={0} max={2} className={fieldClass} value={temperature} onChange={(e) => setTemperature(e.target.value)} />
            </div>
            <div>
              <label htmlFor={`max-${id}`} className="mb-1 block font-medium">
                Maximum response length (tokens, optional)
              </label>
              <input id={`max-${id}`} type="number" min={1} className={fieldClass} value={maxTokens} onChange={(e) => setMaxTokens(e.target.value)} />
            </div>
            {id === "openrouter" ? (
              <label className="flex items-center gap-2 self-end">
                <input type="checkbox" checked={fallbacks} onChange={(e) => setFallbacks(e.target.checked)} />
                Let OpenRouter fall back to other upstream providers (off by default)
              </label>
            ) : null}
          </div>
          <div className="mt-4 flex flex-wrap gap-2">
            <Button onClick={() => void saveConfig()} disabled={busy}>
              Save settings
            </Button>
            <Button variant="outline" onClick={() => void test(model || null)} disabled={busy}>
              Test connection
            </Button>
            {!view.active ? (
              <Button
                variant="outline"
                disabled={busy}
                onClick={async () => {
                  if (await run(() => backend.aiSetActive(id))) onChanged();
                }}
              >
                Make active provider
              </Button>
            ) : null}
          </div>
        </>
      ) : null}

      {busy ? <p role="status" className="mt-3 text-muted-foreground">Working…</p> : null}
      {notice ? <p role="status" className="mt-3 text-sm">{notice}</p> : null}
      {error ? <div className="mt-3"><ErrorNotice error={{ ...error, recovery: error.recovery }} /><button type="button" className="mt-1 text-xs underline" onClick={clearError}>Dismiss</button></div> : null}
      {report ? (
        <section aria-label="Connection test result" className="mt-4 rounded-md border border-border p-3">
          <p className="font-medium">{report.ok ? "Connection verified" : report.state === "key_accepted" ? "Key accepted — model not verified" : "Connection test failed"}</p>
          <ul className="mt-2 space-y-1">
            {report.steps.map((s) => (
              <li key={s.name}>
                <span aria-hidden>{s.ok ? "✓" : "✗"}</span> <span className="sr-only">{s.ok ? "Passed:" : "Failed:"}</span> {s.name} — <span className="text-muted-foreground">{s.detail}</span>
              </li>
            ))}
          </ul>
          {report.error ? <div className="mt-2"><ErrorNotice error={report.error} /></div> : null}
        </section>
      ) : null}

      <ConfirmDialog
        open={confirmDelete}
        onOpenChange={setConfirmDelete}
        title={`Remove the ${view.descriptor.display_name} key?`}
        description="The key is deleted from Windows Credential Manager. Your conversations stay. If this was the active provider, no provider will be active until you choose one."
        confirmLabel="Remove key"
        destructive
        busy={busy}
        onConfirm={() => void remove()}
      />
    </Card>
  );
}

function Panels({ initial }: { initial: AiOverview }) {
  const backend = useBackend();
  const [overview, setOverview] = useState(initial);
  // Refresh in place so a test result or notice stays on screen.
  const reload = () => void backend.aiOverview().then(setOverview, () => undefined);
  const { run, error } = useAction();
  const [selected, setSelected] = useState(overview.providers.find((p) => p.active)?.descriptor.id ?? overview.providers[0]?.descriptor.id ?? "");
  const [days, setDays] = useState(String(overview.settings.history_days));
  const [saved, setSaved] = useState(false);
  const [lookup, setLookup] = useState(overview.settings.allow_vulnerability_lookup);
  const view = overview.providers.find((p) => p.descriptor.id === selected) ?? overview.providers[0];
  const unavailable = overview.providers.find((p) => p.credential_error)?.credential_error;

  return (
    <div className="space-y-4">
      <Card role="note" aria-label="Privacy and security">
        <CardTitle>Before you add a key</CardTitle>
        <ul className="list-disc space-y-1 pl-5">
          <li>Messages you send are transmitted to the cloud provider you choose, and the provider may bill you. Vixalune is not “entirely local” when you use AI.</li>
          <li>Keys are stored in {overview.credential_backend}, for your Windows user only. Vixalune adds no encryption of its own and never keeps a key in its database, settings or logs.</li>
          <li>Malware, or anyone using your signed-in Windows session, may still be able to use credentials that the app can use.</li>
          <li>If secure storage is unavailable, keys cannot be saved at all — there is no fallback.</li>
        </ul>
        {unavailable ? <div className="mt-3"><ErrorNotice error={unavailable} /></div> : null}
      </Card>

      <div>
        <label htmlFor="provider-select" className="mb-1 block font-medium">
          Provider
        </label>
        <select id="provider-select" className={fieldClass + " max-w-xs"} value={selected} onChange={(e) => setSelected(e.target.value)}>
          {overview.providers.map((p) => (
            <option key={p.descriptor.id} value={p.descriptor.id}>
              {p.descriptor.display_name}
              {p.active ? " — active" : p.has_key ? " — key saved" : ""}
            </option>
          ))}
        </select>
      </div>
      {view ? <ProviderPanel key={view.descriptor.id} view={view} onChanged={reload} /> : null}

      <Card>
        <CardTitle>Conversation history</CardTitle>
        <label htmlFor="ai-days" className="mb-1 block font-medium">
          Keep AI conversations for (days; 0 keeps nothing)
        </label>
        <div className="flex gap-2">
          <input id="ai-days" type="number" min={0} className={fieldClass + " max-w-[8rem]"} value={days} onChange={(e) => setDays(e.target.value)} />
          <Button
            onClick={async () => {
              setSaved(false);
              if (await run(() => backend.aiUpdateSettings({ history_days: Number(days), allow_vulnerability_lookup: lookup }))) {
                setSaved(true);
                reload();
              }
            }}
          >
            Save
          </Button>
        </div>
        <label className="mt-3 flex items-start gap-2">
          <input type="checkbox" className="mt-1" checked={lookup} onChange={(e) => setLookup(e.target.checked)} />
          <span>
            Allow vulnerability database lookups. When you ask the assistant about a package, its ecosystem, name and version are sent to the public OSV database (api.osv.dev) and, for CVE identifiers, NVD (nvd.nist.gov). Nothing else is sent. Off by default.
          </span>
        </label>
        <p className="mt-1 text-xs text-muted-foreground">Old conversations are deleted when the app starts and when you save this. Command output and file contents are never added to conversations by Vixalune.</p>
        {saved ? <p role="status" className="mt-2">Saved.</p> : null}
        {error ? <div className="mt-2"><ErrorNotice error={error} /></div> : null}
      </Card>
    </div>
  );
}

export function AiProvidersPage() {
  const backend = useBackend();
  const [state, retry] = useAsync(() => backend.aiOverview(), [backend]);
  return (
    <div>
      <PageHeader title="AI Providers" subtitle="Bring your own key. Choose a provider, store a key securely, test it and pick a model." />
      <AsyncView state={state} retry={retry} label="Loading AI providers">
        {(o) => <Panels initial={o} />}
      </AsyncView>
    </div>
  );
}
