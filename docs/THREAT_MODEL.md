# Vixalune — Threat Model

Method: assets → trust boundaries → threats (STRIDE-flavoured) → mitigations → residual risk. Status column: ✅ implemented in Phase 1, 🔜 planned, ➖ policy/doc only.

## 1. Assets

- User's machine integrity (Vixalune installs software and can run security tools).
- User's privilege level (administrator rights).
- Local data: activity log, config, downloaded tool binaries.
- The user's reputation/legal position (tools aimed at unauthorised targets).

## 2. Trust boundaries

1. **Webview ↔ Rust core** (Tauri IPC). The webview is treated as less trusted than the core.
2. **Core ↔ external processes** (winget, wsl.exe, docker, installed tools).
3. **Core ↔ network** (manifest/package downloads; none in Phase 1).
4. **Core ↔ filesystem** (app data dir; user-chosen paths).
5. **Manifest files** — bundled (trusted by build) vs. downloaded (untrusted).

## 3. Threats and mitigations

| ID | Threat | Mitigation | Status |
|---|---|---|---|
| T1 | Malicious manifest runs arbitrary code (PowerShell/shell/JS/Rust) | Manifest is closed-schema data; no script fields; `deny_unknown_fields`; install methods are a closed enum; tests assert unknown keys are rejected | ✅ |
| T2 | Command injection via parameter values or manifest strings | No shell: executable + `Vec<String>` args only. Values validated per type; text/host values reject control chars and leading `-`; package ids, flags, executables charset-restricted; bare executable names (no paths) | ✅ |
| T3 | Option injection (`-oN evil`) via user input | Leading `-` rejected for text/host parameters; flags come only from the manifest | ✅ |
| T4 | Supply-chain tampering of downloaded archives/images | `download_archive` requires https + SHA-256; containers pinned by `sha256:` digest, tags rejected; winget versions optional pins | ✅ validation / 🔜 enforcement in adapters |
| T5 | Malicious/compromised remote manifest source | Remote manifests off by default (`remote_manifests_enabled=false`); when added: must pass the same validator, shown with `TrustLevel::RemoteUnsigned` warning, signature verification before `Signed` status | 🔜 |
| T6 | Silent privilege escalation | Core never runs elevated; `requires_elevation` is declared per manifest and shown before install; `allow_elevation` is an explicit parameter; elevation scoped to one child command via UAC | ➖ API ✅ / 🔜 implementation |
| T7 | Path traversal via IDs or archive entries | `ToolId`/`LabId` slug-validated at deserialization; executables must be bare names; archive extraction will canonicalise and reject entries escaping the tools dir | ✅ ids / 🔜 extraction |
| T8 | Secrets persisted in logs | `redact()` applied inside the logger on every write (key=value, `--flag value`, Bearer, URL credentials); 500-byte cap; tracing never logs parameter values; no secrets in config | ✅ |
| T9 | Compromised webview calls privileged APIs | Tauri capability `core:default` only; no shell/fs/opener plugins; all commands take typed, validated inputs; strict CSP (no remote scripts, `default-src 'self'`) | ✅ |
| T10 | Hang/DoS from misbehaving probes or tools | Probes have a 5 s timeout and output cap; resource limits in config (concurrent installs/processes, output buffer) | ✅ probes / 🔜 process limits |
| T11 | Orphaned processes after cancel/close | `ProcessManager::terminate` contract = kill process tree; Windows Job Objects planned | 🔜 |
| T12 | Misuse against unauthorised targets | Authorized-use notice in UI/docs; descriptions state scope; lab-first defaults; target allow-listing/confirmation planned | ➖ / 🔜 |
| T13 | Lab containers exposed to the network | Labs pinned by digest, ports published on `127.0.0.1` only (model + docs); enforced in `LabManager` | 🔜 |
| T14 | Tampered app config (e.g., malicious runtime path) | Config validated on load; `deny_unknown_fields`; executable paths are only used as the program of a structured spawn, never in a shell | ✅ validation |
| T15 | Dependency vulnerabilities | Lockfiles committed; plan `cargo audit` / `npm audit` in CI | 🔜 |
| T16 | Spoofed UI content from manifest text | Manifest text rendered as React text (escaped), never as HTML; control characters rejected | ✅ |

## 4. Residual risks and accepted limitations

- Bundled manifests are trusted because they ship with the signed build (no code signing in Phase 1).
- winget and apt packages are trusted to the extent their repositories are; Vixalune pins ids/versions but cannot audit packages.
- Security tools are inherently dual-use; Vixalune reduces accidental harm but cannot enforce legality.
- Redaction is best-effort heuristics; unusual secret formats may pass. Mitigated by not logging command lines or values.
- Phase 1 has no sandboxing of launched tools; they run with the user's rights.

## 5. Security requirements for later phases

1. Installers must pass argument arrays to `Command`, never `cmd /c` or `powershell -Command`.
2. Verify SHA-256 before extraction; extract with path canonicalisation and size/entry-count limits.
3. Show a confirmation dialog listing the exact command preview and elevation need before any install.
4. UAC elevation only for the specific install command; never keep an elevated helper alive.
5. Add signature verification for remote manifest bundles before enabling the feature.

## 6. Phase 3 update: installation engine

Implemented controls (all covered by unit tests; the winget path was also exercised for real):

| Threat | Control | Status |
|---|---|---|
| T1 scripts in manifests | `deny_unknown_fields` on every table **and** tagged variant; only the closed `InstallSpec` set exists; unknown adapter types fail to parse; tests try `powershell_script`, `shell`, `post_install_script`, `pre_install` | Done |
| T2/T3 injection | Adapter re-validates package id and pinned version before building argv; argv arrays only; executor refuses relative programs and clears the environment; tests feed `-s`, `; calc`, `&& del`, `$(x)`, newlines, quotes | Done |
| T4 integrity | Manifest `integrity.level` must match the install method. winget verifies installer hashes itself and the UI says Vixalune does not re-check them. `sha256`/digest pins are validated but their enforcement waits for those adapters | Partly (winget only) |
| T6 privilege | Vixalune never elevates. Plans state `none`/`possible`/`required`; a job that may need it waits in `awaiting_consent` unless the user ticked consent; UAC is raised by the installer, not by Vixalune | Done |
| T7 paths | `domain::paths` rejects `..`, absolute, drive, UNC, ADS, reserved device names, trailing dot/space; verification locations may only start with three known-folder placeholders; only `.exe` files are located | Done |
| T8 secrets | Output and audit text are redacted before storage; child processes get an allow-listed environment, so inherited tokens never reach installers | Done |
| T10 hangs | Per-command timeouts; cancellation kills the process tree | Done |
| New: concurrent corruption | Partial unique index allows one active job per tool, across processes | Done |
| New: false success | Success requires every verification check; `unavailable` ≠ `passed`; failed verification leaves the tool recorded as `verification_failed` | Done |
| New: crash mid-install | Startup recovery closes interrupted jobs as failed/cancelled and the next verify/retry re-reads real state | Done |

Known gaps and accepted risks:

- winget runs installers with the user's rights and may trigger UAC itself; for portable packages **winget (not Vixalune) may add a PATH entry** ("Path environment variable modified" was observed for Trivy). Vixalune never edits PATH, execution policy, firewall or security settings, and never touches antivirus.
- `--accept-package-agreements` is passed so installs are non-interactive; the install dialog states this and shows the license.
- Killing winget with `taskkill /T` ends the process tree but cannot undo what an installer already did; rollback only runs `winget uninstall` for packages this job installed.
- No Job Objects yet; a hard crash of Vixalune can orphan an installer process.
- The 1.5 s UI polling interval is the only progress channel (no push events).
