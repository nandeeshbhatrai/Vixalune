# Troubleshooting

Vixalune shows an error code and a recovery sentence for every failure. This page covers the common ones. Where a fix needs a command, run it in PowerShell.

## Installing Vixalune

| Symptom | Cause and fix |
|---|---|
| "Windows protected your PC" (SmartScreen) | The installer is not code-signed yet. Check its SHA-256 against `SHA256SUMS.txt` (`Get-FileHash`), then *More info, Run anyway*. |
| The window is blank or the app closes at once | The WebView2 runtime is missing or damaged. Install the "Evergreen" runtime from Microsoft and start again. |
| Installer says a newer version is installed | The installer does not downgrade. Uninstall first (keep your data), then install the older version; see [INSTALLATION.md](INSTALLATION.md). |
| Build fails with `rc.exe` or `RC.EXE` not found | The Windows SDK's resource compiler is not on the PATH. `scripts\build-release.ps1` adds it automatically; otherwise add `C:\Program Files (x86)\Windows Kits\10\bin\<version>\x64` to the PATH. |
| Build or tests fail with "Application Control" / blocked by policy | Windows Smart App Control (enforce) blocks freshly compiled unsigned programs. It must be turned off to build; it cannot be turned back on without resetting Windows. Vixalune never changes it. |

## First run and storage

| Symptom | Cause and fix |
|---|---|
| "Vixalune started with problems" banner | Read it. It reports a damaged database or `config.toml` that was moved aside (the old file is kept next to the new one and named `.corrupt-...`, `.invalid-...` or `.newer-...`), or a catalog file that was rejected. |
| The welcome tour appears again | The window's first-run flag lives in `%LOCALAPPDATA%\app.vixalune.desktop`. Deleting that folder (or using a different `VIXALUNE_DATA_DIR`/Windows user) shows it again. Use *Skip the tour* to dismiss it. |
| A storage folder is refused | Use a full path such as `D:\Vixalune\labs`. Network paths, drives, system folders, credential folders and (for labs) anything inside AppData are refused. Changes apply after you restart Vixalune. |
| "VIXALUNE_DATA_DIR is not usable" | The environment variable must be a full local folder path outside system folders. |
| History or output is missing | Retention: by default 90 days of history and 14 days of output (Settings). Output set to 0 days is deleted at the next start. |

## WSL 2 and Docker

| Symptom | Cause and fix |
|---|---|
| WSL "Needs setup" | Open **Setup** and follow the steps. Installing WSL needs Windows' administrator prompt and usually a restart. CPU virtualization must be enabled in firmware (Vixalune cannot change that; it shows the official guide). |
| "The container engine is not running" | Start Docker Desktop and wait until it says the engine is running, then re-check. Docker Desktop needs WSL 2 (or Hyper-V on Pro/Enterprise). |
| "remote container context" / "Windows containers" | Vixalune only uses the local Linux engine. Switch Docker Desktop to Linux containers and use the default `desktop-linux` context. |
| A policy or proxy message | Your organisation may restrict WSL, containers or downloads. Vixalune reports this and does not bypass it. Ask your administrator. |

## Installing tools

| Symptom | Cause and fix |
|---|---|
| "winget was not found" | Install "App Installer" from the Microsoft Store, or update Windows. |
| "winget cannot find package ..." | Package sources can be unreachable (network, proxy) or the package was renamed. Retry; if it persists the catalog entry needs updating. |
| Install waits for approval | An installer may need administrator rights. Approve it under *Running Jobs* (Vixalune shows the exact prompt Windows will show). |
| "Verification failed" | The package installed but running the tool did not work. Use *Verify* to re-check, *Retry* to reinstall, or *Uninstall*. A failed verification is never reported as installed. |
| A tool is installed but Vixalune says "not installed" | Detection uses winget's record. Use *Refresh* or *Verify* on the tool's page. |
| Install interrupted (power loss, crash) | On the next start the job shows as failed ("interrupted"). The tool's state is re-detected; use *Verify* or *Retry*. |

## Running tools

| Symptom | Cause and fix |
|---|---|
| "out of scope" / asks for confirmation | The target is not this computer or your lab list. Add networks you manage in *Settings, Lab list*, or confirm authorisation only if you really are authorised. |
| "could not be resolved" | The name does not resolve, so its scope cannot be checked. Use the IP address or fix the name. |
| "the URL redirects outside your allowed scope" | The site answered with a redirect to another host. Scan the final address only if you own it. |
| "the addresses this target resolves to changed" | DNS answered differently between your review and the run. Review and confirm again. |
| "<tool>.exe was not found" | Install the tool first. ZAP's install folder is unverified; if it is installed but not found, report its folder. |
| Exit code 1 and "Completed with findings" | Not an error: Trivy and Semgrep use exit code 1 for "found something". |
| A run was stopped after its time limit | Each profile has a limit (shown in the preview). Scan less, or a smaller target. |
| Another run is active | One active job per tool, and at most four runs at once (`max_concurrent_processes` in `config.toml`). Wait or cancel. |
| Trivy's first run is slow | It downloads its vulnerability database (needs internet). Later runs are fast. |
| Output lines look like warnings | Many tools log progress on stderr; Vixalune shows stderr lines in amber. They are not necessarily problems. |

## Saving and files

| Symptom | Cause and fix |
|---|---|
| "only these file types can be saved" | Save as `.txt`, `.log`, `.json`, `.xml`, `.html`, `.sarif`, `.csv` or `.md`. |
| "protected folder" | Choose a folder such as Documents, not Windows, Program Files, ProgramData or a start-up folder. |

## Uninstalling and recovery

- **Program removed but data remains:** intended. Delete `%APPDATA%\app.vixalune.desktop` and `%LOCALAPPDATA%\app.vixalune.desktop` yourself to remove everything (this deletes your history).
- **Tools remain after uninstall:** Vixalune does not remove tools installed through winget/WSL/Docker. Remove them first from Vixalune, or with `winget uninstall --id <id>`, `wsl`, or `docker image rm`.
- **Leftover containers** (after a crash): `docker rm -f $(docker ps -aq --filter label=vixalune.managed=true)`.
- **Restoring an older database:** close Vixalune, copy `vixalune.db.bak-v<n>` over `vixalune.db` in the data folder.

## Still stuck?

Note the Vixalune version (Settings), your Windows version, the error code shown, and what you did. For security problems follow [SECURITY.md](SECURITY.md). Do not post output that contains addresses or findings from systems that are not yours.
