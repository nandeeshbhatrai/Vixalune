//! Vulnerability evidence. Records are *external evidence*: identifiers, affected ranges, fixed versions, timestamps and source
//! addresses are kept exactly as the database gave them, and what kind of claim each one supports is stated.

use super::packages::PackageQuery;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::OnceLock;

/// What a piece of evidence actually shows. Only the first two are ever produced by Vixalune; the last two exist so that a screen
/// can say "not assessed" instead of leaving the user to assume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingClass {
    /// A public vulnerability database lists this package version as affected.
    DatabaseMatch,
    /// An installed scanner reported it for a target the user chose.
    ScannerFinding,
    /// Someone verified that this system is configured in a way that makes the flaw reachable. Not assessed here.
    ConfirmedVulnerableConfiguration,
    /// Someone demonstrated exploitation. Not assessed here.
    DemonstratedExploitability,
}

impl FindingClass {
    pub fn describe(self) -> &'static str {
        match self {
            Self::DatabaseMatch => "Vulnerability database match: the database lists this version as affected. It does not show that your use of the package is vulnerable.",
            Self::ScannerFinding => "Scanner finding: a scanner reported it for the target you chose. Scanners can report false positives.",
            Self::ConfirmedVulnerableConfiguration => "Confirmed vulnerable configuration",
            Self::DemonstratedExploitability => "Demonstrated exploitability",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reference {
    pub kind: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AffectedRange {
    pub ecosystem: String,
    pub package: String,
    pub range_type: String,
    pub introduced: Option<String>,
    pub fixed: Option<String>,
    pub last_affected: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Severity {
    /// `CVSS_V3`, `CVSS_V4`, … as the database labels it.
    pub kind: String,
    /// The vector string or score exactly as published.
    pub score: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VulnRecord {
    pub id: String,
    pub aliases: Vec<String>,
    /// Free text written by the database's authors. Untrusted: shown as data, never as instructions.
    pub summary: String,
    pub severity: Vec<Severity>,
    pub affected: Vec<AffectedRange>,
    pub fixed_versions: Vec<String>,
    pub published: Option<String>,
    pub modified: Option<String>,
    pub references: Vec<Reference>,
    /// `OSV` or `NVD`.
    pub source: String,
    pub source_url: String,
    pub class: FindingClass,
}

impl VulnRecord {
    /// Every identifier this record can be cited by.
    pub fn all_ids(&self) -> impl Iterator<Item = &String> {
        std::iter::once(&self.id).chain(self.aliases.iter())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VulnLookup {
    pub source: String,
    /// What was asked, in the database's own terms.
    pub query: String,
    /// When Vixalune asked (RFC 3339).
    pub queried_at: String,
    pub records: Vec<VulnRecord>,
    /// More matched than are listed.
    pub truncated: bool,
    /// Limits that apply to this answer. Always present; an empty result is never described as "safe".
    pub coverage_notes: Vec<String>,
}

pub const MAX_RECORDS: usize = 25;

pub fn coverage_notes_for(source: &str, found: usize) -> Vec<String> {
    let mut v = vec![
        format!("{source} lists publicly reported vulnerabilities. It cannot know about flaws that have not been reported, or that are not in its coverage."),
        "A match means the database lists this version as affected. It does not show that your code reaches the vulnerable function, or that the flaw is exploitable in your setup.".to_string(),
        "Package identity matters: a package with the same name in another ecosystem, or a fork, is a different package.".to_string(),
    ];
    if found == 0 {
        v.insert(0, "No known vulnerability was found in this database. That is not proof that the package is safe.".to_string());
    }
    v
}

fn https_only(url: &str) -> bool {
    url.starts_with("https://") && url.len() <= 500 && !url.chars().any(|c| c.is_whitespace() || c.is_control())
}

fn str_of(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).map(|s| s.to_string())
}

fn cap(s: String, n: usize) -> String {
    super::untrusted::sanitize_untrusted(&s, n)
}

/// Parse one OSV vulnerability object. Only the entries for `package` are kept in `affected`.
pub fn parse_osv_vulnerability(v: &Value, package: Option<&PackageQuery>) -> Option<VulnRecord> {
    let id = str_of(v, "id")?;
    if !valid_vuln_id(&id) {
        return None;
    }
    let aliases: Vec<String> = v.get("aliases").and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|x| x.as_str()).filter(|s| valid_vuln_id(s)).map(|s| s.to_string()).collect()).unwrap_or_default();
    let summary = str_of(v, "summary").or_else(|| str_of(v, "details")).unwrap_or_default();
    let severity = v
        .get("severity")
        .and_then(|a| a.as_array())
        .map(|a| a.iter().filter_map(|s| Some(Severity { kind: cap(str_of(s, "type")?, 20), score: cap(str_of(s, "score")?, 120) })).collect())
        .unwrap_or_default();
    let mut affected = Vec::new();
    let mut fixed: BTreeSet<String> = BTreeSet::new();
    for a in v.get("affected").and_then(|a| a.as_array()).into_iter().flatten() {
        let (eco, name) = (a.pointer("/package/ecosystem").and_then(|x| x.as_str()).unwrap_or(""), a.pointer("/package/name").and_then(|x| x.as_str()).unwrap_or(""));
        if let Some(p) = package {
            let eco_base = eco.split(':').next().unwrap_or(eco);
            if !eco_base.eq_ignore_ascii_case(p.ecosystem.osv_name()) || !name.eq_ignore_ascii_case(&p.name) {
                continue;
            }
        }
        for r in a.get("ranges").and_then(|r| r.as_array()).into_iter().flatten() {
            let range_type = str_of(r, "type").unwrap_or_default();
            let mut current = AffectedRange { ecosystem: cap(eco.to_string(), 40), package: cap(name.to_string(), 214), range_type: cap(range_type, 20), introduced: None, fixed: None, last_affected: None };
            let mut open = false;
            for e in r.get("events").and_then(|e| e.as_array()).into_iter().flatten() {
                if let Some(i) = str_of(e, "introduced") {
                    if open {
                        affected.push(current.clone());
                        current.fixed = None;
                        current.last_affected = None;
                    }
                    current.introduced = Some(cap(i, 80));
                    open = true;
                }
                if let Some(f) = str_of(e, "fixed") {
                    current.fixed = Some(cap(f.clone(), 80));
                    fixed.insert(cap(f, 80));
                    affected.push(current.clone());
                    current.introduced = None;
                    current.fixed = None;
                    open = false;
                }
                if let Some(l) = str_of(e, "last_affected") {
                    current.last_affected = Some(cap(l, 80));
                    affected.push(current.clone());
                    current.introduced = None;
                    current.last_affected = None;
                    open = false;
                }
            }
            if open {
                affected.push(current);
            }
        }
    }
    let references = v
        .get("references")
        .and_then(|a| a.as_array())
        .map(|a| a.iter().filter_map(|r| Some(Reference { kind: cap(str_of(r, "type").unwrap_or_else(|| "WEB".into()), 20), url: str_of(r, "url").filter(|u| https_only(u))? })).take(10).collect())
        .unwrap_or_default();
    Some(VulnRecord {
        source_url: format!("https://osv.dev/vulnerability/{id}"),
        id,
        aliases,
        summary: cap(summary, 600),
        severity,
        affected,
        fixed_versions: fixed.into_iter().collect(),
        published: str_of(v, "published"),
        modified: str_of(v, "modified"),
        references,
        source: "OSV".into(),
        class: FindingClass::DatabaseMatch,
    })
}

/// Parse the NVD CVE API 2.0 answer for one CVE id.
pub fn parse_nvd_cve(v: &Value) -> Option<VulnRecord> {
    let cve = v.pointer("/vulnerabilities/0/cve")?;
    let id = str_of(cve, "id")?;
    if !valid_vuln_id(&id) {
        return None;
    }
    let summary = cve.get("descriptions").and_then(|d| d.as_array()).and_then(|d| d.iter().find(|x| x.get("lang").and_then(|l| l.as_str()) == Some("en"))).and_then(|x| str_of(x, "value")).unwrap_or_default();
    let mut severity = Vec::new();
    for (key, label) in [("cvssMetricV40", "CVSS_V4"), ("cvssMetricV31", "CVSS_V3"), ("cvssMetricV30", "CVSS_V3"), ("cvssMetricV2", "CVSS_V2")] {
        if let Some(m) = cve.pointer(&format!("/metrics/{key}/0/cvssData")) {
            let vector = str_of(m, "vectorString").unwrap_or_default();
            let score = m.get("baseScore").map(|s| s.to_string()).unwrap_or_default();
            severity.push(Severity { kind: label.into(), score: cap(format!("{vector} (base score {score})"), 160) });
        }
    }
    let references = cve
        .get("references")
        .and_then(|a| a.as_array())
        .map(|a| a.iter().filter_map(|r| Some(Reference { kind: "WEB".into(), url: str_of(r, "url").filter(|u| https_only(u))? })).take(10).collect())
        .unwrap_or_default();
    Some(VulnRecord {
        source_url: format!("https://nvd.nist.gov/vuln/detail/{id}"),
        id,
        aliases: vec![],
        summary: cap(summary, 600),
        severity,
        affected: vec![],
        fixed_versions: vec![],
        published: str_of(cve, "published"),
        modified: str_of(cve, "lastModified"),
        references,
        source: "NVD".into(),
        class: FindingClass::DatabaseMatch,
    })
}

fn id_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\b(CVE-\d{4}-\d{4,7}|GHSA(?:-[2-9cfghjmpqrvwx]{4}){3}|OSV-\d{4}-\d+|PYSEC-\d{4}-\d+|RUSTSEC-\d{4}-\d{4}|GO-\d{4}-\d{4,}|GSD-\d{4}-\d+|MAL-\d{4}-\d+|NVD-CWE-[A-Za-z]+-?\d*|DSA-\d+-\d+|USN-\d+-\d+|ALSA-\d{4}:\d+|RHSA-\d{4}:\d+|BIT-[A-Za-z0-9_\-]+-\d{4}-\d+)\b").unwrap()
    })
}

/// Shape check for an identifier taken from a database or typed by the user. Narrow on purpose: it is also what keeps odd
/// characters out of URLs and prompts.
pub fn valid_vuln_id(s: &str) -> bool {
    s.len() <= 40 && id_re().find(s).is_some_and(|m| m.start() == 0 && m.end() == s.len())
}

/// Identifiers that appear in `text` (any text: model output, scanner output).
pub fn ids_in(text: &str) -> BTreeSet<String> {
    id_re().find_iter(text).map(|m| m.as_str().to_ascii_uppercase()).collect()
}

/// Replace identifiers that are not in `known` (compared case-insensitively) and return the cleaned text and the ones removed.
pub fn scrub_unknown_ids(text: &str, known: &BTreeSet<String>) -> (String, Vec<String>) {
    let mut removed = Vec::new();
    let out = id_re().replace_all(text, |c: &regex::Captures<'_>| {
        let id = c[0].to_ascii_uppercase();
        if known.contains(&id) {
            c[0].to_string()
        } else {
            removed.push(id);
            "[unverified identifier removed]".to_string()
        }
    });
    (out.into_owned(), removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::assistant::packages::Ecosystem;
    use serde_json::json;

    fn lodash() -> PackageQuery {
        PackageQuery { ecosystem: Ecosystem::Npm, name: "lodash".into(), version: "4.17.15".into() }
    }

    #[test]
    fn osv_records_keep_ids_ranges_fixed_versions_timestamps_and_source_urls() {
        let v = json!({
            "id": "GHSA-35jh-r3h4-6jhm", "aliases": ["CVE-2021-23337"], "summary": "Command Injection in lodash",
            "published": "2021-05-06T16:05:51Z", "modified": "2024-01-01T00:00:00Z",
            "severity": [{"type": "CVSS_V3", "score": "CVSS:3.1/AV:N/AC:L/PR:H/UI:N/S:U/C:H/I:H/A:H"}],
            "affected": [
                {"package": {"ecosystem": "npm", "name": "lodash"}, "ranges": [{"type": "SEMVER", "events": [{"introduced": "0"}, {"fixed": "4.17.21"}]}]},
                {"package": {"ecosystem": "npm", "name": "lodash-es"}, "ranges": [{"type": "SEMVER", "events": [{"introduced": "0"}, {"fixed": "9.9.9"}]}]}
            ],
            "references": [{"type": "ADVISORY", "url": "https://nvd.nist.gov/vuln/detail/CVE-2021-23337"}, {"type": "WEB", "url": "javascript:alert(1)"}, {"type": "WEB", "url": "http://insecure.example"}]
        });
        let r = parse_osv_vulnerability(&v, Some(&lodash())).unwrap();
        assert_eq!(r.id, "GHSA-35jh-r3h4-6jhm");
        assert_eq!(r.aliases, vec!["CVE-2021-23337"]);
        assert_eq!(r.fixed_versions, vec!["4.17.21"], "only the package asked about");
        assert_eq!(r.affected.len(), 1);
        assert_eq!((r.affected[0].introduced.as_deref(), r.affected[0].fixed.as_deref()), (Some("0"), Some("4.17.21")));
        assert_eq!(r.published.as_deref(), Some("2021-05-06T16:05:51Z"));
        assert_eq!(r.source_url, "https://osv.dev/vulnerability/GHSA-35jh-r3h4-6jhm");
        assert_eq!(r.references.len(), 1, "only https references are kept");
        assert_eq!(r.class, FindingClass::DatabaseMatch);
    }

    #[test]
    fn records_with_odd_identifiers_are_dropped() {
        for id in ["", "x; rm -rf /", "CVE-1", "GHSA-xxxx", "../../etc"] {
            assert!(parse_osv_vulnerability(&json!({"id": id}), None).is_none(), "{id}");
        }
    }

    #[test]
    fn nvd_records_carry_score_vector_and_the_nvd_address() {
        let v = json!({"vulnerabilities": [{"cve": {"id": "CVE-2021-23337", "published": "2021-02-15T13:15:12.560", "lastModified": "2024-01-01T00:00:00", "descriptions": [{"lang": "es", "value": "x"}, {"lang": "en", "value": "Lodash command injection"}], "metrics": {"cvssMetricV31": [{"cvssData": {"vectorString": "CVSS:3.1/AV:N", "baseScore": 7.2}}]}, "references": [{"url": "https://github.com/lodash/lodash/pull/5085"}]}}]});
        let r = parse_nvd_cve(&v).unwrap();
        assert_eq!((r.id.as_str(), r.source.as_str(), r.summary.as_str()), ("CVE-2021-23337", "NVD", "Lodash command injection"));
        assert!(r.severity[0].score.contains("7.2") && r.severity[0].score.contains("CVSS:3.1"));
        assert_eq!(r.source_url, "https://nvd.nist.gov/vuln/detail/CVE-2021-23337");
    }

    #[test]
    fn invented_identifiers_are_found_and_removed() {
        let known: BTreeSet<String> = ["CVE-2021-23337".to_string()].into();
        let (t, removed) = scrub_unknown_ids("Fixes CVE-2021-23337 and also CVE-2099-12345 and GHSA-aaaa-bbbb-cccc.", &known);
        assert!(t.contains("CVE-2021-23337") && !t.contains("CVE-2099-12345"), "{t}");
        assert_eq!(removed, vec!["CVE-2099-12345"], "GHSA-aaaa-bbbb-cccc is not a valid GHSA shape and is left as ordinary text");
        assert_eq!(ids_in("see cve-2021-23337, GHSA-35jh-r3h4-6jhm").len(), 2);
    }

    #[test]
    fn an_empty_result_is_never_called_safe() {
        let n = coverage_notes_for("OSV", 0);
        assert!(n[0].contains("not proof that the package is safe"));
        assert!(coverage_notes_for("OSV", 3).iter().all(|s| !s.contains("not proof")));
    }
}
