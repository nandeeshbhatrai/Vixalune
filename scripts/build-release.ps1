<#
.SYNOPSIS
  Builds the Vixalune Windows installer from a clean checkout and records what was built.

.DESCRIPTION
  Steps: install locked JavaScript dependencies, run the test suites, regenerate the third-party license list,
  build the installer with locked Rust dependencies, then copy it to .\release with a SHA-256 file and a report of its
  Authenticode signature status. Nothing is signed unless you provide a certificate (see docs/RELEASE_CHECKLIST.md).

.PARAMETER SkipInstall  Skip "npm ci" (use the node_modules that are already there).
.PARAMETER SkipTests    Skip the test suites (not for real releases).
.PARAMETER OutDir       Where the artifacts go (default: release).
.PARAMETER Sign         Sign the installer with signtool using the certificate named in VIXALUNE_SIGN_THUMBPRINT.
#>
param(
  [switch]$SkipInstall,
  [switch]$SkipTests,
  [switch]$Sign,
  [string]$OutDir = "release"
)
# Native programs write progress to stderr; success is judged by exit code in Run, not by stderr output.
$ErrorActionPreference = "Continue"
Set-Location (Split-Path $PSScriptRoot -Parent)
$env:Path = [Environment]::GetEnvironmentVariable("Path", "Machine") + ";" + [Environment]::GetEnvironmentVariable("Path", "User")

# The resource compiler (rc.exe) ships with the Windows SDK but is not always on the PATH; the Windows resource step of the build needs it.
if (-not (Get-Command rc.exe -ErrorAction SilentlyContinue)) {
  $rc = Get-ChildItem (Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\bin\*\x64\rc.exe") -ErrorAction SilentlyContinue | Sort-Object FullName -Descending | Select-Object -First 1
  if ($rc) { $env:Path = "$($rc.DirectoryName);$env:Path"; Write-Host "using rc.exe from $($rc.DirectoryName)" }
  else { Write-Warning "rc.exe (Windows SDK) was not found; the build may fail" }
}

function Step($m) { Write-Host "`n== $m" -ForegroundColor Cyan }
function Run($exe, [string[]]$a) {
  & $exe @a
  if ($LASTEXITCODE -ne 0) { throw "$exe $($a -join ' ') failed with exit code $LASTEXITCODE" }
}

Step "Tool versions"
Run node @("--version"); Run npm.cmd @("--version"); Run rustc @("--version"); Run cargo @("--version")
$expectedRust = "1.99.1"
if (-not ((rustc --version) -match [regex]::Escape("rustc $expectedRust "))) {
  if ($env:VIXALUNE_ALLOW_OTHER_RUST) { Write-Warning "building with a different Rust than the documented $expectedRust" }
  else { throw "expected rustc $expectedRust (see docs/INSTALLATION.md). Set VIXALUNE_ALLOW_OTHER_RUST=1 to build anyway; the result is then not the documented build." }
}
$version = (Get-Content src-tauri\tauri.conf.json -Raw | ConvertFrom-Json).version
$commit = (git rev-parse --short HEAD) 2>$null
$dirty = (git status --porcelain) 2>$null
Write-Host "version $version, commit $commit$(if ($dirty) { ' (WORKING TREE HAS UNCOMMITTED CHANGES)' })"

if (-not $SkipInstall) { Step "npm ci (locked dependencies)"; Run npm.cmd @("ci") }

if (-not $SkipTests) {
  Step "Frontend tests, type check"
  Run npm.cmd @("run", "typecheck")
  Run npm.cmd @("test")
  Step "Rust tests (locked)"
  Run cargo @("test", "--manifest-path", "src-tauri/Cargo.toml", "--locked")
  Step "Dependency audit"
  Run npm.cmd @("audit", "--omit=dev")
}

Step "Third-party license list"
Run node @("scripts/gen-third-party.mjs")

Step "Build installer (cargo --locked)"
Run npm.cmd @("run", "tauri:build", "--", "--", "--locked")

$installer = Get-ChildItem "src-tauri\target\release\bundle\nsis\*-setup.exe" | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $installer) { throw "no installer was produced" }

if ($Sign) {
  Step "Sign installer"
  if (-not $env:VIXALUNE_SIGN_THUMBPRINT) { throw "set VIXALUNE_SIGN_THUMBPRINT to the thumbprint of a code-signing certificate in your certificate store" }
  $signtool = (Get-Command signtool.exe -ErrorAction SilentlyContinue).Source
  if (-not $signtool) { throw "signtool.exe (Windows SDK) was not found" }
  Run $signtool @("sign", "/sha1", $env:VIXALUNE_SIGN_THUMBPRINT, "/fd", "SHA256", "/tr", "http://timestamp.digicert.com", "/td", "SHA256", $installer.FullName)
}

Step "Collect artifacts in $OutDir"
New-Item -ItemType Directory -Force $OutDir | Out-Null
Copy-Item $installer.FullName $OutDir -Force
$dest = Join-Path $OutDir $installer.Name
$hash = (Get-FileHash $dest -Algorithm SHA256).Hash.ToLower()
"$hash *$($installer.Name)" | Set-Content (Join-Path $OutDir "SHA256SUMS.txt") -Encoding ascii
$sig = Get-AuthenticodeSignature $dest
$report = @(
  "Vixalune $version",
  "commit: $commit$(if ($dirty) { ' (uncommitted changes present)' })",
  "built: $((Get-Date).ToUniversalTime().ToString('s'))Z",
  "installer: $($installer.Name) ($([math]::Round($installer.Length / 1MB, 2)) MiB)",
  "sha256: $hash",
  "authenticode: $($sig.Status) $(if ($sig.SignerCertificate) { $sig.SignerCertificate.Subject })",
  "rustc: $(rustc --version)",
  "node: $(node --version)"
)
$report | Set-Content (Join-Path $OutDir "BUILD-INFO.txt") -Encoding ascii
$report | ForEach-Object { Write-Host $_ }
