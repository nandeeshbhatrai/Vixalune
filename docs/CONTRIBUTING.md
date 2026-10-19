# Contributing to Vixalune

Thank you for helping. Vixalune teaches beginners, so changes are judged on safety, clarity and honesty as much as on code.

## Ground rules

1. **Safety first.** No shell strings, no scripts in manifests, no silent elevation, no unconfirmed system changes, no secrets in the repository or logs. If a change weakens a safeguard, say so in the pull request and update [THREAT_MODEL.md](THREAT_MODEL.md).
2. **Say what was tested.** Do not write "works" for something that was only mock-tested. Mark what has and has not run on real software, as the existing docs do.
3. **No bundled security tools.** Vixalune installs tools from their publishers; it does not redistribute them. A new tool needs its license checked for the "installs from the publisher" model (see [SUPPORTED_TOOLS.md](SUPPORTED_TOOLS.md)).
4. **Authorized use only.** Do not add features aimed at evading detection, attacking third parties or hiding activity.

## Setting up

Requirements (Windows 11 or Windows 10 22H2, x64): Node.js 22 or newer, Rust 1.99.1 (MSVC toolchain) with the Visual Studio Build Tools, and Microsoft Edge WebView2 (preinstalled on Windows 11). Windows Smart App Control, when set to enforce, blocks freshly compiled unsigned programs and must be off to build; Vixalune never changes that setting for you.

```powershell
git clone https://github.com/nandeeshbhatrai/Vixalune.git
cd Vixalune
npm ci
npm run dev:demo          # the interface with clearly labelled demonstration data, in a browser
npm run tauri:dev         # the real application window (compiles the Rust backend first)
```

## Checks before a pull request

```powershell
npm run typecheck
npm test                                                   # interface tests
npm run test:e2e                                           # Playwright against demo data (uses Microsoft Edge)
cargo test --manifest-path src-tauri/Cargo.toml --locked   # backend unit, property and integration-style tests
npm audit --omit=dev
```

Opt-in tests that touch the real machine (read-only unless stated) are listed in [MANUAL_INTEGRATION_TESTS.md](MANUAL_INTEGRATION_TESTS.md) and `src-tauri/tests`. Never run tests against public targets; use the practice lab.

## Where things are

| Path | What |
|---|---|
| `src/` | React interface; talks to the backend only through `src/domain/backend.ts` |
| `src-tauri/src/domain` | Pure types and rules (manifests, scope, state machines, validation) |
| `src-tauri/src/adapters` | Windows, winget, WSL, Docker, SQLite, process execution |
| `src-tauri/src/engine.rs` | Installation lifecycle |
| `manifests/` | Tool definitions (TOML, data only) |
| `learning/` | Tutorials (TOML, validated against manifests) |
| `docs/` | Documentation; keep it in step with the code |

## Adding a tool

1. Write `manifests/<id>.toml` following [TOOL_MANIFEST_SCHEMA.md](TOOL_MANIFEST_SCHEMA.md). Use winget (`--source winget`), apt, or a container image pinned by digest. Check the package ID and license against the publisher and record `verified_on`.
2. Give each launch profile typed parameters with safe defaults, a timeout, `expected_output`, the right `network_activity`, and `exit_codes` or `findings_patterns` so that "findings" is not shown as a failure.
3. Add a tutorial in `learning/<id>.toml` if the tool can be run. Links must be plain https on the tool's official sites.
4. Run `cargo test`; the bundled-manifest and tutorial tests will tell you what is wrong.
5. Install it on a real machine and record the result (what you ran, what happened) in your pull request. Mock tests are not enough for the "verified" claim.

## Code style

Match the surrounding code. Rust: no `unwrap` on user-reachable paths, errors carry a code and a recovery sentence (`AppError`), new process creation goes through `CommandExecutor`. TypeScript: strict types, no direct `invoke` in components, accessible labels (tests find elements by role and name).

## Commits and pull requests

Small, focused commits with a message that says what and why. Update `CHANGELOG.md` for user-visible changes and the relevant docs. By contributing you agree that your contribution is licensed under the MIT license of this project.

## Releases

Maintainers follow [RELEASE_CHECKLIST.md](RELEASE_CHECKLIST.md).
