//! Container policy enforcement (pure): approved registries, digest-only image references,
//! host-mount validation, and the hardened `docker run` argument builder.
//!
//! A container is **not** a safe sandbox for hostile code. These controls reduce accidental harm
//! from tools we ship; truly untrusted workloads need a separately managed VM.

use super::errors::{AppError, AppResult};
use super::manifest::{ContainerPolicy, RunSpec, RuntimeKind, ToolManifest};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

pub const DEFAULT_APPROVED_REGISTRIES: &[&str] = &["docker.io", "ghcr.io", "quay.io", "mcr.microsoft.com"];

fn bad(msg: impl Into<String>) -> AppError {
    AppError::Validation(msg.into())
}

// ------------------------------------------------------------------ image references

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageRef {
    pub registry: String,
    pub repository: String,
    pub digest: String,
}

impl ImageRef {
    /// Canonical, digest-only reference: `registry/repository@sha256:...`. Tags are never used.
    pub fn reference(&self) -> String {
        format!("{}/{}@{}", self.registry, self.repository, self.digest)
    }
}

pub fn parse_image(image: &str, digest: &str) -> AppResult<ImageRef> {
    if image.contains('@') || image.rsplit('/').next().is_some_and(|last| last.contains(':')) {
        return Err(bad(format!("image '{image}' must not carry a tag or digest; the digest is given separately")));
    }
    if !digest.strip_prefix("sha256:").is_some_and(|h| h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit())) {
        return Err(bad("an image digest of the form sha256:<64 hex> is required"));
    }
    let parts: Vec<&str> = image.split('/').collect();
    let (registry, rest) = if parts.len() > 1 && (parts[0].contains('.') || parts[0].contains(':') || parts[0] == "localhost") {
        (parts[0].to_string(), parts[1..].join("/"))
    } else {
        ("docker.io".to_string(), image.to_string())
    };
    let repository = if registry == "docker.io" && !rest.contains('/') { format!("library/{rest}") } else { rest };
    let ok = !repository.is_empty()
        && repository.split('/').all(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-')));
    if !ok || registry.chars().any(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'))) {
        return Err(bad(format!("invalid image name '{image}'")));
    }
    Ok(ImageRef { registry: registry.to_ascii_lowercase(), repository, digest: digest.to_ascii_lowercase() })
}

/// Images may only come from approved registries. An empty list approves nothing.
pub fn check_registry(img: &ImageRef, approved: &[String]) -> AppResult<()> {
    if approved.iter().any(|r| r.eq_ignore_ascii_case(&img.registry)) {
        Ok(())
    } else {
        Err(AppError::Unsupported(format!(
            "registry '{}' is not on the approved list ({}). Vixalune only pulls images from approved registries.",
            img.registry,
            approved.join(", ")
        )))
    }
}

// ------------------------------------------------------------------ host mounts

const FORBIDDEN_SEGMENTS: &[&str] = &[".ssh", ".aws", ".azure", ".gnupg", ".kube", ".docker", "appdata", "docker.sock", "$recycle.bin", "system volume information"];
const FORBIDDEN_TOP: &[&str] = &["windows", "program files", "program files (x86)", "programdata", "users", "recovery", "$windows.~bt"];

/// Lexical validation of a Windows host folder offered for mounting. Returns the normalised path.
/// The runner additionally canonicalises it (so junctions cannot escape) and re-validates.
pub fn validate_host_mount(raw: &str, home: Option<&Path>) -> AppResult<String> {
    let p = raw.trim();
    if p.is_empty() || p.len() > 240 || p.chars().any(|c| c.is_control() || matches!(c, ',' | '"' | '\'' | '*' | '?' | '<' | '>' | '|')) {
        return Err(bad("the folder path is empty, too long, or contains forbidden characters"));
    }
    if p.starts_with("\\\\") || p.starts_with("//") {
        return Err(bad("network (UNC) and device paths cannot be mounted"));
    }
    let norm = p.replace('/', "\\");
    let b = norm.as_bytes();
    if !(b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\') {
        return Err(bad("use a full path such as D:\\labs\\project"));
    }
    let rest = norm[3..].trim_end_matches('\\');
    if rest.is_empty() {
        return Err(bad("a whole drive cannot be mounted; choose a specific folder"));
    }
    let segs: Vec<&str> = rest.split('\\').collect();
    if segs.iter().any(|s| s.is_empty() || *s == "." || *s == ".." || s.ends_with('.') || s.ends_with(' ') || s.contains(':')) {
        return Err(bad("the folder path contains an empty, relative or invalid segment"));
    }
    let lower: Vec<String> = segs.iter().map(|s| s.to_ascii_lowercase()).collect();
    if FORBIDDEN_TOP.contains(&lower[0].as_str()) && (lower.len() == 1 || lower[0] != "users") {
        return Err(bad(format!("'{}' is a protected system folder and cannot be mounted", segs[0])));
    }
    if lower[0] == "users" && lower.len() <= 2 {
        return Err(bad("user profile folders cannot be mounted as a whole; choose a project folder inside one"));
    }
    if let Some(s) = lower.iter().find(|s| FORBIDDEN_SEGMENTS.contains(&s.as_str())) {
        return Err(bad(format!("folders named '{s}' hold credentials or system data and cannot be mounted")));
    }
    let full = format!("{}{}", &norm[..3].to_ascii_uppercase(), rest);
    if let Some(h) = home {
        let h = h.to_string_lossy().replace('/', "\\").to_ascii_lowercase();
        let f = full.to_ascii_lowercase();
        if f == h || h.starts_with(&format!("{f}\\")) {
            return Err(bad("your home folder (or a folder containing it) cannot be mounted"));
        }
    }
    Ok(full)
}

// ------------------------------------------------------------------ run approvals and the docker run builder

/// What the user explicitly approved for this one run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunApprovals {
    pub network: bool,
    /// mount name -> host folder
    #[serde(default)]
    pub mounts: HashMap<String, String>,
    /// container port -> host port
    #[serde(default)]
    pub ports: HashMap<u16, u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MountSummary {
    pub host: String,
    pub container: String,
    pub read_only: bool,
}

/// Everything the approval dialog shows before anything runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunSummary {
    pub image: String,
    pub command: String,
    pub user: String,
    pub network: String,
    pub mounts: Vec<MountSummary>,
    pub ports: Vec<(u16, u16)>,
    pub limits: String,
    pub read_only_rootfs: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockerRun {
    pub args: Vec<String>,
    pub container_name: String,
    pub summary: RunSummary,
}

pub fn container_name(tool: &str, nonce: &str) -> String {
    format!("vixalune-{tool}-{nonce}")
}

pub fn build_docker_run(
    manifest: &ToolManifest,
    profile: &RunSpec,
    command_args: &[String],
    image_ref: &str,
    approvals: &RunApprovals,
    name: &str,
    home: Option<&Path>,
) -> AppResult<DockerRun> {
    if profile.runtime != RuntimeKind::Container {
        return Err(bad("this launch profile does not run in a container"));
    }
    let policy: &ContainerPolicy = manifest.container.as_ref().ok_or_else(|| bad("the manifest has no container policy"))?;
    if !image_ref.contains("@sha256:") {
        return Err(bad("containers are only started from digest-pinned images"));
    }

    // Network: off unless the profile needs it AND the user approved exactly that.
    if profile.needs_network && !approvals.network {
        return Err(bad("this profile needs network access, which you have not approved"));
    }
    if !profile.needs_network && approvals.network {
        return Err(bad("network access was approved but this profile does not need it"));
    }
    let network = if profile.needs_network { "bridge" } else { "none" };

    // Mounts: exactly the ones the profile declares, each backed by an approved, validated folder.
    for k in approvals.mounts.keys() {
        if !profile.mounts.contains(k) {
            return Err(bad(format!("mount '{k}' is not used by this profile")));
        }
    }
    let mut mounts = Vec::new();
    for mname in &profile.mounts {
        let spec = policy.mounts.iter().find(|m| &m.name == mname).ok_or_else(|| bad(format!("unknown mount '{mname}'")))?;
        let host = approvals.mounts.get(mname).ok_or_else(|| bad(format!("choose and approve a folder for '{mname}'")))?;
        let host = validate_host_mount(host, home)?;
        mounts.push((host, spec.container_path.clone(), spec.read_only));
    }

    // Ports: only declared ones, host side 1024-65535, always loopback.
    for k in approvals.ports.keys() {
        if !profile.ports.contains(k) {
            return Err(bad(format!("port {k} is not published by this profile")));
        }
    }
    let mut ports = Vec::new();
    for cport in &profile.ports {
        let host = approvals.ports.get(cport).ok_or_else(|| bad(format!("approve a host port for container port {cport}")))?;
        if !(1024..=65535).contains(host) {
            return Err(bad(format!("host port {host} must be between 1024 and 65535")));
        }
        ports.push((*host, *cport));
    }

    let mut a: Vec<String> = Vec::new();
    let mut push = |items: &[&str]| a.extend(items.iter().map(|s| s.to_string()));
    push(&["run", "--rm", "--pull", "never", "--name", name]);
    push(&["--label", &format!("vixalune.tool={}", manifest.id), "--label", "vixalune.managed=true"]);
    push(&["--user", policy.user.as_str(), "--cap-drop", "ALL", "--security-opt", "no-new-privileges"]);
    push(&["--pids-limit", &policy.pids_limit.to_string()]);
    push(&["--memory", &format!("{}m", policy.memory_mb), "--memory-swap", &format!("{}m", policy.memory_mb)]);
    push(&["--cpus", &format!("{}.{:02}", policy.cpus_milli / 1000, (policy.cpus_milli % 1000) / 10)]);
    push(&["--network", network]);
    if policy.read_only_rootfs {
        push(&["--read-only"]);
    }
    for t in &policy.tmpfs {
        push(&["--tmpfs", &format!("{t}:rw,noexec,nosuid,size=64m")]);
    }
    for e in &policy.env {
        push(&["--env", &format!("{}={}", e.name, e.value)]);
    }
    for (host, cpath, ro) in &mounts {
        push(&["--mount", &format!("type=bind,source={host},target={cpath}{}", if *ro { ",readonly" } else { "" })]);
    }
    for (host, c) in &ports {
        push(&["--publish", &format!("127.0.0.1:{host}:{c}")]);
    }
    if let Some(w) = &policy.workdir {
        push(&["--workdir", w.as_str()]);
    }
    push(&[image_ref, profile.executable.as_str()]);
    a.extend(command_args.iter().cloned());

    assert_hardened(&a)?;

    let mut warnings = Vec::new();
    if !policy.read_only_rootfs {
        warnings.push("The container's root filesystem is writable for this tool.".to_string());
    }
    if mounts.iter().any(|m| !m.2) {
        warnings.push("A folder is mounted read-write: the tool can change or delete files in it.".to_string());
    }
    if profile.needs_network {
        warnings.push("The container can reach the internet and your local network.".to_string());
    }
    if !ports.is_empty() {
        warnings.push("Container ports are published on 127.0.0.1 only (not reachable from other computers).".to_string());
    }
    let summary = RunSummary {
        image: image_ref.to_string(),
        command: std::iter::once(profile.executable.clone()).chain(command_args.iter().cloned()).collect::<Vec<_>>().join(" "),
        user: policy.user.clone(),
        network: network.to_string(),
        mounts: mounts.iter().map(|(h, c, ro)| MountSummary { host: h.clone(), container: c.clone(), read_only: *ro }).collect(),
        ports,
        limits: format!("{} MB memory, {}.{:02} CPU, {} processes", policy.memory_mb, policy.cpus_milli / 1000, (policy.cpus_milli % 1000) / 10, policy.pids_limit),
        read_only_rootfs: policy.read_only_rootfs,
        warnings,
    };
    Ok(DockerRun { args: a, container_name: name.to_string(), summary })
}

/// Last line of defence on the final argument vector, independent of how it was built.
pub fn assert_hardened(args: &[String]) -> AppResult<()> {
    const FORBIDDEN: &[&str] = &["--privileged", "--cap-add", "--device", "--pid", "--ipc", "--uts", "--userns", "--volumes-from", "--add-host", "--group-add", "--sysctl", "--ulimit"];
    for (i, a) in args.iter().enumerate() {
        let lower = a.to_ascii_lowercase();
        if FORBIDDEN.iter().any(|f| lower == *f || lower.starts_with(&format!("{f}="))) {
            return Err(bad(format!("refusing unsafe docker option '{a}'")));
        }
        if lower.contains("docker.sock") || lower.contains("\\\\.\\pipe") || lower.contains("/var/run") || lower.starts_with("--network=host") {
            return Err(bad("refusing to expose the container engine or host namespaces to a tool container"));
        }
        if matches!(lower.as_str(), "--network" | "--net") && args.get(i + 1).is_some_and(|v| !matches!(v.as_str(), "none" | "bridge")) {
            return Err(bad("only the 'none' and 'bridge' networks are allowed"));
        }
        if lower == "--security-opt" && args.get(i + 1).is_some_and(|v| v != "no-new-privileges") {
            return Err(bad("security options may not be relaxed"));
        }
        if lower == "--mount" {
            let v = args.get(i + 1).map(String::as_str).unwrap_or("");
            if !v.starts_with("type=bind,source=") || v.contains("source=/") || v.contains("source=\\") {
                return Err(bad("only validated bind mounts are allowed"));
            }
        }
        if lower == "--publish" && !args.get(i + 1).is_some_and(|v| v.starts_with("127.0.0.1:")) {
            return Err(bad("ports may only be published on 127.0.0.1"));
        }
    }
    let has_pair = |k: &str, v: &str| args.windows(2).any(|w| w[0] == k && w[1] == v);
    let must = has_pair("--cap-drop", "ALL")
        && has_pair("--security-opt", "no-new-privileges")
        && has_pair("--pull", "never")
        && args.iter().any(|a| a == "--rm")
        && args.iter().any(|a| a == "--user")
        && args.iter().any(|a| a == "--pids-limit")
        && args.iter().any(|a| a == "--memory");
    if !must {
        return Err(bad("the docker run command is missing required hardening options"));
    }
    if !args.iter().any(|a| a.contains("@sha256:")) {
        return Err(bad("the image must be referenced by digest"));
    }
    let user = args.windows(2).find(|w| w[0] == "--user").map(|w| w[1].as_str()).unwrap_or("0");
    if user.split(':').any(|p| p.trim_start_matches('0').is_empty()) {
        return Err(bad("containers must not run as root"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::manifest::{MountSpec, ToolManifest};

    const DIGEST: &str = "sha256:529ee8a277ec8adc5b534d7c74eea0a47e9de21d62852b6ba7ac6ba9566845c3";

    fn manifest() -> ToolManifest {
        ToolManifest::from_toml(include_str!("../../../manifests/semgrep.toml")).unwrap()
    }

    fn approvals(src: &str, network: bool) -> RunApprovals {
        RunApprovals { network, mounts: HashMap::from([("source".to_string(), src.to_string())]), ports: HashMap::new() }
    }

    fn run(profile: &str, a: &RunApprovals) -> AppResult<DockerRun> {
        let m = manifest();
        let p = m.profile(profile).unwrap().clone();
        let img = parse_image("semgrep/semgrep", DIGEST).unwrap().reference();
        build_docker_run(&m, &p, &["scan".into(), "--config".into(), "p/default".into(), "/src".into()], &img, a, "vixalune-semgrep-ab12cd34", Some(Path::new("C:\\Users\\me")))
    }

    #[test]
    fn image_references_are_digest_only_and_normalised() {
        let r = parse_image("semgrep/semgrep", DIGEST).unwrap();
        assert_eq!(r.reference(), format!("docker.io/semgrep/semgrep@{DIGEST}"));
        assert_eq!(parse_image("nginx", DIGEST).unwrap().repository, "library/nginx");
        assert_eq!(parse_image("ghcr.io/org/tool", DIGEST).unwrap().registry, "ghcr.io");
        assert_eq!(parse_image("localhost:5000/x", DIGEST).unwrap().registry, "localhost:5000");
        for bad_img in ["semgrep/semgrep:latest", "semgrep/semgrep@sha256:abc", "UPPER/case", "a//b", "a b", ""] {
            assert!(parse_image(bad_img, DIGEST).is_err(), "{bad_img}");
        }
        assert!(parse_image("semgrep/semgrep", "sha256:short").is_err());
        assert!(parse_image("semgrep/semgrep", "latest").is_err());
    }

    #[test]
    fn only_approved_registries_are_allowed() {
        let ok: Vec<String> = DEFAULT_APPROVED_REGISTRIES.iter().map(|s| s.to_string()).collect();
        assert!(check_registry(&parse_image("semgrep/semgrep", DIGEST).unwrap(), &ok).is_ok());
        let evil = parse_image("evil.example.com/semgrep/semgrep", DIGEST).unwrap();
        let err = check_registry(&evil, &ok).unwrap_err().to_string();
        assert!(err.contains("evil.example.com") && err.contains("approved"));
        assert!(check_registry(&parse_image("localhost:5000/x", DIGEST).unwrap(), &ok).is_err());
        assert!(check_registry(&parse_image("semgrep/semgrep", DIGEST).unwrap(), &[]).is_err(), "an empty allow-list approves nothing");
    }

    #[test]
    fn host_mounts_accept_project_folders_and_reject_dangerous_ones() {
        let home = Some(Path::new("C:\\Users\\me"));
        for ok in ["D:\\labs\\project", "c:/work/code", "C:\\Users\\me\\projects\\app", "E:\\data\\"] {
            assert!(validate_host_mount(ok, home).is_ok(), "{ok}");
        }
        assert_eq!(validate_host_mount("c:/work/code", home).unwrap(), "C:\\work\\code");
        for evil in [
            "C:\\",
            "C:",
            "D:\\",
            "C:\\Windows",
            "C:\\Windows\\System32",
            "C:\\Program Files\\App",
            "C:\\ProgramData",
            "C:\\Users",
            "C:\\Users\\me",
            "C:\\Users\\me\\.ssh",
            "C:\\Users\\me\\proj\\.aws\\x",
            "C:\\Users\\me\\AppData\\Roaming",
            "\\\\server\\share",
            "\\\\.\\pipe\\docker_engine",
            "\\\\?\\C:\\x",
            "/var/run/docker.sock",
            "C:\\work\\..\\Windows",
            "C:\\work\\docker.sock",
            "relative\\path",
            "C:\\a,b",
            "C:\\a\"b",
            "",
            "C:\\work\\x.",
        ] {
            assert!(validate_host_mount(evil, home).is_err(), "should reject {evil:?}");
        }
        // A folder that contains the home folder is rejected too.
        assert!(validate_host_mount("C:\\Users", home).is_err());
    }

    #[test]
    fn builds_a_hardened_command_for_the_offline_profile() {
        let r = run("scan-local", &approvals("D:\\labs\\app", false)).unwrap();
        let a = &r.args;
        let has = |k: &str, v: &str| a.windows(2).any(|w| w[0] == k && w[1] == v);
        assert_eq!(a[0], "run");
        assert!(has("--network", "none") && has("--cap-drop", "ALL") && has("--security-opt", "no-new-privileges") && has("--pull", "never"));
        assert!(a.contains(&"--read-only".to_string()) && a.contains(&"--rm".to_string()));
        assert!(has("--user", "65532:65532"));
        assert!(a.iter().any(|x| x.starts_with("type=bind,source=D:\\labs\\app,target=") && x.ends_with(",readonly")));
        assert!(!a.iter().any(|x| x.contains("docker.sock") || x == "--privileged" || x == "host"));
        let img = a.iter().position(|x| x.contains("@sha256:")).unwrap();
        assert_eq!(a[img + 1], "semgrep");
        assert_eq!(r.summary.network, "none");
        assert!(r.summary.mounts[0].read_only && r.summary.warnings.is_empty());
        assert_eq!(r.container_name, "vixalune-semgrep-ab12cd34");
    }

    #[test]
    fn network_access_needs_explicit_matching_approval() {
        let m = manifest();
        assert!(m.profile("scan-registry-rules").unwrap().needs_network);
        let no = run("scan-registry-rules", &approvals("D:\\labs\\app", false)).unwrap_err().to_string();
        assert!(no.contains("network"), "{no}");
        let yes = run("scan-registry-rules", &approvals("D:\\labs\\app", true)).unwrap();
        assert!(yes.args.windows(2).any(|w| w[0] == "--network" && w[1] == "bridge"));
        assert!(yes.summary.warnings.iter().any(|w| w.contains("internet")));
        // Approving network for an offline profile is also refused (no over-approval).
        assert!(run("scan-local", &approvals("D:\\labs\\app", true)).is_err());
    }

    #[test]
    fn mounts_must_be_declared_approved_and_valid() {
        assert!(run("scan-local", &RunApprovals::default()).is_err(), "missing approval for the mount");
        assert!(run("scan-local", &approvals("C:\\Windows", false)).is_err());
        let mut extra = approvals("D:\\labs\\app", false);
        extra.mounts.insert("host_root".into(), "C:\\".into());
        assert!(run("scan-local", &extra).is_err(), "undeclared mounts are refused");
    }

    #[test]
    fn host_ports_need_approval_and_bind_to_loopback() {
        let mut m = manifest();
        // Extend the manifest in memory: allow publishing 8080.
        m.container.as_mut().unwrap().ports = vec![8080];
        let p = m.launch_profiles.iter_mut().find(|p| p.id == "scan-local").unwrap();
        p.ports = vec![8080];
        let p = p.clone();
        let img = parse_image("semgrep/semgrep", DIGEST).unwrap().reference();
        let build = |ports: HashMap<u16, u16>| {
            let mut a = approvals("D:\\labs\\app", false);
            a.ports = ports;
            build_docker_run(&m, &p, &[], &img, &a, "n", None)
        };
        assert!(build(HashMap::new()).is_err());
        assert!(build(HashMap::from([(8080, 80)])).is_err(), "privileged host ports refused");
        assert!(build(HashMap::from([(9999, 9999)])).is_err(), "undeclared container port refused");
        let ok = build(HashMap::from([(8080, 18080)])).unwrap();
        assert!(ok.args.windows(2).any(|w| w[0] == "--publish" && w[1] == "127.0.0.1:18080:8080"));
    }

    #[test]
    fn the_final_guard_rejects_hand_built_unsafe_commands() {
        let base = run("scan-local", &approvals("D:\\labs\\app", false)).unwrap().args;
        assert!(assert_hardened(&base).is_ok());
        let with = |extra: &[&str]| {
            let mut a = base.clone();
            a.splice(1..1, extra.iter().map(|s| s.to_string()));
            assert_hardened(&a)
        };
        for evil in [
            &["--privileged"][..],
            &["--cap-add", "SYS_ADMIN"],
            &["--cap-add=NET_ADMIN"],
            &["--device", "/dev/sda"],
            &["--pid", "host"],
            &["--ipc=host"],
            &["--userns", "host"],
            &["--volumes-from", "x"],
            &["--network=host"],
            &["--mount", "type=bind,source=/,target=/host"],
            &["--mount", "type=bind,source=\\\\.\\pipe\\docker_engine,target=/x"],
            &["-v", "/var/run/docker.sock:/var/run/docker.sock"],
            &["--publish", "0.0.0.0:80:80"],
            &["--security-opt", "seccomp=unconfined"],
        ] {
            assert!(with(evil).is_err(), "should reject {evil:?}");
        }
        // Removing a required option is also refused.
        let mut no_caps = base.clone();
        let i = no_caps.iter().position(|a| a == "--cap-drop").unwrap();
        no_caps.drain(i..i + 2);
        assert!(assert_hardened(&no_caps).is_err());
        // Root user is refused.
        let mut root = base.clone();
        let i = root.iter().position(|a| a == "--user").unwrap();
        root[i + 1] = "0:0".into();
        assert!(assert_hardened(&root).is_err());
        // Tags instead of digests are refused.
        let tagged: Vec<String> = base.iter().map(|a| a.replace(&format!("@{DIGEST}"), ":latest")).collect();
        assert!(assert_hardened(&tagged).is_err());
    }

    #[test]
    fn policy_validation_rejects_root_users_secret_env_and_unsafe_mounts() {
        let mut p = manifest().container.unwrap();
        assert!(p.validate().is_empty());
        for user in ["root", "0:0", "0", "1000", "1000:0", "abc:def"] {
            p.user = user.into();
            assert!(!p.validate().is_empty(), "{user}");
        }
        p.user = "65532:65532".into();
        p.env.push(crate::domain::manifest::EnvVar { name: "API_TOKEN".into(), value: "x".into() });
        assert!(!p.validate().is_empty());
        p.env.clear();
        p.mounts.push(MountSpec { name: "sock".into(), container_path: "/var/run/docker.sock".into(), read_only: true });
        assert!(!p.validate().is_empty());
        p.mounts.pop();
        p.memory_mb = 10;
        assert!(!p.validate().is_empty());
    }
}

