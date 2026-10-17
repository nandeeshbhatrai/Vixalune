# Vixalune — Architecture

## 1. Stack

| Concern | Choice |
|---|---|
| Shell | Tauri 2 (Rust `tauri = "2"`, `@tauri-apps/api` 2.x) |
| Frontend | React 19, TypeScript, Vite 8, Tailwind CSS 4, shadcn/ui-style components (Radix Slot + cva) |
| Core | Rust 2021 edition |
| Persistence | SQLite via `rusqlite` (bundled); config as TOML |
| Tool definitions | TOML manifests in `manifests/` |
| Tests | Vitest + React Testing Library; `cargo test`; Playwright |

Exact resolved versions are recorded in `package-lock.json` and `src-tauri/Cargo.lock`.

## 2. Layers

```
React UI (src/pages, src/components)
   │ depends only on  `Backend` interface  (src/domain/backend.ts)
   ▼
Backend adapters   tauriBackend (invoke)  |  mockBackend (tests, browser dev)
   │ IPC
   ▼
commands.rs        thin typed IPC handlers; AppState = DI container
   │ depends only on traits
   ▼
ports.rs           ToolRegistry, EnvironmentDetector, DependencyResolver, InstallerAdapter,
                   InstallationManager, ToolRunner, ProcessManager, LabManager, ActivityLogger
   ▲ implemented by
adapters/          ManifestRegistry, SystemEnvironmentDetector, BasicDependencyResolver,
                   SqliteActivityLogger, Unimplemented* (phase 1 placeholders)
   │ use
   ▼
domain/            pure types, validation, state machine, redaction. No I/O, no Tauri.
```

Rules: `domain` imports nothing from other layers; `ports` imports only `domain`; `adapters` implement `ports`; `commands`/`lib.rs` wire concrete adapters into `AppState` (constructor injection via `Arc<dyn Trait>`). The UI never calls `invoke` directly outside `src/infra`.

## 2a. Frontend structure (Phase 2)

```
src/domain/        types.ts (wire mirror), backend.ts (Backend interface), toolStatus.ts, catalog.ts  - pure logic, no React
src/presentation/  labels.ts (text, tones, icons)                                                     - display strings only
src/infra/         tauriBackend | demoBackend | unavailableBackend, backendContext (selectBackend)
src/hooks/         useWorkspace (catalog, capabilities, states, environment), useAction, useAsync, useTheme
src/components/    reusable pieces (StatusBadge, ToolCard, ToolActions, ConfirmDialog, AsyncView, ...)
src/pages/         one file per screen
```

- **Backend selection** (`selectBackend`): Tauri → `tauriBackend`; otherwise `createDemoBackend()` only when `VITE_DEMO_DATA=true` (`npm run dev:demo`); otherwise `unavailableBackend`, whose every call fails with a recovery message. Production builds therefore never contain a path that fabricates status. The demo backend sets `isDemo`, which shows a global banner and per-value "Demo data" tags.
- **Capabilities**: the core reports which features exist (`get_capabilities`). `actionsFor()` disables any action whose capability is false and shows why; no button is a silent no-op.
- **Honest status**: backend `InstallState | null` (null = cannot be determined) → `deriveStatus()` → `unknown`, `not_installed`, `ready_to_install` (only with a checked environment and no missing prerequisites), `installing`, `installed`, `verification_failed`, `update_available`, `uninstalling`, `error`.
- **Links**: the UI asks the backend to open a link by `(tool_id, kind)`; the URL comes from the validated manifest and only `https` is opened (`rundll32 url.dll,FileProtocolHandler`, no shell).
- Components never call `invoke`, run commands or access the filesystem.

## 3. Interfaces (src-tauri/src/ports.rs)

| Interface | Responsibility | Phase 1 implementation |
|---|---|---|
| `ToolRegistry` | List/search/get validated manifests with trust level | `ManifestRegistry` (dir of TOML) |
| `ToolManifest` | Typed, validated tool definition (`domain/manifest.rs`) | Done, with validation tests |
| `EnvironmentDetector` | Probe winget / WSL 2 / Docker, produce `EnvironmentReport` | `SystemEnvironmentDetector` |
| `DependencyResolver` | Choose install method, list missing prerequisites and elevation need | `BasicDependencyResolver` |
| `InstallerAdapter` | Execute one `InstallSpec` kind (winget, wsl_apt, container, archive) | Trait only |
| `InstallationManager` | Own per-tool `InstallState`, orchestrate adapters, cancel/retry/update/uninstall/verify | Trait + placeholder |
| `ToolRunner` | Validate parameters, build command preview, start/stop runs | Command builder done (`build_command`); runner is a placeholder |
| `ProcessManager` | Spawn without shell, stream output, terminate process tree | Trait + placeholder |
| `LabManager` | Start/stop pinned loopback-only container labs | Trait + placeholder |
| `ActivityLogger` | Persist redacted activity events | `SqliteActivityLogger` |
| `JobRegistry` (Phase 2) | List jobs, read output, cancel | Placeholder (empty list; cancel = `NotImplemented`) |
| `LinkOpener` (Phase 2) | Open an https catalog link in the default browser | `SystemLinkOpener` |

`InstallationManager` also gained `states()` and `installed()` read models for the UI; the placeholder returns empty lists, which the UI presents as "not detected yet", never as "nothing installed".

Async traits use `async-trait` so they remain object-safe (`Arc<dyn …>`); detection and logging are synchronous and invoked via `spawn_blocking` where they touch processes.

## 4. Explicit state machine

`InstallState::apply(InstallEvent)` is the only way to change install state; illegal transitions return `AppError::InvalidTransition`.

```
NotInstalled ─Enqueue→ Queued ─BeginDownload→ Downloading ─BeginInstall→ Installing ─InstallFinished→ Verifying ─VerifyPassed→ Installed
Queued ─BeginInstall→ Installing           Verifying ─VerifyFailed→ Failed
Queued/Downloading/Installing ─RequestCancel→ Cancelling ─CancelConfirmed→ Cancelled
Failed/Cancelled ─Retry→ Queued            Installed ─UpdateDetected→ UpdateAvailable ─Enqueue→ Queued
Installed/UpdateAvailable/Failed ─BeginUninstall→ Uninstalling ─UninstallFinished→ NotInstalled
Installed/UpdateAvailable/VerificationFailed ─VerifyRequested→ Verifying      any busy state ─Fail→ Failed
Verifying ─VerifyFailed→ VerificationFailed (distinct from Failed; retryable and uninstallable)
```

## 5. Command execution model

- A manifest yields `RunSpec` (executable bare name + typed parameters + `ArgTemplate` list).
- `build_command` validates each value against its `ParamKind`, rejects unknown/missing parameters and leading `-` in text values, and returns `{ program, args[] }`. A `display` string exists **only for UI preview**.
- `ProcessSpec { program: PathBuf, args: Vec<String>, … }` is passed to `std::process::Command::args` / `tokio::process` — never through `cmd.exe /c` or `powershell -Command`.
- WSL: `wsl.exe --distribution <name> --exec <program> <args…>` (`--exec` avoids shell interpretation). Containers: `docker run … -- <image@digest> <args…>`. (Phase 2.)
- Elevation: `InstallationManager` methods take an explicit `allow_elevation` flag; adapters that need it use a single, narrowly scoped elevated child (UAC) for that one command and nothing else. Never inherited by the app process.

## 6. Data and storage

| Item | Location (default `%APPDATA%\app.vixalune.desktop`) |
|---|---|
| Config | `config.toml` (`AppConfig`; paths and limits only) |
| Activity log | `vixalune.db` table `activity` |
| Installed tools (archives) | `tools\` |
| Lab data | `labs\` |
| Bundled manifests | `manifests\` next to the app (resource); `../manifests` in dev |

## 7. Error handling

`AppError` (thiserror) is the single error type crossing IPC; it serializes to `{ code, message, recovery }`. Every variant has a non-empty recovery instruction. The UI renders failures through `AsyncView`.

## 8. Logging

- `tracing` for developer diagnostics (stdout; `RUST_LOG` to adjust). Parameter values are never logged.
- User-facing activity log in SQLite; `redact()` runs inside `SqliteActivityLogger::record`, so callers cannot bypass it; summaries are truncated to 500 bytes.

## 9. Testing strategy

| Level | Tooling | Covers |
|---|---|---|
| Rust unit | `cargo test` | IDs, manifest validation, injection rejection, state machine, redaction, registry, config, SQLite logger, resolver |
| Frontend unit/component | Vitest + RTL | Pages against `mockBackend`, error/recovery rendering |
| E2E | Playwright (system Edge) | Navigation against Vite dev server + mock backend |
| Tauri-window E2E | Not in phase 1 | Planned (`tauri-driver`/WebDriver) |

## 10. Key decisions

1. **Traits as ports, `Arc<dyn>` injection** — easy to fake in tests; swaps adapters per platform.
2. **Manifests are closed-schema data** (`deny_unknown_fields`, closed enum of install methods) — structurally cannot hold scripts.
3. **Validate-don’t-escape** — hostile argument values are rejected, not quoted.
4. **Frontend `Backend` interface with a mock** — UI is testable and demoable without Rust.
5. **shadcn/ui primitives copied in by hand** (Button, Card, Badge) using the same Radix + cva pattern, because the shadcn CLI needs network scaffolding; `components.json` is present so `npx shadcn add` works later.
6. **TOML for manifests** — comment-friendly for contributors, no code execution; JSON stays the IPC format.
7. **Minimal Tauri capabilities** — `core:default` only; no shell/fs/opener plugins exposed to the webview.
