# Changelog

All notable changes to Vixalune. Versions follow semantic versioning while the project is below 1.0: minor versions may change behavior.
Application updates (this file) are separate from updates to the third-party tools Vixalune installs.

## 0.2.0 - first installable release candidate

Added
- Windows installer (NSIS, per-user install, no administrator rights needed). Your data lives in `%APPDATA%\app.vixalune.desktop` and is kept when you update or uninstall (the uninstaller asks).
- First-run welcome tour: what Vixalune is, supported environments, readiness checks, storage locations, permission prompts and network-scope safeguards, the catalog, and a few starter tools.
- Settings: version, release notes, update guidance, catalog information (manifest versions and digests), storage locations.
- Application and catalog versioning information in the audit log; database schema version with an automatic backup before an upgrade.
- `VIXALUNE_DATA_DIR` to choose the data folder (portable or test installs).

Fixed
- A run cut short by a crash is shown as failed after the next start instead of "running" forever.
- Tab-separated tool output (for example Trivy log lines) keeps its fields apart.

Changed
- Stale "installing" states left by an interrupted operation are shown as failed.
- A damaged database or settings file is moved aside instead of stopping the application.

Known limitations
- The installer and the application are not code-signed; Windows SmartScreen will warn on first run. Builds are not reproducible (see docs/INSTALLATION.md).
- There is no automatic updater. To update, install the newer version over the old one (see docs/INSTALLATION.md).
- Trivy (install, run, cancel, uninstall) and Semgrep in Docker were verified on a real Windows 11 PC. Real Nmap, OWASP ZAP, Wireshark and WSL (Nikto) installs and runs have not been verified; see docs/MANUAL_INTEGRATION_TESTS.md and docs/RELEASE_CHECKLIST.md.

## 0.1.0 - development builds (phases 1 to 6)

Scaffold, catalog and interface, installation engine (winget), WSL and container support, tool execution with scope safeguards, learning labs and tutorials, security review and hardening. Not distributed.
