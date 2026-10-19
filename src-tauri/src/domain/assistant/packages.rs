//! Understanding "check this package": which ecosystem, which package, which version. Parsing is deterministic code, not a
//! model guess, so a missing detail is reported as missing instead of being invented.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// Package ecosystems the vulnerability integration (OSV) can answer for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Ecosystem {
    #[serde(rename = "npm")]
    Npm,
    #[serde(rename = "PyPI")]
    PyPi,
    #[serde(rename = "crates.io")]
    CratesIo,
    #[serde(rename = "Go")]
    Go,
    #[serde(rename = "Maven")]
    Maven,
    #[serde(rename = "NuGet")]
    NuGet,
    #[serde(rename = "RubyGems")]
    RubyGems,
    #[serde(rename = "Packagist")]
    Packagist,
    #[serde(rename = "Hex")]
    Hex,
    #[serde(rename = "Pub")]
    Pub,
}

pub const ALL_ECOSYSTEMS: [Ecosystem; 10] = [
    Ecosystem::Npm,
    Ecosystem::PyPi,
    Ecosystem::CratesIo,
    Ecosystem::Go,
    Ecosystem::Maven,
    Ecosystem::NuGet,
    Ecosystem::RubyGems,
    Ecosystem::Packagist,
    Ecosystem::Hex,
    Ecosystem::Pub,
];

impl Ecosystem {
    /// The name OSV uses.
    pub fn osv_name(self) -> &'static str {
        match self {
            Self::Npm => "npm",
            Self::PyPi => "PyPI",
            Self::CratesIo => "crates.io",
            Self::Go => "Go",
            Self::Maven => "Maven",
            Self::NuGet => "NuGet",
            Self::RubyGems => "RubyGems",
            Self::Packagist => "Packagist",
            Self::Hex => "Hex",
            Self::Pub => "Pub",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let t = s.trim().trim_matches(|c: char| c == '.' || c == ',').to_ascii_lowercase();
        Some(match t.as_str() {
            "npm" | "node" | "nodejs" | "node.js" | "yarn" | "pnpm" | "javascript" => Self::Npm,
            "pypi" | "pip" | "python" | "pipenv" | "poetry" => Self::PyPi,
            "cargo" | "crates" | "crates.io" | "rust" => Self::CratesIo,
            "go" | "golang" => Self::Go,
            "maven" | "java" | "gradle" => Self::Maven,
            "nuget" | "dotnet" | ".net" | "csharp" | "c#" => Self::NuGet,
            "gem" | "gems" | "rubygems" | "ruby" | "bundler" => Self::RubyGems,
            "composer" | "packagist" | "php" => Self::Packagist,
            "hex" | "elixir" | "mix" => Self::Hex,
            "pub" | "dart" | "flutter" => Self::Pub,
            _ => return None,
        })
    }
}

pub fn supported_ecosystem_names() -> Vec<&'static str> {
    ALL_ECOSYSTEMS.iter().map(|e| e.osv_name()).collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageQuery {
    pub ecosystem: Ecosystem,
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissingField {
    Ecosystem,
    Name,
    Version,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageIntake {
    /// The conversation is not about the vulnerabilities of a package.
    NotAPackageQuestion,
    Complete(PackageQuery),
    /// Something that matters is missing; ask, do not guess.
    Incomplete { name: Option<String>, ecosystem: Option<Ecosystem>, version: Option<String>, missing: Vec<MissingField> },
    /// The user named an ecosystem the vulnerability database integration does not cover.
    UnsupportedEcosystem { requested: String },
}

fn version_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^v?\d+(\.\d+){0,3}([-+][0-9A-Za-z][0-9A-Za-z.\-+]*)?$").unwrap())
}

fn name_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(@[a-z0-9][a-z0-9._\-]{0,100}/)?[A-Za-z0-9][A-Za-z0-9._\-]{0,100}$").unwrap())
}

/// A package name as OSV would take it: no spaces, shell characters or control characters.
pub fn valid_package_name(s: &str) -> bool {
    s.len() <= 214 && (name_re().is_match(s) || maven_or_go_name(s))
}

fn maven_or_go_name(s: &str) -> bool {
    // Maven `group:artifact`, Go module paths `github.com/x/y`, Packagist `vendor/package`.
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._\-]*([:/][A-Za-z0-9][A-Za-z0-9._\-]*)+$").unwrap()).is_match(s)
}

pub fn valid_version(s: &str) -> bool {
    s.len() <= 64 && version_re().is_match(s)
}

const STOP: &[&str] = &[
    "the", "a", "an", "this", "that", "my", "our", "your", "package", "packages", "library", "module", "dependency", "dependencies", "version", "versions", "check", "is", "it", "its", "it's", "from", "for", "of", "in",
    "on", "at", "and", "or", "to", "are", "was", "any", "all", "vulnerability", "vulnerabilities", "vulnerable", "known", "security", "issues", "issue", "cve", "cves", "safe", "secure", "what", "which", "are", "there",
    "about", "tell", "me", "can", "you", "please", "using", "use", "uses", "with", "have", "has", "does", "do", "how", "i", "we", "im", "i'm", "want", "know", "find", "look", "scan", "run", "latest", "ecosystem", "pinned",
    "installed", "using:", "version:", "v", "be", "by", "as", "not", "no", "yes", "if", "then", "so", "but", "just", "also", "need", "help", "understand", "explain", "see", "get", "give", "show", "list", "affected",
];

fn is_stop(w: &str) -> bool {
    STOP.contains(&w)
}

const VULN_WORDS: &[&str] = &["vulnerab", "cve", "ghsa", "advisor", "exploit", "security issue", "security flaw", "known issues", "is it safe", "is this safe", "insecure", "cvss", "osv"];

fn has_vuln_intent(text: &str) -> bool {
    let l = text.to_ascii_lowercase();
    if VULN_WORDS.iter().any(|w| l.contains(w)) {
        return true;
    }
    // "check this package", "audit the library": a verb of inspection aimed at a package.
    let noun = ["package", "dependency", "library", "module", "crate"].iter().any(|w| l.contains(w));
    let verb = ["check", "audit", "vet", "inspect", "review", "safe", "trust"].iter().any(|w| l.contains(w));
    noun && verb
}

/// Words that mean the person is talking about scanning something of their own (a project, folder, image), not about one named package.
fn is_about_a_scan_target(text: &str) -> bool {
    let l = text.to_ascii_lowercase();
    ["project", "repository", "repo ", "codebase", "folder", "directory", "source code", "container", "docker", "lockfile", "lock file", "requirements.txt", "package.json", "my code", "my app"].iter().any(|w| l.contains(w))
}

const UNSUPPORTED_ECOSYSTEMS: &[&str] = &["apt", "dpkg", "debian", "ubuntu", "rpm", "yum", "dnf", "homebrew", "brew", "conan", "vcpkg", "cocoapods", "swiftpm", "swift", "cran", "perl", "cpan", "alpine", "apk", "chocolatey", "winget"];

#[derive(Default)]
struct Found {
    name: Option<String>,
    version: Option<String>,
    ecosystem: Option<Ecosystem>,
    unsupported: Option<String>,
}

fn clean_token(t: &str) -> &str {
    t.trim_matches(|c: char| matches!(c, ',' | ';' | ':' | '(' | ')' | '[' | ']' | '"' | '\'' | '`' | '?' | '!' | '<' | '>')).trim_end_matches('.')
}

/// `allow_bare_name`: accept a lone word in a short message as the package name (only sensible when the conversation is already
/// about vulnerabilities, for example as the answer to "which package?").
fn scan(text: &str, allow_bare_name: bool) -> Found {
    let mut f = Found::default();
    let toks: Vec<&str> = text.split_whitespace().map(clean_token).filter(|t| !t.is_empty()).collect();
    for (i, t) in toks.iter().enumerate() {
        let lower = t.to_ascii_lowercase();
        if f.ecosystem.is_none() {
            if let Some(e) = Ecosystem::parse(&lower) {
                // "go" / "rust" / "java" are only taken as an ecosystem when nothing else names the package, see below.
                f.ecosystem = Some(e);
            }
        }
        if f.unsupported.is_none() && UNSUPPORTED_ECOSYSTEMS.contains(&lower.as_str()) {
            f.unsupported = Some(lower.clone());
        }
        // name@version, name==version, name:version (not Maven group:artifact:version, which has two colons)
        for sep in ["@", "==", "=", "#"] {
            if let Some(pos) = t.rfind(sep) {
                if pos > 0 {
                    let (n, v) = (&t[..pos], &t[pos + sep.len()..]);
                    if valid_package_name(n) && valid_version(v) && !is_stop(&n.to_ascii_lowercase()) {
                        f.name.get_or_insert_with(|| n.to_string());
                        f.version.get_or_insert_with(|| v.trim_start_matches('v').to_string());
                    }
                }
            }
        }
        if valid_version(t) && f.version.is_none() {
            f.version = Some(t.trim_start_matches('v').to_string());
            // The word before the version, skipping "version"/"v"/"at".
            let mut j = i;
            while j > 0 {
                j -= 1;
                let p = toks[j];
                let pl = p.to_ascii_lowercase();
                if matches!(pl.as_str(), "version" | "v" | "at" | "==" | "=" | "@" | "pinned" | "to") {
                    continue;
                }
                if valid_package_name(p) && !is_stop(&pl) && Ecosystem::parse(&pl).is_none() && !valid_version(p) {
                    f.name.get_or_insert_with(|| p.to_string());
                }
                break;
            }
        }
    }
    // "package lodash", "library requests"
    if f.name.is_none() {
        for w in toks.windows(2) {
            if matches!(w[0].to_ascii_lowercase().as_str(), "package" | "library" | "module" | "dependency" | "crate" | "gem") {
                let p = w[1];
                let pl = p.to_ascii_lowercase();
                if valid_package_name(p) && !is_stop(&pl) && !valid_version(p) {
                    f.name = Some(p.to_string());
                    break;
                }
            }
        }
    }
    // A bare name in a short message that is clearly a reply, e.g. "lodash from npm".
    if allow_bare_name && f.name.is_none() && toks.len() <= 6 {
        for t in &toks {
            let l = t.to_ascii_lowercase();
            if valid_package_name(t) && !is_stop(&l) && Ecosystem::parse(&l).is_none() && !valid_version(t) && !UNSUPPORTED_ECOSYSTEMS.contains(&l.as_str()) {
                f.name = Some(t.to_string());
                break;
            }
        }
    }
    f
}

/// Work out the package question from the newest user message and up to four earlier ones (newest first).
pub fn extract_package_intake(current: &str, earlier_user_messages: &[&str]) -> PackageIntake {
    let history: Vec<&str> = earlier_user_messages.iter().take(4).copied().collect();
    let intent = has_vuln_intent(current) || history.iter().any(|m| has_vuln_intent(m));
    let strict = scan(current, false);
    // A name together with a dotted version ("lodash 4.17.15", "lodash@4.17.15") is a package spec even without any other wording.
    let looks_like_spec = strict.name.is_some() && strict.version.as_deref().is_some_and(|v| v.contains('.'));
    if !intent && !looks_like_spec {
        return PackageIntake::NotAPackageQuestion;
    }
    let cur = scan(current, true);
    // "Scan my project for vulnerable dependencies" is a question about a scanner, not a lookup of one package.
    if !looks_like_spec && is_about_a_scan_target(current) {
        return PackageIntake::NotAPackageQuestion;
    }
    // Without any vulnerability wording in the whole conversation, a bare "lodash 4.17.15" is still treated as a package question
    // only when it carries both a name and a version.
    let mut name = cur.name.clone();
    let mut version = cur.version.clone();
    let mut eco = cur.ecosystem;
    let mut unsupported = cur.unsupported.clone();
    for m in &history {
        let f = scan(m, true);
        if name.is_none() {
            name = f.name.clone();
        }
        if version.is_none() && (f.name.is_none() || f.name == name) {
            version = f.version.clone();
        }
        if eco.is_none() {
            eco = f.ecosystem;
        }
        if unsupported.is_none() {
            unsupported = f.unsupported;
        }
    }
    // The ecosystem words "go"/"rust"/"java" are also common words; they only count when a package was found or intent is clear.
    if let Some(u) = unsupported {
        if eco.is_none() {
            return PackageIntake::UnsupportedEcosystem { requested: u };
        }
    }
    let mut missing = Vec::new();
    if name.is_none() {
        missing.push(MissingField::Name);
    }
    if eco.is_none() {
        missing.push(MissingField::Ecosystem);
    }
    if version.is_none() {
        missing.push(MissingField::Version);
    }
    match (&name, eco, &version) {
        (Some(n), Some(e), Some(v)) => PackageIntake::Complete(PackageQuery { ecosystem: e, name: n.clone(), version: v.clone() }),
        _ => PackageIntake::Incomplete { name, ecosystem: eco, version, missing },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete(cur: &str, earlier: &[&str]) -> PackageQuery {
        match extract_package_intake(cur, earlier) {
            PackageIntake::Complete(q) => q,
            other => panic!("{cur:?} -> {other:?}"),
        }
    }

    #[test]
    fn complete_questions_in_the_usual_shapes() {
        for text in ["Are there vulnerabilities in lodash 4.17.15 from npm?", "check lodash@4.17.15 (npm) for CVEs", "lodash version 4.17.15 npm vulnerabilities", "Is lodash==4.17.15 safe? ecosystem: npm"] {
            let q = complete(text, &[]);
            assert_eq!((q.ecosystem, q.name.as_str(), q.version.as_str()), (Ecosystem::Npm, "lodash", "4.17.15"), "{text}");
        }
        let q = complete("known vulnerabilities in requests 2.19.0 on pypi", &[]);
        assert_eq!((q.ecosystem, q.name.as_str(), q.version.as_str()), (Ecosystem::PyPi, "requests", "2.19.0"));
        let q = complete("vulnerabilities in @angular/core@12.0.0 npm", &[]);
        assert_eq!((q.name.as_str(), q.version.as_str()), ("@angular/core", "12.0.0"));
    }

    #[test]
    fn a_follow_up_completes_an_earlier_question() {
        let q = complete("It's lodash 4.17.15 from npm.", &["Check this package for vulnerabilities."]);
        assert_eq!((q.ecosystem, q.name.as_str(), q.version.as_str()), (Ecosystem::Npm, "lodash", "4.17.15"));
        // Details spread over several messages.
        let q = complete("npm", &["version 4.17.15", "I want to know the vulnerabilities in the package lodash"]);
        assert_eq!((q.ecosystem, q.name.as_str(), q.version.as_str()), (Ecosystem::Npm, "lodash", "4.17.15"));
    }

    #[test]
    fn missing_details_are_reported_as_missing_not_guessed() {
        match extract_package_intake("Check this package for vulnerabilities", &[]) {
            PackageIntake::Incomplete { missing, .. } => assert_eq!(missing, vec![MissingField::Name, MissingField::Ecosystem, MissingField::Version]),
            other => panic!("{other:?}"),
        }
        match extract_package_intake("any vulnerabilities in the lodash package?", &[]) {
            PackageIntake::Incomplete { name, missing, .. } => {
                assert_eq!(name.as_deref(), Some("lodash"));
                assert_eq!(missing, vec![MissingField::Ecosystem, MissingField::Version]);
            }
            other => panic!("{other:?}"),
        }
        match extract_package_intake("is lodash 4.17.15 vulnerable?", &[]) {
            PackageIntake::Incomplete { missing, version, .. } => {
                assert_eq!(version.as_deref(), Some("4.17.15"));
                assert_eq!(missing, vec![MissingField::Ecosystem], "an ecosystem is never assumed");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn unsupported_ecosystems_are_named() {
        assert_eq!(extract_package_intake("any CVEs in openssl 3.0.1 from apt?", &[]), PackageIntake::UnsupportedEcosystem { requested: "apt".into() });
        assert_eq!(extract_package_intake("vulnerabilities in curl 7.0 on homebrew", &[]), PackageIntake::UnsupportedEcosystem { requested: "homebrew".into() });
    }

    #[test]
    fn other_questions_are_not_package_questions() {
        for t in [
            "Help me scan my own project for vulnerable dependencies.",
            "Which of my installed tools can analyze a container image?","Explain what Nmap does.", "Which tool can inspect HTTP traffic?", "Help me understand this scan output."] {
            assert_eq!(extract_package_intake(t, &[]), PackageIntake::NotAPackageQuestion, "{t}");
        }
    }

    #[test]
    fn names_and_versions_cannot_carry_shell_or_path_characters() {
        for bad in ["lodash; rm -rf /", "a b", "../../etc/passwd", "x$(id)", "`id`", "a|b", ""] {
            assert!(!valid_package_name(bad), "{bad}");
        }
        for ok in ["lodash", "@types/node", "org.apache.logging.log4j:log4j-core", "github.com/gin-gonic/gin", "laravel/framework"] {
            assert!(valid_package_name(ok), "{ok}");
        }
        assert!(valid_version("4.17.15") && valid_version("2.0.0-rc.1") && !valid_version("4.17.15; id") && !valid_version("latest"));
    }
}
