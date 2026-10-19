//! Small network helpers for scope checking: the OS name resolver, and a single `HEAD` request used to
//! learn where a plain-http URL redirects. Nothing here follows a redirect by itself.

use crate::domain::scope::NameResolver;
use crate::domain::{AppError, AppResult};
use crate::ports::RedirectProbe;
use async_trait::async_trait;
use std::net::{IpAddr, ToSocketAddrs};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use url::Url;

#[derive(Default)]
pub struct SystemNameResolver;

impl NameResolver for SystemNameResolver {
    fn resolve(&self, name: &str) -> Result<Vec<IpAddr>, String> {
        (name, 0u16).to_socket_addrs().map(|it| it.map(|s| s.ip()).collect()).map_err(|e| e.to_string())
    }
}

#[derive(Default)]
pub struct HttpRedirectProbe;

const TIMEOUT: Duration = Duration::from_secs(3);
const MAX_HEADER_BYTES: usize = 8 * 1024;

/// Pull the redirect target out of raw response headers. Pure, for testing.
pub fn parse_redirect(head: &str, base: &Url) -> Option<String> {
    let mut lines = head.split("\r\n");
    let status_line = lines.next()?;
    let code: u16 = status_line.split_whitespace().nth(1)?.parse().ok()?;
    if !matches!(code, 301 | 302 | 303 | 307 | 308) {
        return None;
    }
    let loc = lines.filter_map(|l| l.split_once(':')).find(|(k, _)| k.eq_ignore_ascii_case("location")).map(|(_, v)| v.trim().to_string())?;
    if loc.is_empty() || loc.len() > 2000 || loc.chars().any(|c| c.is_control()) {
        return None;
    }
    // Relative locations are resolved against the URL that answered.
    base.join(&loc).ok().map(|u| u.to_string()).or(Some(loc))
}

#[async_trait]
impl RedirectProbe for HttpRedirectProbe {
    async fn head(&self, url: &str) -> AppResult<Option<String>> {
        let parsed = Url::parse(url).map_err(|e| AppError::Validation(e.to_string()))?;
        if parsed.scheme() != "http" {
            return Ok(None); // https redirects cannot be checked without a TLS client; the UI says so
        }
        let host = parsed.host_str().ok_or_else(|| AppError::Validation("the URL has no host".into()))?.trim_matches(['[', ']']).to_string();
        let port = parsed.port_or_known_default().unwrap_or(80);
        let mut path = parsed.path().to_string();
        if let Some(q) = parsed.query() {
            path.push('?');
            path.push_str(q);
        }
        let result = tokio::time::timeout(TIMEOUT, async {
            let mut s = TcpStream::connect((host.as_str(), port)).await.ok()?;
            let req = format!("HEAD {path} HTTP/1.1\r\nHost: {host}:{port}\r\nUser-Agent: Vixalune-scope-check\r\nAccept: */*\r\nConnection: close\r\n\r\n");
            s.write_all(req.as_bytes()).await.ok()?;
            let mut buf = Vec::new();
            let mut chunk = [0u8; 1024];
            while buf.len() < MAX_HEADER_BYTES && !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = s.read(&mut chunk).await.ok()?;
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            Some(String::from_utf8_lossy(&buf).to_string())
        })
        .await;
        Ok(match result {
            Ok(Some(head)) => parse_redirect(&head, &parsed),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_redirects_and_resolves_relative_locations() {
        let base = Url::parse("http://127.0.0.1:18080/a/b").unwrap();
        let r = |h: &str| parse_redirect(h, &base);
        assert_eq!(r("HTTP/1.1 302 Found\r\nLocation: http://93.184.216.34/x\r\n\r\n").as_deref(), Some("http://93.184.216.34/x"));
        assert_eq!(r("HTTP/1.1 301 Moved\r\nlocation: /home\r\n\r\n").as_deref(), Some("http://127.0.0.1:18080/home"));
        assert_eq!(r("HTTP/1.1 308 Permanent\r\nLocation: //evil.example/x\r\n\r\n").as_deref(), Some("http://evil.example/x"), "protocol-relative targets are resolved, not ignored");
        assert_eq!(r("HTTP/1.1 200 OK\r\nLocation: http://evil/\r\n\r\n"), None, "only redirect statuses count");
        assert_eq!(r("HTTP/1.1 302 Found\r\n\r\n"), None);
        assert_eq!(r("garbage"), None);
    }

    #[test]
    fn the_system_resolver_answers_for_localhost_and_fails_for_nonsense() {
        let r = SystemNameResolver;
        let ips = r.resolve("localhost").unwrap();
        assert!(ips.iter().any(|i| i.is_loopback()));
        assert!(r.resolve("definitely-not-a-real-host.invalid").is_err());
    }
}
