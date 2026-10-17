//! Tool manifest model (schema v2) and validation.
//!
//! A manifest is *data only*: it selects one of a closed set of install adapters and supplies
//! typed parameters. It can never carry scripts. Every string that may become a process argument
//! or a path is validated here, and downloads must be pinned by SHA-256 / image digest.

use super::errors::{AppError, AppResult};
use super::ids::ToolId;
use super::paths;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use url::Url;

pub const SUPPORTED_SCHEMA_VERSION: u32 = 2;
pub const MAX_MANIFEST_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    WindowsNative,
    Wsl,
    Container,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Reconnaissance,
    WebApplication,
    Network,
    Passwords,
    Vulnerability,
    Forensics,
    Utilities,
}

/// Curator-asserted maintenance status of the upstream project (not checked at runtime).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceStatus {
    Active,
    Stable,
    Deprecated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Prerequisite {
    Winget,
    Wsl2,
    Docker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Architecture {
    X64,
    Arm64,
}

/// A prerequisite plus the information a user needs to decide: why, how big, admin, restart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrerequisiteSpec {
    pub kind: Prerequisite,
    pub reason: String,
    pub approx_size_mb: Option<u32>,
    #[serde(default)]
    pub requires_admin: bool,
    #[serde(default)]
    pub requires_restart: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceBasis {
    /// Binaries come from the upstream project's own release channel.
    OfficialUpstream,
    /// Distributed through a package repository the package manager vets.
    PackageRepository,
    CommunityMaintained,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub publisher: String,
    pub basis: ProvenanceBasis,
    /// Where the curator checked the facts (https).
    pub source_url: String,
    /// `YYYY-MM-DD` on which the install facts were last verified.
    pub verified_on: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowsSupport {
    /// Minimum Windows build number (19045 = Windows 10 22H2, 22000 = Windows 11).
    pub min_build: u32,
    pub architectures: Vec<Architecture>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VersionPolicy {
    /// Exact version passed to adapters that support pinning (winget `--version`).
    pub pinned: Option<String>,
    /// Lowest acceptable version. Schema-only in this release: recorded and shown, not yet enforced.
    pub minimum: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Storage {
    /// Sub-folder of the app tools directory, for adapters that install into it.
    pub install_subdir: Option<String>,
    pub approx_download_mb: Option<u32>,
    pub approx_disk_mb: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrityLevel {
    /// The package manager verifies installer hashes from its own manifest. Vixalune does not
    /// re-check them.
    PackageManager,
    Sha256Pinned,
    ImageDigest,
    /// No integrity data exists. The UI states this plainly.
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Integrity {
    /// Applies to the first (preferred) entry of `install`.
    pub level: IntegrityLevel,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkNeeds {
    pub install: bool,
    pub run: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElevationNeed {
    None,
    /// Depends on the installer or machine; the user is told before it can happen.
    Possible,
    Required,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Privileges {
    pub install: ElevationNeed,
    pub run: ElevationNeed,
    #[serde(default)]
    pub may_require_restart: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lifecycle {
    pub can_update: bool,
    pub can_uninstall: bool,
}

/// How a tool is installed. A closed set of structured methods; there is deliberately no
/// "run this script" variant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum InstallSpec {
    /// `winget install --id <package_id> --exact [--version <pinned>]`
    Winget { package_id: String },
    /// `apt-get install -y -- <packages...>` inside the configured WSL distribution.
    WslApt { packages: Vec<String> },
    /// `docker pull <image>@<digest>`
    ContainerImage { image: String, digest: String },
    /// HTTPS download verified against `sha256`, extracted into the app-owned tools directory.
    DownloadArchive { url: String, sha256: String, executable: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallMethod {
    Winget,
    WslApt,
    ContainerImage,
    DownloadArchive,
}

impl InstallSpec {
    pub fn runtime(&self) -> RuntimeKind {
        match self {
            Self::Winget { .. } | Self::DownloadArchive { .. } => RuntimeKind::WindowsNative,
            Self::WslApt { .. } => RuntimeKind::Wsl,
            Self::ContainerImage { .. } => RuntimeKind::Container,
        }
    }
    pub fn method(&self) -> InstallMethod {
        match self {
            Self::Winget { .. } => InstallMethod::Winget,
            Self::WslApt { .. } => InstallMethod::WslApt,
            Self::ContainerImage { .. } => InstallMethod::ContainerImage,
            Self::DownloadArchive { .. } => InstallMethod::DownloadArchive,
        }
    }
}

/// One verification step. Installation is only reported successful when every declared check passes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum VerifyCheck {
    /// The adapter's own status query reports the package as installed.
    PackageInstalled,
    /// Run an executable with fixed arguments and inspect its output.
    ExecutableOutput {
        runtime: RuntimeKind,
        /// Bare file name, e.g. `nmap.exe`.
        executable: String,
        #[serde(default)]
        args: Vec<String>,
        /// Extra folders to search, each starting with a known placeholder such as `{ProgramFiles}`.
        #[serde(default)]
        locations: Vec<String>,
        expect_output_contains: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifySpec {
    pub timeout_secs: Option<u32>,
    pub checks: Vec<VerifyCheck>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParamValue {
    Bool(bool),
    Int(i64),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ParamKind {
    Text { max_len: Option<usize> },
    /// Hostname, IP address or CIDR range.
    Host,
    Integer { min: i64, max: i64 },
    Boolean,
    Choice { options: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParameterDef {
    pub name: String,
    pub label: String,
    #[serde(default)]
    pub description: String,
    pub kind: ParamKind,
    #[serde(default)]
    pub required: bool,
    pub default: Option<ParamValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ArgTemplate {
    /// A fixed argument written by the manifest author.
    Literal { value: String },
    /// The validated value of a parameter, as one argument.
    Value { param: String },
    /// `flag` is emitted when the boolean parameter is true.
    Flag { param: String, flag: String },
    /// `flag` followed by the value, emitted only when the parameter has a value.
    #[serde(rename = "option")]
    OptionValue { param: String, flag: String },
}

/// A launch profile: one named way to start the tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunSpec {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub description: String,
    pub runtime: RuntimeKind,
    pub executable: String,
    #[serde(default)]
    pub parameters: Vec<ParameterDef>,
    #[serde(default)]
    pub args: Vec<ArgTemplate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolManifest {
    pub schema_version: u32,
    /// Revision of this tool's definition (`MAJOR.MINOR.PATCH`), independent of `schema_version`.
    pub manifest_version: String,
    pub maintainer: String,
    pub id: ToolId,
    pub name: String,
    pub summary: String,
    pub description: String,
    pub category: Category,
    #[serde(default)]
    pub tags: Vec<String>,
    /// SPDX license expression, e.g. `GPL-2.0-only`.
    pub license: String,
    pub homepage: String,
    pub documentation: String,
    /// Upstream source repository (https), where one exists.
    pub source_repository: Option<String>,
    pub provenance: Provenance,
    pub maintenance: Option<MaintenanceStatus>,
    #[serde(default)]
    pub recommended_for_beginners: bool,
    pub windows: WindowsSupport,
    pub platforms: Vec<RuntimeKind>,
    #[serde(default)]
    pub prerequisites: Vec<PrerequisiteSpec>,
    /// Other catalog tools that must be installed first.
    #[serde(default)]
    pub dependencies: Vec<ToolId>,
    pub install: Vec<InstallSpec>,
    #[serde(default)]
    pub version: VersionPolicy,
    #[serde(default)]
    pub storage: Storage,
    pub integrity: Integrity,
    pub network: NetworkNeeds,
    pub privileges: Privileges,
    pub lifecycle: Lifecycle,
    pub verify: VerifySpec,
    #[serde(default)]
    pub launch_profiles: Vec<RunSpec>,
}

// ---------------------------------------------------------------- validation helpers

fn bare_name_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && !s.starts_with('-')
        && !s.starts_with('.')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.chars().all(|c| c.is_ascii_hexdigit())
}

fn token_ok(s: &str, extra: &[char]) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && !s.starts_with('-')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || extra.contains(&c))
}

fn param_name_ok(s: &str) -> bool {
    let mut cs = s.chars();
    matches!(cs.next(), Some(c) if c.is_ascii_lowercase())
        && s.len() <= 32
        && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn flag_ok(s: &str) -> bool {
    s.len() >= 2 && s.len() <= 32 && s.starts_with('-') && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

fn clean_text(s: &str) -> bool {
    !s.chars().any(|c| c.is_control() && c != '\n' && c != '\t')
}

fn https_url_ok(s: &str) -> bool {
    matches!(Url::parse(s), Ok(u) if u.scheme() == "https" && u.host_str().is_some() && u.username().is_empty() && u.password().is_none())
}

fn semver_ok(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.len() <= 6 && p.chars().all(|c| c.is_ascii_digit()))
}

fn date_ok(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10 && b[4] == b'-' && b[7] == b'-' && b.iter().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

impl ToolManifest {
    pub fn from_toml(input: &str) -> AppResult<Self> {
        if input.len() > MAX_MANIFEST_BYTES {
            return Err(AppError::Validation(format!("manifest exceeds {MAX_MANIFEST_BYTES} bytes")));
        }
        let m: Self = toml::from_str(input).map_err(|e| AppError::Validation(e.to_string()))?;
        m.validate()?;
        Ok(m)
    }

    /// Returns all problems at once, joined with `; `.
    pub fn validate(&self) -> AppResult<()> {
        let mut errs: Vec<String> = Vec::new();
        let mut bad = |cond: bool, msg: String| {
            if !cond {
                errs.push(msg);
            }
        };

        bad(
            self.schema_version == SUPPORTED_SCHEMA_VERSION,
            format!("schema_version must be {SUPPORTED_SCHEMA_VERSION}"),
        );
        bad(semver_ok(&self.manifest_version), "manifest_version must look like 1.0.0".into());
        for (field, v, max) in [
            ("name", &self.name, 80),
            ("summary", &self.summary, 200),
            ("description", &self.description, 4000),
            ("license", &self.license, 100),
            ("maintainer", &self.maintainer, 100),
            ("provenance.publisher", &self.provenance.publisher, 120),
        ] {
            bad(
                !v.trim().is_empty() && v.len() <= max && clean_text(v),
                format!("{field} must be non-empty, <= {max} chars, without control characters"),
            );
        }
        bad(self.tags.len() <= 12, "at most 12 tags".into());
        for t in &self.tags {
            bad(
                !t.is_empty() && t.len() <= 24 && t.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                format!("invalid tag '{t}'"),
            );
        }
        bad(https_url_ok(&self.homepage), "homepage must be an https URL".into());
        bad(https_url_ok(&self.documentation), "documentation must be an https URL".into());
        bad(https_url_ok(&self.provenance.source_url), "provenance.source_url must be an https URL".into());
        bad(date_ok(&self.provenance.verified_on), "provenance.verified_on must be YYYY-MM-DD".into());
        if let Some(src) = &self.source_repository {
            bad(https_url_ok(src), "source_repository must be an https URL".into());
        }
        bad(self.windows.min_build >= 19041, "windows.min_build must be at least 19041 (Windows 10 2004)".into());
        bad(!self.windows.architectures.is_empty(), "windows.architectures must not be empty".into());
        bad(!self.platforms.is_empty(), "platforms must not be empty".into());
        bad(!self.install.is_empty(), "install must list at least one method".into());

        for p in &self.prerequisites {
            bad(
                !p.reason.trim().is_empty() && p.reason.len() <= 300 && clean_text(&p.reason),
                format!("prerequisite {:?} needs a reason (<= 300 chars)", p.kind),
            );
        }
        let mut seen_deps = std::collections::HashSet::new();
        for d in &self.dependencies {
            bad(d != &self.id, "a tool cannot depend on itself".into());
            bad(seen_deps.insert(d), format!("duplicate dependency '{d}'"));
        }

        for spec in &self.install {
            match spec {
                InstallSpec::Winget { package_id } => {
                    bad(token_ok(package_id, &['.', '-', '_']), format!("invalid winget package_id '{package_id}'"));
                }
                InstallSpec::WslApt { packages } => {
                    bad(!packages.is_empty() && packages.len() <= 20, "wsl_apt needs 1-20 packages".into());
                    for p in packages {
                        bad(
                            token_ok(p, &['.', '-', '+']) && !p.chars().any(|c| c.is_ascii_uppercase()),
                            format!("invalid apt package name '{p}'"),
                        );
                    }
                }
                InstallSpec::ContainerImage { image, digest } => {
                    bad(
                        token_ok(image, &['.', '-', '_', '/']) && !image.contains(".."),
                        format!("invalid container image '{image}' (tags are not allowed; pin by digest)"),
                    );
                    bad(
                        digest.strip_prefix("sha256:").is_some_and(|h| is_hex(h, 64)),
                        "container digest must be 'sha256:' + 64 hex chars".into(),
                    );
                }
                InstallSpec::DownloadArchive { url, sha256, executable } => {
                    bad(https_url_ok(url), "download url must be https".into());
                    bad(is_hex(sha256, 64), "sha256 must be 64 hex chars".into());
                    bad(bare_name_ok(executable), format!("invalid executable '{executable}'"));
                }
            }
            bad(
                self.platforms.contains(&spec.runtime()),
                format!("install method targets {:?}, which is not in platforms", spec.runtime()),
            );
        }

        // Integrity metadata must describe the preferred install method honestly.
        if let Some(first) = self.install.first() {
            let ok = match (first, self.integrity.level) {
                (InstallSpec::Winget { .. }, IntegrityLevel::PackageManager | IntegrityLevel::Unavailable) => true,
                (InstallSpec::WslApt { .. }, IntegrityLevel::PackageManager | IntegrityLevel::Unavailable) => true,
                (InstallSpec::ContainerImage { .. }, IntegrityLevel::ImageDigest) => true,
                (InstallSpec::DownloadArchive { .. }, IntegrityLevel::Sha256Pinned) => true,
                _ => false,
            };
            bad(ok, format!("integrity.level {:?} does not match the {:?} install method", self.integrity.level, first.method()));
        }
        if let Some(n) = &self.integrity.note {
            bad(clean_text(n) && n.len() <= 300, "integrity.note is invalid".into());
        }

        if let Some(v) = &self.version.pinned {
            bad(token_ok(v, &['.', '-', '_', '+']), format!("invalid pinned version '{v}'"));
        }
        if let Some(v) = &self.version.minimum {
            bad(token_ok(v, &['.', '-', '_', '+']), format!("invalid minimum version '{v}'"));
        }

        if let Some(sub) = &self.storage.install_subdir {
            if let Err(e) = paths::validate_relative(sub) {
                bad(false, format!("storage.install_subdir: {e}"));
            }
        }

        bad(!self.verify.checks.is_empty(), "verify.checks must contain at least one check".into());
        bad(
            self.verify.timeout_secs.is_none_or(|t| (1..=300).contains(&t)),
            "verify.timeout_secs must be 1-300".into(),
        );
        for c in &self.verify.checks {
            if let VerifyCheck::ExecutableOutput { runtime, executable, args, locations, expect_output_contains } = c {
                bad(bare_name_ok(executable), format!("verify executable '{executable}' must be a bare file name"));
                bad(self.platforms.contains(runtime), "verify runtime is not in platforms".into());
                for a in args {
                    bad(clean_text(a) && a.len() <= 128, "verify args contain an invalid argument".into());
                }
                bad(locations.len() <= 6, "at most 6 verify locations".into());
                for l in locations {
                    if let Err(e) = paths::parse_location(l) {
                        bad(false, format!("verify location: {e}"));
                    }
                }
                if let Some(x) = expect_output_contains {
                    bad(!x.is_empty() && x.len() <= 128 && clean_text(x), "expect_output_contains is invalid".into());
                }
            }
        }

        let mut profile_ids = std::collections::HashSet::new();
        for run in &self.launch_profiles {
            bad(param_name_ok(&run.id.replace('-', "_")), format!("invalid launch profile id '{}'", run.id));
            bad(profile_ids.insert(run.id.as_str()), format!("duplicate launch profile '{}'", run.id));
            bad(!run.label.trim().is_empty() && run.label.len() <= 80, format!("launch profile '{}' needs a label", run.id));
            bad(bare_name_ok(&run.executable), format!("launch profile '{}': executable must be a bare file name", run.id));
            bad(self.platforms.contains(&run.runtime), format!("launch profile '{}': runtime is not in platforms", run.id));
            let mut seen = std::collections::HashSet::new();
            for p in &run.parameters {
                bad(param_name_ok(&p.name), format!("invalid parameter name '{}'", p.name));
                bad(seen.insert(p.name.as_str()), format!("duplicate parameter '{}'", p.name));
                bad(!p.label.trim().is_empty() && p.label.len() <= 80, format!("parameter '{}' needs a label", p.name));
                if let ParamKind::Choice { options } = &p.kind {
                    bad(
                        !options.is_empty() && options.iter().all(|o| clean_text(o) && !o.is_empty() && o.len() <= 64),
                        format!("parameter '{}' has invalid choice options", p.name),
                    );
                }
                if let ParamKind::Integer { min, max } = &p.kind {
                    bad(min <= max, format!("parameter '{}' has min > max", p.name));
                }
                if let Some(d) = &p.default {
                    bad(check_value(p, d).is_ok(), format!("default for parameter '{}' fails its own validation", p.name));
                }
            }
            let known = |n: &str| run.parameters.iter().any(|p| p.name == n);
            for t in &run.args {
                match t {
                    ArgTemplate::Literal { value } => {
                        bad(clean_text(value) && !value.contains('\0') && value.len() <= 256, "literal arg contains invalid characters".into());
                    }
                    ArgTemplate::Value { param } => {
                        bad(known(param), format!("arg references unknown parameter '{param}'"));
                    }
                    ArgTemplate::Flag { param, flag } => {
                        bad(known(param), format!("arg references unknown parameter '{param}'"));
                        bad(flag_ok(flag), format!("invalid flag '{flag}'"));
                        bad(
                            run.parameters.iter().any(|p| &p.name == param && p.kind == ParamKind::Boolean),
                            format!("flag '{flag}' must reference a boolean parameter"),
                        );
                    }
                    ArgTemplate::OptionValue { param, flag } => {
                        bad(known(param), format!("arg references unknown parameter '{param}'"));
                        bad(flag_ok(flag), format!("invalid flag '{flag}'"));
                    }
                }
            }
        }

        if errs.is_empty() {
            Ok(())
        } else {
            Err(AppError::Validation(format!("manifest '{}': {}", self.id, errs.join("; "))))
        }
    }

    pub fn profile(&self, id: &str) -> Option<&RunSpec> {
        self.launch_profiles.iter().find(|p| p.id == id)
    }
}

// ---------------------------------------------------------------- command construction

/// A fully validated command. `program` and `args` are what gets executed (no shell);
/// `display` is for the UI preview only and is never executed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandPreview {
    pub runtime: RuntimeKind,
    pub program: String,
    pub args: Vec<String>,
    pub display: String,
}

fn check_value(def: &ParameterDef, v: &ParamValue) -> AppResult<()> {
    let err = |m: &str| Err(AppError::Validation(format!("parameter '{}': {m}", def.name)));
    match (&def.kind, v) {
        (ParamKind::Boolean, ParamValue::Bool(_)) => Ok(()),
        (ParamKind::Integer { min, max }, ParamValue::Int(n)) => {
            if n >= min && n <= max {
                Ok(())
            } else {
                err(&format!("must be between {min} and {max}"))
            }
        }
        (ParamKind::Text { max_len }, ParamValue::Text(s)) => {
            if s.is_empty() || s.len() > max_len.unwrap_or(256) {
                err("empty or too long")
            } else if s.chars().any(|c| c.is_control()) {
                err("control characters are not allowed")
            } else if s.starts_with('-') {
                err("must not start with '-' (it would be read as an option)")
            } else {
                Ok(())
            }
        }
        (ParamKind::Host, ParamValue::Text(s)) => {
            let ok = !s.is_empty()
                && s.len() <= 253
                && !s.starts_with('-')
                && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '/' | '[' | ']' | '_'));
            if ok {
                Ok(())
            } else {
                err("must be a hostname, IP address or CIDR range")
            }
        }
        (ParamKind::Choice { options }, ParamValue::Text(s)) => {
            if options.contains(s) {
                Ok(())
            } else {
                err("not one of the allowed choices")
            }
        }
        _ => err("wrong value type"),
    }
}

/// Quote for display only (Windows-style). The real execution path uses the arg vector.
fn quote_for_display(s: &str) -> String {
    if !s.is_empty() && !s.chars().any(|c| c.is_whitespace() || c == '"') {
        s.to_string()
    } else {
        format!("\"{}\"", s.replace('"', "\\\""))
    }
}

pub fn build_command(run: &RunSpec, values: &HashMap<String, ParamValue>) -> AppResult<CommandPreview> {
    for k in values.keys() {
        if !run.parameters.iter().any(|p| &p.name == k) {
            return Err(AppError::Validation(format!("unknown parameter '{k}'")));
        }
    }
    let mut effective: HashMap<&str, &ParamValue> = HashMap::new();
    for p in &run.parameters {
        match values.get(&p.name).or(p.default.as_ref()) {
            Some(v) => {
                check_value(p, v)?;
                effective.insert(p.name.as_str(), v);
            }
            None if p.required => {
                return Err(AppError::Validation(format!("parameter '{}' is required", p.name)));
            }
            None => {}
        }
    }

    let mut args: Vec<String> = Vec::new();
    for t in &run.args {
        match t {
            ArgTemplate::Literal { value } => args.push(value.clone()),
            ArgTemplate::Value { param } => {
                if let Some(v) = effective.get(param.as_str()) {
                    args.push(render(v));
                }
            }
            ArgTemplate::Flag { param, flag } => {
                if let Some(ParamValue::Bool(true)) = effective.get(param.as_str()).copied() {
                    args.push(flag.clone());
                }
            }
            ArgTemplate::OptionValue { param, flag } => {
                if let Some(v) = effective.get(param.as_str()) {
                    args.push(flag.clone());
                    args.push(render(v));
                }
            }
        }
    }

    let display = std::iter::once(run.executable.as_str())
        .chain(args.iter().map(String::as_str))
        .map(quote_for_display)
        .collect::<Vec<_>>()
        .join(" ");
    Ok(CommandPreview { runtime: run.runtime, program: run.executable.clone(), args, display })
}

fn render(v: &ParamValue) -> String {
    match v {
        ParamValue::Bool(b) => b.to_string(),
        ParamValue::Int(n) => n.to_string(),
        ParamValue::Text(s) => s.clone(),
    }
}

/// Shared by unit tests in this crate: a complete, valid manifest.
#[cfg(test)]
pub mod fixtures {
    pub const VALID: &str = r#"
schema_version = 2
manifest_version = "1.0.0"
maintainer = "Vixalune catalog"
id = "nmap"
name = "Nmap"
summary = "Network scanner"
description = "Port scanning and service detection."
category = "reconnaissance"
tags = ["scanner", "ports"]
license = "LicenseRef-Nmap"
homepage = "https://nmap.org"
documentation = "https://nmap.org/book/"
source_repository = "https://github.com/nmap/nmap"
maintenance = "active"
platforms = ["windows_native"]

[provenance]
publisher = "Nmap Project"
basis = "package_repository"
source_url = "https://nmap.org/download.html"
verified_on = "2026-10-17"

[windows]
min_build = 19045
architectures = ["x64"]

[[prerequisites]]
kind = "winget"
reason = "Installs Nmap from the Windows Package Manager."

[[install]]
method = "winget"
package_id = "Insecure.Nmap"

[integrity]
level = "package_manager"

[network]
install = true
run = true

[privileges]
install = "possible"
run = "none"

[lifecycle]
can_update = true
can_uninstall = true

[verify]
timeout_secs = 20
[[verify.checks]]
kind = "package_installed"
[[verify.checks]]
kind = "executable_output"
runtime = "windows_native"
executable = "nmap.exe"
args = ["--version"]
locations = ["{ProgramFilesX86}\\Nmap"]
expect_output_contains = "Nmap version"

[[launch_profiles]]
id = "scan"
label = "Scan a target"
runtime = "windows_native"
executable = "nmap.exe"
args = [
  { kind = "flag", param = "service_scan", flag = "-sV" },
  { kind = "option", param = "ports", flag = "-p" },
  { kind = "value", param = "target" },
]

[[launch_profiles.parameters]]
name = "target"
label = "Target"
required = true
kind = { type = "host" }

[[launch_profiles.parameters]]
name = "ports"
label = "Ports"
kind = { type = "text", max_len = 64 }

[[launch_profiles.parameters]]
name = "service_scan"
label = "Service detection"
kind = { type = "boolean" }
default = false
"#;
}

#[cfg(test)]
mod tests {
    use super::fixtures::VALID;
    use super::*;

    fn reject(replace_from: &str, replace_to: &str) -> String {
        assert!(VALID.contains(replace_from), "fixture does not contain {replace_from:?}");
        ToolManifest::from_toml(&VALID.replace(replace_from, replace_to)).unwrap_err().to_string()
    }

    #[test]
    fn parses_valid_manifest() {
        let m = ToolManifest::from_toml(VALID).unwrap();
        assert_eq!(m.id.as_str(), "nmap");
        assert_eq!(m.install[0].method(), InstallMethod::Winget);
        assert_eq!(m.profile("scan").unwrap().executable, "nmap.exe");
    }

    #[test]
    fn rejects_unknown_schema_version_and_adapter_types() {
        assert!(reject("schema_version = 2", "schema_version = 1").contains("schema_version"));
        // Unknown execution/adapter types are a parse error, not silently ignored.
        let e = reject("method = \"winget\"", "method = \"powershell_script\"");
        assert!(e.contains("unknown variant") || e.contains("powershell_script"), "{e}");
        assert!(ToolManifest::from_toml(&VALID.replace("method = \"winget\"", "method = \"shell\"")).is_err());
    }

    #[test]
    fn rejects_unknown_fields_such_as_scripts() {
        for extra in ["post_install_script = \"powershell -c evil\"\n", "run_as_admin = true\n"] {
            let bad = VALID.replacen("[provenance]", &format!("{extra}\n[provenance]"), 1);
            // Top-level keys placed after a table header would belong to that table; also try the root.
            assert!(ToolManifest::from_toml(&bad).is_err(), "{extra}");
            let root = format!("{extra}{VALID}");
            assert!(ToolManifest::from_toml(&root).is_err(), "{extra}");
        }
        let bad = VALID.replace("[verify]", "[verify]\nscript = \"x\"");
        assert!(ToolManifest::from_toml(&bad).is_err());
        let bad = VALID.replace("package_id = \"Insecure.Nmap\"", "package_id = \"Insecure.Nmap\"\npre_install = \"calc\"");
        assert!(ToolManifest::from_toml(&bad).is_err());
    }

    #[test]
    fn rejects_missing_required_sections() {
        let no_verify = VALID.replace("[[verify.checks]]\nkind = \"package_installed\"", "").replace("[[verify.checks]]\nkind = \"executable_output\"", "");
        assert!(ToolManifest::from_toml(&no_verify).is_err());
        let no_prov = VALID.replace("[provenance]", "[provenance_x]");
        assert!(ToolManifest::from_toml(&no_prov).is_err());
    }

    #[test]
    fn rejects_unpinned_or_insecure_downloads() {
        let bad = VALID.replace(
            "method = \"winget\"\npackage_id = \"Insecure.Nmap\"",
            "method = \"download_archive\"\nurl = \"http://example.com/a.zip\"\nsha256 = \"abc\"\nexecutable = \"a.exe\"",
        );
        let e = ToolManifest::from_toml(&bad).unwrap_err().to_string();
        assert!(e.contains("https") && e.contains("sha256"), "{e}");
    }

    #[test]
    fn integrity_level_must_match_install_method() {
        let e = reject("level = \"package_manager\"", "level = \"sha256_pinned\"");
        assert!(e.contains("integrity.level"), "{e}");
    }

    #[test]
    fn rejects_shell_metacharacters_and_option_injection_in_package_id() {
        for evil in ["Insecure.Nmap; calc", "-source", "Insecure.Nmap && dir", "a b", "$(x)", "x|y", "x\ny"] {
            assert!(ToolManifest::from_toml(&VALID.replace("Insecure.Nmap", evil)).is_err(), "{evil:?}");
        }
    }

    #[test]
    fn rejects_path_traversal_in_every_path_field() {
        assert!(reject("executable = \"nmap.exe\"\nargs = [\"--version\"]", "executable = \"..\\\\evil.exe\"\nargs = [\"--version\"]").contains("bare file name"));
        assert!(reject("{ProgramFilesX86}\\\\Nmap", "{ProgramFilesX86}\\\\..\\\\Windows").contains("verify location"));
        assert!(reject("{ProgramFilesX86}\\\\Nmap", "C:\\\\Windows\\\\System32").contains("verify location"));
        let storage = format!("{VALID}\n[storage]\ninstall_subdir = \"..\\\\..\\\\Windows\"\n");
        assert!(ToolManifest::from_toml(&storage).unwrap_err().to_string().contains("install_subdir"));
        let ok = format!("{VALID}\n[storage]\ninstall_subdir = \"Vendor\\\\Nmap\"\napprox_disk_mb = 50\n");
        assert!(ToolManifest::from_toml(&ok).is_ok());
    }

    #[test]
    fn rejects_bad_metadata() {
        assert!(reject("manifest_version = \"1.0.0\"", "manifest_version = \"v1\"").contains("manifest_version"));
        assert!(reject("verified_on = \"2026-10-17\"", "verified_on = \"yesterday\"").contains("verified_on"));
        assert!(reject("min_build = 19045", "min_build = 100").contains("min_build"));
        assert!(reject("homepage = \"https://nmap.org\"", "homepage = \"javascript:alert(1)\"").contains("homepage"));
        assert!(reject("tags = [\"scanner\", \"ports\"]", "tags = [\"Bad Tag\"]").contains("tag"));
        assert!(reject("reason = \"Installs Nmap from the Windows Package Manager.\"", "reason = \"\"").contains("reason"));
    }

    #[test]
    fn source_repository_must_be_https() {
        assert!(reject("https://github.com/nmap/nmap", "git://example.com/x").contains("source_repository"));
    }

    fn run_spec() -> RunSpec {
        ToolManifest::from_toml(VALID).unwrap().launch_profiles.remove(0)
    }

    fn vals(pairs: &[(&str, ParamValue)]) -> HashMap<String, ParamValue> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    #[test]
    fn builds_argument_array_without_shell() {
        let cmd = build_command(
            &run_spec(),
            &vals(&[
                ("target", ParamValue::Text("scanme.local".into())),
                ("ports", ParamValue::Text("80,443".into())),
                ("service_scan", ParamValue::Bool(true)),
            ]),
        )
        .unwrap();
        assert_eq!(cmd.program, "nmap.exe");
        assert_eq!(cmd.args, ["-sV", "-p", "80,443", "scanme.local"]);
        assert_eq!(cmd.display, "nmap.exe -sV -p 80,443 scanme.local");
    }

    #[test]
    fn malicious_arguments_are_rejected_not_escaped() {
        for evil in ["-oN evil", "host; calc", "host && dir", "a b", "$(x)", "host\n", "`calc`", "a|b", "a>out", "%PATH%", "host\"x"] {
            let r = build_command(&run_spec(), &vals(&[("target", ParamValue::Text(evil.into()))]));
            assert!(r.is_err(), "should reject {evil:?}");
        }
        let r = build_command(
            &run_spec(),
            &vals(&[("target", ParamValue::Text("h".into())), ("ports", ParamValue::Text("--script=x".into()))]),
        );
        assert!(r.is_err(), "text values may not start with '-'");
    }

    #[test]
    fn missing_required_and_unknown_params_fail() {
        assert!(build_command(&run_spec(), &HashMap::new()).is_err());
        assert!(build_command(
            &run_spec(),
            &vals(&[("target", ParamValue::Text("h".into())), ("nope", ParamValue::Bool(true))])
        )
        .is_err());
    }
}
