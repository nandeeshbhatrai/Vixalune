# Release checklist

Use this for every release. Tick items only after doing them; write what you observed. A release that skips an item must say so in its notes.

## 0. Decide the version

- [ ] Choose the version (semantic versioning; below 1.0 minor versions may change behavior).
- [ ] The **same** version is in `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json` (a unit test checks this) and `CHANGELOG.md` has a `## <version> - title` section (the same test checks this).
- [ ] `package-lock.json` and `src-tauri/Cargo.lock` carry the version and are committed.
- [ ] Release notes in `CHANGELOG.md` list user-visible changes and **known limitations**, including anything not verified.

## 1. Clean build environment

Documented build environment: Windows 11 x64, Node.js 24.x (22 or newer is required), npm 11.x, Rust 1.99.1 (MSVC), Visual Studio Build Tools (C++), WebView2 runtime. Smart App Control must be off to compile.

- [ ] Fresh clone of the tagged commit; `git status` clean.
- [ ] `scripts\build-release.ps1` runs `npm ci`, the type check, all tests, `cargo test --locked`, `npm audit`, regenerates `THIRD_PARTY_LICENSES.md`, builds with `--locked`, and writes `release\SHA256SUMS.txt` and `release\BUILD-INFO.txt`.
- [ ] `THIRD_PARTY_LICENSES.md` has no uncommitted change (regenerate and commit if it does) and no unreviewed UNKNOWN or strong-copyleft entry.
- [ ] `npm audit` and an OSV query of `Cargo.lock` reviewed; every advisory either fixed or explained in `docs/SECURITY_REVIEW.md`.

## 2. Signing (not yet available for this project)

Status: **no code-signing certificate or infrastructure exists yet, so releases are unsigned.** Do not describe a build as signed unless `Get-AuthenticodeSignature` reports `Valid` and names the expected publisher.

- [ ] Obtain a code-signing certificate (OV/EV) or use Azure Trusted Signing; keep the key in a hardware token or a managed signing service, never in the repository or in CI variables that pull requests can read.
- [ ] Run `scripts\build-release.ps1 -Sign` with `VIXALUNE_SIGN_THUMBPRINT` set, or sign the installer in your own pipeline with a timestamp (RFC 3161).
- [ ] Verify: `Get-AuthenticodeSignature release\Vixalune_<version>_x64-setup.exe` shows `Valid`, the right subject and a timestamp.
- [ ] Application updates: if an updater is ever added, use Tauri's signed-update mechanism with a separate minisign key; the private key never lives in the build pipeline. Until then there is no updater (a unit test asserts none is configured).

## 3. Installer tests (on a machine that is not your development machine where possible)

Record the Windows version, whether it was clean (new user profile or fresh VM) and the outcome of each line.

- [ ] **Clean install** (`/S` silent and interactive): no administrator prompt; installs to `%LOCALAPPDATA%\Vixalune`; Start-menu entry; the application launches **from the installed location**; the bundled `manifests\`, `LICENSE.txt` and `THIRD_PARTY_LICENSES.md` are present next to the program.
- [ ] **First run**: the welcome tour opens once; readiness checks work with and without WSL and Docker; the storage step saves; the catalog appears; skipping works; restarting the app does not show the tour again.
- [ ] **Offline**: with the network disabled the catalog, tutorials, labs and settings work; installing a tool fails with a clear message and the app stays usable.
- [ ] **Tool install and verification**: install one winget tool (for example Trivy), see it verified, run it against the practice sample project, cancel a run, uninstall the tool.
- [ ] **Interrupted operation**: kill the application during an install; on the next start the job shows as failed ("interrupted"), the tool's state reflects reality, nothing hangs.
- [ ] **Upgrade**: install the previous release, create data (settings, a run), install the new release over it: same install folder, data kept, version updated, no administrator prompt, no duplicate Start-menu entry, database backup message only if the layout changed.
- [ ] **Downgrade refused** (installer does not allow older over newer) and, if tried manually, the app moves a newer database aside rather than damaging it.
- [ ] **Uninstall**: removes the program, shortcuts and registry entry; keeps `%APPDATA%\app.vixalune.desktop` unless "delete application data" is chosen; does not remove tools Vixalune installed through winget, WSL or Docker (say so in the notes) or files you saved yourself.
- [ ] **Reinstall after uninstall** with kept data: settings and history return.
- [ ] Antivirus / SmartScreen behavior noted (unsigned builds will warn).

## 4. Documentation

- [ ] `README.md`, `docs/INSTALLATION.md`, `USER_GUIDE.md`, `SUPPORTED_TOOLS.md`, `TROUBLESHOOTING.md`, `SECURITY.md`, `CONTRIBUTING.md` and this checklist are current; screenshots refreshed (`SCREENSHOTS=1 npm run test:e2e`).
- [ ] Supported Windows versions, architecture, dependencies, license limits and known issues are stated and match what was tested.
- [ ] Test counts in the README are real numbers from the last run.

## 5. Publish

- [ ] Tag the commit (`v<version>`), create the GitHub release, attach the installer, `SHA256SUMS.txt` and `BUILD-INFO.txt`, paste the release notes, state clearly whether the installer is signed.
- [ ] Download the published installer on another machine and compare its SHA-256 with the published value.
- [ ] Keep the previous release available for rollback.

## 6. After release

- [ ] Watch for installation problems; fix forward with a new version (do not replace published files).
- [ ] If a serious problem is found, mark the release as such in its notes and publish a fixed version.

## Rollback

Application rollback means installing the previous installer over the new one **after** checking the notes: the installer refuses to downgrade by default, so uninstall first (keep the data folder). If the newer version upgraded the database layout, the older version moves the newer database aside untouched and starts empty; the backup `vixalune.db.bak-v<old>` (made before the upgrade) can be copied back to `vixalune.db` while the app is closed. Tool installs are never rolled back by Vixalune; use winget, WSL or Docker directly for that.

---

# Release test report: 0.2.0

Tested 2026-10-19/20 on the developers' Windows 11 Home (build 26300, x64) PC, **not a clean machine** (it has the development toolchain, WSL 2 with Ubuntu, Docker Desktop and winget; Windows Sandbox is unavailable on Home). The installer was driven with `/S` into a scratch folder with `VIXALUNE_DATA_DIR` and a scratch WebView2 profile, and the real application window was operated through WebView2's remote-debugging port (`scripts/drive-installed.mjs`). The installers tested were builds of the same sources as the released 0.2.0 (the upgrade used a temporary 0.2.1 build); the final 0.2.0 artifact was built by the full `scripts/build-release.ps1` but was **not itself installed and launched**.

| Check | Result |
|---|---|
| Full release script (npm ci, type check, 128 UI tests, 285 Rust tests, npm audit, licence list, locked build) | Passed |
| Clean install, silent, no administrator prompt | Passed: exit 0; program, `manifests\`, `LICENSE.txt`, `THIRD_PARTY_LICENSES.md`, uninstaller present; uninstall registry entry (version, location) and Start-menu shortcut created |
| Launches from the installed location; bundled frontend (not a dev server); catalog loaded from the installed `manifests\` | Passed (process path = install folder; window URL `http://tauri.localhost`; 6 catalog tools) |
| First-run welcome tour on first launch, not on the second | Passed |
| Readiness checks against the real machine (winget, WSL, Docker all present here) | Passed. **Missing-runtime behavior was not tested on a real machine** (this PC has all three); it is covered by unit tests and demo-data UI tests only |
| Storage: invalid path refused, labs folder in AppData refused, change saved, applied after restart (sample project created in the new folder) | Passed |
| Tool install and verification through the UI (Trivy 0.76.0 via winget) | Passed: plan shown, installed, verified by running `trivy.exe` |
| Run, live output, cancel, outcome (Trivy on the practice sample project) | Passed: preview shown; cancel left no `trivy.exe`; completed run = "Completed with findings", exit 1, 68 lines |
| App killed mid-run (forced) | Passed: `trivy.exe` ended with the app (job object); on restart the job was closed as failed/interrupted. **Found a bug here**: the run's history record stayed "running"; fixed (run records without an outcome are closed at start-up) and re-verified |
| Tool uninstall through the UI | Passed (winget confirms the package is gone) |
| Upgrade 0.2.0 to 0.2.1 over the existing install | Passed: same folder, one uninstall entry, one shortcut, no administrator prompt, data files unchanged, history and settings intact, version 0.2.1 |
| Uninstall (`/S`) | Passed: program folder, shortcut and registry entry removed; data folder and labs kept |
| Reinstall over kept data | Passed: history and settings returned |
| Offline catalog | Not tested with the network off. The catalog and tutorials are bundled files and loaded without network access (unit tests); the real offline behavior of installs is untested |
| Logs and data persistence | Database, `config.toml` persisted across restarts and upgrade |
| Interactive installer and uninstaller screens (language, directory, "delete app data" checkbox) | **Not tested** (only silent mode) |
| Nmap, OWASP ZAP, Wireshark, Nikto real installs and runs | **Not tested** |
| SmartScreen / antivirus behavior | Not observed (installer launched from the command line) |

Artifacts: `release\Vixalune_0.2.0_x64-setup.exe`, `release\SHA256SUMS.txt`, `release\BUILD-INFO.txt` (not committed; `release\` is git-ignored). **Signing: not signed** (no certificate). **Reproducibility: not verified**: two consecutive builds of the same sources on the same machine and path gave different installers.

Remaining before a public release: sign the installer; test on a clean VM including an interactive install, a machine without WSL/Docker, and offline; install and run Nmap and ZAP for real; decide whether reproducible builds matter and, if so, work out what varies (embedded timestamps and paths).
