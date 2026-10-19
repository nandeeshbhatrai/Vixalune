//! Vulnerability sources: OSV (package + version queries, advisory lookups) and NVD (CVE lookups). Both are public APIs that need
//! no key. Only the package ecosystem, name and version (or a vulnerability identifier) are sent; nothing else leaves the computer.

use crate::adapters::ai::http;
use crate::domain::ai::{AiError, AiErrorKind, AiResult};
use crate::domain::assistant::packages::PackageQuery;
use crate::domain::assistant::vuln::{coverage_notes_for, parse_nvd_cve, parse_osv_vulnerability, valid_vuln_id, VulnLookup, MAX_RECORDS};
use crate::domain::process::CancellationToken;
use crate::domain::secret::Secret;
use crate::ports::VulnSource;
use async_trait::async_trait;
use std::time::Duration;

pub const OSV_BASE: &str = "https://api.osv.dev";
pub const NVD_BASE: &str = "https://services.nvd.nist.gov";
const TIMEOUT: Duration = Duration::from_secs(20);
const MAX_BODY: usize = 4 * 1024 * 1024;

pub struct OsvSource {
    client: reqwest::Client,
    osv_base: String,
    nvd_base: String,
}

impl Default for OsvSource {
    fn default() -> Self {
        Self::with_bases(OSV_BASE, NVD_BASE)
    }
}

impl OsvSource {
    pub fn with_bases(osv: &str, nvd: &str) -> Self {
        Self { client: http::build_client(), osv_base: osv.trim_end_matches('/').to_string(), nvd_base: nvd.trim_end_matches('/').to_string() }
    }

    async fn get_json(&self, req: reqwest::RequestBuilder, cancel: &CancellationToken) -> AiResult<serde_json::Value> {
        let none = Secret::new(String::new());
        let resp = http::send(req, &none, cancel, TIMEOUT, http::default_error_parser).await?;
        http::read_json(resp, &none, cancel, TIMEOUT, MAX_BODY).await
    }

    async fn osv_by_id(&self, id: &str, cancel: &CancellationToken) -> AiResult<VulnLookup> {
        http::check_base_url(&self.osv_base)?;
        let v = self.get_json(self.client.get(format!("{}/v1/vulns/{id}", self.osv_base)), cancel).await?;
        let records: Vec<_> = parse_osv_vulnerability(&v, None).into_iter().collect();
        if records.is_empty() {
            return Err(AiError::new(AiErrorKind::Protocol, "OSV answered, but the record could not be read"));
        }
        Ok(VulnLookup { source: "OSV".into(), query: format!("advisory {id}"), queried_at: now(), coverage_notes: coverage_notes_for("OSV", records.len()), records, truncated: false })
    }

    async fn nvd_by_id(&self, id: &str, cancel: &CancellationToken) -> AiResult<VulnLookup> {
        http::check_base_url(&self.nvd_base)?;
        let v = self.get_json(self.client.get(format!("{}/rest/json/cves/2.0", self.nvd_base)).query(&[("cveId", id)]), cancel).await?;
        let records: Vec<_> = parse_nvd_cve(&v).into_iter().collect();
        let found = records.len();
        Ok(VulnLookup { source: "NVD".into(), query: format!("CVE {id}"), queried_at: now(), coverage_notes: coverage_notes_for("NVD", found), records, truncated: false })
    }
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

#[async_trait]
impl VulnSource for OsvSource {
    fn name(&self) -> &'static str {
        "OSV"
    }

    async fn query_package(&self, q: &PackageQuery, cancel: &CancellationToken) -> AiResult<VulnLookup> {
        http::check_base_url(&self.osv_base)?;
        let body = serde_json::json!({"package": {"name": q.name, "ecosystem": q.ecosystem.osv_name()}, "version": q.version});
        let v = self.get_json(self.client.post(format!("{}/v1/query", self.osv_base)).json(&body), cancel).await?;
        let all: Vec<_> = v.get("vulns").and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|x| parse_osv_vulnerability(x, Some(q))).collect()).unwrap_or_default();
        let truncated = all.len() > MAX_RECORDS || v.get("next_page_token").is_some();
        let records: Vec<_> = all.into_iter().take(MAX_RECORDS).collect();
        let mut notes = coverage_notes_for("OSV", records.len());
        notes.push("OSV matched the version on its servers; each record lists the affected ranges and fixed versions it was based on.".into());
        if truncated {
            notes.push(format!("Only the first {MAX_RECORDS} records are shown."));
        }
        Ok(VulnLookup { source: "OSV".into(), query: format!("{} {} {}", q.ecosystem.osv_name(), q.name, q.version), queried_at: now(), records, truncated, coverage_notes: notes })
    }

    async fn get_by_id(&self, id: &str, cancel: &CancellationToken) -> AiResult<VulnLookup> {
        if !valid_vuln_id(id) {
            return Err(AiError::new(AiErrorKind::InvalidRequest, "the vulnerability identifier is not valid"));
        }
        if id.to_ascii_uppercase().starts_with("CVE-") {
            self.nvd_by_id(&id.to_ascii_uppercase(), cancel).await
        } else {
            self.osv_by_id(id, cancel).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::ai::mock_server::{MockServer, Reply};
    use crate::domain::assistant::packages::Ecosystem;

    const OSV_FIXTURE: &str = include_str!("fixtures/osv_lodash_4_17_15.json");
    const NVD_FIXTURE: &str = include_str!("fixtures/nvd_cve_2021_23337.json");

    fn lodash() -> PackageQuery {
        PackageQuery { ecosystem: Ecosystem::Npm, name: "lodash".into(), version: "4.17.15".into() }
    }

    #[tokio::test]
    async fn a_package_query_sends_only_the_package_and_keeps_every_identifier_range_and_source() {
        let server = MockServer::start(|_| Reply::json(200, OSV_FIXTURE)).await;
        let src = OsvSource::with_bases(&server.base(), &server.base());
        let l = src.query_package(&lodash(), &CancellationToken::new()).await.unwrap();
        let sent = &server.captured()[0];
        assert_eq!((sent.method.as_str(), sent.path()), ("POST", "/v1/query"));
        assert_eq!(sent.json(), serde_json::json!({"package": {"name": "lodash", "ecosystem": "npm"}, "version": "4.17.15"}));
        assert!(sent.header("authorization").is_none() && sent.header("x-api-key").is_none());
        assert_eq!(l.records.len(), 6);
        let r = l.records.iter().find(|r| r.id == "GHSA-35jh-r3h4-6jhm").unwrap();
        assert!(r.aliases.contains(&"CVE-2021-23337".to_string()));
        assert_eq!(r.source_url, "https://osv.dev/vulnerability/GHSA-35jh-r3h4-6jhm");
        assert!(r.fixed_versions.contains(&"4.17.21".to_string()), "{:?}", r.fixed_versions);
        assert!(r.published.is_some() && r.modified.is_some());
        assert!(r.affected.iter().all(|a| a.package == "lodash"));
        assert!(!l.queried_at.is_empty() && l.source == "OSV");
        assert!(l.coverage_notes.iter().any(|n| n.contains("does not show that your code reaches")));
    }

    #[tokio::test]
    async fn no_results_is_reported_as_no_known_match_with_the_limits_and_never_as_safe() {
        let server = MockServer::start(|_| Reply::json(200, "{}")).await;
        let l = OsvSource::with_bases(&server.base(), &server.base()).query_package(&lodash(), &CancellationToken::new()).await.unwrap();
        assert!(l.records.is_empty());
        assert!(l.coverage_notes[0].contains("not proof that the package is safe"));
    }

    #[tokio::test]
    async fn cve_ids_go_to_nvd_and_advisory_ids_go_to_osv() {
        let server = MockServer::start(|c| if c.path().starts_with("/rest/json/cves") { Reply::json(200, NVD_FIXTURE) } else { Reply::json(200, r#"{"id":"GHSA-35jh-r3h4-6jhm","summary":"s","affected":[]}"#) }).await;
        let src = OsvSource::with_bases(&server.base(), &server.base());
        let n = src.get_by_id("cve-2021-23337", &CancellationToken::new()).await.unwrap();
        assert_eq!(n.source, "NVD");
        assert_eq!(n.records[0].id, "CVE-2021-23337");
        assert_eq!(n.records[0].source_url, "https://nvd.nist.gov/vuln/detail/CVE-2021-23337");
        assert!(server.captured()[0].target.contains("cveId=CVE-2021-23337"));
        let o = src.get_by_id("GHSA-35jh-r3h4-6jhm", &CancellationToken::new()).await.unwrap();
        assert_eq!(o.source, "OSV");
        assert!(src.get_by_id("../../etc/passwd", &CancellationToken::new()).await.is_err());
    }

    #[tokio::test]
    async fn rate_limits_and_server_errors_are_classified() {
        let server = MockServer::start(|_| Reply::json(429, "{}").header("Retry-After", "3")).await;
        let e = OsvSource::with_bases(&server.base(), &server.base()).query_package(&lodash(), &CancellationToken::new()).await.unwrap_err();
        assert_eq!(e.kind, AiErrorKind::RateLimit);
        let server = MockServer::start(|_| Reply::json(200, "not json")).await;
        let e = OsvSource::with_bases(&server.base(), &server.base()).query_package(&lodash(), &CancellationToken::new()).await.unwrap_err();
        assert_eq!(e.kind, AiErrorKind::Protocol);
    }

    /// Opt-in: asks the real OSV and NVD servers about lodash 4.17.15. Run with
    /// `cargo test --lib live_ -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "uses the public OSV and NVD APIs"]
    async fn live_osv_and_nvd_lodash() {
        let src = OsvSource::default();
        let l = src.query_package(&lodash(), &CancellationToken::new()).await.expect("OSV query");
        println!("OSV: {} record(s) at {}", l.records.len(), l.queried_at);
        for r in &l.records {
            println!("  {} {:?} fixed in {:?} ({})", r.id, r.aliases, r.fixed_versions, r.source_url);
        }
        assert!(l.records.iter().any(|r| r.aliases.iter().any(|a| a == "CVE-2021-23337")));
        let n = src.get_by_id("CVE-2021-23337", &CancellationToken::new()).await.expect("NVD query");
        println!("NVD: {} {:?}", n.records[0].id, n.records[0].severity);
        assert_eq!(n.records[0].source, "NVD");
    }
}
