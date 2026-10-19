//! Real command execution: fixed absolute program, argument array, scrubbed environment,
//! streamed output, timeout, and cancellation that kills the whole process tree.

use crate::domain::process::{CancellationToken, OutputStream};
use crate::domain::{AppError, AppResult};
use crate::ports::{CommandExecutor, CommandOutcome, CommandResult, CommandSpec, ExecOptions, LineSink};
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

pub use crate::domain::process::env_name_is_dangerous;

/// Resumes a process created with CREATE_SUSPENDED. `NtResumeProcess` is the long-stable ntdll entry point for this (the documented
/// alternative needs the primary thread handle, which the standard library does not expose).
#[cfg(windows)]
fn nt_resume_process(process: windows_sys::Win32::Foundation::HANDLE) -> i32 {
    #[link(name = "ntdll")]
    extern "system" {
        fn NtResumeProcess(process: windows_sys::Win32::Foundation::HANDLE) -> i32;
    }
    // SAFETY: called with a live process handle.
    unsafe { NtResumeProcess(process) }
}

/// A Windows job object that holds the child and everything it starts. Closing it (including when Vixalune itself exits
/// or crashes) kills what is still inside, so a run cannot leave unmanaged processes behind.
#[cfg(windows)]
struct JobGuard(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
unsafe impl Send for JobGuard {}
#[cfg(windows)]
unsafe impl Sync for JobGuard {}

#[cfg(windows)]
impl JobGuard {
    /// Put `process` into a new job. `None` if Windows refuses (the caller falls back to `taskkill /T`).
    fn adopt(process: std::os::windows::io::RawHandle, max_processes: Option<u32>) -> Option<Self> {
        use windows_sys::Win32::System::JobObjects::*;
        // SAFETY: plain Win32 calls with a zero-initialised, correctly sized structure and a handle we own.
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return None;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if let Some(n) = max_processes {
                info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
                info.BasicLimitInformation.ActiveProcessLimit = n;
            }
            let ok = SetInformationJobObject(job, JobObjectExtendedLimitInformation, &info as *const _ as *const _, std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32) != 0
                && AssignProcessToJobObject(job, process as _) != 0;
            if !ok {
                windows_sys::Win32::Foundation::CloseHandle(job);
                return None;
            }
            Some(Self(job))
        }
    }

    fn terminate(&self) {
        // SAFETY: the handle is valid until drop.
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.0, 1);
        }
    }

    /// The child ended normally: closing the job must no longer kill anything that is still running.
    fn disarm(&self) {
        use windows_sys::Win32::System::JobObjects::*;
        // SAFETY: as above.
        unsafe {
            let info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            SetInformationJobObject(self.0, JobObjectExtendedLimitInformation, &info as *const _ as *const _, std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32);
        }
    }
}

#[cfg(windows)]
impl Drop for JobGuard {
    fn drop(&mut self) {
        // SAFETY: closing the handle we created; with KILL_ON_JOB_CLOSE still set this ends the remaining members.
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

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
        self.run_in(spec, ExecOptions::default(), sink, cancel).await
    }

    async fn run_in(&self, spec: CommandSpec, opts: ExecOptions, sink: &dyn LineSink, cancel: &CancellationToken) -> AppResult<CommandResult> {
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
        if let Some(cwd) = &opts.cwd {
            if !cwd.is_absolute() || !cwd.is_dir() {
                return Err(AppError::Validation(format!("the working folder '{}' must be an existing absolute folder", cwd.display())));
            }
            cmd.current_dir(cwd);
        }
        for (k, v) in &opts.env {
            if env_name_is_dangerous(k) || k.contains(['=', '\0']) || v.contains('\0') {
                return Err(AppError::Validation(format!("refusing to pass the environment variable '{k}' to a child process")));
            }
            cmd.env(k, v);
        }
        #[cfg(windows)]
        // CREATE_NO_WINDOW | CREATE_SUSPENDED: the child is placed in its job object before it runs a single instruction,
        // so nothing it starts can escape the job.
        cmd.creation_flags(0x0800_0000 | 0x0000_0004);

        let mut child = cmd
            .spawn()
            .map_err(|e| AppError::CommandFailed { code: None, message: format!("could not start '{}': {e}", spec.program.display()) })?;
        let pid = child.id();
        #[cfg(windows)]
        let job = child.raw_handle().and_then(|h| JobGuard::adopt(h, opts.max_processes));
        #[cfg(windows)]
        if let Some(h) = child.raw_handle() {
            // SAFETY: `h` is the live process handle owned by `child`.
            let status = nt_resume_process(h as _);
            if status < 0 {
                let _ = child.start_kill();
                return Err(AppError::CommandFailed { code: None, message: format!("could not start '{}' (resume failed: {status:#x})", spec.program.display()) });
            }
        }
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
                    #[cfg(windows)]
                    if let Some(j) = &job {
                        j.terminate();
                    } else if let Some(pid) = pid {
                        kill_tree(pid).await;
                    }
                    #[cfg(not(windows))]
                    if let Some(pid) = pid {
                        kill_tree(pid).await;
                    }
                } else {
                    #[cfg(windows)]
                    if let Some(j) = &job {
                        if opts.kill_leftovers {
                            j.terminate();
                        } else {
                            j.disarm();
                        }
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
    async fn runs_in_the_requested_folder_with_extra_environment() {
        let sink = Collect::default();
        let dir = tempfile::tempdir().unwrap();
        let cmd = std::env::var_os("ComSpec").map(PathBuf::from).unwrap_or_else(|| system32("cmd.exe"));
        let spec = CommandSpec { program: cmd, args: vec!["/C".into(), "cd && echo %VIXALUNE_TEST_VALUE%".into()], timeout: Duration::from_secs(20) };
        let opts = ExecOptions { cwd: Some(dir.path().to_path_buf()), env: vec![("VIXALUNE_TEST_VALUE".into(), "hello-from-profile".into())], ..Default::default() };
        let r = SystemCommandExecutor.run_in(spec, opts, &sink, &CancellationToken::new()).await.unwrap();
        assert_eq!(r.outcome, CommandOutcome::Exited(0));
        let lines = sink.0.lock().unwrap().join("\n");
        assert!(lines.contains("hello-from-profile"), "{lines}");
        assert!(lines.to_lowercase().contains(&dir.path().file_name().unwrap().to_string_lossy().to_lowercase()), "{lines}");
        // A relative or missing working folder is refused.
        let bad = ExecOptions { cwd: Some(PathBuf::from("relative")), ..Default::default() };
        let spec = CommandSpec { program: system32("whoami.exe"), args: vec![], timeout: Duration::from_secs(5) };
        assert!(SystemCommandExecutor.run_in(spec, bad, &sink, &CancellationToken::new()).await.is_err());
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

    #[test]
    fn dangerous_environment_names_are_recognised() {
        for bad in ["PATH", "path", "PATHEXT", "ComSpec", "SystemRoot", "TEMP", "JAVA_TOOL_OPTIONS", "_JAVA_OPTIONS", "NODE_OPTIONS", "PYTHONPATH", "PYTHONSTARTUP", "DOTNET_STARTUP_HOOKS", "COR_PROFILER", "LD_PRELOAD", "HTTPS_PROXY", "GIT_SSH_COMMAND", "PSModulePath", "MSBUILDEXTRA", "SSLKEYLOGFILE", "NMAP_OPTIONS"] {
            assert!(env_name_is_dangerous(bad), "{bad}");
        }
        for ok in ["LANG", "NO_COLOR", "TZ", "CI", "SEMGREP_SEND_METRICS"] {
            assert!(!env_name_is_dangerous(ok), "{ok}");
        }
    }

    #[tokio::test]
    async fn dangerous_or_malformed_environment_is_refused_before_anything_starts() {
        let sink = Collect::default();
        for (k, v) in [("JAVA_TOOL_OPTIONS", "-javaagent:x.jar"), ("PATH", "C:\\evil"), ("A=B", "x"), ("OK", "a\0b")] {
            let opts = ExecOptions { env: vec![(k.into(), v.into())], ..Default::default() };
            let spec = CommandSpec { program: system32("whoami.exe"), args: vec![], timeout: Duration::from_secs(5) };
            assert!(SystemCommandExecutor.run_in(spec, opts, &sink, &CancellationToken::new()).await.is_err(), "{k}");
        }
        assert!(sink.0.lock().unwrap().is_empty(), "nothing ran");
    }

    #[tokio::test]
    async fn the_child_environment_is_scrubbed_of_unlisted_variables() {
        std::env::set_var("VIXALUNE_SECRET_TEST", "should-not-leak");
        let sink = Collect::default();
        let cmd = std::env::var_os("ComSpec").map(PathBuf::from).unwrap_or_else(|| system32("cmd.exe"));
        let spec = CommandSpec { program: cmd, args: vec!["/C".into(), "set".into()], timeout: Duration::from_secs(20) };
        let r = SystemCommandExecutor.run(spec, &sink, &CancellationToken::new()).await.unwrap();
        assert_eq!(r.outcome, CommandOutcome::Exited(0));
        assert!(!sink.0.lock().unwrap().join("
").contains("should-not-leak"));
    }

    /// Starts a grandchild that writes a marker file after ~3 s, then lets the parent exit immediately.
    async fn run_with_late_grandchild(kill_leftovers: bool) -> bool {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("child.cmd"), "@echo off
ping -n 4 127.0.0.1 >nul
echo late> late.txt
").unwrap();
        let cmd = std::env::var_os("ComSpec").map(PathBuf::from).unwrap_or_else(|| system32("cmd.exe"));
        std::fs::write(dir.path().join("parent.cmd"), "@echo off
start \"\" /B cmd /C .\\child.cmd
").unwrap();
        let spec = CommandSpec { program: cmd, args: vec!["/C".into(), ".\\parent.cmd".into()], timeout: Duration::from_secs(20) };
        let opts = ExecOptions { cwd: Some(dir.path().to_path_buf()), kill_leftovers, ..Default::default() };
        let r = SystemCommandExecutor.run_in(spec, opts, &Collect::default(), &CancellationToken::new()).await.unwrap();
        assert!(matches!(r.outcome, CommandOutcome::Exited(_)));
        tokio::time::sleep(Duration::from_secs(5)).await;
        dir.path().join("late.txt").exists()
    }

    #[tokio::test]
    async fn processes_left_behind_by_a_tool_run_are_stopped_but_installers_are_not_disturbed() {
        assert!(!run_with_late_grandchild(true).await, "a tool run must not leave a background process behind");
        assert!(run_with_late_grandchild(false).await, "without kill_leftovers the descendants are left alone");
    }

    #[tokio::test]
    async fn cancelling_kills_grandchildren_too() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("child.cmd"), "@echo off
ping -n 5 127.0.0.1 >nul
echo late> late.txt
").unwrap();
        let cmd = std::env::var_os("ComSpec").map(PathBuf::from).unwrap_or_else(|| system32("cmd.exe"));
        // The parent waits for the child, so only a tree kill can stop the marker from appearing.
        let spec = CommandSpec { program: cmd, args: vec!["/C".into(), ".\\child.cmd".into()], timeout: Duration::from_secs(60) };
        let cancel = CancellationToken::new();
        let c2 = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(700)).await;
            c2.cancel();
        });
        let opts = ExecOptions { cwd: Some(dir.path().to_path_buf()), ..Default::default() };
        let r = SystemCommandExecutor.run_in(spec, opts, &Collect::default(), &cancel).await.unwrap();
        assert_eq!(r.outcome, CommandOutcome::Cancelled);
        tokio::time::sleep(Duration::from_secs(6)).await;
        assert!(!dir.path().join("late.txt").exists(), "the grandchild must not survive a cancel");
    }
}
