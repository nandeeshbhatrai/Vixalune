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

## 2b. Installation engine (Phase 3)

`engine.rs` (`InstallEngine`) implements `InstallationManager` and `JobRegistry`. It depends only on ports: `ToolRegistry`, `EnvironmentDetector`, `DependencyResolver`, `InstallerAdapter`s, `JobStore`, `ActivityLogger`.

```
install(tool) ─ start_job: manifest lookup, lifecycle + adapter checks, INSERT job (partial unique index: one active job per tool -> Busy)
  └ spawned task:
      queued -> checking_prerequisites   env detect, Windows build/arch, prerequisites, tool dependencies, adapter.preflight (winget show + list)
             -> awaiting_consent         only if the plan may need administrator rights and allow_elevation was false; waits for approve/cancel
             -> installing               adapter.install (fixed argv, streamed + redacted output, timeout, cancel kills the process tree)
             -> verifying                adapter.verify: EVERY manifest check must pass
             -> succeeded                installation record written (version, location, verification level)
  failure : -> [rolling_back if this job installed it] -> failed   (JobError: code, message, recovery, detail with exit code + last output)
  cancel  : -> cancelling -> [rolling_back] -> cancelled
  verify failed/unavailable -> failed, record state = verification_failed (tool is NOT rolled back; the user decides)
```

- **State machine**: `JobPhase::apply(JobEvent)` is the only way a phase changes; illegal transitions error (`domain/jobs.rs`, unit-tested).
- **Concurrency**: the unique index makes duplicate/overlapping requests fail with `Busy` atomically, even across processes; rejected requests create no job.
- **Rollback** only removes what the failed/cancelled job itself installed (never a pre-existing install), using a fresh cancellation token, and logs what it does.
- **Adoption**: installing an already-installed tool verifies it instead of reinstalling.
- **Finalize**: after a job that touched the system and did not succeed, the engine re-reads the real package state so the UI never shows a guess.
- **Persistence** (`adapters/sqlite_store.rs`, same SQLite file as the activity log, WAL): `jobs`, `job_output` (redacted, ≤ 2000 lines kept per job), `installations` (JSON record per tool). `recover()` runs at startup: jobs left active by a previous run become `failed` (`interrupted`) if they had touched the system, otherwise `cancelled`.
- **Detection**: `refresh_states()` asks each adapter whether a tool is present (`winget list --id … --exact`). "Unknown" stays unknown; a detected pre-existing install has no verification result.
- **Audit**: lifecycle events go to the activity log; each executed command is the first line of the job output (`> winget install --id … --exact …`).
- **UI progress**: the frontend polls `list_jobs` every 1.5 s while any job is active (no push events yet).

### Command execution (`adapters/executor.rs`)
Absolute program path only (refuses relative); `args` array, `stdin` closed, **environment cleared and rebuilt from an allow-list** (no tokens/API keys inherited); output split on `\n`/`\r`, redacted, capped; timeout and cancellation kill the whole process tree (`taskkill /T /F <pid>`). `winget.exe` is located by `SystemExecutableLocator` (registry PATH + process PATH, `.exe` only, scripts never).

### Adapters
| Adapter | State |
|---|---|
| `WingetAdapter` | Implemented and tested (mock executor + one real run) |
| WSL apt, container image, verified archive | Interfaces and manifest validation exist; **no adapter** — installs are refused with a clear message until each is implemented and tested |

## 2c. Platform integration (Phase 4)

```
SystemPlatformDetector ──(wsl.exe, docker.exe, winget.exe via CommandExecutor; registry/Win32 via HostFacts)──> PlatformReport
        │                    facts → readiness (pure, domain/platform.rs) → proposed SetupActions
        └─ also implements EnvironmentDetector (components used by dependency resolution)

InstallEngine ── adapters: WingetAdapter | WslAptAdapter | ContainerAdapter     (InstallerAdapter port)
ContainerToolRunner ── hardened `docker run` from manifest policy + user approvals  (ToolRunner port)
SetupEngine ── the only code that changes the system: WSL install (UAC), distro install, scheduled restart  (SetupManager port)
```

- **Detection vs readiness vs permission.** `PlatformReport` holds *facts* (unknown stays unknown); `readiness`
  (`ready` / `needs_setup` / `blocked` / `unknown`) is derived from them per capability (Windows tools, Linux tools in
  WSL, containers); `setup_actions` are *proposals* (`automatic` or `manual`). Running one needs `confirmed = true`
  and re-checks that it is still needed. Detection never elevates, never starts a stopped distribution, and reports
  Windows optional-feature state as *unknown* (reading it needs administrator rights).
- **Component view.** The engine's prerequisite check uses `PlatformReport::to_environment()`: `wsl2` is available only
  when a WSL 2 distribution is ready, `docker` only when a local Linux engine answers. A Docker CLI without a running
  engine never counts.
- **WSL adapter.** `wsl.exe --distribution <name> --user root --cd /tmp --exec <absolute program> <args…>`: configured or
  default WSL 2 distribution, never Docker's utility distributions, no shell, validated package names, `apt-get`
  non-interactive via `/usr/bin/env`. Failures are classified by layer (Windows/WSL vs the package manager vs network/disk).
  Cleanup = `apt-get remove` of the named packages only.
- **Container adapter.** Local Docker CLI only (remote contexts, `DOCKER_HOST`, Windows containers refused). Registry
  allow-list (default `docker.io`, `ghcr.io`, `quay.io`, `mcr.microsoft.com`, configurable in `config.toml`), **digest-only**
  references, `--pull never` at run time. Verification runs the tool in the same hardened container. Cleanup removes
  labelled containers and the image.
- **Hardened run** (`domain/container.rs`, enforced twice: when building and by `assert_hardened` on the final argv):
  numeric non-root user, `--cap-drop ALL`, `no-new-privileges`, read-only root, tmpfs for `/tmp`, memory/CPU/pids limits,
  network `none` unless approved, only declared bind mounts of validated, canonicalised folders (read-only by default),
  ports published on `127.0.0.1` only, never `--privileged`, host namespaces, devices, `--volumes-from`, the Docker
  socket or named pipe.
- **Approvals.** Network, host folders and host ports are supplied by the user per run in the Launch dialog, reviewed
  as an exact summary, and echoed back to the backend, which validates them again.
- **Failure diagnosis** (`domain/failure.rs`): `wsl_unavailable`, `virtualization_disabled`, `distribution_missing`,
  `container_runtime_stopped`, `insufficient_disk_space`, `package_download_failed`, `proxy_restricted`,
  `unsupported_architecture`, `missing_permissions`, `policy_restricted`, `remote_container_context`,
  `unsupported_container_os`, `package_manager_error`; each with recovery text and (where useful) an official link.
  Unrecognised output stays a plain command failure with the raw tail; it is never turned into success.
- **No silent fallback.** If WSL, the engine or a prerequisite is unavailable the job fails with a diagnosis; Vixalune
  never switches to another runtime (for example running a Linux tool natively or a container tool in WSL).
- **Jobs.** New phase `running` (a launched tool) and job kind `setup`. `RoutedJobs` sends cancellation to the owner
  (install engine or container runner). Setup jobs cannot be cancelled once the OS prompt is up.

### Unsupported or unverified combinations (explicit)

| Combination | Status |
|---|---|
| Install: winget | Works; verified for real (Trivy) |
| Install: WSL apt (Debian/Ubuntu family) | Implemented, **mock-tested only** (no WSL on the dev PC) |
| Install: container image (local Docker) | Implemented, **mock-tested only** (Docker engine cannot start on the dev PC) |
| Install: verified archive download | **Not implemented** (refused) |
| Launch: container profiles | Implemented, mock-tested |
| Launch: Windows or WSL profiles | **Not implemented** |
| WSL distributions without apt (Fedora, Arch, Alpine, openSUSE…) | Refused with a message; WSL 1 distributions refused |
| Podman, Rancher Desktop, remote engines, Windows containers, Hyper-V isolation | **Not supported** |
| arm64 Windows | Detected and allowed by manifests; **untested** |
| Non-English Windows | Detection works from exit codes and table shape; some diagnoses rely on English message text and degrade to a generic command failure |
| Windows older than build 19045 | Blocked |

## 3. Interfaces (src-tauri/src/ports.rs)

| Interface | Responsibility | Phase 1 implementation |
|---|---|---|
| `ToolRegistry` | List/search/get validated manifests with trust level | `ManifestRegistry` (dir of TOML) |
| `ToolManifest` | Typed, validated tool definition (`domain/manifest.rs`) | Done, with validation tests |
| `EnvironmentDetector` | Probe winget / WSL 2 / Docker, produce `EnvironmentReport` | `SystemEnvironmentDetector` |
| `DependencyResolver` | Choose install method, list missing prerequisites and elevation need | `BasicDependencyResolver` |
| `InstallerAdapter` | preflight / install / status / verify / uninstall for one `InstallSpec` kind | `WingetAdapter` |
| `InstallationManager` | Jobs for install/approve/cancel/retry/update/uninstall/verify, plans, detection | `InstallEngine` |
| `ToolRunner` | Validate parameters, build command preview, start/stop runs | Command builder done (`build_command`); runner is a placeholder |
| `ProcessManager` | Spawn without shell, stream output, terminate process tree | Trait + placeholder |
| `LabManager` | Start/stop pinned loopback-only container labs | Trait + placeholder |
| `ActivityLogger` | Persist redacted activity events | `SqliteActivityLogger` |
| `JobRegistry` | List jobs, read output, cancel | `InstallEngine` |
| `JobStore` (Phase 3) | Durable jobs, output, installation records | `SqliteJobStore` |
| `CommandExecutor` / `ExecutableLocator` (Phase 3) | Run a fixed argv with timeout/cancel; find `.exe` files | `SystemCommandExecutor` / `SystemExecutableLocator` |
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


