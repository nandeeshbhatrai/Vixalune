# Vixalune

A Windows desktop workbench for beginner security learners. It helps you install a small, curated set of open-source security tools, run them with guarded settings against practice targets on your own computer, and understand the results. **For authorized testing and learning only.**

Status: **0.2.0, first installable release candidate.** Unsigned, no auto-update. See [CHANGELOG.md](CHANGELOG.md) for what is and is not verified.

## Install

1. Download `Vixalune_0.2.0_x64-setup.exe` and `SHA256SUMS.txt` from the [releases page](https://github.com/nandeeshbhatrai/Vixalune/releases) and compare the hash (`Get-FileHash`).
2. Run it. It installs for your user, with no administrator prompt. Windows SmartScreen will warn because the installer is not code-signed yet.
3. Start Vixalune; a short welcome tour checks your computer and explains the safeguards.

Full instructions, requirements, updating and uninstalling: [docs/INSTALLATION.md](docs/INSTALLATION.md).

**Supported:** Windows 11 and Windows 10 22H2, x64. **Needed:** the WebView2 runtime (already on current Windows). **Optional:** winget (Windows tools), WSL 2 (Linux tools), Docker Desktop (container tools). Vixalune is useful without the optional parts.

## What you can do

- Browse a curated catalog (Nmap, OWASP ZAP, Trivy, Semgrep, Wireshark, Nikto) that ships with the app and works offline.
- Install a tool (winget, WSL apt or a pinned container image) and have Vixalune **verify** it by running it.
- **Run** tools from ready-made profiles: you see the exact command, target and scope first; output streams live; findings are shown as results, not errors; you can search, copy, save and cancel.
- Follow a **tutorial** for each runnable tool and practise on a **deliberately vulnerable local web app** and a sample project of insecure code.
- Keep run history for as long as you choose.

| Doc | Purpose |
|---|---|
| [docs/INSTALLATION.md](docs/INSTALLATION.md) | Requirements, installing, updating, uninstalling, building from source |
| [docs/USER_GUIDE.md](docs/USER_GUIDE.md) | Using Vixalune |
| [docs/SUPPORTED_TOOLS.md](docs/SUPPORTED_TOOLS.md) | Tools, install methods, licenses, what was verified |
| [docs/TROUBLESHOOTING.md](docs/TROUBLESHOOTING.md) | Common problems |
| [docs/SECURITY.md](docs/SECURITY.md) | Reporting problems, what the app does and does not protect |
| [docs/SECURITY_REVIEW.md](docs/SECURITY_REVIEW.md) | Security review findings, fixes, tests, residual risks |
| [docs/CONTRIBUTING.md](docs/CONTRIBUTING.md) | Developing and contributing |
| [docs/RELEASE_CHECKLIST.md](docs/RELEASE_CHECKLIST.md) | How a release is made and tested, with the 0.2.0 test report |
| [docs/PRODUCT_SPEC.md](docs/PRODUCT_SPEC.md), [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md), [docs/THREAT_MODEL.md](docs/THREAT_MODEL.md), [docs/TOOL_MANIFEST_SCHEMA.md](docs/TOOL_MANIFEST_SCHEMA.md), [docs/MANUAL_INTEGRATION_TESTS.md](docs/MANUAL_INTEGRATION_TESTS.md) | Design and test documentation |

## AI assistant (optional)

Bring your own key for OpenAI, Anthropic, Google Gemini or OpenRouter. Keys live only in Windows Credential Manager; messages are sent to the provider you choose. See [docs/AI_GATEWAY.md](docs/AI_GATEWAY.md). Privacy: [docs/PRIVACY.md](docs/PRIVACY.md). Security review and residual risks: [docs/AI_SECURITY.md](docs/AI_SECURITY.md). The assistant can also recommend tools from the catalog and look up known package vulnerabilities: [docs/ASSISTANT.md](docs/ASSISTANT.md).

## Known limitations

- **Not code-signed**, and releases are **not verified reproducible** (two builds of the same sources on the same machine produced different installers). Check the SHA-256 on the release page.
- **No automatic updates.** You install a newer installer over the old one; your data is kept.
- **Verified on real software:** Trivy (install, verify, run, cancel, uninstall) and Semgrep in Docker. **Not verified on real hardware:** Nmap, OWASP ZAP (including its command-line flags and install folder), Wireshark, and Nikto in WSL.
- **Not tested on a clean machine.** Release tests ran on the developers' Windows 11 PC; see the report in [docs/RELEASE_CHECKLIST.md](docs/RELEASE_CHECKLIST.md).
- Tools run with your user rights and are not sandboxed. The scope checks prevent mistakes; they cannot prove you are authorised.
- Windows only. Linux and macOS are not supported.

## Developing

```powershell
npm ci
npm run dev:demo                 # interface with labelled demo data, in a browser
npm run tauri:dev                # the real application window
npm test                         # 128 interface tests
npm run test:e2e                 # Playwright against demo data (SCREENSHOTS=1 refreshes docs/screenshots)
cargo test --manifest-path src-tauri/Cargo.toml --locked   # 285 backend unit and property tests
.\scripts\build-release.ps1      # tests, audit, licence list, locked build, SHA-256
```

See [docs/CONTRIBUTING.md](docs/CONTRIBUTING.md). Opt-in tests that touch the real machine are described in [docs/MANUAL_INTEGRATION_TESTS.md](docs/MANUAL_INTEGRATION_TESTS.md).

## License

MIT (see [LICENSE](LICENSE)). Third-party components: [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md). Vixalune does not bundle the security tools it installs; they keep their own licenses ([docs/SUPPORTED_TOOLS.md](docs/SUPPORTED_TOOLS.md)).
