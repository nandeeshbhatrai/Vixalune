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

## Adding or changing tools

See [CONTRIBUTING.md](CONTRIBUTING.md) and [TOOL_MANIFEST_SCHEMA.md](TOOL_MANIFEST_SCHEMA.md).
