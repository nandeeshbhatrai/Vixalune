# Vixalune

A Windows desktop app for beginners to discover, install, run and learn about open-source security tools from one place. For **authorized testing and local labs only**.

Status: **Phase 4** — platform integration. Environment readiness, a first-run Setup wizard, and adapters for winget, WSL 2 (apt) and local Docker containers on top of the Phase 3 engine. Only the winget path has been exercised against real software; the WSL and container paths are **mock-tested** because the development PC cannot run them (see [docs/MANUAL_INTEGRATION_TESTS.md](docs/MANUAL_INTEGRATION_TESTS.md)). Launching Windows/WSL tools and the verified-archive adapter are **not implemented**. See [docs/PRODUCT_SPEC.md](docs/PRODUCT_SPEC.md). Screenshots (demo data) are in `docs/screenshots/`.

| Doc | Purpose |
|---|---|
| [docs/PRODUCT_SPEC.md](docs/PRODUCT_SPEC.md) | Scope, requirements, platforms, limitations |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | Layers, interfaces, state machine, decisions |
| [docs/THREAT_MODEL.md](docs/THREAT_MODEL.md) | Threats and mitigations |
| [docs/TOOL_MANIFEST_SCHEMA.md](docs/TOOL_MANIFEST_SCHEMA.md) | Manifest format |
| [docs/MANUAL_INTEGRATION_TESTS.md](docs/MANUAL_INTEGRATION_TESTS.md) | Manual Windows tests for WSL, containers and setup |

## Prerequisites (Windows 11 / Windows 10 22H2, x64)

| Requirement | Verified with | Install |
|---|---|---|
| Node.js ≥ 22 | 24.20.0 (npm 11.19.0) | `winget install OpenJS.NodeJS.LTS` |
| Rust (stable, MSVC) ≥ 1.85 | 1.99.1 | `winget install Rustlang.Rustup` then `rustup default stable` |
| MSVC C++ Build Tools | VS 2022 Build Tools, MSVC 14.44 | `winget install Microsoft.VisualStudio.2022.BuildTools --override "--passive --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"` (needs admin) |
| WebView2 Runtime | preinstalled on Windows 11 | https://developer.microsoft.com/microsoft-edge/webview2/ |
| Microsoft Edge (for Playwright e2e only) | system Edge | — |

Open a **new terminal** after installing so `PATH` updates.

## Commands

```bash
npm install                 # install frontend + Tauri CLI
npm run tauri:dev           # start the desktop app (Vite + Rust, hot reload)
npm run dev:demo            # frontend only, in a browser, with clearly labelled DEMO data (http://localhost:1420)
npm run dev                 # frontend only without a backend: screens show "backend not running" errors

npm run typecheck           # TypeScript
npm test                    # Vitest + React Testing Library
npm run rust:check          # cargo check
npm run rust:test           # Rust unit tests
npm run test:e2e            # Playwright (starts the demo server itself; uses system Edge)
SCREENSHOTS=1 npm run test:e2e   # also refreshes docs/screenshots (PowerShell: $env:SCREENSHOTS=1)
npm run build               # typecheck + production frontend build
npm run tauri:build         # production build (bundling is disabled in tauri.conf.json for now)
```

The first `tauri:dev` compiles ~500 crates (a few minutes); later runs are incremental.

## Layout

```
manifests/        tool definitions (TOML, data only)
docs/             spec, architecture, threat model, manifest schema
src/              React UI
  domain/         TS types mirroring Rust + Backend interface
  infra/          Tauri and mock Backend implementations
  pages/ components/ hooks/
src-tauri/src/
  domain/         pure types, validation, state machine, redaction
  ports.rs        trait interfaces
  adapters/       implementations of the ports
  commands.rs     IPC handlers + dependency-injection container
e2e/              Playwright tests
```

## Data location

`%APPDATA%\app.vixalune.desktop\` — `config.toml`, `vixalune.db` (redacted activity log). No telemetry; nothing leaves the machine.

## What genuinely works (Windows 11, verified 2026-10-17)

- **Adapters**: winget (real-tested), WSL apt and local Docker (mock-tested; see below). Verified-archive installs are refused.
- **Tool proven end to end on real Windows**: **Trivy** — real `winget install`, verification by running `trivy.exe --version` plus winget's package record, persisted record, then real uninstall (see the integration test below).
- **Tools whose manifests are validated and use the same tested adapter, but were not installed here**: Nmap, Wireshark, OWASP ZAP. Their install folders/verification commands are unproven until exercised.
- **Nikto (WSL) and Semgrep (container)**: complete workflows implemented and tested end to end against a simulated `wsl.exe`/`docker.exe` (install, verify, version tracking, hardened run, cancel, cleanup, every diagnosis). **Not yet run against real WSL/Docker**: this PC has no WSL and its Docker Desktop engine cannot start without it. Do the manual steps in [docs/MANUAL_INTEGRATION_TESTS.md](docs/MANUAL_INTEGRATION_TESTS.md).
- **Environment detection is real**: the read-only integration tests ran on this PC and correctly reported WSL not installed and Docker installed-but-stopped.
- Real read-only detection (`winget list`) runs for all winget tools at startup.

## Tests

```bash
npm test                      # 94 frontend tests
cargo test --manifest-path src-tauri/Cargo.toml     # 185 Rust unit tests (simulated winget/wsl/docker, real SQLite)
npm run test:e2e              # Playwright, demo data
```

Opt-in tests against the real winget (never run by default):

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test winget_integration -- --ignored read_only      # read-only
cargo test --manifest-path src-tauri/Cargo.toml --test platform_integration -- --ignored --nocapture # read-only: WSL, Docker, winget, disk
$env:VIXALUNE_INTEGRATION_INSTALL = "1"
cargo test --manifest-path src-tauri/Cargo.toml --test winget_integration -- --ignored --nocapture real_install   # installs then removes Trivy
```

## Known limitations

See "Known limitations" in the [product spec](docs/PRODUCT_SPEC.md). Notably: WSL and container paths are untested against real runtimes, only container tools can be launched, Nmap/Wireshark/ZAP installs are untested on real hardware, and there is no code signing or bundling.

Windows Smart App Control (when set to enforce) blocks freshly compiled unsigned programs, including cargo build scripts and test binaries. Developing Vixalune then requires turning it off yourself (it cannot be re-enabled without resetting Windows) or building elsewhere; Vixalune never changes this setting.




