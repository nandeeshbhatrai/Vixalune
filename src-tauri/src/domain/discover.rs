//! Discovering packages beyond the reviewed catalog (winget). Everything here is pure: parsing of winget's text output, validation
//! of what the person typed, and the generation of a *community* manifest from a package's published facts. A community manifest
//! can only install one winget package and verify that it is installed: it has no launch profile, no dependency and no other
//! install method, and it is labelled as not reviewed.

use super::manifest::ToolManifest;
use super::{AppError, AppResult};
use serde::{Deserialize, Serialize};

pub const WINGET_PACKAGES_URL: &str = "https://github.com/microsoft/winget-pkgs";
pub const COMMUNITY_ID_PREFIX: &str = "community-";
pub const MAX_SEARCH_RESULTS: usize = 30;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredPackage {
    pub id: String,
    pub name: String,
    pub version: String,
    /// Why it matched (for example `Tag: nmap`), as winget reports it.
    pub matched: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredDetails {
    pub id: String,
    pub name: String,
    pub version: String,
    pub publisher: String,
    pub publisher_url: String,
    pub homepage: String,
    pub license: String,
    pub license_url: String,
    pub description: String,
    pub tags: Vec<String>,
    pub installer_type: String,
    pub installer_url: String,
    pub installer_sha256: String,
}

/// A search result as shown to the person, with what the application already knows about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoverResult {
    pub package: DiscoveredPackage,
    /// The catalog tool that installs this package, if any (reviewed or community).
    pub in_catalog: Option<String>,
    /// `true` when that catalog tool is a community one added from Discover.
    pub in_catalog_is_community: bool,
}

pub fn valid_search_query(q: &str) -> bool {
    let t = q.trim();
    (2..=64).contains(&t.chars().count()) && !t.starts_with('-') && t.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-' | '+' | '#' | '/'))
}

pub fn valid_winget_id(id: &str) -> bool {
    (3..=128).contains(&id.len()) && !id.starts_with(['-', '.']) && id.contains('.') && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

fn lines(text: &str) -> Vec<&str> {
    text.split(['\n', '\r']).filter(|l| !l.trim().is_empty()).collect()
}

/// Parse the table printed by `winget search --source winget`. Rows are fixed-width, positioned by the header.
pub fn parse_search_table(text: &str) -> Vec<DiscoveredPackage> {
    let all = lines(text);
    let Some(h) = all.iter().position(|l| l.starts_with("Name") && l.contains("Id") && l.contains("Version")) else { return vec![] };
    let header = all[h];
    let col = |w: &str| header.find(w);
    let (Some(id_at), Some(ver_at)) = (col("Id"), col("Version")) else { return vec![] };
    let match_at = header.find("Match");
    let mut out = vec![];
    for row in all.iter().skip(h + 1) {
        if row.chars().all(|c| c == '-') {
            continue;
        }
        let chars: Vec<char> = row.chars().collect();
        if chars.len() <= ver_at {
            continue;
        }
        let slice = |a: usize, b: usize| chars[a.min(chars.len())..b.min(chars.len())].iter().collect::<String>().trim().to_string();
        let name = slice(0, id_at);
        let id = slice(id_at, ver_at);
        let version = slice(ver_at, match_at.unwrap_or(chars.len())).trim_start_matches(['<', '>', ' ']).to_string();
        let matched = match_at.map(|m| slice(m, chars.len())).unwrap_or_default();
        if valid_winget_id(&id) && !name.is_empty() {
            out.push(DiscoveredPackage { id, name, version, matched });
        }
        if out.len() >= MAX_SEARCH_RESULTS {
            break;
        }
    }
    out
}

/// Parse `winget show --id … --exact`.
pub fn parse_show(text: &str) -> Option<DiscoveredDetails> {
    let all: Vec<&str> = text.split(['\n', '\r']).collect();
    let found = all.iter().find(|l| l.starts_with("Found "))?;
    let (name, id) = {
        let rest = found.strip_prefix("Found ")?;
        let open = rest.rfind('[')?;
        let close = rest.rfind(']')?;
        (rest[..open].trim().to_string(), rest[open + 1..close].trim().to_string())
    };
    if !valid_winget_id(&id) {
        return None;
    }
    let mut d = DiscoveredDetails { id, name, ..Default::default() };
    // Lines may or may not keep their indentation by the time they reach us, so keys are recognised by name alone.
    let mut in_tags = false;
    let mut in_description = false;
    for l in &all {
        let t = l.trim();
        if t.is_empty() || t.starts_with("Found ") {
            continue;
        }
        if let Some((k, v)) = t.split_once(':') {
            let known = matches!(k, "Version" | "Publisher" | "Publisher Url" | "Homepage" | "License" | "License Url" | "Description" | "Tags" | "Installer" | "Installer Type" | "Installer Url" | "Installer SHA256" | "Author" | "Moniker" | "Privacy Url" | "Copyright" | "Copyright Url" | "Release Notes" | "Release Notes Url" | "Purchase Url" | "Documentation" | "Offline Distribution Supported" | "Agreements" | "Installer Locale" | "Architecture" | "Scope" | "Package Url" | "Support Url" | "Release Date");
            if known {
                in_tags = false;
                in_description = false;
                let v = v.trim().to_string();
                match k {
                    "Version" if d.version.is_empty() => d.version = v,
                    "Publisher" => d.publisher = v,
                    "Publisher Url" => d.publisher_url = v,
                    "Homepage" => d.homepage = v,
                    "License" => d.license = v,
                    "License Url" => d.license_url = v,
                    "Description" => {
                        d.description = v;
                        in_description = true;
                    }
                    "Tags" => in_tags = true,
                    "Installer Type" if d.installer_type.is_empty() => d.installer_type = v,
                    "Installer Url" if d.installer_url.is_empty() => d.installer_url = v,
                    "Installer SHA256" if d.installer_sha256.is_empty() => d.installer_sha256 = v.to_lowercase(),
                    _ => {}
                }
                continue;
            }
        }
        if in_tags && !t.contains(' ') {
            d.tags.push(t.to_string());
        } else if in_description {
            d.description.push(' ');
            d.description.push_str(t);
        }
    }
    if d.version.is_empty() {
        return None;
    }
    Some(d)
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

/// The catalog id a community package gets.
pub fn community_tool_id(winget_id: &str) -> String {
    let s = slug(winget_id);
    let mut id = format!("{COMMUNITY_ID_PREFIX}{s}");
    id.truncate(63);
    id.trim_end_matches('-').to_string()
}

pub fn is_community_id(id: &str) -> bool {
    id.starts_with(COMMUNITY_ID_PREFIX)
}

fn toml_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '"' => o.push_str("\\\""),
            '\n' | '\r' | '\t' => o.push(' '),
            c if c.is_control() => {}
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn https_or(url: &str, fallback: &str) -> String {
    let u = url.trim();
    if u.starts_with("https://") && url::Url::parse(u).is_ok() && u.len() <= 500 && !u.chars().any(|c| c.is_whitespace() || c.is_control()) {
        u.to_string()
    } else {
        fallback.to_string()
    }
}

fn clip(s: &str, n: usize) -> String {
    let t: String = s.chars().filter(|c| !c.is_control()).take(n).collect();
    t.trim().to_string()
}

pub const COMMUNITY_NOTICE: &str = "Community package from the Windows Package Manager. Vixalune has not reviewed it, tested it or written run profiles for it; it can install it and check that it is installed. Only install software from publishers you trust.";

/// Build and validate a community manifest from a package's published facts. `today` is `YYYY-MM-DD`.
pub fn community_manifest_toml(d: &DiscoveredDetails, today: &str) -> AppResult<(String, ToolManifest)> {
    if !valid_winget_id(&d.id) {
        return Err(AppError::Validation("that is not a valid winget package id".into()));
    }
    let id = community_tool_id(&d.id);
    let name = clip(if d.name.is_empty() { &d.id } else { &d.name }, 80);
    let desc = clip(&d.description, 3000);
    let summary = if desc.is_empty() { "Community package (no description published).".to_string() } else { clip(desc.split(". ").next().unwrap_or(&desc), 190) };
    let homepage = https_or(&d.homepage, WINGET_PACKAGES_URL);
    let docs = https_or(if d.homepage.is_empty() { &d.publisher_url } else { &d.homepage }, WINGET_PACKAGES_URL);
    let license = if d.license.trim().is_empty() { "Not stated".to_string() } else { clip(&d.license, 100) };
    let publisher = if d.publisher.trim().is_empty() { "Unknown publisher".to_string() } else { clip(&d.publisher, 120) };
    let mut tags = vec!["community".to_string(), "unreviewed".to_string()];
    for t in &d.tags {
        let s = slug(t);
        if !s.is_empty() && s.len() <= 24 && tags.len() < 12 && !tags.contains(&s) {
            tags.push(s);
        }
    }
    let sha_note = if d.installer_sha256.len() == 64 && d.installer_sha256.chars().all(|c| c.is_ascii_hexdigit()) { format!(" The installer SHA-256 published for this version is {}.", d.installer_sha256) } else { String::new() };
    let description = format!("{desc}\n\n{COMMUNITY_NOTICE}");
    let toml = format!(
        r#"schema_version = 2
manifest_version = "1.0.0"
maintainer = "Community (added by you from Discover)"
id = {id}
name = {name}
summary = {summary}
description = {description}
category = "utilities"
tags = [{tags}]
license = {license}
homepage = {homepage}
documentation = {docs}
platforms = ["windows_native"]

[provenance]
publisher = {publisher}
basis = "community_maintained"
source_url = {src}
verified_on = {today}

[windows]
min_build = 19045
architectures = ["x64"]

[[prerequisites]]
kind = "winget"
reason = "Installs the package from the Windows Package Manager."

[[install]]
method = "winget"
package_id = {pkg}

[version]
pinned = {ver}

[integrity]
level = "package_manager"
note = {note}

[network]
install = true
run = true

[privileges]
install = "possible"
run = "possible"

[lifecycle]
can_update = true
can_uninstall = true

[verify]
timeout_secs = 60
[[verify.checks]]
kind = "package_installed"
"#,
        id = toml_str(&id),
        name = toml_str(&name),
        summary = toml_str(&summary),
        description = toml_str(&description),
        tags = tags.iter().map(|t| toml_str(t)).collect::<Vec<_>>().join(", "),
        license = toml_str(&license),
        homepage = toml_str(&homepage),
        docs = toml_str(&docs),
        publisher = toml_str(&publisher),
        src = toml_str(WINGET_PACKAGES_URL),
        today = toml_str(today),
        pkg = toml_str(&d.id),
        ver = toml_str(&d.version),
        note = toml_str(&format!("winget checks the installer SHA-256 from its catalog.{sha_note} Vixalune does not re-check it.")),
    );
    let m = ToolManifest::from_toml(&toml)?;
    community_ok(&m)?;
    Ok((toml, m))
}

/// What a community manifest is allowed to contain, checked again whenever one is loaded from disk.
pub fn community_ok(m: &ToolManifest) -> AppResult<()> {
    let only_winget = m.install.len() == 1 && matches!(m.install[0], super::manifest::InstallSpec::Winget { .. });
    if !is_community_id(m.id.as_str()) || !only_winget || !m.launch_profiles.is_empty() || !m.dependencies.is_empty() || m.container.is_some() || m.platforms != [super::manifest::RuntimeKind::WindowsNative] {
        return Err(AppError::Validation("a community entry can only install one winget package; it cannot define run profiles, dependencies or other install methods".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEARCH: &str = "Name      Id                       Version Match\r\n----------------------------------------------------\r\nNmap      Insecure.Nmap            7.80    \r\nHopMatrix RedEyeNetworks.HopMatrix 1.19.0  Tag: nmap\r\n";
    const SHOW: &str = "Found Nmap [Insecure.Nmap]\r\nVersion: 7.80\r\nPublisher: Nmap Project\r\nPublisher Url: https://nmap.org\r\nAuthor: Nmap Project\r\nMoniker: nmap\r\nDescription: Nmap (\"Network Mapper\") is a free and open source utility for network discovery and security auditing.\r\nHomepage: https://nmap.org\r\nLicense: Modified GNU GPLv2\r\nLicense Url: https://nmap.org/book/man-legal.html\r\nTags:\r\n  network\r\n  scan\r\n  security\r\nInstaller:\r\n  Installer Type: nullsoft\r\n  Installer Url: https://nmap.org/dist/nmap-7.80-setup.exe\r\n  Installer SHA256: 3b4d726bd366e7439367fa78a186dfa9b641d3b2ad354fd915581b6567480f94\r\n";

    #[test]
    fn search_tables_from_winget_are_parsed_by_column() {
        let r = parse_search_table(SEARCH);
        assert_eq!(r.len(), 2);
        assert_eq!((r[0].id.as_str(), r[0].name.as_str(), r[0].version.as_str(), r[0].matched.as_str()), ("Insecure.Nmap", "Nmap", "7.80", ""));
        assert_eq!((r[1].id.as_str(), r[1].matched.as_str()), ("RedEyeNetworks.HopMatrix", "Tag: nmap"));
        assert!(parse_search_table("No package found matching input criteria.").is_empty());
        assert!(parse_search_table("- \\ | /\r\n").is_empty());
    }

    #[test]
    fn rows_with_odd_ids_are_dropped() {
        let t = "Name Id Version\r\n-------\r\nEvil x; calc 1.0\r\nOk   A.B   1.0\r\n";
        let r = parse_search_table(t);
        assert!(r.iter().all(|p| valid_winget_id(&p.id)), "{r:?}");
    }

    #[test]
    fn package_details_are_read_from_winget_show() {
        let d = parse_show(SHOW).unwrap();
        assert_eq!((d.id.as_str(), d.name.as_str(), d.version.as_str(), d.publisher.as_str()), ("Insecure.Nmap", "Nmap", "7.80", "Nmap Project"));
        assert_eq!((d.license.as_str(), d.installer_type.as_str()), ("Modified GNU GPLv2", "nullsoft"));
        assert_eq!(d.tags, vec!["network", "scan", "security"]);
        assert_eq!(d.installer_sha256.len(), 64);
        assert!(parse_show("No package found matching input criteria.").is_none());
    }

    #[test]
    fn details_are_read_even_when_indentation_is_lost_on_the_way() {
        let flat: String = SHOW.lines().map(|l| l.trim_start().to_string() + "\n").collect();
        let d = parse_show(&flat).unwrap();
        assert_eq!((d.installer_type.as_str(), d.installer_sha256.len(), d.tags.len()), ("nullsoft", 64, 3));
        assert!(d.description.starts_with("Nmap") && !d.description.contains("network scan security"), "{}", d.description);
    }

    #[test]
    fn typed_input_is_validated() {
        for ok in ["nmap", "port scanner", "c++", "wire shark"] {
            assert!(valid_search_query(ok), "{ok}");
        }
        for bad in ["", "a", "--help", "nmap; calc", "a|b", "x\u{0}y", &"x".repeat(80), "$(id)"] {
            assert!(!valid_search_query(bad), "{bad}");
        }
        for bad in ["calc", "-x.y", "a b.c", "A.B;C", "../x.y", ""] {
            assert!(!valid_winget_id(bad), "{bad}");
        }
        assert!(valid_winget_id("Insecure.Nmap"));
    }

    #[test]
    fn a_community_manifest_can_only_install_and_verify_one_winget_package() {
        let d = parse_show(SHOW).unwrap();
        let (toml, m) = community_manifest_toml(&d, "2026-10-20").unwrap();
        assert_eq!(m.id.as_str(), "community-insecure-nmap");
        assert!(m.launch_profiles.is_empty() && m.dependencies.is_empty());
        assert_eq!(m.version.pinned.as_deref(), Some("7.80"));
        assert!(m.tags.contains(&"community".to_string()) && m.tags.contains(&"unreviewed".to_string()));
        assert!(m.description.contains("has not reviewed it"));
        assert!(m.integrity.note.as_deref().unwrap().contains(&d.installer_sha256));
        assert!(toml.contains("package_id = \"Insecure.Nmap\""));
        assert_eq!(m.provenance.basis, crate::domain::manifest::ProvenanceBasis::CommunityMaintained);
    }

    #[test]
    fn hostile_package_text_cannot_break_out_of_the_manifest() {
        let mut d = parse_show(SHOW).unwrap();
        d.name = "Evil\"\n[[launch_profiles]]\nid = \"run\"".into();
        d.description = "x\"\n\n[[launch_profiles]]\nid = \"p\"\nexecutable = \"calc.exe\"\n".into();
        d.license = "MIT\"\n[privileges]".into();
        d.homepage = "javascript:alert(1)".into();
        d.tags = vec!["a\"b".into(), "UPPER Case!".into()];
        let (_, m) = community_manifest_toml(&d, "2026-10-20").unwrap();
        assert!(m.launch_profiles.is_empty(), "text stays text");
        assert_eq!(m.homepage, WINGET_PACKAGES_URL, "only https links are kept");
        assert!(m.tags.iter().all(|t| t.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')));
        d.id = "calc".into();
        assert!(community_manifest_toml(&d, "2026-10-20").is_err());
    }

    #[test]
    fn hand_edited_community_files_cannot_gain_run_profiles() {
        let d = parse_show(SHOW).unwrap();
        let (toml, _) = community_manifest_toml(&d, "2026-10-20").unwrap();
        let sneaky = format!("{toml}\n[[launch_profiles]]\nid = \"run\"\nlabel = \"Run\"\nruntime = \"windows_native\"\nexecutable = \"calc.exe\"\nargs = []\n");
        let m = ToolManifest::from_toml(&sneaky).unwrap();
        assert!(community_ok(&m).is_err());
    }

    #[test]
    fn community_ids_cannot_collide_with_reviewed_ones() {
        assert!(community_tool_id("Some.Very-Long.Package.Name.That.Keeps.Going.And.Going.Forever.Ever.Ever").len() <= 63);
        assert!(is_community_id(&community_tool_id("A.B")));
        assert!(!is_community_id("trivy"));
    }
}
