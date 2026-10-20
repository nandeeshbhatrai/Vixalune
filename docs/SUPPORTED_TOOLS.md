# Supported tools

Vixalune ships a small curated catalog. It installs each tool **from its publisher's channel at your request**. It does **not** bundle or redistribute any of them, so their licenses apply to you directly, not through Vixalune. License summaries below are for orientation and are not legal advice; read the tool's own license, especially for use at work.

| Tool | What it is for | How Vixalune installs it | Runs where | License (as declared) | Runnable from Vixalune |
|---|---|---|---|---|---|
| **Nmap** | Find hosts and open ports; identify services | winget package `Insecure.Nmap` | Windows | Nmap Public Source License (custom; restricts redistribution) | Yes: host discovery, quick scan, service scan |
| **OWASP ZAP** | Scan a web application for common weaknesses | winget package `ZAP.ZAP` | Windows (needs Java; the installer normally supplies it) | Apache-2.0 | Yes: quick scan (writes an HTML report) |
| **Trivy** | Find vulnerable dependencies and exposed secrets in a project folder | winget package `AquaSecurity.Trivy` | Windows | Apache-2.0 | Yes: scan a folder |
| **Semgrep** | Find risky code patterns | Docker image `semgrep/semgrep`, pinned by digest | Container (Docker Desktop, Linux engine) | LGPL-2.1 (the program); the public rule registry has its own license | Yes: scan the practice project, a folder with your rules, or with registry rules |
| **Wireshark** | Capture and inspect network traffic | winget package `WiresharkFoundation.Wireshark` | Windows | GPL-2.0-or-later | No (install and verify only; you start it from the Start menu) |
| **Nikto** | Scan a web server for known problems | apt package `nikto` inside WSL 2 | WSL 2 (Ubuntu) | GPL-2.0-or-later | No (install and verify only) |
| **Gitleaks** | Find hard-coded secrets in code and git history | winget package `Gitleaks.Gitleaks` | Windows | MIT | No (install and verify only; run it from a terminal) |
| **OSV-Scanner** | Check a project's dependencies against the OSV database | winget package `Google.OSVScanner` | Windows | Apache-2.0 | No |
| **Grype** | Scan container images and folders for known vulnerabilities | winget package `Anchore.Grype` | Windows | Apache-2.0 | No |
| **Syft** | List what software an image or folder contains (SBOM) | winget package `Anchore.Syft` | Windows | Apache-2.0 | No |
| **mitmproxy** | Inspect and modify HTTP/HTTPS traffic through a proxy | winget package `mitmproxy.mitmproxy` | Windows | MIT | No |
| **Autopsy** | Digital forensics on disk images | winget package `SleuthKit.Autopsy` | Windows | Apache-2.0 | No |
| **Angry IP Scanner** | Scan IP address ranges and ports (graphical) | winget package `angryziber.AngryIPScanner` | Windows | GPL-2.0 | No |

## Installation methods

- **winget** (Windows Package Manager): Vixalune runs `winget install --id <id> --exact --source winget --silent`. winget downloads the publisher's installer and checks the hash recorded in the community repository. Some installers show a Windows administrator prompt. Vixalune records the result, then runs the tool (for example `trivy.exe --version`) to confirm it works.
- **WSL apt**: Vixalune runs `apt-get install` inside your WSL 2 distribution through `wsl.exe --exec`, never through a shell. Requires WSL 2 and a distribution (the Setup guide can install Ubuntu for you after you confirm).
- **Container image**: Vixalune pulls the image by digest and runs it with strict limits (no network by default, non-root, read-only root filesystem, one read-only folder). Requires Docker Desktop with the Linux engine running. Vixalune never installs Docker Desktop for you.
- **Direct download archives** are *not supported*: Vixalune refuses them because a checksum fetched from the same place as the file proves nothing about who made it.

## Redistribution and licensing limits

- Vixalune is MIT licensed and contains none of the tools above. The installer carries only Vixalune's own program, the catalog files, the tutorials, this license and the third-party notices for Vixalune's own dependencies (`THIRD_PARTY_LICENSES.md`).
- **Nmap** and its **Npcap** driver have licenses that restrict redistribution and commercial use; this is a main reason they are installed from the publisher instead of being bundled. Check the Nmap and Npcap licenses before using them in an organisation.
- **Wireshark** and **Nikto** are GPL: you may use them freely; bundling them in a differently licensed installer would raise obligations Vixalune does not take on.
- **Docker Desktop** has subscription terms for larger organisations. Check Docker's terms for your situation. Docker Engine inside WSL is an alternative Vixalune does not manage.
- **Semgrep registry rules** (used by the "registry rules" profile) have their own license that can restrict commercial use. The practice profile uses rules shipped with the sample project.
- Installing through winget accepts the package's license on your behalf; the install dialog says so and shows the license identifier.

## What has been verified

Verification here means "run on a real Windows machine and observed", not "unit tested with mocks". See [MANUAL_INTEGRATION_TESTS.md](MANUAL_INTEGRATION_TESTS.md) and the release test report in [RELEASE_CHECKLIST.md](RELEASE_CHECKLIST.md).

| Tool | Real-machine status |
|---|---|
| Trivy | winget install, verification by running the program, and uninstall verified on Windows 11 |
| Semgrep | Image pull by digest, hardened verification run, scan of the practice project (3 findings, exit code 1 treated as findings) and image removal verified on Windows 11 with Docker Desktop |
| Nmap, OWASP ZAP, Wireshark | Catalog entries validated; install and runs **not** verified on real hardware by the developers unless the release report says otherwise. The ZAP command-line flags and install folder are the least certain |
| Nikto | Mock-tested only; not installed in a real WSL distribution |
| Gitleaks, OSV-Scanner, Grype, Syft, mitmproxy, Autopsy, Angry IP Scanner | Added after reading each package's facts (id, version, publisher, licence, installer type) from the real `winget show` on 2026-10-20 and validating the manifests. **Not installed, run or uninstalled on a test machine.** They have no run profiles; the only check after installing is that winget reports the package as installed. Treat them as reviewed on paper, untested in practice |

## Discover: packages that are not in the reviewed catalog

The catalog's **Discover more tools** tab searches the winget community repository (thousands of packages). Anything you add from there becomes a **community entry**: it is labelled *Community, not reviewed*, it can only install one winget package (pinned to the version you were shown) and check that winget reports it as installed, it has no run profiles or dependencies, and the AI Assistant never recommends it. Adding installs nothing; installing is a separate confirmation. See [ASSISTANT.md](ASSISTANT.md) and [PRIVACY.md](PRIVACY.md) for what that means for the AI features and for what is sent where.

## Adding or changing tools

See [CONTRIBUTING.md](CONTRIBUTING.md) and [TOOL_MANIFEST_SCHEMA.md](TOOL_MANIFEST_SCHEMA.md).
