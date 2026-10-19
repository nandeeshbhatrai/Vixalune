# AI gateway and bring-your-own-key

Vixalune can talk to a cloud AI provider using **your own API key**. This page covers what it does, what it does not do, how keys are protected, which providers are verified, and how to add another provider.

## What you should know first

- **Your messages leave your computer.** Anything you type in the AI Assistant is sent to the provider you chose. It is not "entirely local". The provider may log it and bills **your** account. Do not paste secrets, credentials, or data you are not allowed to share.
- **Vixalune adds nothing to your messages.** It does not attach API keys, environment variables, files, scan output or run history. If you want the assistant to see command output, paste it yourself.
- **The assistant cannot act.** No tools are offered to a model, so an answer can never start a program, change a file or reach a target. Replies are shown as text.
- **Costs are reported, never estimated.** Token counts and a dollar figure appear only when the provider returns them; otherwise the screen says "cost not reported by the provider".
- **A failure never switches provider or model.** If a request fails, you see the error and decide what to do. Retry is a button.

The tool-aware assistant built on this gateway is described in [ASSISTANT.md](ASSISTANT.md). Its one extra setting is **Allow vulnerability database lookups** (off by default).

## Where keys are stored

Keys are stored in **Windows Credential Manager** (`keyring` crate, Windows native backend), under the service name `app.vixalune.desktop.ai`, one entry per provider, for the signed-in Windows user only. Windows protects the value with the user's credentials; Vixalune adds **no encryption of its own**.

| Where | Key stored? |
|---|---|
| Windows Credential Manager | Yes (the only place) |
| SQLite database, `config.toml`, settings, logs, activity log, conversations | Never |
| Browser/WebView storage, React state after saving | Never. The field is cleared as soon as the key is saved; the backend returns only a masked hint such as `sk-ant-••••••••  (108 characters)` |
| Error messages, event payloads | Never. Provider error text is scrubbed of the key (and anything key-shaped) before it is shown or stored |

If Credential Manager cannot be used, saving fails with an explanation. There is **no fallback** to a file or the database.

**Limit of the protection:** malware running as you, or anyone using your unlocked Windows session, may be able to use credentials the app can use. Use provider-side spending limits and revoke a key you think was exposed.

A key is sent only in the provider's authentication header over HTTPS (Gemini's key is in the `x-goog-api-key` header, never in the URL). Redirects are disabled so a key can never follow a redirect to another host.

## Providers

All four adapters are implemented against the providers' **documented** current APIs and are exercised in automated tests against a local mock server that speaks each documented wire format. **None has been verified live in this repository's CI or by the author with a real key**; treat each as "implemented, awaiting a live check" until you run the opt-in tests below.

| Provider | API used | List models | Streaming | Reports cost |
|---|---|---|---|---|
| OpenAI | Responses API (`/v1/responses`, `/v1/models`), `store:false` | Yes | Yes | No (tokens only) |
| Anthropic | Messages API (`/v1/messages`, `anthropic-version: 2023-06-01`, `/v1/models`) | Yes | Yes | No (tokens only) |
| Google Gemini | Gemini API (`models.list`, `streamGenerateContent?alt=sse`) | Yes | Yes | No (tokens only) |
| OpenRouter | OpenAI-compatible `/api/v1/chat/completions`, `/models`, `/key` | Yes | Yes | Yes (`usage.cost`) |

OpenRouter: upstream fallbacks are **off** by default (`provider.allow_fallbacks:false`) so a request stays with the provider you picked; you can enable them in Settings.

Each provider's screen shows its own implementation note.

### Opt-in live checks

These are ignored by default, never run in CI and need no key unless you set one:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib live_ -- --ignored --nocapture
```

Set `VIXALUNE_TEST_OPENAI_KEY`, `..._ANTHROPIC_KEY`, `..._GEMINI_KEY`, `..._OPENROUTER_KEY` (and optionally `VIXALUNE_TEST_<NAME>_MODEL` for a streamed answer). Providers without a variable are skipped and printed as `SKIPPED`. Each makes one tiny request that may be billed. The Credential Manager test needs no key; it writes and deletes a throwaway entry under a random test service name.

## Connection status

- **Not tested** – nothing has been tried.
- **Key accepted, model not verified** – the provider accepted the key (for example by listing models), but no model has answered a request.
- **Verified** – a minimal real request to the chosen model succeeded.
- **Failed** – with the classified reason (authentication, quota, rate limit, network, timeout, model not found…).

Only models the provider **listed** or a test request **verified** can be selected. If a provider cannot list models, type the name and use **Verify model**.

## Reliability

Timeouts (default 60 s, 5–600), idle timeout between stream chunks, cancellation ("Stop generating"), bounded retries (up to 3 attempts, exponential backoff with jitter, honours `Retry-After`) only for transient errors **and only before any text arrived**, size limits (50,000 characters per message, 200,000 characters of context, 200,000 characters per answer), and partial answers kept as *Stopped* or *Interrupted*. Retries after text has started, and after authentication, quota, model or request errors, are never automatic.

## History

Conversations and messages are stored in the local SQLite database (never keys). Retention is configurable (default 30 days, 0 = keep nothing); expired conversations are deleted at start-up and when the setting is saved. You can delete a single conversation or all of them. Vixalune never saves command output or file contents into a conversation unless you paste them.

## Adding a provider

1. Implement `AiProvider` (`src-tauri/src/ports.rs`) in a new file under `src-tauri/src/adapters/ai/`: `descriptor()` (id, name, capabilities, links, billing note, honest `verification_note`), `validate_config`, `validate_request`, `check_auth`, `list_models`, `chat_stream`. Use `http.rs` (HTTPS-only client, no redirects, bounded error bodies), `sse.rs` and `common.rs`. For OpenAI-compatible APIs reuse `openai_compat.rs`.
2. Register it in `ProviderRegistry::builtin()` (one line).
3. Add a table entry in `adapter_tests.rs` using the mock server, covering success, auth, rate limit, network and cancel.

No change to the service, commands or React is needed; screens are driven by the provider's descriptor.
