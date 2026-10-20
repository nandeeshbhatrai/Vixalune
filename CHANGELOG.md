# Changelog

All notable changes to Vixalune. Versions follow semantic versioning while the project is below 1.0: minor versions may change behavior.
Application updates (this file) are separate from updates to the third-party tools Vixalune installs.

## Unreleased

Added
- Discover: search the winget community repository from the Tool Catalog and add a package as a labelled, install-only community entry (not reviewed, no run profiles, never recommended by the AI Assistant). Seven more reviewed catalog entries: Gitleaks, OSV-Scanner, Grype, Syft, mitmproxy, Autopsy and Angry IP Scanner (validated against real `winget show` output; not yet installed or run on a test machine).
- AI gateway with bring-your-own-key for OpenAI, Anthropic, Google Gemini and OpenRouter (see docs/AI_GATEWAY.md). Keys are stored only in Windows Credential Manager and are never returned to the interface; if secure storage is unavailable, saving fails.
- Settings → AI Providers (key, model, connection test, timeout/temperature/length, retention) and an AI Assistant screen (streaming, stop, retry, Markdown with code highlighting, conversation history).

- Tool-aware assistant (see docs/ASSISTANT.md): recommendations grounded in the real tool registry with live installation status, a package vulnerability workflow using OSV and NVD (off until you allow it), explanation of scan-run output (only after you confirm what is sent), and strict validation of everything a model returns. Cards offer View Details, Check Prerequisites, Install and Prepare Scan, which use the existing confirmation flows.

- Security hardening of the AI features (docs/AI_SECURITY.md): local-only mode, request size limit, confirmation of large requests, per-session request limit, masking of secrets in pasted text and shared scan output, removal of local paths from provider error text, redacted panic messages, and a source-level check that only the AI service reads credentials.
- Privacy documentation (docs/PRIVACY.md).

Known limitations
- Not production-ready: no AI provider has been verified live; see docs/AI_SECURITY.md for residual risks.
- The tool-aware assistant's model-facing behavior is tested with scripted models only; OSV and NVD were verified live. Retrieval is lexical.
- The four provider adapters are implemented against the documented APIs and tested against local mock servers; none has been verified with a real key by the project. Opt-in live tests are provided.
- Prompts are sent to the cloud provider you choose and billed by it. The assistant cannot run commands.

## 0.2.0 - first installable release candidate

Added
- Windows installer (NSIS, per-user install, no administrator rights needed). Your data lives in `%APPDATA%\app.vixalune.desktop` and is kept when you update or uninstall (the uninstaller asks).
- First-run welcome tour: what Vixalune is, supported environments, readiness checks, storage locations, permission prompts and network-scope safeguards, the catalog, and a few starter tools.
- Settings: version, release notes, update guidance, catalog information (manifest versions and digests), storage locations.
- Application and catalog versioning information in the audit log; database schema version with an automatic backup before an upgrade.
- `VIXALUNE_DATA_DIR` to choose the data folder (portable or test installs).

Fixed
- A run cut short by a crash is shown as failed after the next start instead of "running" forever.
- Tab-separated tool output (for example Trivy log lines) keeps its fields apart.

Changed
- Stale "installing" states left by an interrupted operation are shown as failed.
- A damaged database or settings file is moved aside instead of stopping the application.

Known limitations
- The installer and the application are not code-signed; Windows SmartScreen will warn on first run. Builds are not reproducible (see docs/INSTALLATION.md).
- There is no automatic updater. To update, install the newer version over the old one (see docs/INSTALLATION.md).
- Trivy (install, run, cancel, uninstall) and Semgrep in Docker were verified on a real Windows 11 PC. Real Nmap, OWASP ZAP, Wireshark and WSL (Nikto) installs and runs have not been verified; see docs/MANUAL_INTEGRATION_TESTS.md and docs/RELEASE_CHECKLIST.md.

## 0.1.0 - development builds (phases 1 to 6)

Scaffold, catalog and interface, installation engine (winget), WSL and container support, tool execution with scope safeguards, learning labs and tutorials, security review and hardening. Not distributed.
