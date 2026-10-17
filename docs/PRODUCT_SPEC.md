# Vixalune — Product Specification

Status: Phase 1 (specification and scaffold). Version 0.1.

## 1. Purpose

Vixalune is a **local Windows desktop application** that lets beginner security learners discover, install, manage, launch and learn about open-source penetration-testing and security-auditing tools from a single interface. It hides the differences between Windows-native packages (winget), Linux tools in WSL 2, and container images.

It is **not** a remote scanning service, has no server component, and sends no scan data anywhere.

## 2. Intended use and boundaries

- Authorized security testing, education, and intentionally vulnerable **local** lab environments.
- Every tool page and the app shell show an authorized-use notice. Run targets default to loopback/lab addresses in later phases.
- Out of scope: hosting scans for others, stealth/evasion features, exploit-development tooling, bundling malware or credential dumps, automatic updates of tools without user action.

## 3. Users

| Persona | Needs |
|---|---|
| Beginner learner | One place to install tools, safe defaults, plain-language errors, guided labs |
| Instructor | Reproducible lab environment, curated tool list |
| Hobbyist / junior tester | Fast tool setup without learning winget/WSL/Docker commands |

## 4. Supported platforms and assumptions

- **Supported:** Windows 11, and Windows 10 22H2, x64.
- **Assumptions:** WebView2 runtime present (ships with Windows 11 and current Windows 10); winget available for native installs; WSL 2 and Docker are optional and only required by tools/labs that declare them.
- **Not supported in this phase:** macOS, Linux hosts, ARM64 Windows (untested), offline-only installs.
- Phase 1 was developed and verified on Windows 11 only.

## 5. Functional requirements

Phase column: **1** = delivered as scaffold/skeleton in this phase; later phases implement behaviour.

| # | Requirement | Phase 1 state |
|---|---|---|
| F1 | Dashboard: environment readiness, installed tools, recent activity, setup issues | Working: readiness + issues + activity; installed count placeholder |
| F2 | Searchable, filterable catalog | Working (text + category) over bundled manifests |
| F3 | Tool details: description, category, platforms, install method, prerequisites, license, docs, verification status | Working except verification status (needs installer, phase 2) |
| F4 | Install, cancel, retry, update, uninstall, verify | Interfaces + state machine; adapters return `NotImplemented` |
| F5 | Run interface: structured parameters, command preview, streaming output, termination | Command builder + validation implemented and tested; UI/streaming later |
| F6 | Learning section: tutorials and local lab exercises | Placeholder page; `LabDefinition` model defined |
| F7 | Settings: storage, runtime paths, resource usage | `AppConfig` model + TOML load/save implemented; UI later |
| F8 | Local activity log without secrets | Working (SQLite, redaction on write) |
| F9 | Clear errors with recovery instructions | Working: every `AppError` has a code and recovery text; UI renders both |
| F10 | Separation of UI / logic / adapters / tool definitions | Enforced by module layout (see ARCHITECTURE.md) |

## 6. Non-functional requirements

- **Safety:** no shell string construction; manifests are data only; no silent elevation (THREAT_MODEL.md).
- **Privacy:** all data stays on the machine; no telemetry; activity log redacts secrets.
- **Reliability:** every long operation has an explicit state machine, is cancellable, and leaves a recoverable state.
- **Accessibility:** keyboard-navigable, semantic landmarks, light/dark via OS preference.
- **Testability:** domain logic is pure; adapters are behind traits; UI depends on a `Backend` interface with a mock.

## 7. Key user flows (target behaviour)

1. **First launch:** dashboard runs environment detection; missing optional components appear as setup issues with exact fix steps.
2. **Install a tool:** catalog → tool details → Install. The resolver shows the chosen method and any missing prerequisite or required elevation *before* anything runs. User confirms; progress streams; verification runs automatically.
3. **Run a tool:** fill typed parameters → live command preview → Run → streamed output → Stop.
4. **Lab exercise:** start a pinned, loopback-only container lab → follow tutorial → stop lab.

## 8. Success criteria for Phase 1

See "Acceptance criteria" in the README and the results summarized at the end of the Phase 1 hand-off.

## 9. Known limitations (Phase 1)

- No installation, run, process or lab behaviour yet — only interfaces, state machine, validation and the read-only catalog/dashboard.
- Only three sample manifests; tool metadata (package ids, licenses) must be re-verified before release.
- No code signing, installer packaging, or auto-update.
- No remote manifest source; `remote_manifests_enabled` is forced to `false` in practice.
- Playwright tests exercise the web frontend against the mock backend, not the packaged Tauri window.
