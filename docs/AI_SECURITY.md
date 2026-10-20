# AI security review (Phase 10)

This is a product-security review of the AI features: the provider gateway, the credential store, the tool registry and installer as the assistant sees them, the scanner integrations and the execution engine. It lists what is enforced, how each claim is tested, what is not covered, and what must be fixed or verified before anyone should call this production-ready.

**Verdict: not production-ready.** The controls below are implemented and tested, but the most important unverified item is that no AI provider adapter has been exercised against the real provider in this repository (see [Residual risks](#residual-risks-and-untested-areas)). Do not rely on this document as a certification.

Related: [AI_GATEWAY.md](AI_GATEWAY.md) (providers and keys), [ASSISTANT.md](ASSISTANT.md) (tool-aware assistant), [PRIVACY.md](PRIVACY.md) (what leaves your computer), [THREAT_MODEL.md](THREAT_MODEL.md).

## 1. Security requirements and how they are met

| # | Requirement | Control | Where it is tested |
|---|---|---|---|
| 1 | Credentials never in frontend state (beyond the input being submitted), logs, database, crash reports or analytics | Key goes frontend → backend once, is wrapped in `Secret` (no `Debug`/`Display`/`Serialize`, wiped on drop) and written to Windows Credential Manager; the input is cleared after saving; the UI receives only a masked hint. No database column, setting or log call can hold it. A panic hook logs panic text through the same redaction and writes no report. No analytics/telemetry/crash-reporting dependency exists. | `the_key_never_reaches_the_database_logs_events_views_or_stored_answers` (dumps every table, captures all log output, serializes every event and view), `the_product_has_no_analytics_telemetry_or_crash_reporting`, `Phase8.test.tsx` (field cleared, key not in the DOM), `panic_text_is_redacted…` |
| 2 | Credentials retrieved only by the trusted backend component that needs them | Only `AiService` reads the credential store, and only for the one provider it is about to call. The assistant, executor, commands and engine modules cannot (checked by a source scan). `.expose()` appears only in the HTTP adapters, to put the key in an auth header. | `only_the_ai_service_ever_reads_the_credential_store` |
| 3 | Credentials and sensitive headers redacted from errors and logs | `sanitize_message` removes the exact key (and its URL-encoded form), key-shaped strings, `Authorization`/`x-api-key`/`x-goog-api-key` headers, bearer tokens, `?key=` parameters, URL credentials, and local paths; it runs on every provider error before it is shown, stored or logged. Provider errors are built from a bounded body and never include request headers. | `local_paths_and_headers_are_removed_from_cloud_error_text`, the adapter suites, `errors_that_echo_the_key_are_scrubbed…` |
| 4 | Files and scan results not uploaded without disclosure and consent | The only path that sends local content is **Explain a scan run**: a dialog names the provider and what is sent (≤ 400 lines, credential-looking values masked), and the backend refuses without `consent = true`. Nothing else reads files or run output. The output is not stored in the conversation. | `explaining_a_run_needs_consent…`, `local_only_mode…`, `Phase9.test.tsx` (dialog) |
| 5 | The AI cannot bypass backend authorization via prompt injection, malicious retrieved content or fabricated tool calls | No tools are offered to a model. The action set is closed and typed; authorization is a function of the *origin* (model: catalog reads only) and the person's consent, not of text. Untrusted text is fenced, but nothing depends on the model obeying. | `malicious_text_in_retrieved_data_cannot_authorize_anything`, `action_authorization_is_decided_in_code…` |
| 6 | Every tool action and its arguments validated independently | `deny_unknown_fields` types, id/charset/length checks, then registry checks (tool, profile, parameter names), then the runner's own validation at preview time. | `arguments_are_typed_and_unknown_fields_are_rejected`, `identifiers_and_values_are_validated…`, `invalid_or_hostile_structured_replies…` |
| 7 | No dangerous action runs automatically from a model message | There is no install, run, delete or settings action. *Install…* and *Prepare Scan* open the existing screens with their confirmations. Suggested actions are displayed as suggestions. | `malicious_text…` (no preview, no plan read, no install), `read_only_actions_report_the_real_state` |
| 8 | Request limits, timeouts, cancellation, bounded retries | Per-message (50 000 chars), per-request (configurable, default and maximum 200 000 chars), per-answer (200 000) limits; confirmation above a configurable size; per-session request limit; response token limit per provider; request and idle timeouts; cancellation; at most 3 attempts with backoff, only for transient errors and only before text arrives. | `request_size_confirmation_and_session_limits_are_enforced…`, `stopping_an_assistant_request…`, service tests (retries, cancellation) |
| 9 | Provider endpoints under application control | Base URLs are constants in the adapters; configuration structures reject unknown fields and unknown "extra" keys; HTTPS is required (loopback only for tests); redirects are disabled so a key is never forwarded; no response field is used as a URL. | `provider_endpoints_cannot_be_changed_by_configuration_or_content`, the per-adapter redirect test |
| 10 | Cloud errors do not disclose secrets or local filesystem details | Same sanitizer, including Windows, UNC and Unix-style paths; messages are length-bounded. | `local_paths_and_headers…`, `the_key_never_reaches…` (asserts a path in a provider error does not appear anywhere) |

## 2. Privacy controls

- **Provider disclosure.** The Assistant always states which provider receives the request and what goes with it: your message, the conversation so far, and the catalog records of the tools considered. It also states what is *not* shared.
- **Visible indicator for shared local content.** Scan output is shared only through *Explain a scan run*, with a dialog naming the provider, the size limit and the credential masking. No project folder or file is ever uploaded.
- **Per-request consent.** Each scan-run explanation needs its own confirmation. A large request needs its own confirmation. Vulnerability lookups need a standing consent (off by default).
- **Retention and deletion.** Conversations are kept for a configurable number of days (default 30, 0 keeps nothing) and can be deleted one by one or all at once; structured answers are deleted with their message (database trigger).
- **Secret masking.** Passwords, tokens, access keys, private keys, JWTs, bearer headers and URL credentials are masked in scan output before sharing and in a typed message before it is stored or sent (best effort; a message that contains a recognisable API key or private key is refused instead). Masking is a safety net, not a guarantee: unusual secret formats will not be recognised.
- **Local-only mode.** Disables every cloud AI request: chat, the assistant, scan explanations, connection tests, model lists and vulnerability lookups. Still available: the catalog, installing and running tools, learning labs, saved conversations. **It does not make Vixalune network-silent**: `winget`, container engines, scanners that download databases (for example Trivy), and the tools you run can still use the network, and Windows or your antivirus may too.

## 3. Cost controls

Configurable in Settings → AI Providers: largest request, confirmation size, requests per session, and (per provider) maximum response length and timeout. Usage metadata is shown when the provider supplies it; when it does not, the answer carries a warning that size and cost are unknown. Vixalune never estimates a price. The session counter counts requests sent (automatic retries are not counted) and resets when the application restarts.

Not a spending cap: limits are in requests and characters, not money or tokens. Use your provider's own budget and rate limits as the real cap.

## 4. Review findings fixed in this phase

1. Error text from a provider could carry a local path (for example one echoed back from a message) → paths are now removed from all provider-facing error text.
2. No way to switch AI off or to cap spend → local-only mode, request-size limits, large-request confirmation and a session limit.
3. Secrets in pasted scan output or in a typed message were only caught when key-shaped → a broader masking pass was added (and the response shows how many values were masked).
4. Panic messages were printed unredacted → a panic hook redacts them, and no crash report is written.
5. A placeholder `Secret` was built from an empty string for public APIs → an explicit `Secret::none()`, so "no secret constructed outside the AI service" is checkable.
6. A package question produced a 24 000-character request (reference lists were sent unnecessarily) → the payload is slimmed; the default confirmation size was set to 60 000 characters so ordinary questions do not prompt.
7. Retention tests did not cover the structured-answer table → added, with a delete trigger.

## 5. Test evidence

Run in this repository (Windows 11, no provider key):

- Rust: `cargo test` — 423 passed, 0 failed, 7 ignored (opt-in live tests).
- Frontend: `npx vitest run` — 159 passed. `npm run test:e2e` (Playwright) and `npm run build` — see the release report.
- Opt-in live tests (not run in CI): real Windows Credential Manager round trip (**run, passed**); real OSV and NVD lookups for `lodash 4.17.15` (**run, passed**: 6 records); real AI providers (**not run** — no keys).

Coverage map for the release checklist: provider authentication and failure recovery (`authentication_failure_then_replacing_the_key_recovers…`, adapter suites), credential storage and deletion (service tests, `live_windows_credential_manager_round_trip`), secret redaction (above), prompt injection (above), unauthorized action attempts, file-sharing consent, invalid structured replies, excessive request sizes, cancellation, offline operation (`offline_operation_fails_clearly…`), conversation deletion, application upgrade (`an_upgrade_keeps_existing_conversations…` opens a database laid out like the previous release).

## Residual risks and untested areas

**High**
- **No live verification of the four provider adapters.** They follow the documented APIs and pass every test against a local mock that speaks each wire format, but a change or a misunderstanding on the provider side would only show up with a real key. Run `cargo test --lib live_ -- --ignored --nocapture` with your own keys before relying on any provider. Until that is done and the results are reviewed, treat each provider as unverified.
- **Same-user compromise.** Windows Credential Manager protects a key from other Windows users and from disk theft, not from malware or another program running as you, nor from someone using your unlocked session. The key is also in this process's memory while a request is being sent (wiped when the wrapper is dropped, but not from copies made by the HTTP stack), and Windows Error Reporting or a debugger could capture it. There is no Vixalune-side mitigation for this.
- **The model's prose can still be wrong or misleading.** Facts are checked against the application's records, but explanations are labelled model-written for a reason. A real vulnerability identifier can be attached to a wrong explanation.

**Medium**
- The application, installer and updates are not code-signed (see INSTALLATION.md).
- Retrieval is lexical; a question that shares no vocabulary with a tool's manifest finds nothing.
- `cargo audit` was not run (the tool is not installed here); `npm audit` reports no known vulnerabilities. Rust dependency advisories are unchecked.
- A compromised WebView could call the IPC commands (it cannot read a key, but could change settings such as lookup consent or local-only). The content-security policy is strict (`default-src 'self'`, no inline script, no remote connections) and no HTML from a model is rendered, which is the mitigation; there is no separate authentication between the window and the backend.
- Application *upgrade* is tested at the database level (a previous-release layout is opened and migrated). An end-to-end installer upgrade with AI settings present was not run in this phase.
- Size limits are in characters, session limits in requests; neither is a budget.
- `RUST_BACKTRACE` or a user-set `RUST_LOG=trace` can make dependencies log more than the defaults (which keep dependencies at `info`). Do not enable trace logging while a key is in use.

**Low**
- The per-session request counter is in memory and resets on restart.
- The explain-a-run limit is 400 lines; long scans are cut, and the cut is reported.

## Provider-specific limitations

- **OpenAI**: Responses API with `store: false`. Retention of API data is set by OpenAI's terms and your account settings, not by Vixalune.
- **Anthropic**, **Google Gemini**: data-use and retention differ by plan (for example free tiers may allow use of prompts to improve products). Check your plan before sending anything sensitive.
- **OpenRouter**: a router. Even with upstream fallbacks off (the default), your request goes to OpenRouter and the upstream model provider it selects, each with its own terms.
- Token usage and cost are reported only by providers that include them; OpenRouter reports a cost, the others tokens only.
- Models differ in how reliably they return the required JSON; a malformed reply is rejected and never repaired.

## What would be needed to call this production-ready

1. Run the live provider tests with real keys for all four providers (or remove the ones that cannot be verified), review failures, and re-record.
2. Install the signed installer on a clean Windows 10 and Windows 11 machine, upgrade over a build containing AI data, and verify settings and conversations survive.
3. Run `cargo audit` / `cargo deny` and review the results; pin and review the new dependencies (`reqwest`, `keyring`, `zeroize`, `react-markdown`, `rehype-highlight`).
4. Independent review of the WebView-to-backend boundary and of the prompt/fence design by someone other than the author.
