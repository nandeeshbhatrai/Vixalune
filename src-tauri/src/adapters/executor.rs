//! Real command execution: fixed absolute program, argument array, scrubbed environment,
//! streamed output, timeout, and cancellation that kills the whole process tree.

use crate::domain::process::{CancellationToken, OutputStream};
use crate::domain::{AppError, AppResult};
use crate::ports::{CommandExecutor, CommandOutcome, CommandResult, CommandSpec, LineSink};
use async_trait::async_trait;
use std::collections::VecDeque;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

const TAIL_LINES: usize = 40;
const MAX_LINE: usize = 2000;

/// Variables passed to child processes. Everything else (tokens, API keys, proxies, ...) is dropped.
const ENV_ALLOWLIST: &[&str] = &[
    "SystemRoot",
    "SystemDrive",
    "windir",
    "ComSpec",
    "PATHEXT",
    "TEMP",
    "TMP",
    "USERPROFILE",
    "USERNAME",
    "USERDOMAIN",
    "APPDATA",
    "LOCALAPPDATA",
    "ProgramData",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramW6432",
    "CommonProgramFiles",
    "CommonProgramFiles(x86)",
    "CommonProgramW6432",
    "ALLUSERSPROFILE",
    "PUBLIC",
    "HOMEDRIVE",
    "HOMEPATH",
    "OS",
    "PROCESSOR_ARCHITECTURE",
    "PROCESSOR_ARCHITEW6432",
    "NUMBER_OF_PROCESSORS",
    "LANG",
];

#[derive(Default)]
pub struct SystemCommandExecutor;

/// Splits a byte stream into lines on `\n` and `\r` (winget redraws progress with `\r`), drops
/// blank lines and pure spinner glyphs, and forwards the rest.
async fn pump<R: AsyncRead + Unpin>(mut reader: R, stream: OutputStream, sink: &dyn LineSink, tail: &std::sync::Mutex<VecDeque<String>>) {
    let mut buf = [0u8; 4096];
    let mut pending: Vec<u8> = Vec::new();
    let emit = |bytes: &[u8]| {
        let text = String::from_utf8_lossy(bytes);
        let cleaned: String = text.chars().filter(|c| !c.is_control()).collect();
        let t = cleaned.trim();
        if t.is_empty() || t.chars().all(|c| matches!(c, '-' | '\\' | '|' | '/')) {
            return;
        }
        let mut line = t.to_string();
        line.truncate(line.floor_char_boundary(MAX_LINE));
        sink.line(stream, &line);
        if let Ok(mut q) = tail.lock() {
            if q.len() == TAIL_LINES {
                q.pop_front();
            }
            q.push_back(line);
        }
    };
    loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                for &b in &buf[..n] {
                    if b == b'\n' || b == b'\r' {
                        emit(&pending);
                        pending.clear();
                    } else if pending.len() < 16 * 1024 {
                        pending.push(b);
                    }
                }
            }
        }
    }
    if !pending.is_empty() {
        emit(&pending);
    }
}

/// Kill `pid` and its descendants. `taskkill` is invoked with a fixed program and integer pid only.
async fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        let system_root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        let taskkill = std::path::Path::new(&system_root).join("System32").join("taskkill.exe");
        let mut cmd = Command::new(taskkill);
        cmd.args(["/PID", &pid.to_string(), "/T", "/F"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        cmd.creation_flags(0x0800_0000);
        let _ = cmd.status().await;
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
    }
}

#[async_trait]
impl CommandExecutor for SystemCommandExecutor {
    async fn run(&self, spec: CommandSpec, sink: &dyn LineSink, cancel: &CancellationToken) -> AppResult<CommandResult> {
        if !spec.program.is_absolute() {
            return Err(AppError::Validation(format!("refusing to run non-absolute program '{}'", spec.program.display())));
        }
        let mut cmd = Command::new(&spec.program);
        cmd.args(&spec.args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
        cmd.env_clear();
        for name in ENV_ALLOWLIST {
            if let Some(v) = std::env::var_os(name) {
                cmd.env(name, v);
            }
        }
        // PATH is passed through so child installers can find system tools; it holds no secrets.
        if let Some(p) = std::env::var_os("PATH") {
            cmd.env("PATH", p);
        }
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW

        let mut child = cmd
            .spawn()
            .map_err(|e| AppError::CommandFailed { code: None, message: format!("could not start '{}': {e}", spec.program.display()) })?;
        let pid = child.id();
        let stdout = child.stdout.take().expect("piped");
        let stderr = child.stderr.take().expect("piped");
        let tail = std::sync::Mutex::new(VecDeque::new());

        let io = async {
            tokio::join!(pump(stdout, OutputStream::Stdout, sink, &tail), pump(stderr, OutputStream::Stderr, sink, &tail));
        };
        let wait = async {
            let status = child.wait().await;
            status.map(|s| s.code())
        };

        enum Stop {
            Done(Option<i32>),
            TimedOut,
            Cancelled,
        }
        let cancelled = async {
            while !cancel.is_cancelled() {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        };
        let main = async {
            tokio::select! {
                r = wait => Stop::Done(r.ok().flatten()),
                _ = tokio::time::sleep(spec.timeout) => Stop::TimedOut,
                _ = cancelled => Stop::Cancelled,
            }
        };
        let (stop, ()) = tokio::join!(
            async {
                let stop = main.await;
                if !matches!(stop, Stop::Done(_)) {
                    if let Some(pid) = pid {
                        kill_tree(pid).await;
                    }
                }
                stop
            },
            io
        );

        let outcome = match stop {
            Stop::Done(Some(code)) => CommandOutcome::Exited(code),
            Stop::Done(None) => CommandOutcome::Exited(-1),
            Stop::TimedOut => CommandOutcome::TimedOut,
            Stop::Cancelled => CommandOutcome::Cancelled,
        };
        let tail = tail.into_inner().map(|q| q.into_iter().collect()).unwrap_or_default();
        Ok(CommandResult { outcome, tail })
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Collect(Mutex<Vec<String>>);
    impl LineSink for Collect {
        fn line(&self, _: OutputStream, text: &str) {
            self.0.lock().unwrap().push(text.to_string());
        }
    }

    fn system32(exe: &str) -> PathBuf {
        PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32").join(exe)
    }

    #[tokio::test]
    async fn runs_a_real_process_and_streams_output() {
        let sink = Collect::default();
        let spec = CommandSpec { program: system32("whoami.exe"), args: vec![], timeout: Duration::from_secs(20) };
        let r = SystemCommandExecutor.run(spec, &sink, &CancellationToken::new()).await.unwrap();
        assert_eq!(r.outcome, CommandOutcome::Exited(0));
        assert!(!sink.0.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn reports_nonzero_exit_codes() {
        let sink = Collect::default();
        let spec = CommandSpec { program: system32("whoami.exe"), args: vec!["/definitely-not-a-flag".into()], timeout: Duration::from_secs(20) };
        let r = SystemCommandExecutor.run(spec, &sink, &CancellationToken::new()).await.unwrap();
        assert!(matches!(r.outcome, CommandOutcome::Exited(code) if code != 0));
    }

    #[tokio::test]
    async fn times_out_and_kills() {
        let sink = Collect::default();
        let spec = CommandSpec { program: system32("ping.exe"), args: vec!["-n".into(), "30".into(), "127.0.0.1".into()], timeout: Duration::from_millis(600) };
        let started = std::time::Instant::now();
        let r = SystemCommandExecutor.run(spec, &sink, &CancellationToken::new()).await.unwrap();
        assert_eq!(r.outcome, CommandOutcome::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[tokio::test]
    async fn cancellation_kills_the_process() {
        let sink = Collect::default();
        let cancel = CancellationToken::new();
        let c2 = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            c2.cancel();
        });
        let spec = CommandSpec { program: system32("ping.exe"), args: vec!["-n".into(), "30".into(), "127.0.0.1".into()], timeout: Duration::from_secs(60) };
        let started = std::time::Instant::now();
        let r = SystemCommandExecutor.run(spec, &sink, &cancel).await.unwrap();
        assert_eq!(r.outcome, CommandOutcome::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[tokio::test]
    async fn refuses_relative_programs_and_missing_files() {
        let sink = Collect::default();
        let rel = CommandSpec { program: PathBuf::from("whoami.exe"), args: vec![], timeout: Duration::from_secs(5) };
        assert!(SystemCommandExecutor.run(rel, &sink, &CancellationToken::new()).await.is_err());
        let missing = CommandSpec { program: system32("no-such-program.exe"), args: vec![], timeout: Duration::from_secs(5) };
        assert!(SystemCommandExecutor.run(missing, &sink, &CancellationToken::new()).await.is_err());
    }

    #[tokio::test]
    async fn arguments_are_not_interpreted_by_a_shell() {
        let sink = Collect::default();
        // If a shell were involved, "&" would start a second command. ping simply fails on the bad host name.
        let spec = CommandSpec { program: system32("ping.exe"), args: vec!["-n".into(), "1".into(), "127.0.0.1 & whoami".into()], timeout: Duration::from_secs(20) };
        let r = SystemCommandExecutor.run(spec, &sink, &CancellationToken::new()).await.unwrap();
        assert!(matches!(r.outcome, CommandOutcome::Exited(code) if code != 0));
        let lines = sink.0.lock().unwrap().join("\n").to_lowercase();
        assert!(!lines.contains("\\"), "whoami output (DOMAIN\\user) must not appear: {lines}");
    }
}
