import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Backend } from "@/domain/backend";
import type { AiConversation, AiMessage, AiOverview } from "@/domain/types";
import { createDemoBackend } from "@/infra/demoBackend";
import { renderApp } from "@/test/render";

const conv: AiConversation = { id: "c1", title: "Chat", provider: "fake", model: "m", created_at: "2026-10-20T10:00:00Z", updated_at: "2026-10-20T10:00:00Z" };
const base = (over: Partial<AiOverview["settings"]> = {}, session = { requests_used: 3, limit: 200 }): AiOverview => ({
  providers: [
    {
      descriptor: { id: "fake", display_name: "FakeAI", capabilities: { streaming: true, tool_calling: false, structured_output: false, model_listing: true, usage_reporting: true, cost_reporting: false, cancellation: true, context_limits: false }, key_hint: "", key_page: "https://example.test/k", docs_page: "https://example.test/d", billing_note: "billed", verification_note: "test", extra_settings: [] },
      config: { provider: "fake", label: null, default_model: "m", timeout_secs: 60, temperature: null, max_output_tokens: null, extra: {} },
      has_key: true,
      key_hint: "sk-••••",
      status: { state: "verified", tested_at: null, error_code: null, message: null, steps: [], models_listed: 1, verified_models: ["m"] },
      active: true,
      credential_error: null,
    },
  ],
  credential_backend: "Windows Credential Manager",
  settings: { history_days: 30, allow_vulnerability_lookup: false, local_only: false, max_request_chars: 200000, confirm_large_chars: 60000, session_request_limit: 200, ...over },
  session,
});

function backendWith(o: AiOverview, extra: Partial<Backend> = {}): Backend {
  return {
    ...createDemoBackend(),
    isDemo: false,
    aiOverview: async () => o,
    aiListModels: async () => ({ models: [{ id: "m", display_name: null, context_window: null, max_output_tokens: null, supports_tools: null, pricing: null, source: "listed" }], from_provider: true, note: "" }),
    aiListConversations: async () => [],
    aiCreateConversation: async () => conv,
    assistantTurns: async () => [],
    onAiEvent: async () => () => undefined,
    ...extra,
  };
}

describe("privacy and cost controls in settings", () => {
  it("explains local-only mode honestly and saves every control", async () => {
    const update = vi.fn(async (s) => s);
    renderApp("/settings/ai", backendWith(base(), { aiUpdateSettings: update }));
    const card = await screen.findByRole("region", { name: "Privacy and cost controls" });
    expect(card).toHaveTextContent("Unavailable while it is on");
    expect(card).toHaveTextContent("not a promise that Vixalune makes no network connections");
    expect(card).toHaveTextContent("package managers");
    expect(screen.getByTestId("session-usage")).toHaveTextContent("Requests sent this session: 3 of 200");
    expect(card).toHaveTextContent("never estimates a price");
    await userEvent.click(within(card).getByRole("checkbox", { name: /Local-only mode/ }));
    await userEvent.clear(screen.getByLabelText("Largest request (characters)"));
    await userEvent.type(screen.getByLabelText("Largest request (characters)"), "50000");
    await userEvent.clear(screen.getByLabelText(/Requests per session/));
    await userEvent.type(screen.getByLabelText(/Requests per session/), "10");
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(update).toHaveBeenCalledWith({ history_days: 30, allow_vulnerability_lookup: false, local_only: true, max_request_chars: 50000, confirm_large_chars: 60000, session_request_limit: 10 }));
  });
});

describe("assistant privacy and cost behavior", () => {
  it("in local-only mode says nothing is sent, what is unavailable and that it is not a guarantee, and cannot send", async () => {
    const ask = vi.fn();
    renderApp("/assistant", backendWith(base({ local_only: true }), { assistantAsk: ask }));
    const banner = await screen.findByRole("status", { name: "Local-only mode" });
    expect(banner).toHaveTextContent("Nothing is sent to an AI provider");
    expect(banner).toHaveTextContent("vulnerability lookups are unavailable");
    expect(banner).toHaveTextContent("does not stop installers, scanners or other tools from using the network");
    await userEvent.type(await screen.findByLabelText("Message"), "hello");
    expect(screen.getByRole("button", { name: "Send" })).toBeDisabled();
    expect(ask).not.toHaveBeenCalled();
  });

  it("states which provider receives each request and what is not shared", async () => {
    renderApp("/assistant", backendWith(base()));
    const line = await screen.findByLabelText("What is shared");
    expect(line).toHaveTextContent("Goes to FakeAI with each question");
    expect(line).toHaveTextContent("Not shared: files, project folders, scan output or credentials");
    expect(line).toHaveTextContent("unless you press");
    expect(line).toHaveTextContent("Requests this session: 3 of 200");
  });

  it("asks before sending an unusually large request, and sends nothing until confirmed", async () => {
    const ack = { request_id: "r1", conversation_id: "c1", user_message: { id: "u1", conversation_id: "c1", seq: 1, role: "user", content: "big", status: "complete", error_code: null, error_message: null, provider: null, model: null, usage: null, created_at: "t" } satisfies AiMessage, assistant_message_id: "a1" };
    const ask = vi
      .fn()
      .mockRejectedValueOnce({ code: "ai_needs_confirmation", message: "this request is large: about 70000 characters (roughly 17500 tokens) will be sent to FakeAI and may cost more than usual", recovery: "Confirm or shorten." })
      .mockResolvedValueOnce(ack);
    renderApp("/assistant", backendWith(base(), { assistantAsk: ask }));
    await waitFor(() => expect(screen.getByLabelText("Model")).toBeEnabled());
    await userEvent.selectOptions(screen.getByLabelText("Model"), "m");
    await userEvent.type(screen.getByLabelText("Message"), "big");
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent("about 70000 characters");
    expect(ask).toHaveBeenCalledTimes(1);
    expect(ask.mock.calls[0]![0].confirm_large).toBe(false);
    await userEvent.click(within(dialog).getByRole("button", { name: "Send anyway" }));
    await waitFor(() => expect(ask).toHaveBeenCalledTimes(2));
    expect(ask.mock.calls[1]![0].confirm_large).toBe(true);
  });

  it("warns when a provider did not report usage", async () => {
    const msgs: AiMessage[] = [
      { id: "u1", conversation_id: "c1", seq: 1, role: "user", content: "hi", status: "complete", error_code: null, error_message: null, provider: null, model: null, usage: null, created_at: "t" },
      { id: "a1", conversation_id: "c1", seq: 2, role: "assistant", content: "hello", status: "complete", error_code: null, error_message: null, provider: "fake", model: "m", usage: null, created_at: "t" },
    ];
    renderApp("/assistant", backendWith(base(), { aiListConversations: async () => [conv], aiGetConversation: async () => ({ conversation: conv, messages: msgs }) }));
    await userEvent.click(await screen.findByRole("button", { name: "Chat" }));
    expect(await screen.findByText(/did not report usage, so the size and cost of this request are unknown/)).toBeInTheDocument();
  });
});
