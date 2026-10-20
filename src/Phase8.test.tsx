import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Backend } from "@/domain/backend";
import type { AiConversation, AiMessage, AiOverview, AiProviderView, AiStreamEvent, AiTestReport } from "@/domain/types";
import { createDemoBackend } from "@/infra/demoBackend";
import { renderApp } from "@/test/render";

const KEY = "sk-test-0123456789abcdefghij";

function provider(over: Partial<AiProviderView> = {}, id = "openai"): AiProviderView {
  return {
    descriptor: {
      id,
      display_name: id === "openai" ? "OpenAI" : "Anthropic",
      capabilities: { streaming: true, tool_calling: false, structured_output: false, model_listing: true, usage_reporting: true, cost_reporting: false, cancellation: true, context_limits: false },
      key_hint: "sk-…",
      key_page: "https://example.test/keys",
      docs_page: "https://example.test/docs",
      billing_note: "Usage is billed to your account by the provider.",
      verification_note: "Implemented against the documented API; not yet verified with a real key.",
      extra_settings: [],
    },
    config: { provider: id, label: null, default_model: null, timeout_secs: 60, temperature: null, max_output_tokens: null, extra: {} },
    has_key: false,
    key_hint: null,
    status: { state: "untested", tested_at: null, error_code: null, message: null, steps: [], models_listed: null, verified_models: [] },
    active: false,
    credential_error: null,
    ...over,
  };
}

function overview(providers: AiProviderView[]): AiOverview {
  return { providers, credential_backend: "Windows Credential Manager (protected for your Windows account)", settings: { history_days: 30, allow_vulnerability_lookup: false, local_only: false, max_request_chars: 200000, confirm_large_chars: 60000, session_request_limit: 200 }, session: { requests_used: 0, limit: 200 } };
}

const KEYED = (extra: Partial<AiProviderView> = {}) => provider({ has_key: true, active: true, key_hint: "sk-••••••••  (28 characters)", ...extra });

function backendWith(o: Partial<Backend> = {}): Backend {
  return {
    ...createDemoBackend(),
    isDemo: false,
    aiOverview: async () => overview([provider(), provider({}, "anthropic")]),
    aiListModels: async () => ({ models: [{ id: "model-a", display_name: null, context_window: null, max_output_tokens: null, supports_tools: null, pricing: null, source: "listed" }], from_provider: true, note: "2 models listed." }),
    aiOpenProviderPage: async () => undefined,
    aiListConversations: async () => [],
    onAiEvent: async () => () => undefined,
    ...o,
  };
}

describe("Settings → AI Providers", () => {
  it("explains that prompts go to a cloud provider and that local malware can still reach credentials", async () => {
    renderApp("/settings/ai", backendWith());
    const note = await screen.findByRole("note", { name: "Privacy and security" });
    expect(note).toHaveTextContent("transmitted to the cloud provider");
    expect(note).toHaveTextContent("Malware");
    expect(note).toHaveTextContent("no fallback");
    expect(note).toHaveTextContent("Windows Credential Manager");
  });

  it("sends the key once, clears the field, and only ever shows a masked hint afterwards", async () => {
    const save = vi.fn(async () => KEYED());
    const be = backendWith({ aiSaveCredential: save });
    const view = renderApp("/settings/ai", be);
    const input = await screen.findByLabelText("API key");
    expect(input).toHaveAttribute("type", "password");
    await userEvent.type(input, KEY);
    await userEvent.click(screen.getByRole("button", { name: "Save key" }));
    await waitFor(() => expect(save).toHaveBeenCalledWith("openai", KEY, null));
    await waitFor(() => expect(screen.getByLabelText("API key")).toHaveValue(""));
    expect(view.container.innerHTML).not.toContain(KEY);
    expect(document.body.textContent).not.toContain(KEY);
  });

  it("shows a credential-store failure instead of pretending the key was saved", async () => {
    const save = vi.fn(async () => {
      throw { code: "ai_credential_store", message: "Windows Credential Manager could not be used", recovery: "Keys are never stored anywhere else. Check that Credential Manager is available and try again." };
    });
    renderApp("/settings/ai", backendWith({ aiSaveCredential: save }));
    await userEvent.type(await screen.findByLabelText("API key"), KEY);
    await userEvent.click(screen.getByRole("button", { name: "Save key" }));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Credential Manager could not be used");
    expect(alert).toHaveTextContent("never stored anywhere else");
    expect(screen.queryByTestId("key-hint")).toBeNull();
  });

  it("does not call a key-only check 'verified'", async () => {
    const report: AiTestReport = { ok: false, state: "key_accepted", steps: [{ name: "API key accepted", ok: true, detail: "Listed 2 models." }, { name: "Model not yet verified", ok: false, detail: "Choose a model and test again." }], models_listed: 2, verified_model: null, error: null };
    const test = vi.fn(async () => report);
    renderApp("/settings/ai", backendWith({ aiOverview: async () => overview([KEYED(), provider({}, "anthropic")]), aiTestConnection: test }));
    await userEvent.click(await screen.findByRole("button", { name: "Test connection" }));
    const result = await screen.findByRole("region", { name: "Connection test result" });
    expect(result).toHaveTextContent("Key accepted — model not verified");
    expect(result).not.toHaveTextContent("Connection verified");
    expect(test).toHaveBeenCalledWith("openai", null);
  });

  it("reports authentication failures with the recovery step", async () => {
    const report: AiTestReport = {
      ok: false,
      state: "failed",
      steps: [{ name: "API key check", ok: false, detail: "rejected" }],
      models_listed: null,
      verified_model: null,
      error: { code: "ai_authentication", kind: "authentication", message: "The provider rejected the API key.", recovery: "Check the key, or replace it." },
    };
    renderApp("/settings/ai", backendWith({ aiOverview: async () => overview([KEYED(), provider({}, "anthropic")]), aiTestConnection: async () => report }));
    await userEvent.click(await screen.findByRole("button", { name: "Test connection" }));
    const result = await screen.findByRole("region", { name: "Connection test result" });
    expect(result).toHaveTextContent("Connection test failed");
    expect(within(result).getByRole("alert")).toHaveTextContent("rejected the API key");
  });

  it("offers only listed or verified models, plus a way to verify one that is not listed", async () => {
    const test = vi.fn(async (): Promise<AiTestReport> => ({ ok: true, state: "verified", steps: [], models_listed: 1, verified_model: "typed-model", error: null }));
    const v = KEYED({ status: { state: "verified", tested_at: null, error_code: null, message: null, steps: [], models_listed: 1, verified_models: ["verified-only"] } });
    renderApp("/settings/ai", backendWith({ aiOverview: async () => overview([v, provider({}, "anthropic")]), aiTestConnection: test }));
    const select = await screen.findByLabelText("Default model");
    await waitFor(() => expect(within(select).getByRole("option", { name: "model-a" })).toBeInTheDocument());
    const names = within(select).getAllByRole("option").map((o) => o.textContent);
    expect(names).toEqual(["No model selected", "model-a", "verified-only (verified by a test request)"]);
    await userEvent.type(screen.getByLabelText("A model that is not in the list"), "typed-model");
    await userEvent.click(screen.getByRole("button", { name: "Verify model" }));
    expect(test).toHaveBeenCalledWith("openai", "typed-model");
  });

  it("asks before removing a key", async () => {
    const del = vi.fn(async () => provider());
    renderApp("/settings/ai", backendWith({ aiOverview: async () => overview([KEYED(), provider({}, "anthropic")]), aiDeleteCredential: del }));
    await userEvent.click(await screen.findByRole("button", { name: "Remove key" }));
    expect(del).not.toHaveBeenCalled();
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "Remove key" }));
    await waitFor(() => expect(del).toHaveBeenCalledWith("openai"));
  });

  it("says AI is unavailable in the demonstration instead of inventing providers", async () => {
    renderApp("/settings/ai", createDemoBackend());
    expect(await screen.findByRole("alert")).toHaveTextContent("not available in the demonstration");
  });
});

async function plainMode() {
  await userEvent.click(await screen.findByRole("checkbox", { name: /Use the tool catalog/ }));
}

describe("AI Assistant", () => {
  const conv: AiConversation = { id: "c1", title: "Nmap question", provider: "openai", model: "model-a", created_at: "2026-01-01T00:00:00Z", updated_at: "2026-01-01T00:00:00Z" };
  const userMsg: AiMessage = { id: "u1", conversation_id: "c1", seq: 0, role: "user", content: "Explain nmap -F", status: "complete", error_code: null, error_message: null, provider: null, model: null, usage: null, created_at: "2026-01-01T00:00:00Z" };

  function chatBackend(extra: Partial<Backend> = {}) {
    let handler: (e: AiStreamEvent) => void = () => undefined;
    const be = backendWith({
      aiOverview: async () => overview([KEYED({ config: { ...provider().config, default_model: "model-a" } })]),
      aiListConversations: async () => [],
      aiCreateConversation: async () => conv,
      onAiEvent: async (h) => {
        handler = h;
        return () => undefined;
      },
      ...extra,
    });
    return { be, emit: (e: AiStreamEvent) => act(() => handler(e)) };
  }

  it("points to Settings when no provider is configured", async () => {
    renderApp("/assistant", backendWith());
    expect(await screen.findByText("No AI provider is set up")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /AI Providers/ })).toHaveAttribute("href", "/settings/ai");
  });

  it("warns that prompts go to the cloud provider and may cost money", async () => {
    const { be } = chatBackend();
    renderApp("/assistant", be);
    expect(await screen.findByRole("note")).toHaveTextContent("sent to OpenAI over the internet");
  });

  it("streams an answer, can stop it, and shows cost only when the provider reported one", async () => {
    const send = vi.fn(async () => ({ request_id: "r1", conversation_id: "c1", user_message: userMsg, assistant_message_id: "a1" }));
    const cancel = vi.fn(async () => undefined);
    const final: AiMessage = { ...userMsg, id: "a1", seq: 1, role: "assistant", content: "It scans **100** ports.", provider: "openai", model: "model-a", usage: { input_tokens: 9, output_tokens: 3, total_tokens: 12, cost_usd: null } };
    const { be, emit } = chatBackend({
      aiSendMessage: send,
      aiCancel: cancel,
      aiGetConversation: async () => ({ conversation: conv, messages: [userMsg, final] }),
    });
    renderApp("/assistant", be);
    await plainMode();
    await waitFor(() => expect(screen.getByLabelText("Model")).toBeEnabled());
    await userEvent.selectOptions(screen.getByLabelText("Model"), "model-a");
    await userEvent.type(screen.getByLabelText("Message"), "Explain nmap -F");
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    await waitFor(() => expect(send).toHaveBeenCalledWith({ conversation_id: "c1", text: "Explain nmap -F", provider: "openai", model: "model-a", confirm_large: false }));
    await emit({ request_id: "r1", conversation_id: "c1", message_id: "a1", event: "delta", text: "It scans " });
    await emit({ request_id: "r1", conversation_id: "c1", message_id: "a1", event: "delta", text: "**100** ports." });
    expect(await screen.findByText("100")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: /Stop generating/ }));
    expect(cancel).toHaveBeenCalledWith("r1");
    await emit({ request_id: "r1", conversation_id: "c1", message_id: "a1", event: "finished", status: "complete", finish_reason: "stop", usage: final.usage!, error: null });
    await waitFor(() => expect(screen.queryByRole("button", { name: /Stop generating/ })).toBeNull());
    const answers = await screen.findAllByRole("article", { name: "Assistant" });
    expect(answers[0]).toHaveTextContent("9 in / 3 out tokens");
    expect(answers[0]).toHaveTextContent("cost not reported by the provider");
    expect(answers[0]).not.toHaveTextContent("$");
    expect(screen.getByRole("button", { name: "Copy response" })).toBeInTheDocument();
  });

  it("renders model output safely: no raw HTML, images or live links, and syntax-highlights code", async () => {
    const evil: AiMessage = { ...userMsg, id: "a1", seq: 1, role: "assistant", content: '<script>alert(1)</script> [click](https://evil.test/x) ![t](https://evil.test/p.png)\n\n```bash\nnmap -F 127.0.0.1\n```', model: "model-a", provider: "openai" };
    const { be } = chatBackend({ aiListConversations: async () => [conv], aiGetConversation: async () => ({ conversation: conv, messages: [userMsg, evil] }) });
    const view = renderApp("/assistant", be);
    await userEvent.click(await screen.findByRole("button", { name: conv.title }));
    await screen.findByText(/127.0.0.1/);
    expect(view.container.querySelector("script")).toBeNull();
    expect(view.container.querySelector("img")).toBeNull();
    expect(view.container.querySelector("a[href^='http']")).toBeNull();
    expect(view.container.querySelector("pre code")).not.toBeNull();
    expect(screen.getByRole("button", { name: "Copy code" })).toBeInTheDocument();
  });

  it("shows a failed answer with a Retry button that re-asks the same question", async () => {
    const failed: AiMessage = { ...userMsg, id: "a1", seq: 1, role: "assistant", content: "", status: "failed", error_code: "ai_rate_limit", error_message: "The provider is rate limiting requests.", model: "model-a", provider: "openai" };
    const retry = vi.fn(async () => ({ request_id: "r2", conversation_id: "c1", user_message: userMsg, assistant_message_id: "a2" }));
    const { be } = chatBackend({ aiListConversations: async () => [conv], aiGetConversation: async () => ({ conversation: conv, messages: [userMsg, failed] }), aiRetry: retry });
    renderApp("/assistant", be);
    await plainMode();
    await userEvent.click(await screen.findByRole("button", { name: conv.title }));
    expect(await screen.findByText(/rate limiting/)).toBeInTheDocument();
    await waitFor(() => expect(screen.getByLabelText("Model")).toBeEnabled());
    await userEvent.selectOptions(screen.getByLabelText("Model"), "model-a");
    await userEvent.click(screen.getByRole("button", { name: /Retry/ }));
    await waitFor(() => expect(retry).toHaveBeenCalledWith("c1", "openai", "model-a"));
  });

  it("confirms before deleting a conversation", async () => {
    const del = vi.fn(async () => undefined);
    const { be } = chatBackend({ aiListConversations: async () => [conv], aiDeleteConversation: del });
    renderApp("/assistant", be);
    await userEvent.click(await screen.findByRole("button", { name: `Delete conversation ${conv.title}` }));
    expect(del).not.toHaveBeenCalled();
    await userEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(del).toHaveBeenCalledWith("c1"));
  });
});
