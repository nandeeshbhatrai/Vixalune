# Tool-aware assistant

The AI Assistant screen can answer security questions **using the tools that are actually in Vixalune's catalog**, and can look up known vulnerabilities for a package. It builds on the [AI gateway](AI_GATEWAY.md) (your own API key, your chosen provider). Leave **Use the tool catalog** switched on to get this behavior; switch it off for a plain chat.

## What it does

1. **Understands the question in code, not by guessing.** A package question ("check lodash 4.17.15 from npm") is parsed deterministically into ecosystem, name and version. If something is missing, Vixalune asks for it itself and does not call a model or a database. An unsupported ecosystem is named and declined.
2. **Retrieves only the relevant catalog records.** The registry is searched with weighted term matching over each tool's name, tags, category, summary, description and launch profiles (plus a small table of generic synonyms such as *traffic → packet, proxy*). Typically one to four records are sent, never the whole catalog, and no tool name is hard-coded anywhere in the assistant. What the model sees for each tool: id, name, description, platforms, **live installation state and verified version**, prerequisites, launch profiles and parameters, limitations derived from the manifest, license, documentation link and the date the facts were verified.
3. **Looks up vulnerabilities when you allow it** (see below).
4. **Asks the model for one JSON object**, then **validates it** (next section). Only the validated result is stored and shown.
5. **Shows cards** with the tool name, why it fits, installation status, prerequisites, the official documentation link, and the buttons **View Details**, **Check Prerequisites**, **Install…** and **Prepare Scan**.

## Trusted and untrusted text

The prompt has two parts. The **system part** contains the fixed rules and the catalog records read from the registry a moment ago. The **user part** contains, inside `<<UNTRUSTED-…>>` markers with a random per-request marker: your question, vulnerability database text, and scanner output you chose to share. Text is cleaned of control and direction-override characters, length-bounded, and cannot forge the closing marker.

The prompt rules are not the security boundary. Even a model that obeys text hidden in a package description cannot do anything, because:

- **No tools are offered to the model**, so it cannot call anything. It can only suggest.
- **Authorization is in code** (`ActionExecutor::authorize`). A model-origin request may read catalog records and nothing else. Lookups need your standing consent; preparing a run and reading run output need you to click.
- **Every field of the reply is checked against facts the application holds** (see next section).

## What is validated

| Model output | Check |
|---|---|
| Tool ids | Must be tools the application retrieved for this question. An id that is not in the catalog, or exists but was not considered, is dropped with a visible warning. |
| Launch profiles | Must exist for that tool. |
| Citations (`E1`…) | Must be ids in the evidence list the application built. The model never supplies web addresses; links shown next to claims come from that list. |
| Web addresses and images in model text | Removed unless they are in the evidence list. |
| Vulnerability identifiers (CVE, GHSA, OSV, …) | Must appear in the database results or scanner output for this question. Others are replaced by “[unverified identifier removed]” and reported. |
| Installation status, versions, licence, prerequisites | Always taken from the registry and installer. A model saying “I installed it” changes nothing. |
| Suggested actions | Parsed into the typed `Action` set with `deny_unknown_fields`; unknown actions (such as `install_tool` or `run_command`) are rejected; tool, profile and parameter names must exist. They are shown as suggestions that have not run. |
| Claims of safety | Flagged: a lookup or scan cannot show that something is safe. |
| Reply that does not match the format | Not used at all; the answer is marked failed and no unvalidated text is kept. |

## Actions

The complete set (`domain/assistant/actions.rs`): `search_catalog`, `get_tool_details`, `check_prerequisites`, `get_installation_status`, `prepare_tool_run`, `explain_scan_result`, `query_supported_vulnerability_sources`. Typed arguments, validated for shape and again against the registry.

- There is no action that installs software, starts a process, runs a command, edits a file or changes a setting.
- **Install…** opens the tool's page, where the usual install plan and confirmation apply. **Prepare Scan** opens *Run a Tool*, where parameters, scope checks and your confirmation apply. **Check Prerequisites** reads the installer's plan; it changes nothing.
- `prepare_tool_run` describes a run (it requires the tool to be installed) and never starts one.

## Vulnerability workflow

Modes: lookup of a package version in **OSV** (and CVE detail in **NVD**), dependency scanning with an installed scanner from the catalog (suggested as a card; you run it), container image or project folder scanning (the same, and only a folder or image you choose), and **interpretation of an existing run's output**.

- **Lookup** sends only the ecosystem, package name and version (or a vulnerability id) to `api.osv.dev` / `services.nvd.nist.gov`. It is **off until you allow it** in Settings → AI Providers, or by pressing **Allow lookups and ask again** on the answer that asked for permission. Supported ecosystems: npm, PyPI, crates.io, Go, Maven, NuGet, RubyGems, Packagist, Hex, Pub.
- **Records are kept as the database gave them**: identifiers and aliases, affected ranges, fixed versions, published/modified timestamps, severity vectors, references (https only) and a source link, with the time Vixalune asked.
- **Four different claims are kept apart.** *Database match* (a database lists this version as affected) and *scanner finding* (a scanner reported it for your target) are produced. *Confirmed vulnerable configuration* and *demonstrated exploitability* are never assessed and are shown as “not assessed”.
- **No result is not “safe”.** An empty answer says so, with coverage notes (only publicly reported flaws; package identity and forks; stale data).
- If the database cannot be reached, nothing is claimed and the model is not asked to speculate.
- **Explain a scan run** (button in the assistant): choose a finished run, confirm what will be sent. At most 400 lines (about 30 KB) of its stored output are sent, lines that look like credentials are withheld, the text is fenced as untrusted, and the output is **not** saved in the conversation. Only identifiers that appear in that output count as scanner findings.

## Privacy

Your questions, the catalog records for the tools considered, and (if you allow it or choose it) database results or scan output are sent to **the AI provider you configured**. The package lookup goes to OSV/NVD. Vixalune never sends API keys, environment variables, source trees, credential files or private repositories. If project content were ever needed, it would require its own consent screen; today only the output of a run you pick can be shared.

## Verified and not verified

- Verified in this repository: all of the above logic with deterministic fixtures and mocked providers (`cargo test`), including injection, fabricated-finding and approval cases; the **real OSV and NVD APIs** (opt-in `cargo test --lib live_ -- --ignored --nocapture`) returned 6 records for npm `lodash 4.17.15` including `GHSA-35jh-r3h4-6jhm` / `CVE-2021-23337`, and the recorded responses are the test fixtures.
- **Not verified:** the model-facing behavior with a real provider. Providers differ in how reliably they return the JSON object; a malformed reply is rejected, never repaired. No Trivy scan was run by the assistant: running tools remains in *Run a Tool*.
- Retrieval is lexical. A question phrased with none of a tool's vocabulary may retrieve nothing, in which case the assistant says no catalog tool matched.
