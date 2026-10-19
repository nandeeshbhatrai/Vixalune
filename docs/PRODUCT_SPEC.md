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
| F5 | Run interface: structured parameters, command preview, streaming output, termination | Phase 5: Run a Tool page with preview, scope checks, live output, cancel, history |
| F6 | Learning section: tutorials and local lab exercises | Phase 5: tutorials for four tools, practice web app and sample project |
| F7 | Settings: storage, runtime paths, resource usage | `AppConfig` model + TOML load/save implemented; UI later |
| F8 | Local activity log without secrets | Working (SQLite, redaction on write) |
| F9 | Clear errors with recovery instructions | Working: every `AppError` has a code and recovery text; UI renders both |
| F10 | Separation of UI / logic / adapters / tool definitions | Enforced by module layout (see ARCHITECTURE.md) |

### Phase 2 UI status

All seven screens exist: Dashboard, Tool Catalog (search + category/platform/execution-method/status filters + sort), Tool detail, Installed Tools, Running Jobs, Learning Labs, Activity, Settings (theme, app info). Every action calls a `Backend` method or is shown disabled with a reason. In this build the core reports `install`, `uninstall`, `verify`, `launch`, `jobs`, `labs` and `installed_detection` as unavailable, so the real app shows "not detected yet" / "not available yet" states instead of numbers. Demo data exists only behind `VITE_DEMO_DATA=true` and is labelled everywhere it appears.

### Phase 3 status

Install, cancel, retry, update, uninstall and verify now run as persisted, audited jobs through the winget adapter (F4 delivered for winget). Progress, output, exit codes and recovery text reach the UI; state survives restarts. Launching tools, labs, and non-winget adapters remain future work.

### Phase 4 status

Environment readiness (Windows, virtualization, WSL + distributions, container engine, winget, disk, restart/proxy/policy hints), the first-run Setup wizard (check, explain, list missing, guided or confirmed steps, restart control, re-check), a WSL apt adapter, a local-Docker container adapter and a hardened container launcher. Supported setup changes: install WSL (UAC, explicit), install Ubuntu (explicit), schedule/cancel a restart (explicit). Everything else is guidance with official links. See ARCHITECTURE.md for the unsupported-combination table.

### Phase 5 status

Tools can now be run from the **Run a Tool** page: pick a tool and one of its ready-made launch profiles, fill the typed form (defaults point at the local lab), review the exact command, target, scope, limits and expected output, tick the review box (and, for targets outside your lab, confirm authorisation), then Run. Output streams live; the run ends as *completed, nothing reported*, *completed with findings*, *failed*, *timed out* or *cancelled*. You can search and copy output, save it to a location you choose (Save dialog), cancel, open the run folder and look through recent runs. The lab list and data-retention periods are in Settings.

**Learning:** a tutorial for Nmap, OWASP ZAP, Semgrep and Trivy (what it does, the problem it solves, requirements, first run, parameter meanings, example output and how to read it, common errors, a practice exercise, official links, and the authorization boundary first). **Learning Labs** now run a deliberately vulnerable web application (127.0.0.1 only) and create a sample project of insecure files that are never executed.

Verified in this build: command building, validation, scope enforcement (including a real redirect from the practice server), execution failure/timeout/cancel paths, history and retention, save-path validation and the practice server (real sockets) with simulated tool processes; the UI with the demo backend. **Not verified on real hardware:** real Nmap, ZAP and Trivy runs (see MANUAL_INTEGRATION_TESTS.md section G), Semgrep in a real container, and the ZAP command-line flags.

## 6a. Phase 5 requirements and where they are met

| Requirement | Where |
|---|---|
| Typed launch profiles with safe defaults, limits, scope and network flags | `RunSpec`, TOOL_MANIFEST_SCHEMA.md |
| Argv built only from validated fields, human-readable preview | `build_command_with`, Run page preview |
| Execution through the backend only; live output, exit code, duration, cancel | `NativeToolRunner`, `ContainerToolRunner`, `OutputPanel` |
| Output search, copy, save to a chosen location, recent history | `OutputPanel`, `ResultSaver`, `RunHistory` |
| Failed distinguished from successful-with-findings | `RunOutcome`, `classify_outcome` |
| Scope: show target first, lab default, confirmation outside the lab, configurable allow-list, resolved addresses and redirects, alias/URL tricks, no auto-elevation | `domain/scope.rs`, `RunSupport`, THREAT_MODEL.md section 8 |
| Learning mode per tool, tutorials for the four tools | `learning/*.toml`, Learn pages |
| Local practice targets | `practice_lab.rs` |
| Persistence with retention; no secrets kept indefinitely | `runs`/`job_output` tables, `settings.json`, redaction |

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


