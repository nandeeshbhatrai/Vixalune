# Tool Manifest Schema (v2)

Manifests are **TOML data files** in `manifests/*.toml`. The authoritative definition is the Rust types in `src-tauri/src/domain/manifest.rs`; this document describes them. Unknown keys are rejected everywhere (tables *and* tagged variants). A manifest cannot contain scripts or commands: it selects one of a closed set of install adapters and supplies typed values.

Limits: 256 KiB per file. Invalid files are skipped (and listed in `AppInfo.rejected_manifests`) without affecting other tools. Schema v1 files are rejected (`schema_version must be 2`).

## Identity and metadata

| Field | Type | Rules |
|---|---|---|
| `schema_version` | integer | Must be `2` |
| `manifest_version` | string | `MAJOR.MINOR.PATCH` revision of *this tool definition* |
| `maintainer` | string | Who curates the entry |
| `id` | string | Slug `a-z0-9-`, 1–63 chars, unique across the catalog |
| `name`, `summary`, `description` | string | ≤ 80 / 200 / 4000 chars, no control chars |
| `category` | enum | `reconnaissance`, `web_application`, `network`, `passwords`, `vulnerability`, `forensics`, `utilities` |
| `tags` | array of string | ≤ 12, each `a-z0-9-` ≤ 24 |
| `license` | string | SPDX expression or `LicenseRef-…` |
| `homepage`, `documentation`, `source_repository` | string | `https` URLs (`source_repository` optional) |
| `maintenance` | enum, optional | `active`, `stable`, `deprecated` — curator-asserted, not checked at run time |
| `recommended_for_beginners` | bool | Default false |
| `[provenance]` | table | `publisher`, `basis` (`official_upstream` / `package_repository` / `community_maintained`), `source_url` (https), `verified_on` (`YYYY-MM-DD`) |

## Platform, prerequisites, dependencies

| Field | Rules |
|---|---|
| `platforms` | non-empty array of `windows_native`, `wsl`, `container` |
| `[windows]` | `min_build` ≥ 19041 (19045 = Windows 10 22H2, 22000 = Windows 11) and `architectures` (`x64`, `arm64`) |
| `[[prerequisites]]` | `kind` (`winget`, `wsl2`, `docker`), `reason` (required, shown to the user), optional `approx_size_mb`, `requires_admin`, `requires_restart` |
| `dependencies` | other catalog tool ids that must be **Installed** first; unknown ids reject the manifest |

## Installation

`[[install]]` entries are tried in order; the first whose adapter exists in this build and whose prerequisites are best satisfied is used.

| `method` | Fields | Validation | Adapter status |
|---|---|---|---|
| `winget` | `package_id` | `[A-Za-z0-9._-]`, ≤128, no leading `-` | **Implemented** (`WingetAdapter`) |
| `wsl_apt` | `packages` (1–20) | lowercase `[a-z0-9.+-]` | Not implemented |
| `container_image` | `image`, `digest` | no tags; `sha256:` + 64 hex | Not implemented |
| `download_archive` | `url`, `sha256`, `executable` | https only; 64-hex SHA-256; bare file name | Not implemented |

Other install tables:

| Table | Fields |
|---|---|
| `[version]` | `pinned` → passed to winget as `--version`; `minimum` is recorded and shown but **not enforced yet** |
| `[storage]` | `install_subdir` (relative path, traversal-checked), `approx_download_mb`, `approx_disk_mb` — all optional; unknown sizes are shown as "not recorded" |
| `[integrity]` | `level`: `package_manager` (winget verifies the installer hash; Vixalune does not re-check it), `sha256_pinned`, `image_digest`, `unavailable`; must be consistent with `install[0]`'s method; optional `note` |
| `[network]` | `install`, `run` booleans |
| `[privileges]` | `install` and `run`: `none` / `possible` / `required`; `may_require_restart` |
| `[lifecycle]` | `can_update`, `can_uninstall` — gate the corresponding operations |

## Verification

An install is reported **Succeeded only if every check passes**.

```toml
[verify]
timeout_secs = 30
[[verify.checks]]
kind = "package_installed"          # the adapter's package record says installed
[[verify.checks]]
kind = "executable_output"          # run a program with fixed args and look at its output
runtime = "windows_native"
executable = "tool.exe"             # bare file name; only .exe is ever located (no scripts)
args = ["--version"]
locations = ["{LocalAppData}\\Microsoft\\WinGet\\Links"]   # extra folders to search
expect_output_contains = "Version"
```

`locations` must start with `{ProgramFiles}`, `{ProgramFilesX86}` or `{LocalAppData}` followed by a validated relative path (no `..`, drive letters, UNC, device names, ADS). Executables are searched on the machine+user PATH re-read from the registry, then in `locations`.

Outcomes: `passed`, `failed`, and `unavailable` (the check could not run, e.g. winget timed out). **`unavailable` is never success**: the job fails with `verification_unavailable`. The stored level says what was actually checked: `executable` (program run) or `package_record` (package manager record only).

## Launch profiles

`[[launch_profiles]]` (replaces v1 `run`): `id`, `label`, `description`, `runtime`, `executable` (bare name), `parameters`, `args`.

Parameters: `name` (`[a-z][a-z0-9_]*`), `label`, `description`, `kind` (`{type="text", max_len}`, `{type="host"}`, `{type="integer", min, max}`, `{type="boolean"}`, `{type="choice", options}`), `required`, `default`.

Arg templates: `literal`, `value`, `flag` (boolean param → flag), `option` (flag + value). Values are **validated, never escaped**: text/host values reject control characters and a leading `-`, hosts are limited to `[A-Za-z0-9.:/_\-\[\]]`. Launching is not implemented yet; the command builder is (`build_command`, tested against injection strings).

## Example

See `manifests/trivy.toml` (verified end to end on Windows 11) and `manifests/nmap.toml` (with a launch profile).

## Container policy and container launch profiles

A manifest with a container launch profile needs a `[container]` table. It can only *tighten*; privileged mode, host namespaces, devices, extra capabilities and the Docker socket cannot be expressed.

| Field | Rules |
|---|---|
| `user` | numeric non-root `uid:gid` (`root`, `0`, `0:0`, names are rejected) |
| `read_only_rootfs` | default true |
| `memory_mb` 64-16384, `cpus_milli` 100-8000, `pids_limit` 16-4096 | resource limits (always applied) |
| `tmpfs` | absolute POSIX paths, mounted `rw,noexec,nosuid,size=64m` |
| `workdir` | absolute POSIX path |
| `env` | up to 16 fixed `{name,value}` pairs; names matching TOKEN/SECRET/PASSWORD/API_KEY/… are rejected |
| `mounts` | up to 4 `{name, container_path, read_only}`; container paths under `/proc /sys /dev /etc /run /var/run` are rejected; the **host folder is never in the manifest** |
| `ports` | container ports a profile may publish (host side chosen by the user, loopback only) |

Launch profile additions (`runtime = "container"`): `mounts` (names the user must supply a folder for), `needs_network` (default false; requires approval each run), `ports`. Profiles with other runtimes may not set them.

Install `image` must be a bare name without tag; its registry must be on the configured allow-list; the digest is mandatory (`docker pull registry/repo@sha256:…`).

For `wsl_apt` installs the `[[prerequisites]]` entry `wsl2` describes WSL itself; its `requires_admin`/`requires_restart` only apply while WSL is missing.

## Trust

Trust is **not** a manifest field. The registry assigns `bundled` to manifests loaded from the app bundle and `remote_unsigned` to any future downloaded manifest. Remote manifests are disabled; when added they pass the same validator and can only select existing adapters.

## Seed catalog and what was verified (2026-10-17)

| Tool | Method | Verified how | End-to-end tested |
|---|---|---|---|
| Trivy | winget `AquaSecurity.Trivy` (portable zip, user scope) | `winget show`; real install, verify, uninstall | **Yes, on Windows 11** |
| Nmap | winget `Insecure.Nmap` (nullsoft; winget carries 7.80) | `winget show` | No (mock-tested adapter) |
| Wireshark | winget `WiresharkFoundation.Wireshark` (MSI) | `winget show` | No |
| OWASP ZAP | winget `ZAP.ZAP` (exe) | `winget show`; no runnable check known, record-only verification | No |
| Semgrep | container image `semgrep/semgrep` pinned by digest | Docker Hub API | No — no container adapter yet, install is refused |

Install locations in `verify.locations` for Nmap and Wireshark are conventional guesses until those installs are exercised; a wrong guess makes verification fail (never pass).

