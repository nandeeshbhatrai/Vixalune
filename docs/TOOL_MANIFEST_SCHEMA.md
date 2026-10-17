# Tool Manifest Schema (v1)

Manifests are **TOML data files** in `manifests/*.toml`. The authoritative definition is the Rust types in `src-tauri/src/domain/manifest.rs`; this document describes them. Unknown keys are rejected. A manifest cannot contain scripts or commands — only the closed set of structured options below.

Max size: 256 KiB. Invalid files are skipped (and listed in `AppInfo.rejected_manifests`) without affecting other tools.

## Top-level fields

| Field | Type | Required | Rules |
|---|---|---|---|
| `schema_version` | integer | yes | Must be `1` |
| `id` | string | yes | Slug: `a-z0-9-`, 1–63 chars, starts with letter/digit. Unique across the catalog |
| `name` | string | yes | ≤ 80 chars, no control chars |
| `summary` | string | yes | ≤ 200 chars |
| `description` | string | yes | ≤ 4000 chars. Should state authorized-use scope |
| `category` | enum | yes | `reconnaissance`, `web_application`, `network`, `passwords`, `vulnerability`, `forensics`, `utilities` |
| `license` | string | yes | SPDX expression or `LicenseRef-…` |
| `homepage`, `documentation` | string | yes | `https` URL, no credentials |
| `platforms` | array of enum | yes, non-empty | `windows_native`, `wsl`, `container` |
| `prerequisites` | array of enum | no | `winget`, `wsl2`, `docker` |
| `requires_elevation` | bool | no (false) | Declares that install needs administrator rights; user must approve |
| `install` | array of tables | yes, ≥ 1 | See below. Each method's runtime must be listed in `platforms` |
| `verify` | table | yes | See below |
| `run` | table | no | See below |

## `[[install]]` methods

Discriminated by `method`.

| `method` | Fields | Validation | Executed as (phase 2) |
|---|---|---|---|
| `winget` | `package_id`, optional `version` | `[A-Za-z0-9._-]`, ≤128, no leading `-` | `winget install --id <id> --exact [--version <v>]` |
| `wsl_apt` | `packages` (1–20) | lowercase `[a-z0-9.+-]` | `wsl --exec apt-get install -y -- <pkgs…>` |
| `container_image` | `image`, `digest` | image `[A-Za-z0-9._/-]`, **no tag**; digest `sha256:` + 64 hex | `docker pull <image>@<digest>` |
| `download_archive` | `url`, `sha256`, `executable` | `https` only; 64-hex SHA-256 required; bare executable name | Download → verify hash → extract into tools dir |

## `[verify]`

| Field | Type | Rules |
|---|---|---|
| `runtime` | enum | as `platforms` |
| `executable` | string | bare file name `[A-Za-z0-9._-]`, ≤64, no path separators, not starting with `-` or `.` |
| `args` | array of string | ≤128 chars each, no control chars |
| `expect_output_contains` | string, optional | Substring expected in stdout |

## `[run]` (optional)

| Field | Type | Rules |
|---|---|---|
| `runtime` | enum | Must be in `platforms` |
| `executable` | string | Bare file name (as above) |
| `parameters` | array of tables | See below |
| `args` | array of inline tables | Ordered argument templates |

### `[[run.parameters]]`

| Field | Type | Rules |
|---|---|---|
| `name` | string | `[a-z][a-z0-9_]*`, ≤32, unique |
| `label` | string | Shown in UI, ≤80 |
| `description` | string | optional |
| `kind` | inline table | `{type="text", max_len=N}`, `{type="host"}`, `{type="integer", min=…, max=…}`, `{type="boolean"}`, `{type="choice", options=[…]}` |
| `required` | bool | default false |
| `default` | bool / int / string | Must pass the parameter's own validation |

Value validation at run time: `text` — non-empty, ≤ `max_len` (default 256), no control chars, **must not start with `-`**; `host` — `[A-Za-z0-9.:/_\-\[\]]`, ≤253, no leading `-`; `integer` — within range; `choice` — exact match; unknown parameter names are rejected.

### `run.args` templates (`kind` discriminates)

| `kind` | Fields | Output |
|---|---|---|
| `literal` | `value` | fixed argument (≤256, no control chars) |
| `value` | `param` | the parameter value, if set |
| `flag` | `param` (boolean), `flag` | `flag` if the boolean is true |
| `option` | `param`, `flag` | `flag`, then value, if set |

`flag` must match `-[A-Za-z0-9_-]+`. Every `param` must exist.

## Example

See `manifests/nmap.toml`:

```toml
schema_version = 1
id = "nmap"
name = "Nmap"
summary = "Network discovery and port scanner."
description = "…"
category = "reconnaissance"
license = "LicenseRef-Nmap"
homepage = "https://nmap.org"
documentation = "https://nmap.org/book/man.html"
platforms = ["windows_native"]
prerequisites = ["winget"]

[[install]]
method = "winget"
package_id = "Insecure.Nmap"

[verify]
runtime = "windows_native"
executable = "nmap"
args = ["--version"]

[run]
runtime = "windows_native"
executable = "nmap"
args = [
  { kind = "flag", param = "service_scan", flag = "-sV" },
  { kind = "value", param = "target" },
]
[[run.parameters]]
name = "target"
label = "Target"
required = true
kind = { type = "host" }
[[run.parameters]]
name = "service_scan"
label = "Detect service versions"
kind = { type = "boolean" }
```

## Trust levels

Trust is **not** a manifest field (a file cannot vouch for itself). The registry assigns `bundled` to manifests loaded from the app bundle and `remote_unsigned` to any future downloaded manifest. All manifests pass the same validator.

## Versioning

`schema_version` is bumped on breaking changes; the loader rejects unknown versions. Adding optional fields requires updating the Rust types, the TypeScript mirror (`src/domain/types.ts`) and this document together.

## Known gaps (v1)

- Package ids and licenses in the sample manifests need verification against the live winget/apt catalogs before release.
- No per-tool version-detection or update-source fields yet (needed for `update`).
- No `labs` or `tutorials` manifests yet (`LabDefinition` exists as a Rust model).
