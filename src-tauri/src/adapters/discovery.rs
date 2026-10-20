//! Package discovery through `winget search` and `winget show`. Both are read-only. The program is the absolute path of
//! `winget.exe`, the arguments are a fixed array whose only variable parts are a validated search text or package id, and nothing
//! passes through a shell.

use crate::domain::discover::{parse_search_table, parse_show, valid_search_query, valid_winget_id, DiscoveredDetails, DiscoveredPackage, MAX_SEARCH_RESULTS};
use crate::domain::process::{CancellationToken, OutputStream};
use crate::domain::{AppError, AppResult};
use crate::ports::{CommandExecutor, CommandOutcome, CommandSpec, ExecutableLocator, LineSink, PackageDiscovery};
use async_trait::async_trait;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(60);
const MAX_LINES: usize = 600;

pub struct WingetDiscovery {
    executor: Arc<dyn CommandExecutor>,
    locator: Arc<dyn ExecutableLocator>,
}

#[derive(Default)]
struct Collect(Mutex<Vec<String>>);
impl LineSink for Collect {
    fn line(&self, _: OutputStream, text: &str) {
        let mut v = self.0.lock().unwrap();
        if v.len() < MAX_LINES {
            v.push(text.to_string());
        }
    }
}

impl WingetDiscovery {
    pub fn new(executor: Arc<dyn CommandExecutor>, locator: Arc<dyn ExecutableLocator>) -> Self {
        Self { executor, locator }
    }

    async fn run(&self, args: Vec<String>, cancel: &CancellationToken) -> AppResult<(i32, String)> {
        let program = self.locator.locate("winget.exe", &[]).ok_or_else(|| AppError::PrerequisiteMissing("winget (Windows Package Manager) was not found, so packages cannot be searched".into()))?;
        let sink = Collect::default();
        let result = self.executor.run(CommandSpec { program, args, timeout: TIMEOUT }, &sink, cancel).await?;
        let text = sink.0.lock().unwrap().join("\n");
        match result.outcome {
            CommandOutcome::Exited(code) => Ok((code, text)),
            CommandOutcome::TimedOut => Err(AppError::TimedOut("searching the package repository".into())),
            CommandOutcome::Cancelled => Err(AppError::Cancelled),
        }
    }

    pub fn search_args(query: &str) -> Vec<String> {
        ["search", query.trim(), "--source", "winget", "--count", &MAX_SEARCH_RESULTS.to_string(), "--accept-source-agreements", "--disable-interactivity"].map(String::from).to_vec()
    }

    pub fn show_args(id: &str) -> Vec<String> {
        ["show", "--id", id, "--exact", "--source", "winget", "--accept-source-agreements", "--disable-interactivity"].map(String::from).to_vec()
    }
}

#[async_trait]
impl PackageDiscovery for WingetDiscovery {
    async fn search(&self, query: &str, cancel: &CancellationToken) -> AppResult<Vec<DiscoveredPackage>> {
        if !valid_search_query(query) {
            return Err(AppError::Validation("search for 2 to 64 letters, digits, spaces or . _ - + # /".into()));
        }
        let (code, text) = self.run(Self::search_args(query), cancel).await?;
        let found = parse_search_table(&text);
        // winget exits with a non-zero code when nothing matches; that is an empty result, not a failure.
        if found.is_empty() && code != 0 && !text.to_lowercase().contains("no package found") {
            return Err(AppError::CommandFailed { code: Some(code), message: "the package search did not complete".into() });
        }
        Ok(found)
    }

    async fn details(&self, id: &str, cancel: &CancellationToken) -> AppResult<DiscoveredDetails> {
        if !valid_winget_id(id) {
            return Err(AppError::Validation("that is not a valid package id".into()));
        }
        let (code, text) = self.run(Self::show_args(id), cancel).await?;
        let d = parse_show(&text).ok_or_else(|| if code == 0 { AppError::Validation("the package details could not be read".into()) } else { AppError::NotFound(format!("package '{id}'")) })?;
        if !d.id.eq_ignore_ascii_case(id) {
            return Err(AppError::Validation("winget answered for a different package".into()));
        }
        Ok(d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::testing::{MockExecutor, MockLocator, Step};

    fn make(handler: impl Fn(&CommandSpec) -> Step + Send + Sync + 'static) -> (WingetDiscovery, Arc<MockExecutor>) {
        let ex = Arc::new(MockExecutor::new(handler));
        let mut loc = MockLocator::default();
        loc.add("winget.exe", "C:\\fake\\winget.exe");
        (WingetDiscovery::new(ex.clone(), Arc::new(loc)), ex)
    }

    #[tokio::test]
    async fn search_runs_a_fixed_argument_array_and_parses_the_table() {
        let (d, ex) = make(|_| Step::ok(&["Name      Id                       Version Match", "----------------------------------------------------", "Nmap      Insecure.Nmap            7.80    ", "HopMatrix RedEyeNetworks.HopMatrix 1.19.0  Tag: nmap"]));
        let r = d.search("port scanner", &CancellationToken::new()).await.unwrap();
        assert_eq!(r.len(), 2);
        let calls = ex.calls();
        assert_eq!(calls[0].program.to_string_lossy(), "C:\\fake\\winget.exe");
        assert_eq!(calls[0].args[..2], ["search".to_string(), "port scanner".to_string()], "the text is one argument, never split or interpreted");
        assert!(calls[0].args.contains(&"--disable-interactivity".to_string()) && calls[0].args.contains(&"winget".to_string()));
    }

    #[tokio::test]
    async fn nothing_found_is_an_empty_list_and_odd_input_never_reaches_winget() {
        let (d, ex) = make(|_| Step::exit(1, &["No package found matching input criteria."]));
        assert!(d.search("zzzzzz", &CancellationToken::new()).await.unwrap().is_empty());
        let calls_before = ex.calls().len();
        for bad in ["--help", "a;b", "", "x"] {
            assert!(d.search(bad, &CancellationToken::new()).await.is_err(), "{bad}");
        }
        assert!(d.details("calc", &CancellationToken::new()).await.is_err());
        assert_eq!(ex.calls().len(), calls_before, "nothing was run for invalid input");
    }

    #[tokio::test]
    async fn details_must_describe_the_package_that_was_asked_for() {
        let show = ["Found Other [Other.Package]", "Version: 1.0", "Publisher: X"];
        let (d, _) = make(move |_| Step::ok(&show));
        assert!(d.details("Insecure.Nmap", &CancellationToken::new()).await.is_err());
        let (d, _) = make(|_| Step::ok(&["Found Nmap [Insecure.Nmap]", "Version: 7.80", "Publisher: Nmap Project"]));
        assert_eq!(d.details("Insecure.Nmap", &CancellationToken::new()).await.unwrap().version, "7.80");
    }

    #[tokio::test]
    async fn a_missing_winget_is_explained() {
        let ex = Arc::new(MockExecutor::new(|_| Step::ok(&[])));
        let d = WingetDiscovery::new(ex, Arc::new(MockLocator::default()));
        let e = d.search("nmap", &CancellationToken::new()).await.unwrap_err();
        assert!(e.to_string().contains("winget"));
    }

    /// Opt-in: the real winget. `cargo test --lib live_winget -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "runs the real winget and uses the network"]
    async fn live_winget_search_and_show() {
        let ex = Arc::new(crate::adapters::executor::SystemCommandExecutor);
        let loc = Arc::new(crate::adapters::locator::SystemExecutableLocator);
        let d = WingetDiscovery::new(ex, loc);
        let r = d.search("nmap", &CancellationToken::new()).await.expect("search");
        println!("{} result(s): {:?}", r.len(), r.iter().map(|p| (&p.id, &p.version)).collect::<Vec<_>>());
        assert!(r.iter().any(|p| p.id == "Insecure.Nmap"));
        let det = d.details("Insecure.Nmap", &CancellationToken::new()).await.expect("show");
        println!("{} {} by {} ({}), {} sha256 {}", det.name, det.version, det.publisher, det.license, det.installer_type, det.installer_sha256);
        assert_eq!(det.installer_sha256.len(), 64);
    }
}
