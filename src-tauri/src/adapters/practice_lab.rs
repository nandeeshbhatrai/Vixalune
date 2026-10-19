//! Learning labs that ship with Vixalune.
//!
//! 1. **Practice web app**: a tiny, *deliberately vulnerable* HTTP server bound to `127.0.0.1` only. It serves
//!    fixed, embedded pages (it never reads files from disk, never executes input, and never contacts other
//!    hosts) so scanners such as Nmap and ZAP have something realistic and safe to find.
//! 2. **Sample code project**: a folder of small, intentionally insecure files for Semgrep and Trivy exercises.
//!    The files are never executed. Secret-looking values are assembled at run time so this repository never
//!    contains a complete key.
//!
//! The deliberate weaknesses are listed in [`WEAKNESSES`] and explained in the tutorials.

use crate::domain::lab::{LabDefinition, LabKind, LabState};
use crate::domain::{AppError, AppResult, LabId};
use crate::ports::{LabInfo, LabManager};
use async_trait::async_trait;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, Semaphore};

pub const DEFAULT_PORT: u16 = 18080;
pub const WEB_LAB_ID: &str = "practice-web";
pub const SAMPLE_LAB_ID: &str = "sample-code";

pub const WEAKNESSES: &[&str] = &[
    "No security headers (no Content-Security-Policy, X-Frame-Options, X-Content-Type-Options or HSTS).",
    "The session cookie has no HttpOnly, Secure or SameSite attributes.",
    "/search reflects the 'q' parameter into the page without escaping (reflected cross-site scripting).",
    "/files/ shows a directory listing that includes a backup file.",
    "/admin/ has no login.",
    "/robots.txt advertises the sensitive paths.",
    "/redirect?to=... sends the browser anywhere (open redirect).",
    "The Server header names the software and version.",
];

const MAX_REQUEST: usize = 8 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_CONNECTIONS: usize = 16;

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < b.len() => {
                if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    out.push(v);
                    i += 3;
                } else {
                    out.push(b'%');
                    i += 1;
                }
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

fn query_param(query: &str, key: &str) -> Option<String> {
    query.split('&').filter_map(|kv| kv.split_once('=')).find(|(k, _)| *k == key).map(|(_, v)| percent_decode(v))
}

struct Response {
    status: &'static str,
    content_type: &'static str,
    extra: Vec<(&'static str, String)>,
    body: String,
}

fn page(title: &str, body: &str) -> String {
    format!("<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title></head><body><h1>{title}</h1>{body}<hr><small>Vixalune practice application. Intentionally vulnerable. Local use only.</small></body></html>")
}

/// Pure request router: `(method, target)` to a response. Everything it can return is fixed text.
fn route(method: &str, target: &str) -> Response {
    let html = |status: &'static str, body: String| Response { status, content_type: "text/html; charset=utf-8", extra: vec![], body };
    if method != "GET" && method != "HEAD" {
        return html("405 Method Not Allowed", page("405", "<p>Only GET is supported.</p>"));
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    match path {
        "/" => html("200 OK", page("Vixalune Practice Shop", "<p>Welcome. Try the <a href=\"/search?q=widgets\">search</a>, the <a href=\"/files/\">files</a> or the <a href=\"/admin/\">admin area</a>.</p>")),
        "/search" => {
            let q = query_param(query, "q").unwrap_or_default();
            let q: String = q.chars().take(200).collect();
            // DELIBERATE: the value is placed in the page without escaping (reflected XSS practice).
            html("200 OK", page("Search", &format!("<p>Results for: {q}</p><p>No products found.</p>")))
        }
        "/files/" => html("200 OK", page("Index of /files/", "<ul><li><a href=\"/files/notes.txt\">notes.txt</a></li><li><a href=\"/files/backup-2024.zip.txt\">backup-2024.zip.txt</a></li></ul>")),
        "/files/notes.txt" => Response { status: "200 OK", content_type: "text/plain; charset=utf-8", extra: vec![], body: "Remember to rotate the practice passwords. (fake data)\n".into() },
        "/files/backup-2024.zip.txt" => Response { status: "200 OK", content_type: "text/plain; charset=utf-8", extra: vec![], body: "This stands in for a forgotten backup file. (fake data)\n".into() },
        "/admin/" | "/admin" => html("200 OK", page("Admin panel (demo)", "<p>No login required. In a real application this would be a serious problem.</p>")),
        "/robots.txt" => Response { status: "200 OK", content_type: "text/plain; charset=utf-8", extra: vec![], body: "User-agent: *\nDisallow: /admin/\nDisallow: /files/\n".into() },
        "/redirect" => {
            let to = query_param(query, "to").unwrap_or_default();
            // The lab is vulnerable to open redirects, but never to header injection.
            let ok = !to.is_empty() && to.len() <= 500 && !to.chars().any(|c| c.is_control()) && (to.starts_with("http://") || to.starts_with("https://") || to.starts_with('/'));
            if ok {
                Response { status: "302 Found", content_type: "text/html; charset=utf-8", extra: vec![("Location", to)], body: String::new() }
            } else {
                html("400 Bad Request", page("400", "<p>Give a valid ?to= address.</p>"))
            }
        }
        _ => html("404 Not Found", page("Not found", &format!("<p>No page at {}.</p>", path.chars().take(100).filter(|c| c.is_ascii_graphic()).collect::<String>().replace('<', "&lt;")))),
    }
}

fn serialize(method: &str, r: Response) -> Vec<u8> {
    let mut head = format!("HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\n", r.status, r.content_type, r.body.len());
    // DELIBERATE: verbose banner and a cookie without protective attributes; no security headers at all.
    head.push_str("Server: VixaluneLab/0.1 (intentionally vulnerable practice app)\r\nSet-Cookie: session=practice-demo; Path=/\r\nConnection: close\r\n");
    for (k, v) in &r.extra {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    let mut out = head.into_bytes();
    if method != "HEAD" {
        out.extend_from_slice(r.body.as_bytes());
    }
    out
}

async fn handle(mut s: TcpStream) {
    let work = async {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 1024];
        while buf.len() < MAX_REQUEST && !buf.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = s.read(&mut chunk).await.ok()?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        let text = String::from_utf8_lossy(&buf);
        let line = text.lines().next()?;
        let mut parts = line.split_whitespace();
        let (method, target) = (parts.next()?.to_string(), parts.next()?.to_string());
        if !target.starts_with('/') || target.len() > 2000 {
            return None;
        }
        let bytes = serialize(&method, route(&method, &target));
        s.write_all(&bytes).await.ok()?;
        let _ = s.shutdown().await;
        Some(())
    };
    let _ = tokio::time::timeout(IO_TIMEOUT, work).await;
}

struct WebServer {
    addr: SocketAddr,
    stop: Option<oneshot::Sender<()>>,
}

async fn serve(listener: TcpListener, mut stop: oneshot::Receiver<()>) {
    let permits = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    loop {
        tokio::select! {
            _ = &mut stop => break,
            accepted = listener.accept() => {
                let Ok((stream, peer)) = accepted else { continue };
                // Belt and braces: the listener is bound to loopback, but never answer anything else.
                if !peer.ip().is_loopback() {
                    continue;
                }
                let Ok(permit) = permits.clone().try_acquire_owned() else { continue };
                tokio::spawn(async move {
                    handle(stream).await;
                    drop(permit);
                });
            }
        }
    }
}

pub struct PracticeLabs {
    sample_dir: PathBuf,
    port: u16,
    web: Mutex<Option<WebServer>>,
}

/// The sample project files. `(relative path, content)`.
fn sample_files() -> Vec<(&'static str, String)> {
    vec![
        (
            "README.md",
            "# Vixalune sample project (INTENTIONALLY INSECURE)\n\nThis folder exists for code-analysis practice with Semgrep and Trivy.\nNothing here is meant to be run, and every weakness is on purpose.\n\n- `app.py`: three insecure code patterns Semgrep rules can find.\n- `requirements.txt` and `package-lock.json`: old dependency versions with known public vulnerabilities (Trivy).\n- `config.env`: a fake, non-working credential (Trivy secret scanning).\n- `.semgrep.yml`: the practice rules used by the Semgrep exercise.\n".to_string(),
        ),
        (
            "app.py",
            "# INTENTIONALLY INSECURE PRACTICE CODE. Never run this.\nimport subprocess\n\nPASSWORD = \"hunter2-practice\"  # 1. a hard-coded password\n\n\ndef calculate(user_input):\n    return eval(user_input)  # 2. eval() on text a user controls\n\n\ndef ping(host):\n    return subprocess.call(\"ping \" + host, shell=True)  # 3. a shell command built from input\n".to_string(),
        ),
        ("requirements.txt", "flask==0.12.2\nrequests==2.19.1\njinja2==2.10\n".to_string()),
        (
            "package-lock.json",
            "{\n  \"name\": \"vixalune-sample\",\n  \"version\": \"1.0.0\",\n  \"lockfileVersion\": 3,\n  \"requires\": true,\n  \"packages\": {\n    \"\": {\n      \"name\": \"vixalune-sample\",\n      \"version\": \"1.0.0\",\n      \"dependencies\": {\n        \"lodash\": \"4.17.15\"\n      }\n    },\n    \"node_modules/lodash\": {\n      \"version\": \"4.17.15\",\n      \"resolved\": \"https://registry.npmjs.org/lodash/-/lodash-4.17.15.tgz\"\n    }\n  }\n}\n".to_string(),
        ),
        // Assembled at run time: the repository never contains a complete key. This is AWS's documented
        // example value; it is not a working credential.
        ("config.env", format!("# fake credential for secret-scanning practice\nAWS_ACCESS_KEY_ID={}{}\n", "AKIA", "IOSFODNN7EXAMPLE")),
        (
            ".semgrep.yml",
            "rules:\n  - id: practice-eval-of-input\n    languages: [python]\n    severity: ERROR\n    message: eval() runs whatever text it is given. If a user controls that text, they control your program.\n    pattern: eval(...)\n  - id: practice-shell-true\n    languages: [python]\n    severity: WARNING\n    message: Building a shell command from input lets the input add commands. Pass a list of arguments and drop shell=True.\n    pattern: subprocess.call(..., shell=True)\n  - id: practice-hardcoded-password\n    languages: [generic]\n    severity: ERROR\n    message: A password is written in the source code. Load secrets from the environment or a secret store.\n    paths:\n      include: [\"*.py\"]\n    pattern-regex: PASSWORD\\s*=\\s*\".+\"\n".to_string(),
        ),
    ]
}

impl PracticeLabs {
    pub fn new(labs_dir: &Path, port: u16) -> Self {
        Self { sample_dir: labs_dir.join(SAMPLE_LAB_ID), port, web: Mutex::new(None) }
    }

    pub fn sample_dir(&self) -> &Path {
        &self.sample_dir
    }

    /// Create (or refresh) the sample project. Only Vixalune's own files are written.
    pub fn ensure_sample_project(&self) -> AppResult<()> {
        std::fs::create_dir_all(&self.sample_dir)?;
        for (name, content) in sample_files() {
            let path = self.sample_dir.join(name);
            if std::fs::read_to_string(&path).map(|c| c != content).unwrap_or(true) {
                std::fs::write(&path, content)?;
            }
        }
        Ok(())
    }

    fn web_address(&self) -> Option<String> {
        self.web.lock().unwrap().as_ref().map(|w| format!("http://127.0.0.1:{}/", w.addr.port()))
    }

    /// Bound address when running (tests use port 0 and need the real port).
    pub fn bound_addr(&self) -> Option<SocketAddr> {
        self.web.lock().unwrap().as_ref().map(|w| w.addr)
    }
}

fn lab_id(s: &str) -> LabId {
    LabId::new(s).expect("static slug")
}

#[async_trait]
impl LabManager for PracticeLabs {
    fn list(&self) -> Vec<LabDefinition> {
        vec![
            LabDefinition {
                id: lab_id(WEB_LAB_ID),
                name: "Practice web application".into(),
                description: "A small shop website with deliberate, harmless weaknesses. It runs only on this computer (127.0.0.1) and serves fixed pages.".into(),
                kind: LabKind::WebApp,
                address: self.web_address().or_else(|| Some(format!("http://127.0.0.1:{}/ (when started)", self.port))),
                folder: None,
                weaknesses: WEAKNESSES.iter().map(|s| s.to_string()).collect(),
                warning: "Practise only on this lab or on systems you own or have written permission to test.".into(),
            },
            LabDefinition {
                id: lab_id(SAMPLE_LAB_ID),
                name: "Sample code project".into(),
                description: "Insecure sample code and out-of-date dependency lists for Semgrep and Trivy practice. The files are never executed.".into(),
                kind: LabKind::SampleProject,
                address: None,
                folder: Some(self.sample_dir.display().to_string()),
                weaknesses: vec![
                    "Hard-coded password, eval() on user input and a shell command built from input (app.py).".into(),
                    "Old Flask, Requests, Jinja2 and lodash versions with known vulnerabilities.".into(),
                    "A fake access key in config.env.".into(),
                ],
                warning: "Never run these files. They are intentionally unsafe examples.".into(),
            },
        ]
    }

    fn state(&self, id: &LabId) -> AppResult<LabState> {
        match id.as_str() {
            WEB_LAB_ID => Ok(if self.web.lock().unwrap().is_some() { LabState::Running } else { LabState::Stopped }),
            SAMPLE_LAB_ID => Ok(if self.sample_dir.join("app.py").exists() { LabState::Running } else { LabState::Stopped }),
            other => Err(AppError::NotFound(format!("lab '{other}'"))),
        }
    }

    async fn start(&self, id: &LabId) -> AppResult<()> {
        match id.as_str() {
            WEB_LAB_ID => {
                if self.web.lock().unwrap().is_some() {
                    return Ok(());
                }
                let listener = TcpListener::bind(("127.0.0.1", self.port))
                    .await
                    .map_err(|e| AppError::Io(format!("the practice application could not start on 127.0.0.1:{} ({e}). Another program may be using that port.", self.port)))?;
                let addr = listener.local_addr()?;
                let (tx, rx) = oneshot::channel();
                tokio::spawn(serve(listener, rx));
                *self.web.lock().unwrap() = Some(WebServer { addr, stop: Some(tx) });
                Ok(())
            }
            SAMPLE_LAB_ID => self.ensure_sample_project(),
            other => Err(AppError::NotFound(format!("lab '{other}'"))),
        }
    }

    async fn stop(&self, id: &LabId) -> AppResult<()> {
        match id.as_str() {
            WEB_LAB_ID => {
                if let Some(mut w) = self.web.lock().unwrap().take() {
                    if let Some(tx) = w.stop.take() {
                        let _ = tx.send(());
                    }
                }
                Ok(())
            }
            SAMPLE_LAB_ID => Ok(()), // files stay; they are harmless
            other => Err(AppError::NotFound(format!("lab '{other}'"))),
        }
    }
}

impl LabInfo for PracticeLabs {
    fn placeholders(&self) -> HashMap<String, String> {
        let folder = self.sample_dir.display().to_string().replace('/', "\\");
        let port = self.web.lock().unwrap().as_ref().map(|w| w.addr.port()).unwrap_or(self.port);
        HashMap::from([
            ("{lab:web_host}".to_string(), "127.0.0.1".to_string()),
            ("{lab:web_port}".to_string(), port.to_string()),
            ("{lab:web_url}".to_string(), format!("http://127.0.0.1:{port}/")),
            ("{lab:sample_rules}".to_string(), format!("{folder}\\.semgrep.yml")),
            ("{lab:sample_folder}".to_string(), folder),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn get(addr: SocketAddr, request_target: &str, method: &str) -> (String, String) {
        let mut s = TcpStream::connect(addr).await.unwrap();
        s.write_all(format!("{method} {request_target} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes()).await.unwrap();
        let mut out = Vec::new();
        s.read_to_end(&mut out).await.unwrap();
        let text = String::from_utf8_lossy(&out).to_string();
        let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
        (head.to_string(), body.to_string())
    }

    async fn started() -> (PracticeLabs, SocketAddr, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let labs = PracticeLabs::new(dir.path(), 0);
        labs.start(&lab_id(WEB_LAB_ID)).await.unwrap();
        let addr = labs.bound_addr().unwrap();
        (labs, addr, dir)
    }

    #[test]
    fn percent_decoding_handles_edges() {
        assert_eq!(percent_decode("a%20b+c%3Cx%3E"), "a b c<x>");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
        assert_eq!(percent_decode("%2"), "%2");
    }

    #[tokio::test]
    async fn serves_only_on_loopback_and_shows_the_deliberate_weaknesses() {
        let (labs, addr, _d) = started().await;
        assert!(addr.ip().is_loopback(), "the practice app must only listen on loopback");
        let (head, body) = get(addr, "/", "GET").await;
        assert!(head.starts_with("HTTP/1.1 200 OK"));
        assert!(head.contains("Server: VixaluneLab/0.1"));
        assert!(head.contains("Set-Cookie: session=practice-demo; Path=/") && !head.to_lowercase().contains("httponly"));
        for protective in ["content-security-policy", "x-frame-options", "x-content-type-options", "strict-transport-security"] {
            assert!(!head.to_lowercase().contains(protective), "{protective} should be missing on purpose");
        }
        assert!(body.contains("Vixalune Practice Shop"));
        let (_, search) = get(addr, "/search?q=%3Cb%3Ehi%3C%2Fb%3E", "GET").await;
        assert!(search.contains("Results for: <b>hi</b>"), "reflected without escaping on purpose: {search}");
        assert!(get(addr, "/files/", "GET").await.1.contains("backup-2024.zip.txt"));
        assert!(get(addr, "/admin/", "GET").await.1.contains("No login required"));
        assert!(get(addr, "/robots.txt", "GET").await.1.contains("Disallow: /admin/"));
        assert!(get(addr, "/nope", "GET").await.0.starts_with("HTTP/1.1 404"));
        assert!(get(addr, "/", "POST").await.0.starts_with("HTTP/1.1 405"));
        let (head, body) = get(addr, "/", "HEAD").await;
        assert!(head.starts_with("HTTP/1.1 200") && body.is_empty());
        labs.stop(&lab_id(WEB_LAB_ID)).await.unwrap();
    }

    #[tokio::test]
    async fn the_open_redirect_is_an_open_redirect_but_not_a_header_injection() {
        let (_labs, addr, _d) = started().await;
        let (head, _) = get(addr, "/redirect?to=http%3A%2F%2F93.184.216.34%2F", "GET").await;
        assert!(head.starts_with("HTTP/1.1 302") && head.contains("Location: http://93.184.216.34/"));
        let (head, _) = get(addr, "/redirect?to=%2Fadmin%2F", "GET").await;
        assert!(head.contains("Location: /admin/"));
        for evil in ["/redirect?to=http%3A%2F%2Fx%0D%0ASet-Cookie%3A%20a%3Db", "/redirect?to=javascript%3Aalert(1)", "/redirect", "/redirect?to="] {
            let (head, _) = get(addr, evil, "GET").await;
            assert!(head.starts_with("HTTP/1.1 400"), "{evil}: {head}");
            assert!(!head.contains("Set-Cookie: a=b"));
        }
    }

    #[tokio::test]
    async fn survives_garbage_oversized_requests_and_stops_on_demand() {
        let (labs, addr, _d) = started().await;
        for payload in [&b"\x00\x01\x02garbage\r\n\r\n"[..], b"GET\r\n\r\n", b"GET relative HTTP/1.1\r\n\r\n", &vec![b'A'; 100_000][..]] {
            let mut s = TcpStream::connect(addr).await.unwrap();
            let _ = s.write_all(payload).await;
            let mut sink = Vec::new();
            let _ = tokio::time::timeout(Duration::from_secs(7), s.read_to_end(&mut sink)).await;
        }
        assert!(get(addr, "/", "GET").await.0.starts_with("HTTP/1.1 200"), "still serving after bad input");
        assert_eq!(labs.state(&lab_id(WEB_LAB_ID)).unwrap(), LabState::Running);
        labs.stop(&lab_id(WEB_LAB_ID)).await.unwrap();
        assert_eq!(labs.state(&lab_id(WEB_LAB_ID)).unwrap(), LabState::Stopped);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(TcpStream::connect(addr).await.is_err(), "the port is released after stopping");
    }

    #[tokio::test]
    async fn starting_twice_is_harmless_and_a_busy_port_is_explained() {
        let (labs, addr, _d) = started().await;
        labs.start(&lab_id(WEB_LAB_ID)).await.unwrap();
        assert_eq!(labs.bound_addr().unwrap(), addr);
        let dir = tempfile::tempdir().unwrap();
        let other = PracticeLabs::new(dir.path(), addr.port());
        let e = other.start(&lab_id(WEB_LAB_ID)).await.unwrap_err();
        assert!(e.to_string().contains("Another program may be using that port"), "{e}");
        assert!(matches!(labs.start(&lab_id("nope")).await, Err(AppError::NotFound(_))));
    }

    #[test]
    fn the_sample_project_is_created_idempotently_and_contains_no_complete_secret_in_source() {
        let dir = tempfile::tempdir().unwrap();
        let labs = PracticeLabs::new(dir.path(), 0);
        labs.ensure_sample_project().unwrap();
        labs.ensure_sample_project().unwrap();
        for f in ["README.md", "app.py", "requirements.txt", "package-lock.json", "config.env", ".semgrep.yml"] {
            assert!(labs.sample_dir().join(f).is_file(), "{f}");
        }
        let env = std::fs::read_to_string(labs.sample_dir().join("config.env")).unwrap();
        assert!(env.contains("AKIAIOSFODNN7EXAMPLE"));
        // The key only exists at run time: the Rust source has it in two pieces.
        let source = include_str!("practice_lab.rs");
        let whole = ["AKIA", "IOSFODNN7EXAMPLE"].concat();
        assert_eq!(source.matches(&whole).count(), 1, "only this assertion spells it out");
        let app = std::fs::read_to_string(labs.sample_dir().join("app.py")).unwrap();
        assert!(app.contains("INTENTIONALLY INSECURE") && app.contains("eval(") && app.contains("shell=True"));
        // Our own file is refreshed if someone edits it; their extra files are left alone.
        std::fs::write(labs.sample_dir().join("app.py"), "tampered").unwrap();
        std::fs::write(labs.sample_dir().join("mine.txt"), "keep").unwrap();
        labs.ensure_sample_project().unwrap();
        assert!(std::fs::read_to_string(labs.sample_dir().join("app.py")).unwrap().contains("INTENTIONALLY INSECURE"));
        assert_eq!(std::fs::read_to_string(labs.sample_dir().join("mine.txt")).unwrap(), "keep");
    }

    #[test]
    fn placeholders_point_at_loopback_and_the_sample_folder() {
        let dir = tempfile::tempdir().unwrap();
        let labs = PracticeLabs::new(dir.path(), DEFAULT_PORT);
        let p = labs.placeholders();
        assert_eq!(p["{lab:web_host}"], "127.0.0.1");
        assert_eq!(p["{lab:web_url}"], "http://127.0.0.1:18080/");
        assert_eq!(p["{lab:web_port}"], "18080");
        assert!(p["{lab:sample_folder}"].ends_with("sample-code") && p["{lab:sample_rules}"].ends_with(".semgrep.yml"));
        let defs = labs.list();
        assert_eq!(defs.len(), 2);
        assert!(defs[0].weaknesses.len() >= 6 && defs[0].warning.contains("permission"));
    }
}

