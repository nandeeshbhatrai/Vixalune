# Installing Vixalune

## What you need

| Requirement | Details |
|---|---|
| Windows | Windows 11, or Windows 10 version 22H2 (build 19045) or newer. 64-bit (x64) only. ARM64 is untested and unsupported. |
| Disk space | About 20 MB for Vixalune. Tools need more (the Semgrep container image is a few hundred MB; Docker Desktop and WSL are several GB). |
| Microsoft Edge WebView2 Runtime | Draws the window. Already present on Windows 11 and current Windows 10. If it is missing the installer downloads Microsoft's small bootstrapper, which needs an internet connection once. |
| Administrator rights | **Not needed** to install or run Vixalune (it installs for your user only). Some tools' installers, and the single step that installs WSL, show Windows' own administrator prompt when you choose them. |

Optional, only for the tools that need them (Vixalune explains and checks these on first run, and works without them):

| Optional component | Needed for | Notes |
|---|---|---|
| Windows Package Manager (winget, "App Installer") | Installing Windows tools (Nmap, ZAP, Trivy, Wireshark) | Included with current Windows 11 and Windows 10 22H2 updates; otherwise install "App Installer" from the Microsoft Store |
| WSL 2 with a Linux distribution | Nikto and other Linux tools | Setup guide can install WSL and Ubuntu after you confirm; needs a restart and CPU virtualization enabled in firmware |
| Docker Desktop (Linux containers) | Semgrep and container labs | You install it yourself; Vixalune never installs Docker Desktop. Check Docker's licence terms for your organisation |
| Internet access | Installing tools, the first Trivy run (downloads its vulnerability database), pulling container images | The catalog, tutorials, labs and settings work offline |

Vixalune does not bundle any security tool. See [SUPPORTED_TOOLS.md](SUPPORTED_TOOLS.md).

## Installing

1. Download `Vixalune_<version>_x64-setup.exe` and `SHA256SUMS.txt` from the project's releases page: <https://github.com/nandeeshbhatrai/Vixalune/releases>.
2. **Check the download.** In PowerShell:
   ```powershell
   Get-FileHash .\Vixalune_0.2.0_x64-setup.exe -Algorithm SHA256
   ```
   The hash must equal the one in `SHA256SUMS.txt`. A matching hash shows the file was not damaged or swapped on the way; it does not prove who built it.
3. **The installer is not code-signed yet.** Windows SmartScreen may say "Windows protected your PC". Choose **More info**, then **Run anyway**, only if the hash matched. Releases will say clearly if and when they become signed (check *Properties, Digital Signatures* on the file).
4. Run the installer. It installs for your user, with no administrator prompt, to `%LOCALAPPDATA%\Vixalune` (Tauri's per-user default; in the developers' tests the silent installer was pointed at a scratch folder with `/D=`), adds a Start-menu entry, and registers an uninstaller under *Settings, Apps*.
5. Start **Vixalune** from the Start menu. On the first launch a short welcome tour opens (see the [user guide](USER_GUIDE.md)).

Silent install (for scripting): `Vixalune_<version>_x64-setup.exe /S` (add `/D=C:\Path\To\Folder` as the **last** argument to choose the folder).

## Where things live

| What | Where | Removed by uninstall? |
|---|---|---|
| Program, catalog files, licences | the install folder (`%LOCALAPPDATA%\Vixalune` by default) | Yes |
| Settings, database (run history, installed-tool records, audit log), run folders | `%APPDATA%\app.vixalune.desktop\` (`config.toml`, `settings.json`, `vixalune.db`, `runs\`) | **No**; the interactive uninstaller offers to delete it |
| Window data (first-run flag) | `%LOCALAPPDATA%\app.vixalune.desktop\` | with the same option |
| Practice labs (sample code you can open) | `%USERPROFILE%\Vixalune\labs\` (changeable in the welcome tour) | No |
| Files you saved with *Save output* | where you chose | Never touched |
| Tools installed through winget, WSL or Docker | managed by those systems | No: remove them from Vixalune before uninstalling it, or with winget / `wsl` / Docker |

Set the environment variable `VIXALUNE_DATA_DIR` to a full folder path before starting Vixalune to keep settings and data somewhere else (for example on another drive, or in a throw-away folder for testing). Vixalune refuses a value that is relative, a network path or a protected folder.

## Updating

Vixalune never updates itself and never downloads or runs updater scripts. To update:

1. Download the newer installer and check its SHA-256 as above.
2. Close Vixalune and run the new installer. It installs **over** the existing version, in the same folder, and keeps your data.
3. On the next start, if the database layout changed, Vixalune saves a backup (`vixalune.db.bak-v<old>`) before upgrading it and tells you where it is.

Updates to the tools themselves (Nmap, Trivy, ...) are separate; use the tool's page, or winget directly.

**If an update goes wrong:** your data is not in the program folder, so uninstalling and reinstalling is safe. The installer refuses to install an older version over a newer one; uninstall first, keep the data folder, then install the older version. If the newer version had upgraded the database, the older one sets that database aside untouched (and says where) and starts empty; copy the backup `vixalune.db.bak-v<old>` over `vixalune.db` with Vixalune closed to get your older history back.

## Uninstalling

*Settings, Apps, Installed apps, Vixalune, Uninstall*, or run `uninstall.exe` in the install folder (`/S` for silent). This removes the program, the Start-menu entry and the registry entry, and keeps your data unless you tick the box offered by the interactive uninstaller. It does **not** remove tools that Vixalune installed for you. To remove those first, use *Uninstall* on each tool's page in Vixalune.

## Building from source

For developers and for people who want to build the installer themselves.

Requirements: Windows 11 x64, Node.js 22 or newer (24.x was used), Rust **1.99.1** (MSVC) with the Visual Studio Build Tools (C++) and the Windows SDK, WebView2. Smart App Control, if set to enforce, blocks freshly compiled unsigned programs and must be off; Vixalune never changes it.

```powershell
git clone https://github.com/nandeeshbhatrai/Vixalune.git
cd Vixalune
git checkout v0.2.0                # a release tag
.\scripts\build-release.ps1        # npm ci, tests, audit, licence list, locked build, SHA-256
```

The script writes `release\Vixalune_<version>_x64-setup.exe`, `release\SHA256SUMS.txt` and `release\BUILD-INFO.txt` (commit, tool versions, hash, signature status). The first run downloads the NSIS packaging tool that Tauri uses (into your user profile). The script refuses to build with a Rust version other than 1.99.1 unless you set `VIXALUNE_ALLOW_OTHER_RUST=1`.

**Reproducibility:** the build uses locked dependencies (`npm ci`, `cargo --locked`) and records its inputs, but it is **not reproducible**: two consecutive builds of the same sources on the same machine and path produced different installers (hash differs). Compare an installer only against the hash its publisher gives you for that exact file; do not expect to rebuild an identical one.

**Signing:** nothing is signed unless you provide a code-signing certificate; see [RELEASE_CHECKLIST.md](RELEASE_CHECKLIST.md).
