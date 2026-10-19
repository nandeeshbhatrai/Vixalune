//! A tiny HTTP server on 127.0.0.1 for tests. The adapters talk to it with the real HTTP client, so request building, headers,
//! streaming, timeouts, cancellation and error handling are exercised for real; only the provider is pretend.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Debug, Clone)]
pub struct Captured {
    pub method: String,
    /// Path and query, as sent.
    pub target: String,
    pub headers: HashMap<String, String>,
    pub body: String,
}

impl Captured {
    pub fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or("")
    }
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(&name.to_ascii_lowercase()).map(|s| s.as_str())
    }
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).unwrap_or(serde_json::Value::Null)
    }
}

#[derive(Clone)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    /// Body pieces, each sent after its delay.
    pub chunks: Vec<(Duration, Vec<u8>)>,
    /// Announce more bytes than are sent, then hang up: a connection that breaks mid-response.
    pub break_connection: bool,
}

impl Reply {
    pub fn json(status: u16, body: &str) -> Self {
        Self { status, headers: vec![("Content-Type".into(), "application/json".into())], chunks: vec![(Duration::ZERO, body.as_bytes().to_vec())], break_connection: false }
    }

    pub fn sse(parts: &[&str]) -> Self {
        Self { status: 200, headers: vec![("Content-Type".into(), "text/event-stream".into())], chunks: parts.iter().map(|p| (Duration::from_millis(5), p.as_bytes().to_vec())).collect(), break_connection: false }
    }

    pub fn header(mut self, k: &str, v: &str) -> Self {
        self.headers.push((k.into(), v.into()));
        self
    }

    pub fn delayed_chunks(mut self, chunks: Vec<(Duration, Vec<u8>)>) -> Self {
        self.chunks = chunks;
        self
    }

    pub fn broken(mut self) -> Self {
        self.break_connection = true;
        self
    }
}

pub struct MockServer {
    pub addr: std::net::SocketAddr,
    pub requests: Arc<Mutex<Vec<Captured>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl MockServer {
    pub fn base(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn captured(&self) -> Vec<Captured> {
        self.requests.lock().unwrap().clone()
    }

    pub async fn start(handler: impl Fn(&Captured) -> Reply + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let requests: Arc<Mutex<Vec<Captured>>> = Arc::default();
        let handler = Arc::new(handler);
        let log = requests.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let handler = handler.clone();
                let log = log.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    let head_end = loop {
                        let n = sock.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            break p + 4;
                        }
                    };
                    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                    let mut lines = head.split("\r\n");
                    let mut first = lines.next().unwrap_or("").split_whitespace();
                    let (method, target) = (first.next().unwrap_or("").to_string(), first.next().unwrap_or("").to_string());
                    let headers: HashMap<String, String> = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string())).collect();
                    let want: usize = headers.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
                    while buf.len() < head_end + want {
                        let n = sock.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                    }
                    let body = String::from_utf8_lossy(&buf[head_end..]).to_string();
                    let cap = Captured { method, target, headers, body };
                    let reply = handler(&cap);
                    log.lock().unwrap().push(cap);

                    let total: usize = reply.chunks.iter().map(|(_, c)| c.len()).sum();
                    let mut out = format!("HTTP/1.1 {} X\r\nConnection: close\r\n", reply.status);
                    for (k, v) in &reply.headers {
                        out.push_str(&format!("{k}: {v}\r\n"));
                    }
                    // A known length for ordinary replies; for streams the connection close ends the body.
                    let is_stream = reply.headers.iter().any(|(_, v)| v.contains("event-stream"));
                    if reply.break_connection {
                        out.push_str(&format!("Content-Length: {}\r\n", total + 1000));
                    } else if !is_stream {
                        out.push_str(&format!("Content-Length: {total}\r\n"));
                    }
                    out.push_str("\r\n");
                    if sock.write_all(out.as_bytes()).await.is_err() {
                        return;
                    }
                    for (delay, bytes) in reply.chunks {
                        if !delay.is_zero() {
                            tokio::time::sleep(delay).await;
                        }
                        if sock.write_all(&bytes).await.is_err() {
                            return;
                        }
                        let _ = sock.flush().await;
                    }
                    if reply.break_connection {
                        return; // drop: fewer bytes than announced
                    }
                    let _ = sock.shutdown().await;
                });
            }
        });
        Self { addr, requests, task }
    }
}
