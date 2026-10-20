# Vixalune user guide

Vixalune is for **authorized testing and learning only**: your own computers, the practice labs it provides, and systems you have written permission to test. Nothing here makes a scan legal or authorized; that is your responsibility.

## First launch

A short welcome tour opens the first time. You can skip it; it never installs or changes anything by itself.

1. **Welcome**: what Vixalune is and where it works (Windows 11 / Windows 10 22H2, x64). WSL 2 and Docker are optional.
2. **Your computer**: readiness checks for Windows tools (winget), Linux tools (WSL 2) and container tools (Docker). "Optional" things that are missing are explained, not required. The **Setup guide** (also in the sidebar) walks through each prerequisite, shows the exact command, and asks before changing anything. Vixalune never restarts Windows without asking.
3. **Where things are stored**: your data folder (fixed, kept apart from the program), and the tools and labs folders, which you can change. Changes apply after you restart Vixalune.
4. **Permissions and safety**: what Windows prompts mean, and the network-scope safeguards (below).
5. **The catalog**: the curated tools, from files that ship with Vixalune (works offline).
6. **Your first tools**: a few tools that install through winget. *Install* opens the tool's page, where you review the plan before confirming.

## The sidebar

| Page | What it does |
|---|---|
| Dashboard | Readiness, installed tools, recent activity |
| Tool Catalog | Browse and search tools; each tool page shows what it is, license, install method, prerequisites, and actions (Install, Launch, Verify, Uninstall) |
| Installed Tools | What is installed, versions, last verification |
| Run a Tool | Pick a tool and a ready-made way to run it (a *launch profile*) |
| Running Jobs | Installs, verifications and runs; output; cancel |
| Learn | A tutorial per tool: what it does, a first run, what options mean, how to read output, common problems, an exercise |
| Learning Labs | A deliberately vulnerable practice web app (only on 127.0.0.1) and a sample project of insecure code |
| Activity | A local log of what Vixalune did (secrets masked) |
| Setup | The WSL 2 / Docker readiness guide |
| Settings | Appearance, lab list, data retention, version and release notes, catalog information |

## Installing a tool

Open the tool's page and choose **Install**. The dialog shows the method (winget, WSL apt or container image), whether administrator rights or a restart may be needed, the download size, prerequisites that are missing, and that installing accepts the package's license on your behalf. Confirm to start. Vixalune then **verifies** the tool by running it; a tool that fails verification is never shown as installed. You can cancel an install; a partly finished install is rolled back.

## Finding more tools (Discover)

The Tool Catalog has two views. **Catalog** lists the tools Vixalune's maintainers reviewed. **Discover more tools** searches the whole Windows Package Manager (winget) community repository.

1. Type a word (for example `packet capture` or `forensics`) and press Search. Your search text goes to Microsoft's package service.
2. Choose **Details…** on a result to see the publisher, licence, version and the installer hash the repository published.
3. **Add to my catalog** creates a catalog entry labelled *Community, not reviewed*. This installs nothing.
4. Open the entry and use **Install** as for any tool. You confirm the plan first, and the installer may ask for administrator rights.

Community entries can be installed, verified and uninstalled from Vixalune, but Vixalune has not checked the software, offers no guided run for it, and the AI Assistant will not recommend it. **Remove from catalog** (after uninstalling) deletes the entry. Install only software from publishers you trust.

## Running a tool

1. **Run a Tool**, choose the tool and what to run. The form starts with safe defaults pointing at your practice lab.
2. Press **Review what will run**. You see the exact command, who it will talk to and whether that is inside your lab, the time limit, whether it uses the network, and what output to expect.
3. Tick that you reviewed it (and, for targets outside your lab, confirm authorisation; see below), then **Run**.
4. Watch the output stream. You can **search**, **copy**, **save** (a Save dialog lets you choose where; only plain document types are written), **cancel**, and open the run's folder (for reports such as ZAP's HTML file).
5. The result is one of: *Completed, nothing reported*; *Completed with findings* (the tool worked and reported something; **not** an error); *Failed*; *Timed out*; *Cancelled*.

### Network-scope safeguards

- The default target is this computer. Loopback and the built-in lab are always allowed.
- A **private-network** address that is not on your lab list needs a tick box. A **public** address needs you to type its name. Reserved, multicast and unspecified addresses, and names that cannot be resolved, are never allowed.
- Names are checked by the addresses they point to. If those addresses change between your review and the run, the run is refused.
- A web address that redirects outside your lab is stopped (plain http redirects are checked; https redirects cannot be inspected).
- Add networks you manage in **Settings, Lab list** (CIDR such as `192.168.56.0/24`).

These checks catch mistakes. They cannot tell whether you are allowed to test a target.

## Learning labs

*Learning Labs, Start lab* runs a small shop website that is **deliberately** weak (no security headers, unsafe cookie, reflected search, a forgotten backup file, an unprotected admin page, an open redirect). It listens only on this computer and serves fixed pages. *Create or refresh the files* writes a sample project of insecure code for Semgrep and Trivy; the files are never run. Each tutorial's exercise tells you which lab to use.

## Your data

Run history and output are stored locally and expire: by default 90 days of history and 14 days of output (output set to 0 days is deleted at the next start). Change this, or **delete history and output now**, in Settings. Files you chose to save are never deleted by Vixalune. Output can still contain sensitive findings; treat it accordingly.

## Updates

Settings shows the version, the release notes and how to update. Vixalune never downloads or runs updates. Tool updates are separate and started by you from the tool's page.

## Getting help

See [TROUBLESHOOTING.md](TROUBLESHOOTING.md). To report a security problem see [SECURITY.md](SECURITY.md).
