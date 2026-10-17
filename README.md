# Vixalune

A Windows desktop app for beginners to discover, install, run and learn about open-source security tools from one place. For **authorized testing and local labs only**.

Status: **Phase 2** — complete workbench UI (dashboard, catalog, installed tools, jobs, labs, activity, settings) on the Phase 1 core. Install, launch, verify, uninstall, jobs and labs are **not implemented in the backend yet**; the UI calls the defined backend interface and shows those actions as unavailable. See [docs/PRODUCT_SPEC.md](docs/PRODUCT_SPEC.md). Screenshots (demo data) are in `docs/screenshots/`.

| Doc | Purpose |
|---|---|
| [docs/PRODUCT_SPEC.md](docs/PRODUCT_SPEC.md) | Scope, requirements, platforms, limitations |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | Layers, interfaces, state machine, decisions |
| [docs/THREAT_MODEL.md](docs/THREAT_MODEL.md) | Threats and mitigations |
| [docs/TOOL_MANIFEST_SCHEMA.md](docs/TOOL_MANIFEST_SCHEMA.md) | Manifest format |

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

## Known limitations

See "Known limitations" in the [product spec](docs/PRODUCT_SPEC.md). Notably: no installing/running yet, sample manifests need package-id verification, no code signing or bundling.
