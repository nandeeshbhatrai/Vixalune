//! HTTP plumbing shared by every provider adapter: one client configuration, request sending with timeout and cancellation,
//! error classification, and the streaming loop. Nothing here knows about a particular provider's JSON.

use super::sse::{SseEvent, SseParser};
use crate::domain::ai::{AiError, AiErrorKind, AiResult, MAX_ERROR_BODY_BYTES};
use crate::domain::process::CancellationToken;
use crate::domain::secret::Secret;
use futures_util::StreamExt;
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// One client shape for everything: no redirects (a redirect could carry a key header to another host), no cookies, a plain
/// user agent, the system proxy and certificate store.
pub fn build_client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(CONNECT_TIMEOUT)
        .user_agent(concat!("Vixalune/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("static client configuration")
}

/// `https` always; plain `http` only to this computer (used by tests and local gateways).
pub fn check_base_url(base: &str) -> AiResult<()> {
    let u = url::Url::parse(base).map_err(|_| AiError::new(AiErrorKind::Configuration, "the provider address is not a valid URL"))?;
    let local = matches!(u.host_str(), Some("127.0.0.1") | Some("localhost") | Some("[::1]"));
    match u.scheme() {
        "https" => Ok(()),
        "http" if local => Ok(()),
        _ => Err(AiError::new(AiErrorKind::Configuration, "provider addresses must use https")),
    }
}

pub async fn cancelled(c: &CancellationToken) {
    while !c.is_cancelled() {
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
}

fn map_transport(e: reqwest::Error, secret: &Secret) -> AiError {
    let e = e.without_url();
    if e.is_timeout() {
        AiError::new(AiErrorKind::Timeout, "the provider did not answer in time")
    } else if e.is_connect() {
        AiError::with_secret(AiErrorKind::Network, format!("could not connect to the provider ({e})"), secret)
    } else if e.is_redirect() {
        AiError::new(AiErrorKind::Protocol, "the provider tried to redirect the request; Vixalune does not follow redirects with a key")
    } else {
        AiError::with_secret(AiErrorKind::Network, format!("network error ({e})"), secret)
    }
}

pub fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?.trim().parse::<u64>().ok().map(|s| s.min(3600))
}

/// How a provider turns an error response into a kind and a message. The default handles the status code alone.
pub type ErrorParser = fn(status: u16, body: &str) -> (AiErrorKind, String);

pub fn kind_from_status(status: u16) -> AiErrorKind {
    match status {
        401 => AiErrorKind::Authentication,
        402 => AiErrorKind::Quota,
        403 => AiErrorKind::Permission,
        404 => AiErrorKind::ModelNotFound,
        408 => AiErrorKind::Timeout,
        413 => AiErrorKind::TooLarge,
        429 => AiErrorKind::RateLimit,
        529 => AiErrorKind::Overloaded,
        500..=599 => AiErrorKind::Server,
        _ => AiErrorKind::InvalidRequest,
    }
}

pub fn default_error_parser(status: u16, body: &str) -> (AiErrorKind, String) {
    (kind_from_status(status), body.to_string())
}

async fn error_from_response(resp: reqwest::Response, secret: &Secret, parser: ErrorParser) -> AiError {
    let status = resp.status().as_u16();
    let ra = retry_after(resp.headers());
    let mut body = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(Ok(chunk)) = stream.next().await {
        body.extend_from_slice(&chunk);
        if body.len() >= MAX_ERROR_BODY_BYTES {
            break;
        }
    }
    body.truncate(MAX_ERROR_BODY_BYTES);
    let text = String::from_utf8_lossy(&body).to_string();
    let (kind, message) = parser(status, &text);
    let message = if message.trim().is_empty() { format!("the provider answered with status {status}") } else { format!("{message} (status {status})") };
    AiError::with_secret(kind, message, secret).status(status).retry_after(ra)
}

/// Send a request and return the response if the status is a success. Honours cancellation and a time limit for the response
/// headers; error statuses become classified errors.
pub async fn send(req: reqwest::RequestBuilder, secret: &Secret, cancel: &CancellationToken, limit: Duration, parser: ErrorParser) -> AiResult<reqwest::Response> {
    if cancel.is_cancelled() {
        return Err(AiError::cancelled());
    }
    let sent = tokio::select! {
        biased;
        _ = cancelled(cancel) => return Err(AiError::cancelled()),
        r = tokio::time::timeout(limit, req.send()) => r,
    };
    let resp = match sent {
        Err(_) => return Err(AiError::new(AiErrorKind::Timeout, "the provider did not answer in time")),
        Ok(Err(e)) => return Err(map_transport(e, secret)),
        Ok(Ok(r)) => r,
    };
    if resp.status().is_success() {
        Ok(resp)
    } else {
        Err(error_from_response(resp, secret, parser).await)
    }
}

/// Read a (small) JSON body with a size bound.
pub async fn read_json(resp: reqwest::Response, secret: &Secret, cancel: &CancellationToken, limit: Duration, max_bytes: usize) -> AiResult<serde_json::Value> {
    let mut body = Vec::new();
    let mut stream = resp.bytes_stream();
    loop {
        let next = tokio::select! {
            biased;
            _ = cancelled(cancel) => return Err(AiError::cancelled()),
            r = tokio::time::timeout(limit, stream.next()) => r,
        };
        match next {
            Err(_) => return Err(AiError::new(AiErrorKind::Timeout, "the provider stopped answering")),
            Ok(None) => break,
            Ok(Some(Err(e))) => return Err(map_transport(e, secret)),
            Ok(Some(Ok(chunk))) => {
                body.extend_from_slice(&chunk);
                if body.len() > max_bytes {
                    return Err(AiError::new(AiErrorKind::TooLarge, "the provider's answer is larger than Vixalune allows"));
                }
            }
        }
    }
    serde_json::from_slice(&body).map_err(|_| AiError::new(AiErrorKind::Protocol, "the provider's answer was not valid JSON"))
}

#[derive(Debug, PartialEq, Eq)]
pub enum StreamEnd {
    /// The handler said the response is complete.
    Done,
    /// The server closed the stream without the handler reporting completion.
    Closed,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    Continue,
    Done,
}

/// Read a server-sent-event response, handing each event to `handle`. Fails with a network error if the connection breaks and
/// with a timeout if nothing arrives for `idle`. Cancellation is checked continuously.
pub async fn stream_sse(
    resp: reqwest::Response,
    secret: &Secret,
    cancel: &CancellationToken,
    idle: Duration,
    mut handle: impl FnMut(SseEvent) -> AiResult<Step>,
) -> AiResult<StreamEnd> {
    let mut parser = SseParser::new();
    let mut stream = resp.bytes_stream();
    loop {
        let next = tokio::select! {
            biased;
            _ = cancelled(cancel) => return Err(AiError::cancelled()),
            r = tokio::time::timeout(idle, stream.next()) => r,
        };
        let chunk = match next {
            Err(_) => return Err(AiError::new(AiErrorKind::Timeout, "the provider stopped sending the response")),
            Ok(None) => {
                if let Some(ev) = parser.finish()? {
                    if handle(ev)? == Step::Done {
                        return Ok(StreamEnd::Done);
                    }
                }
                return Ok(StreamEnd::Closed);
            }
            Ok(Some(Err(e))) => return Err(map_transport(e, secret)),
            Ok(Some(Ok(c))) => c,
        };
        for ev in parser.feed(&chunk)? {
            if handle(ev)? == Step::Done {
                return Ok(StreamEnd::Done);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_addresses_must_be_https_or_this_computer() {
        for ok in ["https://api.openai.com/v1", "http://127.0.0.1:8080", "http://localhost:1/x"] {
            assert!(check_base_url(ok).is_ok(), "{ok}");
        }
        for bad in ["http://api.openai.com", "ftp://x", "api.openai.com", "http://192.168.1.5", "file:///x", "http://127.0.0.1.evil.test"] {
            assert!(check_base_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn statuses_map_to_the_right_kinds() {
        use AiErrorKind::*;
        for (s, k) in [(401, Authentication), (402, Quota), (403, Permission), (404, ModelNotFound), (408, Timeout), (413, TooLarge), (429, RateLimit), (500, Server), (503, Server), (529, Overloaded), (400, InvalidRequest), (422, InvalidRequest)] {
            assert_eq!(kind_from_status(s), k, "{s}");
        }
    }
}
