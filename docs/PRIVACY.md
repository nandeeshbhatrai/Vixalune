# Privacy

Vixalune has no accounts, no analytics, no telemetry and no crash reporting. It does not phone home. This page lists the only things that can leave your computer, and what stays.

## What stays on your computer

- The tool catalog, installation records, run history and output, activity log, learning progress and your settings (in the data folder, see INSTALLATION.md).
- Your AI conversations (SQLite, in the same data folder), kept for the number of days you choose (default 30; 0 keeps nothing) and deletable at any time.
- Your AI provider API keys, in **Windows Credential Manager** under your Windows account. Never in the database, a settings file, a log or the interface.

## What can leave your computer, and when

| What | Where to | When | Controlled by |
|---|---|---|---|
| Your messages, the conversation so far and the catalog records of the tools considered | The AI provider you configured (OpenAI, Anthropic, Google, or OpenRouter and the model provider it chooses) | Each time you ask the assistant | You choose the provider; **Local-only mode** stops all of it |
| Part of a scan run's output (≤ 400 lines, passwords/tokens/keys masked) | The same provider | Only when you press *Explain a scan run* and confirm | Per-request confirmation |
| Ecosystem, package name and version (or a vulnerability id) | OSV (`api.osv.dev`) and, for CVE ids, NVD (`services.nvd.nist.gov`) | When you ask about a package, **only after you allow lookups** (off by default) | Setting; Local-only mode |
| Your search text | Microsoft's winget package service | When you search in Discover (not an AI request; Local-only mode does not block it) | You choose whether to search |
| Downloads of tools and their data | Package managers and tool vendors (winget, Docker registries, Trivy's database…) | When you install or run those tools | Those tools; **not** controlled by Local-only mode |
| Opening a link | Your default browser | When you click a documentation or key-page link | You |

Vixalune never sends API keys to anyone but the provider they belong to, never sends environment variables, never uploads project folders or files, and never sends scan output on its own.

## Cloud AI is not local

When you use the AI Assistant, your text is transmitted to a third party, which may log it, may use it according to your plan's terms, and bills your account. Do not put in anything you are not allowed to share. Vixalune masks recognisable secrets in what it sends, but that is best effort.

## Local-only mode

Turns off every cloud AI request and vulnerability lookup. It does **not** stop installers, scanners or other tools from using the network, and it cannot make a guarantee about your operating system or other software. It exists so you can use the catalog, installer and runner with no AI traffic from Vixalune.

## Your controls

Settings → AI Providers: add/replace/remove a key, local-only mode, vulnerability lookups, request size and confirmation limits, requests per session, retention. Assistant: delete one conversation or all. Settings → Scope and retention: how long run history and output are kept.

See [AI_SECURITY.md](AI_SECURITY.md) for how these are enforced and what is not covered.
