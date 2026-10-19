# Manual integration tests (Windows, WSL 2, containers)

Automated tests use a simulated `wsl.exe`/`docker.exe`. They prove Vixalune's logic, state, diagnoses and
safety checks, but **only these manual runs prove the real programs behave as assumed.** Run them on a
Windows 10 22H2 / Windows 11 x64 machine (a throw-away VM is best for the setup tests). Never run them on
a production or work-managed PC without your administrator's agreement.

Status when this document was written (2026-10-19, Windows 11 Home build 26300): the read-only
automated checks in section A were run for real; **sections B-F could not be run because that PC has no WSL
and its Docker Desktop engine cannot start without WSL 2.** Nothing below has therefore been exercised
against a live WSL distribution or a running Docker engine yet.

## A. Read-only detection (safe, changes nothing)

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test platform_integration -- --ignored --nocapture
```

Expected on any machine: 3 tests pass; the printed JSON report matches `wsl --status`, `wsl --list --verbose`,
`docker version` and `winget --version` run by hand; `system.is_elevated` is `false`.
Record the printed `readiness` block and diagnoses in the test log. Pass = the readiness states are what a
human reading those four commands would conclude.

## B. First-run wizard and WSL setup (needs administrator approval and a restart; use a VM)

Start state: Windows without WSL.

1. `npm run tauri:dev`. Expected: the Setup wizard opens on first launch. **Nothing runs yet.**
2. Step 1: facts match reality; "Windows features" lists optional components as `unknown` with the explanation.
3. Step 4, "Install WSL" → *Review and run…*. The dialog shows `wsl.exe --install --no-distribution`, the UAC
   note, and the checkbox. Confirm that **Run this step stays disabled until the box is ticked** and that
   Cancel changes nothing.
4. Tick, run. Expected: Windows shows its own UAC prompt. **Decline once**: the job fails with
   "you declined the administrator prompt; nothing was changed" and `wsl --status` is unchanged.
5. Run again and approve. Expected: the job ends *Succeeded*; if Windows needs a restart the job message says
   so. Step 5 shows the restart section. **Confirm no restart happened by itself.**
6. Step 5 → *Restart Windows…*: dialog warns about unsaved work; choose *Schedule restart*, then immediately
   *Cancel the restart* (equivalent to `shutdown /a`). Expected: Windows shows the restart is aborted. Repeat and let
   it restart once to finish setup.
7. After the restart re-open Setup → step 6 → *Re-check*. WSL is now installed; "Install a Linux distribution"
   is available (it was greyed-out *Finish the step before this one first* until WSL existed).
8. Run *Install a Linux distribution* (no UAC expected). Expected output includes
   `> wsl.exe --install --distribution Ubuntu-24.04 --no-launch`; on success `wsl --list --verbose` shows
   `Ubuntu-24.04` version 2 and Re-check reports **Linux tools: Ready**.

Failure variants worth trying in the VM: disable virtualization in the VM settings and rerun step 4/8 (expect
code `virtualization_disabled` with the BIOS/UEFI guidance and **no** offer to install anything); fill the
disk so < 8 GB are free (expect `needs setup` with the exact numbers).

## C. Linux tool workflow: Nikto in WSL

Prerequisite: B complete (WSL 2 + Ubuntu).

1. Catalog → Nikto → Install. The plan dialog must say it installs *inside WSL*, that **WSL is not a sandbox**,
   needs no Windows administrator approval, and shows license/integrity text.
2. Job output must contain `> wsl.exe --distribution Ubuntu-24.04 --user root --cd /tmp --exec …apt-get update`
   then `…apt-get install -y --no-install-recommends -- nikto`. Expected: *Succeeded*, level
   "package record only" (package check only: `nikto` options were not verified against a live distribution).
3. Installed Tools shows version `1:2.1.5-3.1` (or newer) and location `WSL: Ubuntu-24.04`. In Ubuntu,
   `dpkg -l nikto` agrees.
4. Stop the distribution (`wsl --terminate Ubuntu-24.04`), click *Verify*: it must start the distro and pass.
5. Error checks: (a) `wsl --shutdown`, uninstall WSL's distro (`wsl --unregister Ubuntu-24.04`) then Verify →
   `wsl_unavailable`/`distribution_missing`, not "not installed"; (b) inside Ubuntu run
   `sudo flock /var/lib/dpkg/lock-frontend sleep 120` and install again → `package_manager_error`
   ("Could not get lock") attributed to the **package manager**, not WSL; (c) disconnect the network → apt
   failure shown as `package_download_failed`.
6. Uninstall. Expected: `apt-get remove` runs; the message mentions `apt-get autoremove`; `dpkg -l nikto` shows
   removed. Packages pulled in as dependencies remain (by design).

## D. Container workflow: Semgrep in Docker

Prerequisite: Docker Desktop installed, **Linux containers**, engine running (`docker version` shows Server).

1. Catalog → Semgrep → Install. Plan notes: pulled by digest from `docker.io`, no signature verification, not a safe
   sandbox. Expected job output `> docker pull docker.io/semgrep/semgrep@sha256:529ee8a2…`; *Succeeded* with
   verification level *Executable* (a hardened offline container ran `semgrep --version`).
   `docker image ls --digests` shows the digest. **If verification fails because the image needs a writable
   directory or different user, that is a finding about the manifest; record the output.**
2. Launch → "Scan a folder with your own rules (offline)". Pick a small folder such as `D:\labs\sample`
   containing `.semgrep.yml`. *Review what will run*: network "none", the folder read-only, user 65532:65532.
   Approve and run. While it runs, `docker inspect` the container: `HostConfig.NetworkMode=none`,
   `ReadonlyRootfs=true`, `CapDrop=[ALL]`, `SecurityOpt=[no-new-privileges]`, `Binds` contains only your folder with `ro`,
   `Privileged=false`, no `docker.sock` anywhere. Output streams into Running Jobs.
3. Try to break the rules (each must be refused *before* anything starts): folder `C:\`, `C:\Windows`,
   `C:\Users\<you>`, `C:\Users\<you>\.ssh`, a UNC path, a junction to `C:\Windows`
   (`mklink /J D:\labs\w C:\Windows`), a path containing a comma.
4. "…registry rules (needs internet)": unticked network → refused; ticked → summary says **ALLOWED** and warns;
   `docker inspect` shows a bridge network.
5. Cancel a long scan: the job ends *Cancelled* and `docker ps -a --filter label=vixalune.tool=semgrep` is empty.
6. Failure checks: quit Docker Desktop then Verify/Install → `container_runtime_stopped` with start guidance and
   **no** fallback; `docker context use <ssh or tcp context>` → `remote_container_context`; switch to Windows
   containers → `unsupported_container_os`; set a proxy that blocks the registry → `proxy_restricted`.
7. Uninstall. Expected: leftover labelled containers removed, `docker image rm` of the digest reference.

## E. Corporate and policy behaviour

On a PC with WSL/Hyper-V group policy: the wizard lists the policy keys as notes and any failure keeps the
Windows error text; **Vixalune must not attempt to change policy, registry or security settings.** On a PC behind a
proxy the proxy host (never credentials) appears under "Worth knowing".

## F. Cleanup of leftovers after testing

```powershell
wsl --unregister Ubuntu-24.04                      # removes the distribution and ITS DATA
docker rm -f $(docker ps -aq --filter label=vixalune.managed=true)
docker image rm docker.io/semgrep/semgrep@sha256:529ee8a277ec8adc5b534d7c74eea0a47e9de21d62852b6ba7ac6ba9566845c3
```
