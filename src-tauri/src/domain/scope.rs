//! Authorization scope guardrails for tools that send network traffic.
//!
//! These are product guardrails against *mistakes* (typing the wrong address, a lab name that resolves
//! somewhere unexpected). They are not a substitute for legal authorization: you may only test systems you
//! own or have written permission to test.
//!
//! Defaults: loopback and the configured lab allow-list are in scope without extra confirmation. Everything
//! else needs an explicit acknowledgement (private networks) or typing the target (public addresses).
//!
//! What is checked, and what cannot be:
//! * Inputs are parsed strictly. Alternate spellings of an address (`127.1`, `0x7f.0.0.1`, `2130706433`,
//!   `017700000001`, URL tricks such as `user@host`, backslashes, percent-encoding, unicode hosts) are rejected
//!   rather than normalised, because the *tool* may interpret them differently from us.
//! * Hostnames are resolved and **every** resulting address is classified; a name is only a lab name if all of its
//!   addresses are loopback/private, so a public DNS record cannot masquerade as a lab host.
//! * Addresses are re-resolved immediately before a run and must match what the user confirmed.
//! * For plain `http://` URLs, redirects are followed manually and each hop is checked.
//! * Not checkable: what a tool does after it starts (it resolves names again, follows `https` redirects, crawls
//!   links, or scans beyond what it was asked). Tool profiles therefore avoid crawling flags, and the UI says so.

use super::errors::{AppError, AppResult};
use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;
use url::Url;

fn bad(msg: impl Into<String>) -> AppError {
    AppError::Validation(msg.into())
}

/// Largest range a single parameter may name: 4096 IPv4 addresses (/20) or 4096 IPv6 addresses (/116).
pub const MAX_V4_PREFIX_LEN: u8 = 20;
pub const MAX_V6_PREFIX_LEN: u8 = 116;

// ------------------------------------------------------------------ localities and verdicts

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Locality {
    Loopback,
    /// In the user's configured lab allow-list.
    Lab,
    PrivateNetwork,
    Public,
    /// `0.0.0.0`, `::`: not a specific target.
    Unspecified,
    /// Multicast, broadcast or reserved ranges.
    Special,
    /// A name that does not resolve, so its scope cannot be checked.
    Unresolvable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Allowed,
    NeedsConfirmation,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetForm {
    /// Hostname, IP address or CIDR range.
    Host,
    Ip,
    Url,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    Name(String),
    Ip(IpAddr),
    Net(IpNet),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UrlParts {
    pub scheme: String,
    pub port: u16,
    pub path_and_query: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedTarget {
    pub raw: String,
    /// The one spelling Vixalune uses (and passes to the tool).
    pub canonical: String,
    pub endpoint: Endpoint,
    pub url: Option<UrlParts>,
}

// ------------------------------------------------------------------ strict parsing

fn only_ascii_printable(s: &str) -> bool {
    s.chars().all(|c| c.is_ascii() && !c.is_ascii_control() && c != ' ')
}

/// True for inputs the OS or a tool might read as an IPv4 address even though they are not strict dotted-quad.
fn looks_numeric_address(s: &str) -> bool {
    !s.is_empty() && s.chars().any(|c| c.is_ascii_digit()) && s.chars().all(|c| c.is_ascii_hexdigit() || matches!(c, 'x' | 'X' | '.'))
}

fn parse_hostname(h: &str) -> AppResult<String> {
    let name = h.trim_end_matches('.').to_ascii_lowercase();
    if name.is_empty() || name.len() > 253 {
        return Err(bad("the host name is empty or too long"));
    }
    if looks_numeric_address(&name) {
        return Err(bad(format!("'{h}' looks like an address in a non-standard notation; write IPv4 addresses as four numbers, for example 127.0.0.1")));
    }
    for label in name.split('.') {
        let ok = !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !ok {
            return Err(bad(format!("'{h}' is not a valid host name (letters, digits and hyphens only; internationalised names must be written in their ASCII 'xn--' form)")));
        }
    }
    // A top-level label that is all digits is how numeric shorthand hides.
    if name.rsplit('.').next().is_some_and(|tld| tld.chars().all(|c| c.is_ascii_digit())) {
        return Err(bad(format!("'{h}' is not a valid host name")));
    }
    Ok(name)
}

fn parse_net(s: &str) -> AppResult<IpNet> {
    let net = IpNet::from_str(s).map_err(|_| bad(format!("'{s}' is not a valid network range; use CIDR such as 192.168.56.0/24")))?;
    let max = match net {
        IpNet::V4(_) => MAX_V4_PREFIX_LEN,
        IpNet::V6(_) => MAX_V6_PREFIX_LEN,
    };
    if net.prefix_len() < max {
        return Err(bad(format!("the range {s} is too large; at most 4096 addresses (/{max} or smaller) can be targeted at once")));
    }
    if net.addr() != net.network() {
        return Err(bad(format!("'{s}' has host bits set; write the network as {}", net.trunc())));
    }
    Ok(net)
}

fn parse_ip(s: &str) -> AppResult<IpAddr> {
    let inner = s.strip_prefix('[').and_then(|r| r.strip_suffix(']')).unwrap_or(s);
    if s.starts_with('[') != s.ends_with(']') {
        return Err(bad("unbalanced brackets in the address"));
    }
    if inner.contains('%') {
        return Err(bad("scoped (zone) IPv6 addresses are not supported"));
    }
    IpAddr::from_str(inner).map_err(|_| bad(format!("'{s}' is not a valid IP address")))
}

fn parse_url(raw: &str) -> AppResult<ParsedTarget> {
    if raw.len() > 2000 || !only_ascii_printable(raw) {
        return Err(bad("the URL is too long or contains spaces, control characters or non-ASCII characters"));
    }
    if raw.contains('\\') {
        return Err(bad("backslashes are not allowed in URLs (different programs read them differently)"));
    }
    let url = Url::parse(raw).map_err(|e| bad(format!("'{raw}' is not a valid URL: {e}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(bad("only http:// and https:// URLs are supported"));
    }
    if !url.username().is_empty() || url.password().is_some() || raw.split("://").nth(1).is_some_and(|r| r.split(['/', '?', '#']).next().unwrap_or("").contains('@')) {
        return Err(bad("user names and passwords inside URLs are not allowed"));
    }
    let host = url.host().ok_or_else(|| bad("the URL has no host"))?;
    let authority = raw.split("://").nth(1).unwrap_or("").split(['/', '?', '#']).next().unwrap_or("").to_ascii_lowercase();
    let (endpoint, host_text) = match host {
        url::Host::Domain(d) => (Endpoint::Name(parse_hostname(d)?), d.to_string()),
        url::Host::Ipv4(v4) => {
            // The URL parser normalises odd spellings (127.1, 0x7f.1, 2130706433); a tool may not, so require the plain form.
            let canon = v4.to_string();
            let host_only = authority.rsplit_once(':').filter(|(_, p)| p.chars().all(|c| c.is_ascii_digit())).map(|(h, _)| h).unwrap_or(&authority);
            if host_only != canon {
                return Err(bad(format!("the URL host is written in a non-standard notation; use {canon}")));
            }
            (Endpoint::Ip(IpAddr::V4(v4)), canon)
        }
        url::Host::Ipv6(v6) => (Endpoint::Ip(IpAddr::V6(v6)), format!("[{v6}]")),
    };
    let port = url.port_or_known_default().ok_or_else(|| bad("the URL has no port"))?;
    let mut pq = url.path().to_string();
    if let Some(q) = url.query() {
        pq.push('?');
        pq.push_str(q);
    }
    let canonical = {
        let default = (url.scheme() == "http" && port == 80) || (url.scheme() == "https" && port == 443);
        let hp = if default { host_text } else { format!("{host_text}:{port}") };
        format!("{}://{hp}{pq}", url.scheme())
    };
    Ok(ParsedTarget { raw: raw.to_string(), canonical, endpoint, url: Some(UrlParts { scheme: url.scheme().into(), port, path_and_query: pq }) })
}

/// Parse one target value according to its declared form.
pub fn parse_target(raw: &str, form: TargetForm) -> AppResult<ParsedTarget> {
    let s = raw.trim();
    if s.is_empty() || s.len() > 2000 {
        return Err(bad("the target is empty or too long"));
    }
    if form == TargetForm::Url {
        return parse_url(s);
    }
    if !only_ascii_printable(s) {
        return Err(bad("the target contains spaces, control characters or non-ASCII characters"));
    }
    if s.starts_with('-') {
        return Err(bad("a target must not start with '-' (it would be read as an option)"));
    }
    if s.contains(',') || s.contains(';') {
        return Err(bad("enter one target per field (lists with commas are not accepted)"));
    }
    if s.contains('@') || s.contains('\\') || s.contains('?') || s.contains('#') || s.contains('*') {
        return Err(bad("the target contains characters that are not allowed in a host name or address"));
    }
    if s.contains('/') {
        if form == TargetForm::Ip {
            return Err(bad("enter a single IP address, not a range"));
        }
        let net = parse_net(s)?;
        return Ok(ParsedTarget { raw: s.into(), canonical: net.to_string(), endpoint: Endpoint::Net(net), url: None });
    }
    if s.contains('-') && s.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | '-')) {
        return Err(bad("address ranges such as 10.0.0.1-50 are not supported; use CIDR notation such as 10.0.0.0/24"));
    }
    if s.contains(':') || s.starts_with('[') {
        let ip = parse_ip(s)?;
        return Ok(ParsedTarget { raw: s.into(), canonical: ip.to_string(), endpoint: Endpoint::Ip(ip), url: None });
    }
    if let Ok(v4) = Ipv4Addr::from_str(s) {
        return Ok(ParsedTarget { raw: s.into(), canonical: v4.to_string(), endpoint: Endpoint::Ip(IpAddr::V4(v4)), url: None });
    }
    if form == TargetForm::Ip {
        return Err(bad(format!("'{s}' is not a valid IP address")));
    }
    let name = parse_hostname(s)?;
    Ok(ParsedTarget { raw: s.into(), canonical: name.clone(), endpoint: Endpoint::Name(name), url: None })
}

// ------------------------------------------------------------------ policy and classification

#[derive(Debug, Clone, PartialEq, Eq)]
enum LabEntry {
    Name(String),
    Net(IpNet),
}

/// The user's configured managed labs: host names, IP addresses or CIDR ranges.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScopePolicy {
    entries: Vec<LabEntry>,
}

impl ScopePolicy {
    pub fn parse(list: &[String]) -> AppResult<Self> {
        if list.len() > 64 {
            return Err(bad("at most 64 lab entries"));
        }
        let mut entries = Vec::new();
        for raw in list {
            let t = parse_target(raw, TargetForm::Host).map_err(|e| bad(format!("lab entry '{raw}': {e}")))?;
            entries.push(match t.endpoint {
                Endpoint::Name(n) => LabEntry::Name(n),
                Endpoint::Ip(ip) => LabEntry::Net(IpNet::from(ip)),
                Endpoint::Net(n) => LabEntry::Net(n),
            });
        }
        Ok(Self { entries })
    }

    fn lab_net_contains(&self, net: &IpNet) -> bool {
        self.entries.iter().any(|e| matches!(e, LabEntry::Net(n) if n.contains(net)))
    }

    fn lab_name(&self, name: &str) -> bool {
        self.entries.iter().any(|e| matches!(e, LabEntry::Name(n) if n == name))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum IpClass {
    Loopback,
    Private,
    Public,
    Unspecified,
    Special,
}

fn v4_class(v: Ipv4Addr) -> IpClass {
    let o = v.octets();
    if v.is_loopback() {
        IpClass::Loopback
    } else if o[0] == 0 {
        IpClass::Unspecified
    } else if v.is_multicast() || o[0] >= 240 || v.is_broadcast() {
        IpClass::Special
    } else if v.is_private() || v.is_link_local() || (o[0] == 100 && (64..128).contains(&o[1])) {
        IpClass::Private
    } else {
        IpClass::Public
    }
}

fn v6_class(v: Ipv6Addr) -> IpClass {
    // Addresses that embed an IPv4 address are judged by that address: they reach it.
    if let Some(v4) = v.to_ipv4_mapped() {
        return v4_class(v4);
    }
    let s = v.segments();
    if s[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        return v4_class(Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8)); // NAT64
    }
    if s[0] == 0x2002 {
        return v4_class(Ipv4Addr::new((s[1] >> 8) as u8, s[1] as u8, (s[2] >> 8) as u8, s[2] as u8)); // 6to4
    }
    if v.is_loopback() {
        IpClass::Loopback
    } else if v.is_unspecified() {
        IpClass::Unspecified
    } else if v.is_multicast() {
        IpClass::Special
    } else if (s[0] & 0xfe00) == 0xfc00 || (s[0] & 0xffc0) == 0xfe80 {
        IpClass::Private
    } else {
        IpClass::Public
    }
}

fn ip_class(ip: IpAddr) -> IpClass {
    match ip {
        IpAddr::V4(v) => v4_class(v),
        IpAddr::V6(v) => v6_class(v),
    }
}

fn locality_of_class(c: IpClass, lab: bool) -> Locality {
    match c {
        IpClass::Loopback => Locality::Loopback,
        IpClass::Private if lab => Locality::Lab,
        IpClass::Private => Locality::PrivateNetwork,
        IpClass::Public if lab => Locality::Lab, // a lab entry that names a public address is the user's explicit choice
        IpClass::Public => Locality::Public,
        IpClass::Unspecified => Locality::Unspecified,
        IpClass::Special => Locality::Special,
    }
}

pub fn verdict_for(l: Locality) -> Verdict {
    match l {
        Locality::Loopback | Locality::Lab => Verdict::Allowed,
        Locality::PrivateNetwork | Locality::Public => Verdict::NeedsConfirmation,
        Locality::Unspecified | Locality::Special | Locality::Unresolvable => Verdict::Blocked,
    }
}

/// Resolves host names to addresses. Production uses the OS resolver; tests inject fixed answers.
pub trait NameResolver: Send + Sync {
    fn resolve(&self, name: &str) -> Result<Vec<IpAddr>, String>;
}

// ------------------------------------------------------------------ assessment

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetAssessment {
    pub parameter: String,
    pub raw: String,
    pub canonical: String,
    pub form: TargetForm,
    /// Addresses the target maps to (for a name: every resolved address; for a CIDR: the range).
    pub addresses: Vec<String>,
    pub locality: Locality,
    pub verdict: Verdict,
    pub reasons: Vec<String>,
}

fn net_class(net: &IpNet, policy: &ScopePolicy) -> Locality {
    let first = ip_class(net.network());
    let last = ip_class(net.broadcast());
    let class = first.max(last);
    locality_of_class(class, policy.lab_net_contains(net) && class != IpClass::Loopback)
}

/// Classify one parsed target.
pub fn assess_target(parameter: &str, t: &ParsedTarget, form: TargetForm, policy: &ScopePolicy, resolver: &dyn NameResolver) -> TargetAssessment {
    let mut reasons = Vec::new();
    let (addresses, locality) = match &t.endpoint {
        Endpoint::Ip(ip) => {
            let lab = policy.lab_net_contains(&IpNet::from(*ip));
            (vec![ip.to_string()], locality_of_class(ip_class(*ip), lab && ip_class(*ip) != IpClass::Loopback))
        }
        Endpoint::Net(n) => (vec![n.to_string()], net_class(n, policy)),
        Endpoint::Name(name) => match resolver.resolve(name) {
            Err(e) => {
                reasons.push(format!("'{name}' could not be resolved ({e}), so its scope cannot be checked"));
                (vec![], Locality::Unresolvable)
            }
            Ok(ips) if ips.is_empty() => {
                reasons.push(format!("'{name}' resolves to no address, so its scope cannot be checked"));
                (vec![], Locality::Unresolvable)
            }
            Ok(mut ips) => {
                ips.sort();
                ips.dedup();
                let named_lab = policy.lab_name(name);
                let classes: Vec<IpClass> = ips.iter().map(|i| ip_class(*i)).collect();
                // A name only counts as a lab host if EVERY address is non-public; a public record cannot be labelled a lab.
                let all_local = classes.iter().all(|c| matches!(c, IpClass::Loopback | IpClass::Private));
                let loc = ips
                    .iter()
                    .map(|ip| {
                        let in_lab_net = policy.lab_net_contains(&IpNet::from(*ip));
                        locality_of_class(ip_class(*ip), (named_lab && all_local || in_lab_net) && ip_class(*ip) != IpClass::Loopback)
                    })
                    .max()
                    .unwrap_or(Locality::Unresolvable);
                if named_lab && !all_local {
                    reasons.push(format!("'{name}' is on your lab list but resolves to a public address, so it is treated as a public target"));
                }
                if ips.len() > 1 && classes.iter().any(|c| *c != classes[0]) {
                    reasons.push(format!("'{name}' resolves to addresses in different networks; the widest scope applies"));
                }
                (ips.iter().map(|i| i.to_string()).collect(), loc)
            }
        },
    };
    let verdict = verdict_for(locality);
    reasons.push(match locality {
        Locality::Loopback => "This computer (loopback). Always in scope.".to_string(),
        Locality::Lab => "On your configured lab list.".to_string(),
        Locality::PrivateNetwork => "A private-network address that is not on your lab list. Only continue if you own or are authorised to test it.".to_string(),
        Locality::Public => "A public internet address. Only continue if you own it or have written permission to test it.".to_string(),
        Locality::Unspecified => "Not a specific target (0.0.0.0 / ::).".to_string(),
        Locality::Special => "A multicast, broadcast or reserved address; it cannot be a test target.".to_string(),
        Locality::Unresolvable => "Unresolvable.".to_string(),
    });
    TargetAssessment { parameter: parameter.into(), raw: t.raw.clone(), canonical: t.canonical.clone(), form, addresses, locality, verdict, reasons }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationLevel {
    /// A checkbox is enough (private network).
    Acknowledge,
    /// The user must type the target (public address).
    TypeTarget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfirmationRequirement {
    pub level: ConfirmationLevel,
    /// What the user must type for `TypeTarget`.
    pub phrase: Option<String>,
}

/// What the user explicitly confirmed. Sent with the run request and checked again by the backend.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeAck {
    #[serde(default)]
    pub acknowledged: bool,
    #[serde(default)]
    pub typed: Option<String>,
    /// Addresses the user saw when confirming. They must still match at run time.
    #[serde(default)]
    pub addresses: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeReport {
    pub targets: Vec<TargetAssessment>,
    pub verdict: Verdict,
    pub confirmation: Option<ConfirmationRequirement>,
    /// Human-readable problems when blocked.
    pub blocked_reasons: Vec<String>,
    /// Redirect hops (plain http only) with their classification, when followed.
    pub redirects: Vec<RedirectHop>,
    pub lab_targets: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedirectHop {
    pub from: String,
    pub to: String,
    pub locality: Locality,
}

impl ScopeReport {
    pub fn build(targets: Vec<TargetAssessment>, lab_targets: Vec<String>) -> Self {
        let verdict = targets.iter().map(|t| t.verdict).max().unwrap_or(Verdict::Allowed);
        let blocked_reasons = targets.iter().filter(|t| t.verdict == Verdict::Blocked).map(|t| format!("{}: {}", t.canonical, t.reasons.join(" "))).collect();
        let non_local: Vec<&TargetAssessment> = targets.iter().filter(|t| t.verdict == Verdict::NeedsConfirmation).collect();
        let confirmation = if verdict == Verdict::NeedsConfirmation {
            let public = non_local.iter().find(|t| t.locality == Locality::Public);
            Some(match public {
                Some(p) => ConfirmationRequirement { level: ConfirmationLevel::TypeTarget, phrase: Some(host_phrase(p)) },
                None => ConfirmationRequirement { level: ConfirmationLevel::Acknowledge, phrase: None },
            })
        } else {
            None
        };
        Self { targets, verdict, confirmation, blocked_reasons, redirects: vec![], lab_targets }
    }

    /// Every address the run would touch, sorted, for comparing "what was confirmed" with "what it is now".
    pub fn addresses(&self) -> Vec<String> {
        let mut v: Vec<String> = self.targets.iter().flat_map(|t| t.addresses.clone()).collect();
        v.sort();
        v.dedup();
        v
    }

    /// Enforce the confirmation rules. Blocked targets can never run, whatever was acknowledged.
    pub fn check(&self, ack: Option<&ScopeAck>) -> AppResult<()> {
        if let Some(h) = self.redirects.iter().find(|h| verdict_for(h.locality) != Verdict::Allowed && !self.redirect_confirmed(h)) {
            return Err(bad(format!("the URL redirects from {} to {} which is outside your allowed scope ({:?}); use the final address as the target if you own it", h.from, h.to, h.locality)));
        }
        match self.verdict {
            Verdict::Allowed => Ok(()),
            Verdict::Blocked => Err(bad(format!("out of scope: {}", self.blocked_reasons.join(" | ")))),
            Verdict::NeedsConfirmation => {
                let need = self.confirmation.as_ref().expect("confirmation present when needed");
                let ack = ack.filter(|a| a.acknowledged).ok_or_else(|| bad("this target is outside your lab: confirm that you are authorised to test it before running"))?;
                if need.level == ConfirmationLevel::TypeTarget {
                    let want = need.phrase.clone().unwrap_or_default();
                    let typed = ack.typed.as_deref().unwrap_or("").trim().trim_end_matches('.').to_ascii_lowercase();
                    if typed != want.to_ascii_lowercase() {
                        return Err(bad(format!("type the target exactly ({want}) to confirm you are authorised to test it")));
                    }
                }
                let mut seen = ack.addresses.clone();
                seen.sort();
                seen.dedup();
                if seen != self.addresses() {
                    return Err(bad("the addresses this target resolves to changed since you reviewed them; review and confirm again"));
                }
                Ok(())
            }
        }
    }

    fn redirect_confirmed(&self, _h: &RedirectHop) -> bool {
        false
    }
}

fn host_phrase(t: &TargetAssessment) -> String {
    // Type the host (or network) only, so URLs do not need to be retyped in full.
    if t.form == TargetForm::Url {
        let rest = t.canonical.split("://").nth(1).unwrap_or(&t.canonical);
        let auth = rest.split(['/', '?']).next().unwrap_or(rest);
        let host = if auth.starts_with('[') { auth.split(']').next().map(|h| format!("{h}]")).unwrap_or_default() } else { auth.split(':').next().unwrap_or(auth).to_string() };
        host
    } else {
        t.canonical.clone()
    }
}

#[cfg(test)]
pub mod testing {
    use super::*;
    use std::collections::HashMap;

    /// Fixed DNS answers for tests.
    #[derive(Default)]
    pub struct FakeResolver(pub HashMap<String, Result<Vec<IpAddr>, String>>);

    impl FakeResolver {
        pub fn with(mut self, name: &str, ips: &[&str]) -> Self {
            self.0.insert(name.into(), Ok(ips.iter().map(|s| s.parse().unwrap()).collect()));
            self
        }
        pub fn failing(mut self, name: &str) -> Self {
            self.0.insert(name.into(), Err("no such host".into()));
            self
        }
    }

    impl NameResolver for FakeResolver {
        fn resolve(&self, name: &str) -> Result<Vec<IpAddr>, String> {
            self.0.get(name).cloned().unwrap_or_else(|| Err("no such host".into()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::FakeResolver;
    use super::*;

    fn resolver() -> FakeResolver {
        FakeResolver::default()
            .with("localhost", &["127.0.0.1", "::1"])
            .with("juice.lab.test", &["192.168.56.10"])
            .with("scanme.example.org", &["93.184.216.34"])
            .with("sneaky.lab.test", &["192.168.56.10", "93.184.216.34"])
            .with("evil.lab.test", &["93.184.216.34"])
            .with("internal.corp", &["10.1.2.3"])
            .failing("nonexistent.invalid")
    }

    fn policy() -> ScopePolicy {
        ScopePolicy::parse(&["192.168.56.0/24".to_string(), "juice.lab.test".to_string(), "evil.lab.test".to_string(), "sneaky.lab.test".to_string()]).unwrap()
    }

    fn assess(raw: &str, form: TargetForm) -> AppResult<TargetAssessment> {
        let t = parse_target(raw, form)?;
        Ok(assess_target("target", &t, form, &policy(), &resolver()))
    }

    fn loc(raw: &str, form: TargetForm) -> Locality {
        assess(raw, form).unwrap().locality
    }

    #[test]
    fn loopback_aliases_that_are_spelled_normally_are_in_scope() {
        for t in ["127.0.0.1", "127.9.9.9", "::1", "[::1]", "localhost", "LOCALHOST", "localhost.", "::ffff:127.0.0.1", "127.0.0.0/28"] {
            let a = assess(t, TargetForm::Host).unwrap();
            assert_eq!((a.locality, a.verdict), (Locality::Loopback, Verdict::Allowed), "{t}");
        }
    }

    #[test]
    fn alternate_spellings_of_addresses_are_rejected_not_normalised() {
        for evil in [
            "127.1",
            "0x7f.0.0.1",
            "0x7f000001",
            "2130706433",
            "017700000001",
            "0177.0.0.1",
            "127.0.1",
            "127.0.0.01",
            "1.2.3",
            "0x7f.1",
            "127.0.0.1.",
            "127。0。0。1",
            "１２７.0.0.1",
            "localhost%00.evil.com",
            "127.0.0.1%25eth0",
            "fe80::1%eth0",
            "10.0.0.1-50",
            "10.0.0.1,10.0.0.2",
            "10.0.0.1;calc",
            "user@127.0.0.1",
            "127.0.0.1\\evil.com",
            "evil.com/../127.0.0.1",
            "-oN evil",
            "a b",
            "",
            "exa*mple.com",
            "ex_ample.com",
            "-bad.com",
            "bad-.com",
            "xn--",
            "例え.jp",
        ] {
            // "127.0.0.1." is the one trailing-dot spelling that is a valid FQDN literal only for names, never for IPs.
            assert!(parse_target(evil, TargetForm::Host).is_err(), "should reject {evil:?}");
        }
    }

    #[test]
    fn private_and_public_targets_need_confirmation_and_specials_are_blocked() {
        let a = assess("192.168.1.10", TargetForm::Host).unwrap();
        assert_eq!((a.locality, a.verdict), (Locality::PrivateNetwork, Verdict::NeedsConfirmation));
        for p in ["10.1.2.3", "172.16.0.1", "172.31.255.255", "169.254.1.1", "100.64.0.1", "fd00::1", "fe80::1"] {
            assert_eq!(loc(p, TargetForm::Host), Locality::PrivateNetwork, "{p}");
        }
        for p in ["8.8.8.8", "93.184.216.34", "172.32.0.1", "2001:4860:4860::8888", "100.128.0.1", "1.1.1.1/32"] {
            assert_eq!(loc(p, TargetForm::Host), Locality::Public, "{p}");
        }
        for (p, l) in [("0.0.0.0", Locality::Unspecified), ("::", Locality::Unspecified), ("224.0.0.1", Locality::Special), ("255.255.255.255", Locality::Special), ("240.0.0.1", Locality::Special), ("ff02::1", Locality::Special)] {
            let a = assess(p, TargetForm::Host).unwrap();
            assert_eq!((a.locality, a.verdict), (l, Verdict::Blocked), "{p}");
        }
    }

    #[test]
    fn addresses_that_embed_ipv4_are_judged_by_the_embedded_address() {
        assert_eq!(loc("::ffff:8.8.8.8", TargetForm::Host), Locality::Public);
        assert_eq!(loc("::ffff:10.0.0.1", TargetForm::Host), Locality::PrivateNetwork);
        assert_eq!(loc("64:ff9b::808:808", TargetForm::Host), Locality::Public); // NAT64 to 8.8.8.8
        assert_eq!(loc("64:ff9b::7f00:1", TargetForm::Host), Locality::Loopback); // NAT64 to 127.0.0.1
        assert_eq!(loc("2002:0808:0808::1", TargetForm::Host), Locality::Public); // 6to4 of 8.8.8.8
    }

    #[test]
    fn lab_allowlist_covers_ips_ranges_and_names_but_never_loopback_exceptions() {
        assert_eq!(loc("192.168.56.77", TargetForm::Host), Locality::Lab);
        assert_eq!(loc("192.168.56.0/26", TargetForm::Host), Locality::Lab);
        assert_eq!(loc("192.168.57.1", TargetForm::Host), Locality::PrivateNetwork);
        assert_eq!(loc("192.168.0.0/20", TargetForm::Host), Locality::PrivateNetwork, "a range only partly inside the lab is not a lab range");
        assert_eq!(loc("juice.lab.test", TargetForm::Host), Locality::Lab);
        assert_eq!(loc("internal.corp", TargetForm::Host), Locality::PrivateNetwork, "private but not listed");
        assert_eq!(loc("localhost", TargetForm::Host), Locality::Loopback);
    }

    #[test]
    fn a_lab_name_with_any_public_address_is_treated_as_public() {
        let a = assess("evil.lab.test", TargetForm::Host).unwrap();
        assert_eq!(a.locality, Locality::Public, "{a:?}");
        assert!(a.reasons.iter().any(|r| r.contains("treated as a public target")));
        let a = assess("sneaky.lab.test", TargetForm::Host).unwrap();
        assert_eq!((a.locality, a.verdict), (Locality::Public, Verdict::NeedsConfirmation));
        assert_eq!(a.addresses, vec!["93.184.216.34", "192.168.56.10"]);
        assert_eq!(loc("scanme.example.org", TargetForm::Host), Locality::Public);
    }

    #[test]
    fn unresolvable_names_are_blocked_because_scope_cannot_be_checked() {
        let a = assess("nonexistent.invalid", TargetForm::Host).unwrap();
        assert_eq!((a.locality, a.verdict), (Locality::Unresolvable, Verdict::Blocked));
        let a = assess("unknown-host.example", TargetForm::Host).unwrap();
        assert_eq!(a.verdict, Verdict::Blocked);
    }

    #[test]
    fn ranges_are_size_limited_and_must_be_well_formed() {
        assert!(parse_target("10.0.0.0/20", TargetForm::Host).is_ok());
        for bad in ["10.0.0.0/8", "10.0.0.0/16", "10.0.0.0/19", "0.0.0.0/0", "::/0", "10.0.0.1/24", "10.0.0.0/33", "10.0.0.0/", "10.0.0.0/abc", "fd00::/64"] {
            assert!(parse_target(bad, TargetForm::Host).is_err(), "{bad}");
        }
        assert!(parse_target("fd00::/116", TargetForm::Host).is_ok());
        assert!(parse_target("10.0.0.0/24", TargetForm::Ip).is_err(), "an IP field takes a single address");
        assert!(parse_target("example.com", TargetForm::Ip).is_err());
    }

    #[test]
    fn urls_are_parsed_strictly_and_classified_by_their_real_host() {
        let ok = |u: &str| parse_target(u, TargetForm::Url).unwrap();
        assert_eq!(ok("http://127.0.0.1:18080/search?q=1").canonical, "http://127.0.0.1:18080/search?q=1");
        assert_eq!(ok("HTTP://LocalHost:80/").canonical, "http://localhost/");
        assert_eq!(ok("https://[::1]:8443/x").canonical, "https://[::1]:8443/x");
        for evil in [
            "http://127.0.0.1@evil.com/",
            "http://evil.com@127.0.0.1/",
            "http://user:pass@127.0.0.1/",
            "http://127.0.0.1\\@evil.com/",
            "http://127.1/",
            "http://0x7f.0.0.1/",
            "http://2130706433/",
            "http://017700000001/",
            "http://exa mple.com/",
            "http://evil.com/\nfoo",
            "ftp://127.0.0.1/",
            "file:///C:/Windows/win.ini",
            "javascript:alert(1)",
            "//127.0.0.1/",
            "127.0.0.1",
            "http://",
            "http://例え.jp/",
            "http://%31%32%37.0.0.1/",
        ] {
            assert!(parse_target(evil, TargetForm::Url).is_err(), "should reject {evil:?}");
        }
        // Classification follows the host the URL really points to, not decoy text in the path or query.
        assert_eq!(loc("http://scanme.example.org/127.0.0.1", TargetForm::Url), Locality::Public);
        assert_eq!(loc("http://scanme.example.org/?next=http://127.0.0.1", TargetForm::Url), Locality::Public);
        assert_eq!(loc("http://127.0.0.1/?host=scanme.example.org", TargetForm::Url), Locality::Loopback);
        assert_eq!(loc("http://[::ffff:8.8.8.8]/", TargetForm::Url), Locality::Public);
        // A host name that merely starts with an address is a name, resolved and classified like any other.
        let t = parse_target("http://127.0.0.1.evil.com./x", TargetForm::Url).unwrap();
        assert_eq!(t.endpoint, Endpoint::Name("127.0.0.1.evil.com".into()));
    }

    #[test]
    fn report_requires_the_right_confirmation_and_blocks_what_can_never_run() {
        let rep = |targets: &[(&str, TargetForm)]| {
            let a = targets.iter().map(|(t, f)| assess_target("target", &parse_target(t, *f).unwrap(), *f, &policy(), &resolver())).collect();
            ScopeReport::build(a, vec![])
        };
        // Local: no confirmation.
        let r = rep(&[("127.0.0.1", TargetForm::Host)]);
        assert_eq!((r.verdict, r.confirmation.clone()), (Verdict::Allowed, None));
        assert!(r.check(None).is_ok());
        // Private: acknowledge, and the addresses seen must match.
        let r = rep(&[("192.168.1.10", TargetForm::Host)]);
        assert_eq!(r.confirmation.as_ref().unwrap().level, ConfirmationLevel::Acknowledge);
        assert!(r.check(None).is_err());
        assert!(r.check(Some(&ScopeAck { acknowledged: false, ..Default::default() })).is_err());
        let ok = ScopeAck { acknowledged: true, typed: None, addresses: vec!["192.168.1.10".into()] };
        assert!(r.check(Some(&ok)).is_ok());
        let stale = ScopeAck { addresses: vec!["192.168.1.99".into()], ..ok.clone() };
        assert!(r.check(Some(&stale)).unwrap_err().to_string().contains("changed"));
        // Public: the target must be typed.
        let r = rep(&[("scanme.example.org", TargetForm::Host)]);
        let c = r.confirmation.clone().unwrap();
        assert_eq!((c.level, c.phrase.as_deref()), (ConfirmationLevel::TypeTarget, Some("scanme.example.org")));
        let mut ack = ScopeAck { acknowledged: true, typed: Some("nope".into()), addresses: vec!["93.184.216.34".into()] };
        assert!(r.check(Some(&ack)).unwrap_err().to_string().contains("type the target exactly"));
        ack.typed = Some("Scanme.Example.Org".into());
        assert!(r.check(Some(&ack)).is_ok());
        // Public URL: type the host, not the whole URL.
        let r = rep(&[("http://scanme.example.org/login?x=1", TargetForm::Url)]);
        assert_eq!(r.confirmation.unwrap().phrase.as_deref(), Some("scanme.example.org"));
        // A blocked target can never be confirmed away; mixing local and blocked is blocked.
        let r = rep(&[("127.0.0.1", TargetForm::Host), ("224.0.0.1", TargetForm::Host)]);
        assert_eq!(r.verdict, Verdict::Blocked);
        assert!(r.check(Some(&ScopeAck { acknowledged: true, typed: Some("224.0.0.1".into()), addresses: r.addresses() })).is_err());
        // A local target next to a public one still needs the public confirmation.
        let r = rep(&[("127.0.0.1", TargetForm::Host), ("scanme.example.org", TargetForm::Host)]);
        assert_eq!(r.verdict, Verdict::NeedsConfirmation);
    }

    #[test]
    fn redirects_outside_the_allowed_scope_block_the_run() {
        let a = assess_target("url", &parse_target("http://127.0.0.1:18080/redirect", TargetForm::Url).unwrap(), TargetForm::Url, &policy(), &resolver());
        let mut r = ScopeReport::build(vec![a], vec![]);
        assert!(r.check(None).is_ok());
        r.redirects = vec![RedirectHop { from: "http://127.0.0.1:18080/redirect".into(), to: "http://127.0.0.1:18080/home".into(), locality: Locality::Loopback }];
        assert!(r.check(None).is_ok());
        r.redirects.push(RedirectHop { from: "http://127.0.0.1:18080/home".into(), to: "http://93.184.216.34/".into(), locality: Locality::Public });
        assert!(r.check(None).unwrap_err().to_string().contains("redirects"));
    }

    #[test]
    fn policy_entries_are_validated_with_the_same_strict_parser() {
        assert!(ScopePolicy::parse(&["192.168.56.0/24".into(), "lab.example.test".into(), "10.0.0.5".into()]).is_ok());
        for bad in [vec!["127.1".to_string()], vec!["10.0.0.0/8".into()], vec!["bad host".into()], vec!["*.corp".into()], vec!["http://x".into()]] {
            assert!(ScopePolicy::parse(&bad).is_err(), "{bad:?}");
        }
        assert!(ScopePolicy::parse(&vec!["a.test".to_string(); 65]).is_err());
    }
}


