# Security policy

Vixalune is a desktop application that starts other programs, installs software and (on request) sends network traffic from your computer. It is built to make mistakes hard, not to be a sandbox. This page explains how to report a problem, what the application does to protect you, and what it does not do. The detailed analysis is in [SECURITY_REVIEW.md](SECURITY_REVIEW.md) and [THREAT_MODEL.md](THREAT_MODEL.md).

## Reporting a vulnerability

- Use GitHub's **private vulnerability reporting** for the repository (Security tab, "Report a vulnerability") if it is enabled for the repository.
- If it is not, open an issue that says only that you need a private security contact. **Do not put details, proof-of-concept code or logs in a public issue.**
- Include: the Vixalune version (Settings shows it), your Windows version, what you did, what you expected, what happened, and whether it needs a malicious catalog file, a malicious tool, or only normal use.
- There is no bug-bounty program and no guaranteed response time; this is a volunteer project. Reports that include a way to reproduce them locally get fixed first.

## Supported versions

Only the newest release receives fixes. Vixalune is below 1.0; behavior may change between minor versions.

## What Vixalune does for you

- **No shell, no scripts.** Programs are started with an argument list, from an absolute path, by Windows' process API. Tool definitions (manifests) are data, cannot contain scripts, and are validated when the app starts.
- **Typed parameters.** Values for tool runs are validated against their type (host, URL, port list, folder, choice, number). Values that could be read as options or that contain control characters or shell syntax are refused, not escaped.
- **Network-scope safeguards.** Runs default to this computer and the built-in practice lab. Other targets need confirmation; reserved addresses are refused; redirects that leave your lab are stopped.
- **No silent elevation.** Vixalune runs as a normal user. The only elevated step is installing WSL, through Windows' own UAC prompt, for one fixed command, and only from the System32 folder.
- **Contained processes.** Each tool runs in a Windows job object with a scrubbed environment and its own working folder; cancel, time-out and closing the app stop the whole process tree.
- **Hardened containers.** Container tools run as a non-root user, without network unless you approve it, with a read-only root filesystem, dropped capabilities, resource limits and only the folder you choose (read-only).
- **Pinned and verified installs.** Windows tools come from winget's community source (winget checks the installer hash); container images are pinned by digest. After installing, Vixalune runs the tool to check it works; a failed check is never reported as success.
- **Local data only.** Nothing is sent to the Vixalune project. There is no telemetry and no automatic update check. Output is stored locally, masked for obvious secrets, and expires according to your retention settings.
- **Damaged state is survivable.** A damaged database or settings file is moved aside (not deleted) and the app starts anyway.

## What Vixalune does not do

- It does not make a scan **authorized**. The scope checks catch mistakes; they cannot know whether you are allowed to test a target. That is your responsibility and may be a legal one.
- It does not sandbox tools. Native tools run with your user rights; WSL shares your Windows files and network; a container is not a safe place for hostile code. Use a separate virtual machine for that.
- It does not prove the authenticity of what it installs beyond what winget, apt and Docker provide. There is no independent signature check on catalog files, installers or images (see "Supply chain" below).
- It is not code-signed yet. Windows SmartScreen will warn when you run the installer. Check the SHA-256 shown on the release page against `Get-FileHash`.

## Supply chain

- **Application releases:** published on the project's GitHub releases page with a SHA-256 file. Releases are not yet signed with a code-signing certificate and are not yet verified to build bit-for-bit reproducibly (see [RELEASE_CHECKLIST.md](RELEASE_CHECKLIST.md)).
- **Catalog:** the tool catalog ships inside the application; there is no remote catalog and no catalog update channel. Each catalog file's SHA-256 is shown in Settings and written to the audit log with every install.
- **Updates:** you download and run a newer installer yourself. Vixalune never downloads or executes updater scripts. Updates to third-party tools are separate and are started by you.
- **Dependencies:** the lock files pin every Rust and npm dependency; `THIRD_PARTY_LICENSES.md` lists them. `npm audit` and an OSV query of `Cargo.lock` are run for each release; there is no continuous scanning yet.

## Safe use

Only test systems you own or have written permission to test. Do not point Vixalune at production systems, other people's networks or the public internet because a tutorial says so. Treat saved output as sensitive: it can contain addresses, banners and findings.

## AI provider keys and prompts

- API keys are stored only in Windows Credential Manager (no custom encryption, no file/database fallback; saving fails closed). They are never returned to the interface, logged, put in URLs or error messages, or kept in SQLite/settings.
- The `Secret` type has no `Debug`/`Display`/`Serialize` and is wiped on drop; provider errors are scrubbed of the key before display or storage; messages that contain key-shaped text are refused before sending.
- Redirects are disabled and provider addresses must be HTTPS, so a key cannot follow a redirect.
- Model output is untrusted: rendered as Markdown without raw HTML, images or live links; no tools are offered to a model, so output can never run a command.
- Residual risk: local malware or a compromised Windows session can use credentials the app can use. Prompts go to the cloud provider. See docs/AI_GATEWAY.md.

## Tool-aware assistant

- Model output is validated against the tool registry and the evidence the application collected; unknown tools, profiles, citations, links and vulnerability identifiers are removed and reported. See docs/ASSISTANT.md.
- No tools are offered to a model. The action set is closed and typed, has no install/run/command action, and authorization (model origin may only read catalog records; lookups need consent; runs need the person) is decided in code.
- Untrusted text (user messages, vulnerability descriptions, scan output) is cleaned, bounded and fenced with a per-request random marker, but the defence does not rely on the model obeying it.
- Package lookups send only ecosystem, name and version to OSV/NVD, and only after consent. Scan output is shared only after the person confirms, with credential-looking lines withheld, and is not stored in the conversation.

## AI hardening

Local-only mode, request-size/confirmation/session limits, secret masking, and path redaction in provider errors: see AI_SECURITY.md and PRIVACY.md.
