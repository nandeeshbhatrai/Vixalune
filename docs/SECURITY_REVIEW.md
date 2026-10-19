# Vixalune: Security Review (Phase 6)

Scope: the Rust core (`src-tauri`), the bundled manifests and tutorials, the Tauri configuration and the React front end, reviewed as of this commit on Windows 11.
Method: reading every process-creation, file-system, persistence and trust path; writing failing tests for suspected weaknesses; fixing what failed; re-running everything.
This is a defensive review by the developer of the code, not an independent audit. Passing tests show that specific failure modes are covered; they do not show that the application is secure.

## 1. Summary

| ID | Severity | Finding | Status |
|---|---|---|---|
| F1 | High | Child processes were not contained: descendants that detached, or that were started before a tree-kill ran, survived cancel, timeout and application exit | Fixed |
| F2 | High | A manifest could set any environment variable for a tool run, including `PATH`, `COMSPEC`, `JAVA_TOOL_OPTIONS`, `NODE_OPTIONS`, `PYTHONSTARTUP`, `DOTNET_STARTUP_HOOKS` (code injection into Java-based ZAP, for example) | Fixed |
| F3 | Medium | Executable lookup searched the PATH *before* the folders Vixalune knows (install record, known folders), so a program planted earlier on the PATH shadowed the installed tool, including during verification | Fixed |
| F4 | Medium | The elevated runner accepted any existing absolute program; the setup code fell back to a PATH search for `wsl.exe` and `shutdown.exe` | Fixed |
| F5 | Medium | `explorer.exe` and `rundll32.exe` were started by bare name, which Windows resolves from the application's own folder first | Fixed |
| F6 | Medium | A damaged database, an invalid `config.toml` or an unreadable catalog folder stopped the application from starting | Fixed |
| F7 | Medium | A leftover "installing/uninstalling" installation record (crash, restored backup, damaged row) was reported as work in progress forever | Fixed |
| F8 | Medium | Manifest validation accepted `executable = "x.bat"` or `.cmd`/`.ps1` names for Windows programs (the locator refused them later, so not exploitable, but the manifest should never get that far) | Fixed |
| F9 | Low | Manifest loading was unbounded in file size and count, and followed links | Fixed |
| F10 | Low | One unreadable row made whole lists (runs, jobs, installations) fail | Fixed |
| F11 | Low | Cleanup of stale run folders removed by path and could in principle follow a junction named like a run id | Fixed |
| F12 | Low | Concurrent runs across tools were unbounded (the `max_concurrent_processes` setting existed but was not enforced); runs were not stopped when the app closed | Fixed |
| F13 | Info | Installs were not tied to the exact manifest that drove them | Fixed (audit records carry manifest version, SHA-256 and trust level) |

Not fixed, by design or because it needs infrastructure that does not exist yet, are listed under "Residual risks" (section 6). Several of them matter.

## 2. Threat model

Assets: the user's files and credentials; the integrity of installed tools; the user's authorisation boundary (what gets scanned); local run history and output; the availability of the application.
Trust boundaries: web view to Rust (Tauri IPC); manifests and tutorials (data shipped with the app) to the executor; Vixalune to winget, WSL, Docker and the tools; tools to the network; the local user account to other local software.
Attackers considered: a malicious or compromised manifest or release; a compromised upstream package or image; malicious software already running as the same user (it can do anything the user can; the review looks at what Vixalune adds); a malicious web page or tool output rendered in the UI; a careless user.

| Threat (from the brief) | Where it was examined | Result |
|---|---|---|
| Malicious or compromised manifests | `domain/manifest.rs`, `adapters/registry.rs`, closed schema (`deny_unknown_fields`, closed install-method enum, no scripts) | F2, F8, F9 fixed; the *authenticity* of manifests is unsolved (6.1) |
| Compromised upstream repositories and releases | winget adapter (`--source winget --exact`, hash checked by winget), container adapter (digest-only, `--pull never`), no archive downloads (refused) | No independent signature check exists (6.1) |
| Malicious package dependencies | `npm audit`, OSV lookup of `Cargo.lock` (section 5) | No advisories in the Windows build; two non-Windows/dev-only advisories |
| Command injection | Every process path (section 3); proptest on argument building | No shell anywhere; arguments are arrays; properties hold |
| Path traversal | manifest locations, run files, save paths, mounts, settings | Covered by tests; no issue found beyond F8/F11 |
| Unsafe executable resolution / PATH hijacking | `adapters/locator.rs`, `setup.rs`, `link_opener.rs`, `elevation.rs` | F3, F4, F5 fixed |
| Symlink and reparse-point abuse | manifests, mounts (canonicalised), run-folder cleanup, save path | F9, F11 fixed; mount TOCTOU remains (6.4) |
| Privilege escalation | elevation path | Narrowed (F4); one fixed command (`wsl --install --no-distribution`) via UAC |
| Insecure temporary files | all non-test file creation | Application writes only inside its own data folder; settings use write-then-rename; tests use `tempfile` |
| Excessive filesystem permissions | data folder ACLs | Inherited from `%APPDATA%` (user only); not hardened further (6.6) |
| Sensitive log exposure | `redact`, output store, tracing | Redaction property-tested; command lines masked before storage; parameter values not logged |
| Resource exhaustion | output trimming, retention, manifest limits, process limit, concurrency limit, practice-server limits | F9, F12 fixed; no memory limit on tool processes (6.5) |
| Unsafe container configuration | `domain/container.rs` (`assert_hardened`) | Unchanged and re-tested; real Docker run verified in Phase 5 |
| Unauthorized network scanning | `domain/scope.rs` | Unchanged; adversarial tests in `native_runner_tests.rs` |
| Cross-process interference | job objects, one active job per tool (SQLite partial unique index) | F1 fixed |
| Corrupted SQLite state | `prepare_database`, row-skipping lists | F6, F7, F10 fixed |
| Application update and signing | `tauri.conf.json` (`bundle.active = false`), no updater plugin | Nothing to attack yet, and nothing signed (6.2) |

## 3. Process-creation inventory

Every place a process is started (searched for `Command::new`, `tokio::process`, `ShellExecuteExW`):

| Site | Program | Arguments | Working dir / env | Notes |
|---|---|---|---|---|
| `SystemCommandExecutor::run_in` (winget, WSL, Docker, tools, verification) | Absolute path required (relative is refused) | Array only, no shell | Optional absolute existing folder; environment cleared, fixed allow-list plus PATH, then profile variables with the denylist (F2) | Now started suspended, placed in a job object, then resumed (F1); timeout and cancel terminate the job; tool runs also end leftover descendants; fork-bomb limit of 512 processes for tool runs |
| `kill_tree` (fallback when the job object cannot be created) | `%SystemRoot%\System32\taskkill.exe` | `/PID <int> /T /F` | none | Only used if Windows refuses the job |
| `UacElevatedRunner::run` | Must be in `%SystemRoot%\System32` (new) | Characters limited to `[A-Za-z0-9-_.:/\]`, at most 64 | n/a | Only `wsl --install --no-distribution`; output not capturable; a timeout leaves the elevated process running and says so |
| `SystemLinkOpener::open` | `%SystemRoot%\System32\rundll32.exe` (was a bare name) | `url.dll,FileProtocolHandler <https URL>` | none | https only |
| `SystemLinkOpener::open_folder` | `%SystemRoot%\explorer.exe` (was a bare name) | one folder path produced by the backend | none | Only run folders the app created |
| Test-only | `cmd`, `mklink`, `ping`, `whoami` | | | Not compiled into the product |

User input cannot choose the program: programs come from manifests (bare `.exe` names, validated), are located by the locator and passed as absolute paths. User values reach only validated argument slots; a leading `-`, control characters, whitespace and shell metacharacters are rejected rather than escaped.

## 4. Fixes in detail

**F1 containment.** `JobGuard` (Win32 job object, `KILL_ON_JOB_CLOSE`, optional active-process limit). The child is created `CREATE_SUSPENDED`, adopted into the job, then resumed with `NtResumeProcess`, so it cannot start a descendant outside the job. (An earlier iteration adopted the child after it was already running; the test showed a descendant escaping, which is why suspended start is used.) On cancel or timeout the job is terminated; on normal exit leftover descendants are terminated for tool runs and left alone for installers (`ExecOptions.kill_leftovers`). If Vixalune itself crashes, closing the job kills the members. At normal shutdown `LaunchRouter::shutdown` cancels runs and waits up to 8 s (containers are removed by name).
Caveats: `NtResumeProcess` is an ntdll function that Microsoft does not document as a stable API; it has been stable for decades and is widely used for exactly this purpose. Job termination is abrupt (no graceful shutdown of tools).

**F2 environment.** `env_name_is_dangerous` (exact names, prefixes such as `JAVA`, `NODE_`, `PYTHON`, `DOTNET_`, `LD_`, and substrings such as `OPTIONS`, `PRELOAD`, `HOOK`) is enforced twice: manifest validation rejects such names, and the executor refuses them even if a caller passes them. The inherited environment was already an allow-list.

**F3/F4/F5 resolution.** `locate` searches folders passed in (install record, known folders) first, then the registry PATH, then the process PATH. `locate_system` returns only `System32` files and is used for `wsl.exe`/`shutdown.exe`. The elevated runner now independently requires the program to sit directly in `System32`. `explorer.exe`/`rundll32.exe` use absolute paths.
The PATH is still consulted for non-elevated tools, because that is how winget's links folder works. Software that can write to a PATH folder as the same user can already run code as that user; the fix removes Vixalune's contribution (it no longer prefers such a program over the verified install, and never elevates one).

**F6/F7/F10 recovery.** `prepare_database` runs `PRAGMA quick_check` before opening; a damaged file is renamed (kept as evidence), stale `-wal/-shm/-journal` files go with it, and a notice appears in a banner at the top of every screen. `AppConfig::load_or_recover` does the same for `config.toml`. `ManifestRegistry::load_dir_or_empty` returns an empty catalog plus the reason. List queries skip undecodable rows with a log entry. An in-progress installation state with no active job is shown as *failed*.

**F8/F9.** Windows executables must be plain `.exe` names; manifests are limited to 256 KiB and 500 files, must be regular files (links are refused), and are hashed (SHA-256) as loaded.

**F11/F12/F13.** Run-folder cleanup skips links and reparse points. `LaunchRouter` enforces `resources.max_concurrent_processes`. Install, uninstall and verify audit events contain `manifest <version> sha256:<prefix> (<trust level>)`.

## 5. Tests and how to reproduce them

Everything below uses temporary folders, mock programs, in-memory databases and a loopback-only practice server. Nothing contacts a public host.

```powershell
cargo test --manifest-path src-tauri/Cargo.toml                       # 272 unit/property/integration-style tests
npm test                                                              # 117 UI tests
npm run test:e2e                                                      # Playwright against demo data
cargo test --manifest-path src-tauri/Cargo.toml --test platform_integration -- --ignored   # read-only checks of this PC
$env:VIXALUNE_INTEGRATION_CONTAINER = "1"; cargo test --manifest-path src-tauri/Cargo.toml --test container_integration -- --ignored   # real Docker (pulls and removes an image)
npm audit
```

| Area | Tests (module::name) |
|---|---|
| Process containment | `adapters::executor::tests::cancelling_kills_grandchildren_too`, `processes_left_behind_by_a_tool_run_are_stopped_but_installers_are_not_disturbed`, `times_out_and_kills`, `cancellation_kills_the_process`, `arguments_are_not_interpreted_by_a_shell`, `the_child_environment_is_scrubbed_of_unlisted_variables`, `dangerous_or_malformed_environment_is_refused_before_anything_starts`, `refuses_relative_programs_and_missing_files` |
| Shutdown | `adapters::native_runner::tests::shutdown_asks_every_active_run_to_stop_and_leaves_a_truthful_record` |
| Executable resolution | `security_tests::folders_vixalune_knows_win_over_the_path_and_names_with_separators_are_refused`, `system_programs_come_only_from_the_system_folder`, `adapters::elevation::tests::only_programs_in_the_system_folder_may_be_elevated`, `elevated_arguments_cannot_carry_shell_syntax_or_spaces` |
| Malformed or hostile manifests | `security_tests::hostile_manifest_files_are_rejected_one_by_one_and_never_hide_good_ones`, `manifests_cannot_set_environment_variables_that_redirect_or_inject_code`, `executables_and_locations_in_manifests_cannot_point_elsewhere`, `a_manifest_that_is_a_link_is_not_followed`, `the_manifest_digest_tracks_the_exact_bytes_loaded`, `a_missing_or_unreadable_catalog_folder_gives_an_empty_catalog_and_a_reason`, plus the Phase 1-3 manifest tests (unknown fields, scripts, bad hashes, insecure URLs) |
| Argument and path adversaries | `adapters::native_runner::tests::malformed_parameters_never_reach_the_process`, `hostile_spellings_of_local_addresses_do_not_slip_through`; property tests `target_parsing_never_panics_and_accepted_targets_are_clean`, `hostlike_values_become_exactly_one_argument`, `accepted_port_lists_contain_only_valid_ports`, `protected_folders_are_never_accepted_for_mounting`, `save_destinations_need_an_allowed_extension`; `adapters::result_saver::tests::*`; `domain::container::tests::*`; `adapters::container_runner::tests::junctions_to_protected_folders_are_resolved_and_refused` |
| Secrets | property test `redaction_removes_credentials`; `engine::tests::output_secrets_are_redacted_before_storage` |
| Corrupt state | `security_tests::a_damaged_database_is_moved_aside_and_a_fresh_one_works`, `a_truncated_database_is_detected`, `unreadable_rows_are_skipped_not_fatal`, `config::tests::a_broken_config_is_moved_aside_and_defaults_are_used`, `adapters::settings_store::tests::a_damaged_or_invalid_file_falls_back_to_defaults` |
| Interrupted jobs | `engine::tests::state_survives_restart_and_interrupted_jobs_are_closed`, `a_leftover_in_progress_record_is_reported_as_failed_not_as_work_in_progress`, `cancelling_an_install_rolls_back_a_partial_installation`, `failed_verification_is_never_reported_as_installed`, `uninstall_that_did_not_remove_the_package_is_not_reported_as_success`, `cancel_does_not_remove_a_tool_the_user_already_had` |
| Uninstall touches nothing else | `adapters::native_runner::tests::cleaning_up_stale_run_folders_never_follows_a_junction`; winget/WSL/container uninstall tests assert the exact fixed arguments |
| Permission failures | `elevation` tests (declined UAC, refused programs), `engine::tests::elevation_requires_consent_and_waits_without_running_the_installer`, `adapters::result_saver::tests::refuses_dangerous_names_types_and_places` |
| Scope (unauthorised scanning) | `adapters::native_runner::tests::*` (public/private targets, DNS change, redirects, lab list), `domain::scope::tests::*`, end-to-end redirect test against the real practice server |
| Network unavailable | `adapters::winget` timeout tests; offline behaviour: the catalog, Learn pages and local labs need no network; installs and the first Trivy run fail with a diagnosed error |
| UI | `src/Phase5.test.tsx` (start-up notices, scope confirmation, failed vs findings, cancel, save), `src/Phase4.test.tsx` |

Dependency checks run for this review (results on this machine at the time of writing):
- `npm audit`: 0 vulnerabilities.
- OSV lookup of all 473 registry crates in `Cargo.lock` (package names and versions only): two advisories, `glib 0.18.5` (RUSTSEC-2024-0429) and `proc-macro-error 1.0.4` (RUSTSEC-2024-0370, unmaintained). `cargo tree --target x86_64-pc-windows-msvc` shows neither is part of the Windows build (glib is the Linux GTK stack). They would matter if a Linux build were made.
- Not done: `cargo audit`/`cargo deny` as a recurring CI check (no CI exists; the OSV query is a one-off), SBOM generation, licence review of dependencies.

## 6. Residual risks (honest list)

1. **Supply-chain authenticity is not independently proven.** Windows installs trust winget's community source (`--source winget`; winget checks the installer hash recorded in the winget-pkgs manifest). A malicious change merged into that repository would pass. Container images are pinned by digest, but the digest is whatever the bundled manifest says; Sigstore/cosign signatures are not checked. Vixalune does not treat a checksum fetched from the same place as the artifact as proof, and for that reason `download_archive` installs remain *refused* rather than half-verified. The proper fix needs an independent root of trust: signed manifests (for example minisign/ed25519 with a key held outside the release pipeline) and signature checks for artifacts where the publisher provides them.
2. **No signing, no updater.** `bundle.active` is false: there is no installer, no code signing, no update channel. When an updater is added it must use Tauri's signed-update mechanism and the signing key must not live in the build pipeline's reachable secrets. Until then users build from source and the integrity of the build depends on their own checkout.
3. **Manifests are trusted once bundled.** Vixalune's own files can be modified by anyone who can write to its install folder. Manifest digests are logged, not verified against a signed list. Remote manifests are disabled.
4. **Time-of-check/time-of-use.** Mount folders are canonicalised and validated at start, then handed to Docker; a user-writable folder could be swapped for a junction between the check and the container start. The scope check resolves names at start; a tool may resolve again later (DNS changes are caught at start only).
5. **Native tools are not sandboxed.** They run as the user, with the user's network and files. Limits: own run folder as working directory, scrubbed environment, timeouts, process-count limit. There is no memory/CPU limit (a Java tool such as ZAP would need a profile-specific value) and no AppContainer/restricted token.
6. **Local data protection.** The SQLite database and settings are plain files in the user's profile, protected only by the account's ACLs; output is redacted but may still contain sensitive findings. Retention limits exposure; nothing is encrypted.
7. **Updates have no rollback.** `winget upgrade` is used as is. Version pinning exists in manifests; reviewing an update (diff of manifest and digest) and reverting to the previous version are not implemented.
8. **Elevation.** Only `wsl --install --no-distribution` is elevated, via UAC, and an elevated process cannot be killed from here. Windows' consent prompt is the control.
9. **Platform coverage.** Windows 11 only. Real Nmap, ZAP and Trivy runs and the ZAP command-line flags have still not been exercised (`docs/MANUAL_INTEGRATION_TESTS.md` section G). WSL tool installs were not run in this phase. The packaged Tauri window (as opposed to the web front end) was compiled but not driven by automated UI tests.
10. **Web view.** CSP is `default-src 'self'` with `style-src 'unsafe-inline'` (needed by the UI library); the web view has `core:default` only and no plugin permissions. Tool output is rendered as text (React escaping), never as HTML. No automated test fuzzes the IPC surface from the web view side.
11. **Property tests are shallow.** They run 512 cases per property per execution with simple generators; they are not a substitute for fuzzing the parsers for hours. `NtResumeProcess` and job-object behaviour were tested on one Windows build.

## 7. Recommended next steps (not done)

Sign manifests and verify them at load; add a recurring dependency check in CI (`cargo audit`/`cargo deny`, `npm audit`, SBOM); verify image signatures where publishers provide them; add profile-level memory limits and a restricted token for native runs; implement update review and rollback; code-sign release builds; fuzz the manifest and target parsers; add a Tauri-window end-to-end test.
