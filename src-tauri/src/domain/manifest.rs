//! Tool manifest model and validation.
//!
//! A manifest is *data only*: it can name an executable and describe typed parameters, but
//! it can never carry scripts. Every string that may become a process argument is validated
//! here, and all downloads must be pinned by SHA-256 / image digest.

use super::errors::{AppError, AppResult};
use super::ids::ToolId;
use serde::{Deserialize, Serialize};
use url::Url;

pub const SUPPORTED_SCHEMA_VERSION: u32 = 1;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Prerequisite {
    Winget,
    Wsl2,
    Docker,
}

/// How a tool is installed. A closed set of structured methods; there is deliberately no
/// "run this script" variant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum InstallSpec {
    /// `winget install --id <package_id> --exact [--version <version>]`
    Winget {
        package_id: String,
        version: Option<String>,
    },
    /// `apt-get install -y -- <packages...>` inside the configured WSL distribution.
    WslApt { packages: Vec<String> },
    /// `docker pull <image>@<digest>`
    ContainerImage { image: String, digest: String },
    /// HTTPS download verified against `sha256`, extracted into the app-owned tools directory.
    DownloadArchive {
        url: String,
        sha256: String,
        executable: String,
    },
}

impl InstallSpec {
    pub fn runtime(&self) -> RuntimeKind {
        match self {
            Self::Winget { .. } | Self::DownloadArchive { .. } => RuntimeKind::WindowsNative,
            Self::WslApt { .. } => RuntimeKind::Wsl,
            Self::ContainerImage { .. } => RuntimeKind::Container,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifySpec {
    pub runtime: RuntimeKind,
    pub executable: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub expect_output_contains: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParamValue {
    Bool(bool),
    Int(i64),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
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
#[serde(tag = "kind", rename_all = "snake_case")]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunSpec {
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
    pub id: ToolId,
    pub name: String,
    pub summary: String,
    pub description: String,
    pub category: Category,
    /// SPDX license expression, e.g. `GPL-2.0-only`.
    pub license: String,
    pub homepage: String,
    pub documentation: String,
    pub platforms: Vec<RuntimeKind>,
    #[serde(default)]
    pub prerequisites: Vec<Prerequisite>,
    pub install: Vec<InstallSpec>,
    /// Installing needs Windows administrator rights. Elevation is never implicit.
    #[serde(default)]
    pub requires_elevation: bool,
    pub verify: VerifySpec,
    pub run: Option<RunSpec>,
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
    s.len() >= 2
        && s.len() <= 32
        && s.starts_with('-')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

fn clean_text(s: &str) -> bool {
    !s.chars().any(|c| c.is_control() && c != '\n' && c != '\t')
}

fn https_url_ok(s: &str) -> bool {
    matches!(Url::parse(s), Ok(u) if u.scheme() == "https" && u.host_str().is_some() && u.username().is_empty() && u.password().is_none())
}

impl ToolManifest {
    pub fn from_toml(input: &str) -> AppResult<Self> {
        if input.len() > MAX_MANIFEST_BYTES {
            return Err(AppError::Validation(format!(
                "manifest exceeds {MAX_MANIFEST_BYTES} bytes"
            )));
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
        for (field, v, max) in [
            ("name", &self.name, 80),
            ("summary", &self.summary, 200),
            ("description", &self.description, 4000),
            ("license", &self.license, 100),
        ] {
            bad(
                !v.trim().is_empty() && v.len() <= max && clean_text(v),
                format!("{field} must be non-empty, <= {max} chars, without control characters"),
            );
        }
        bad(https_url_ok(&self.homepage), "homepage must be an https URL".into());
        bad(https_url_ok(&self.documentation), "documentation must be an https URL".into());
        bad(!self.platforms.is_empty(), "platforms must not be empty".into());
        bad(!self.install.is_empty(), "install must list at least one method".into());

        for spec in &self.install {
            match spec {
                InstallSpec::Winget { package_id, version } => {
                    bad(token_ok(package_id, &['.', '-', '_']), format!("invalid winget package_id '{package_id}'"));
                    if let Some(v) = version {
                        bad(token_ok(v, &['.', '-', '_', '+']), format!("invalid winget version '{v}'"));
                    }
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

        bad(bare_name_ok(&self.verify.executable), "verify.executable must be a bare file name".into());
        for a in &self.verify.args {
            bad(clean_text(a) && !a.contains('\0') && a.len() <= 128, "verify.args contains an invalid argument".into());
        }

        if let Some(run) = &self.run {
            bad(bare_name_ok(&run.executable), "run.executable must be a bare file name".into());
            bad(self.platforms.contains(&run.runtime), "run.runtime is not in platforms".into());
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
                    bad(
                        check_value(p, d).is_ok(),
                        format!("default for parameter '{}' fails its own validation", p.name),
                    );
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

pub fn build_command(
    run: &RunSpec,
    values: &std::collections::HashMap<String, ParamValue>,
) -> AppResult<CommandPreview> {
    for k in values.keys() {
        if !run.parameters.iter().any(|p| &p.name == k) {
            return Err(AppError::Validation(format!("unknown parameter '{k}'")));
        }
    }
    // Resolve effective values: provided > default; enforce `required`.
    let mut effective: std::collections::HashMap<&str, &ParamValue> = std::collections::HashMap::new();
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    pub const VALID: &str = r#"
schema_version = 1
id = "nmap"
name = "Nmap"
summary = "Network scanner"
description = "Port scanning and service detection."
category = "reconnaissance"
license = "LicenseRef-Nmap"
homepage = "https://nmap.org"
documentation = "https://nmap.org/book/"
platforms = ["windows_native"]
prerequisites = ["winget"]

[[install]]
method = "winget"
package_id = "Insecure.Nmap"

[verify]
runtime = "windows_native"
executable = "nmap"
args = ["--version"]
expect_output_contains = "Nmap version"

[run]
runtime = "windows_native"
executable = "nmap"
args = [
  { kind = "flag", param = "service_scan", flag = "-sV" },
  { kind = "option", param = "ports", flag = "-p" },
  { kind = "value", param = "target" },
]

[[run.parameters]]
name = "target"
label = "Target"
required = true
kind = { type = "host" }

[[run.parameters]]
name = "ports"
label = "Ports"
kind = { type = "text", max_len = 64 }

[[run.parameters]]
name = "service_scan"
label = "Service detection"
kind = { type = "boolean" }
default = false
"#;

    #[test]
    fn parses_valid_manifest() {
        let m = ToolManifest::from_toml(VALID).unwrap();
        assert_eq!(m.id.as_str(), "nmap");
    }

    #[test]
    fn rejects_unknown_fields_such_as_scripts() {
        let bad = format!("{VALID}\npost_install_script = \"powershell -c evil\"\n");
        // Appended after [run.parameters] table, so it lands in that table: still unknown.
        assert!(ToolManifest::from_toml(&bad).is_err());
        let bad2 = VALID.replace("[verify]", "[verify]\nscript = \"x\"");
        assert!(ToolManifest::from_toml(&bad2).is_err());
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
    fn rejects_shell_metacharacters_in_package_id() {
        let bad = VALID.replace("Insecure.Nmap", "Insecure.Nmap; calc");
        assert!(ToolManifest::from_toml(&bad).is_err());
    }

    #[test]
    fn rejects_path_in_executable() {
        let bad = VALID.replace("executable = \"nmap\"\nargs", "executable = \"..\\\\evil.exe\"\nargs");
        assert!(ToolManifest::from_toml(&bad).is_err());
    }

    fn run_spec() -> RunSpec {
        ToolManifest::from_toml(VALID).unwrap().run.unwrap()
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
        assert_eq!(cmd.program, "nmap");
        assert_eq!(cmd.args, ["-sV", "-p", "80,443", "scanme.local"]);
        assert_eq!(cmd.display, "nmap -sV -p 80,443 scanme.local");
    }

    #[test]
    fn injection_attempts_are_rejected_not_escaped() {
        for evil in ["-oN evil", "host; calc", "host && dir", "a b", "$(x)", "host\n"] {
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
