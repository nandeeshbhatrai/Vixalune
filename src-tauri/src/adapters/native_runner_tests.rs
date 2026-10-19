use super::*;
use crate::adapters::activity_sqlite::SqliteActivityLogger;
use crate::adapters::practice_lab::PracticeLabs;
use crate::adapters::registry::ManifestRegistry;
use crate::adapters::settings_store::FileSettings;
use crate::adapters::sqlite_store::SqliteJobStore;
use crate::adapters::testing::{MockExecutor, MockLocator, Step};
use crate::domain::container::RunApprovals;
use crate::domain::scope::{NameResolver, ScopeAck};
use crate::ports::{RedirectProbe, RunHistory, SettingsProvider, TrustLevel};
use std::net::IpAddr;

/// DNS that tests can change between "preview" and "start".
#[derive(Default)]
struct Dns(Mutex<HashMap<String, Vec<IpAddr>>>);
impl Dns {
    fn set(&self, name: &str, ips: &[&str]) {
        self.0.lock().unwrap().insert(name.into(), ips.iter().map(|s| s.parse().unwrap()).collect());
    }
}
struct SharedDns(Arc<Dns>);
impl NameResolver for SharedDns {
    fn resolve(&self, name: &str) -> Result<Vec<IpAddr>, String> {
        self.0 .0.lock().unwrap().get(name).cloned().ok_or_else(|| "no such host".to_string())
    }
}

#[derive(Default)]
struct Redirects(Mutex<HashMap<String, String>>);
struct SharedRedirects(Arc<Redirects>);
#[async_trait]
impl RedirectProbe for SharedRedirects {
    async fn head(&self, url: &str) -> AppResult<Option<String>> {
        Ok(self.0 .0.lock().unwrap().get(url).cloned())
    }
}

struct Fixture {
    runner: NativeToolRunner,
    exec: Arc<MockExecutor>,
    store: Arc<SqliteJobStore>,
    dns: Arc<Dns>,
    redirects: Arc<Redirects>,
    settings: Arc<FileSettings>,
    support: Arc<RunSupport>,
    _dir: tempfile::TempDir,
}

fn fixture(handler: impl Fn(&CommandSpec) -> Step + Send + Sync + 'static, installed: &[&str]) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let mdir = dir.path().join("manifests");
    std::fs::create_dir_all(&mdir).unwrap();
    std::fs::write(mdir.join("nmap.toml"), include_str!("../../../manifests/nmap.toml")).unwrap();
    std::fs::write(mdir.join("trivy.toml"), include_str!("../../../manifests/trivy.toml")).unwrap();
    std::fs::write(mdir.join("zap.toml"), include_str!("../../../manifests/zap.toml")).unwrap();
    std::fs::write(mdir.join("semgrep.toml"), include_str!("../../../manifests/semgrep.toml")).unwrap();
    let registry = Arc::new(ManifestRegistry::load_dir(&mdir, TrustLevel::Bundled).unwrap());
    let exec = Arc::new(MockExecutor::new(handler));
    let mut loc = MockLocator::default();
    for n in installed {
        loc.add(n, &format!("C:\\fake\\{n}"));
    }
    let store = Arc::new(SqliteJobStore::in_memory().unwrap());
    let dns = Arc::new(Dns::default());
    dns.set("localhost", &["127.0.0.1"]);
    let redirects = Arc::new(Redirects::default());
    let settings = Arc::new(FileSettings::in_memory());
    let support = Arc::new(RunSupport {
        settings: settings.clone(),
        labs: Arc::new(PracticeLabs::new(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("mount-tests").join(format!("labs-{}", uuid::Uuid::new_v4().simple())), 18080)),
        history: store.clone(),
        resolver: Arc::new(SharedDns(dns.clone())),
        probe: Arc::new(SharedRedirects(redirects.clone())),
        runs_dir: dir.path().join("runs"),
    });
    let runner = NativeToolRunner::new(
        registry,
        exec.clone(),
        Arc::new(loc),
        store.clone(),
        Arc::new(SqliteActivityLogger::in_memory().unwrap()),
        support.clone(),
        Some(PathBuf::from("C:\\Users\\me")),
    );
    Fixture { runner, exec, store, dns, redirects, settings, support, _dir: dir }
}

const WAIT: Duration = Duration::from_secs(10);

fn nmap() -> ToolId {
    ToolId::new("nmap").unwrap()
}
fn text(s: &str) -> ParamValue {
    ParamValue::Text(s.into())
}
fn vals(pairs: &[(&str, &str)]) -> HashMap<String, ParamValue> {
    pairs.iter().map(|(k, v)| (k.to_string(), text(v))).collect()
}
fn none() -> RunApprovals {
    RunApprovals::default()
}
fn confirm(typed: &str, addresses: &[&str]) -> RunApprovals {
    RunApprovals { scope: Some(ScopeAck { acknowledged: true, typed: Some(typed.into()), addresses: addresses.iter().map(|s| s.to_string()).collect() }), ..Default::default() }
}

#[tokio::test]
async fn a_lab_scan_that_finds_an_open_port_is_findings_not_a_failure() {
    let f = fixture(|_| Step::ok(&["Starting Nmap", "PORT      STATE SERVICE", "18080/tcp open  http", "Nmap done"]), &["nmap.exe"]);
    let op = f.runner.start(&nmap(), "quick-scan", &HashMap::new(), &none()).await.unwrap();
    let job = f.runner.wait(op, WAIT).await.unwrap();
    assert_eq!(job.phase, JobPhase::Succeeded);
    assert_eq!(job.exit_code, Some(0));
    let call = &f.exec.calls()[0];
    assert_eq!(call.args, ["-F", "--open", "127.0.0.1"], "the default target is the local lab");
    assert_eq!(call.timeout, Duration::from_secs(600));
    let o = &f.exec.options()[0];
    assert_eq!(o.cwd.as_deref(), Some(f.support.run_dir(op).as_path()), "runs start in their own folder");
    assert!(f.support.run_dir(op).is_dir());
    assert!(o.env.is_empty());
    let rec = f.store.get_run(op).unwrap();
    assert_eq!(rec.outcome, Some(RunOutcome::Findings));
    assert_eq!(rec.exit_code, Some(0));
    assert_eq!(rec.scope, "this computer (loopback)");
    assert_eq!(rec.targets, ["127.0.0.1"]);
    assert!(rec.finished_at.is_some() && rec.duration_ms.is_some());
    assert!(rec.command.contains("-F --open 127.0.0.1"), "{}", rec.command);
    assert!(f.store.job_output(op, 100).unwrap().iter().any(|l| l.contains("18080/tcp open")));
}

#[tokio::test]
async fn clean_runs_failed_runs_and_findings_exit_codes_are_distinguished() {
    // Nothing open: finished, nothing reported.
    let f = fixture(|_| Step::ok(&["PORT STATE SERVICE", "Nmap done: 1 IP address (1 host up)"]), &["nmap.exe"]);
    let op = f.runner.start(&nmap(), "quick-scan", &HashMap::new(), &none()).await.unwrap();
    assert_eq!(f.runner.wait(op, WAIT).await.unwrap().phase, JobPhase::Succeeded);
    assert_eq!(f.store.get_run(op).unwrap().outcome, Some(RunOutcome::Clean));

    // Unexpected exit code: failed, with the exit code and last output lines.
    let f = fixture(|_| Step::exit(2, &["QUITTING!", "Failed to resolve"]), &["nmap.exe"]);
    let op = f.runner.start(&nmap(), "quick-scan", &HashMap::new(), &none()).await.unwrap();
    let job = f.runner.wait(op, WAIT).await.unwrap();
    assert_eq!(job.phase, JobPhase::Failed);
    assert_eq!(job.exit_code, Some(2));
    assert!(job.error.as_ref().unwrap().detail.as_deref().unwrap().contains("Failed to resolve"));
    assert_eq!(f.store.get_run(op).unwrap().outcome, Some(RunOutcome::Failed));

    // Trivy: exit code 1 means findings.
    let f = fixture(|_| Step::exit(1, &["Total: 3 (HIGH: 2, CRITICAL: 1)"]), &["trivy.exe"]);
    let folder = f.support.labs.placeholders()["{lab:sample_folder}"].clone();
    std::fs::create_dir_all(&folder).unwrap();
    let op = f.runner.start(&ToolId::new("trivy").unwrap(), "scan-folder", &HashMap::new(), &none()).await.unwrap();
    let job = f.runner.wait(op, WAIT).await.unwrap();
    assert_eq!(job.phase, JobPhase::Succeeded, "exit code 1 is a finding, not a crash");
    assert_eq!(f.store.get_run(op).unwrap().outcome, Some(RunOutcome::Findings));
}

#[tokio::test]
async fn a_process_that_cannot_start_is_a_failed_run_with_a_reason() {
    struct Broken;
    #[async_trait]
    impl CommandExecutor for Broken {
        async fn run(&self, _: CommandSpec, _: &dyn LineSink, _: &CancellationToken) -> AppResult<crate::ports::CommandResult> {
            Err(AppError::Io("access is denied".into()))
        }
    }
    let f = fixture(|_| Step::ok(&[]), &["nmap.exe"]);
    let runner = NativeToolRunner::new(
        f.runner.inner.registry.clone(),
        Arc::new(Broken),
        f.runner.inner.locator.clone(),
        f.store.clone(),
        f.runner.inner.activity.clone(),
        f.support.clone(),
        None,
    );
    let op = runner.start(&nmap(), "quick-scan", &HashMap::new(), &none()).await.unwrap();
    let job = runner.wait(op, WAIT).await.unwrap();
    assert_eq!(job.phase, JobPhase::Failed);
    assert!(job.message.contains("access is denied"), "{}", job.message);
    assert_eq!(f.store.get_run(op).unwrap().outcome, Some(RunOutcome::Failed));
}

#[tokio::test]
async fn timeouts_are_reported_as_timeouts() {
    let f = fixture(|_| Step::timed_out(), &["nmap.exe"]);
    let op = f.runner.start(&nmap(), "quick-scan", &HashMap::new(), &none()).await.unwrap();
    let job = f.runner.wait(op, WAIT).await.unwrap();
    assert_eq!(job.phase, JobPhase::Failed);
    assert_eq!(job.error.as_ref().unwrap().code, "timed_out");
    assert_eq!(f.store.get_run(op).unwrap().outcome, Some(RunOutcome::TimedOut));
}

#[tokio::test]
async fn cancelling_stops_the_run_and_a_second_run_of_the_same_tool_waits() {
    let f = fixture(|_| Step::hang(&["scanning..."]), &["nmap.exe"]);
    let op = f.runner.start(&nmap(), "quick-scan", &HashMap::new(), &none()).await.unwrap();
    // Same tool again while it is running: refused, nothing else starts.
    let busy = f.runner.start(&nmap(), "quick-scan", &HashMap::new(), &none()).await.unwrap_err();
    assert!(matches!(busy, AppError::Busy(_)), "{busy:?}");
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(f.store.get_job(op).unwrap().phase, JobPhase::Running);
    f.runner.cancel(op).await.unwrap();
    let job = f.runner.wait(op, WAIT).await.unwrap();
    assert_eq!(job.phase, JobPhase::Cancelled);
    assert_eq!(f.store.get_run(op).unwrap().outcome, Some(RunOutcome::Cancelled));
    assert!(f.runner.cancel(op).await.is_err(), "nothing left to cancel");
}

#[tokio::test]
async fn malformed_parameters_never_reach_the_process() {
    let f = fixture(|_| Step::ok(&[]), &["nmap.exe", "trivy.exe", "zap.exe"]);
    let cases: Vec<(&str, &str, Vec<(&str, &str)>)> = vec![
        ("nmap", "quick-scan", vec![("target", "127.0.0.1; calc.exe")]),
        ("nmap", "quick-scan", vec![("target", "127.0.0.1 -p-")]),
        ("nmap", "quick-scan", vec![("target", "-iL C:\\secrets.txt")]),
        ("nmap", "quick-scan", vec![("target", "host\nx")]),
        ("nmap", "quick-scan", vec![("target", "")]),
        ("nmap", "quick-scan", vec![("target", "a,b")]),
        ("nmap", "quick-scan", vec![("target", "10.0.0.1-50")]),
        ("nmap", "quick-scan", vec![("extra", "--script=vuln")]),
        ("nmap", "service-scan", vec![("ports", "80;whoami")]),
        ("nmap", "service-scan", vec![("ports", "0")]),
        ("nmap", "service-scan", vec![("ports", "70000")]),
        ("nmap", "service-scan", vec![("ports", "443,80-")]),
        ("nmap", "host-discovery", vec![("target", "10.0.0.0/8")]),
        ("nmap", "host-discovery", vec![("target", "127.0.0.0/8")]),
        ("nmap", "no-such-profile", vec![]),
        ("zap", "quick-scan", vec![("url", "ftp://127.0.0.1/")]),
        ("zap", "quick-scan", vec![("url", "http://user:pw@127.0.0.1/")]),
        ("zap", "quick-scan", vec![("url", "javascript:alert(1)")]),
        ("zap", "quick-scan", vec![("url", "http://127.0.0.1:99999/")]),
        ("trivy", "scan-folder", vec![("severity", "ALL")]),
        ("trivy", "scan-folder", vec![("folder", "C:\\Windows")]),
        ("trivy", "scan-folder", vec![("folder", "\\\\server\\share")]),
        ("trivy", "scan-folder", vec![("folder", "..\\..\\secrets")]),
        ("trivy", "scan-folder", vec![("folder", "C:\\does\\not\\exist\\anywhere")]),
    ];
    for (tool, profile, v) in cases {
        let r = f.runner.start(&ToolId::new(tool).unwrap(), profile, &vals(&v), &none()).await;
        assert!(r.is_err(), "{tool} {profile} {v:?} must be refused");
    }
    assert!(f.exec.calls().is_empty(), "nothing was executed");
    assert!(f.store.list_jobs(10).unwrap().is_empty(), "no job was created for refused runs");
    assert!(f.store.list_runs(10).unwrap().is_empty());
}

#[tokio::test]
async fn hostile_spellings_of_local_addresses_do_not_slip_through() {
    let f = fixture(|_| Step::ok(&[]), &["nmap.exe"]);
    // Alternate IP spellings are refused outright (not "normalised" into something that looks local).
    for t in ["2130706433", "0x7f.0.0.1", "127.1", "0177.0.0.1", "0.0.0.0", "::", "255.255.255.255", "224.0.0.1"] {
        let r = f.runner.start(&nmap(), "quick-scan", &vals(&[("target", t)]), &confirm(t, &[])).await;
        assert!(r.is_err(), "{t} must not run");
    }
    assert!(f.exec.calls().is_empty());
    // A look-alike name is a name: it must resolve, and then its real address decides.
    f.dns.set("127.0.0.1.evil.test", &["93.184.216.34"]);
    let e = f.runner.start(&nmap(), "quick-scan", &vals(&[("target", "127.0.0.1.evil.test")]), &none()).await.unwrap_err();
    assert!(e.to_string().contains("confirm"), "{e}");
    // Uppercase localhost still resolves to loopback and is allowed.
    assert!(f.runner.start(&nmap(), "quick-scan", &vals(&[("target", "LOCALHOST")]), &none()).await.is_ok());
}

#[tokio::test]
async fn targets_outside_the_lab_need_explicit_confirmation_and_the_right_words() {
    let f = fixture(|_| Step::ok(&["Nmap done"]), &["nmap.exe"]);
    f.dns.set("scanme.example.org", &["93.184.216.34"]);
    let t = vals(&[("target", "scanme.example.org")]);

    let p = f.runner.preview(&nmap(), "quick-scan", &t, &none()).await.unwrap();
    let scope = p.scope.unwrap();
    assert_eq!(scope.confirmation.as_ref().unwrap().phrase.as_deref(), Some("scanme.example.org"));
    assert_eq!(scope.addresses(), ["93.184.216.34"]);

    let refused = |r: AppResult<OperationId>| r.unwrap_err().to_string();
    assert!(refused(f.runner.start(&nmap(), "quick-scan", &t, &none()).await).contains("confirm"));
    assert!(refused(f.runner.start(&nmap(), "quick-scan", &t, &confirm("wrong.example", &["93.184.216.34"])).await).contains("type the target"));
    let unticked = RunApprovals { scope: Some(ScopeAck { acknowledged: false, typed: Some("scanme.example.org".into()), addresses: vec!["93.184.216.34".into()] }), ..Default::default() };
    assert!(refused(f.runner.start(&nmap(), "quick-scan", &t, &unticked).await).contains("confirm"));
    assert!(f.exec.calls().is_empty());

    let op = f.runner.start(&nmap(), "quick-scan", &t, &confirm("scanme.example.org", &["93.184.216.34"])).await.unwrap();
    f.runner.wait(op, WAIT).await.unwrap();
    let rec = f.store.get_run(op).unwrap();
    assert_eq!(rec.scope, "confirmed: public address");
    assert_eq!(rec.targets, ["scanme.example.org"]);
}

#[tokio::test]
async fn the_confirmation_is_void_if_the_name_now_points_somewhere_else() {
    let f = fixture(|_| Step::ok(&[]), &["nmap.exe"]);
    f.dns.set("shop.example.org", &["93.184.216.34"]);
    let t = vals(&[("target", "shop.example.org")]);
    let approval = confirm("shop.example.org", &["93.184.216.34"]);
    assert!(f.runner.preview(&nmap(), "quick-scan", &t, &approval).await.is_ok());
    // DNS changed between review and run: it now also points at an internal address.
    f.dns.set("shop.example.org", &["93.184.216.34", "10.0.0.5"]);
    let e = f.runner.start(&nmap(), "quick-scan", &t, &approval).await.unwrap_err();
    assert!(e.to_string().contains("changed"), "{e}");
    assert!(f.exec.calls().is_empty());
}

#[tokio::test]
async fn the_lab_list_is_configurable_and_names_cannot_be_labelled_into_public_space() {
    let f = fixture(|_| Step::ok(&[]), &["nmap.exe"]);
    f.dns.set("juice.lab.test", &["192.168.56.10"]);
    let t = vals(&[("target", "juice.lab.test")]);
    // Private but not on the lab list: confirmation needed.
    assert!(f.runner.start(&nmap(), "quick-scan", &t, &none()).await.is_err());
    let mut s = f.settings.get();
    s.scope.lab_targets = vec!["192.168.56.0/24".into()];
    f.settings.update(s).unwrap();
    let op = f.runner.start(&nmap(), "quick-scan", &t, &none()).await.unwrap();
    f.runner.wait(op, WAIT).await.unwrap();
    assert_eq!(f.store.get_run(op).unwrap().scope, "your configured lab");
    // A lab name that resolves to a public address is still public.
    let mut s = f.settings.get();
    s.scope.lab_targets = vec!["evil.lab.test".into()];
    f.settings.update(s).unwrap();
    f.dns.set("evil.lab.test", &["93.184.216.34"]);
    assert!(f.runner.start(&nmap(), "quick-scan", &vals(&[("target", "evil.lab.test")]), &none()).await.is_err());
}

#[tokio::test]
async fn an_unresolvable_target_is_refused_even_with_a_confirmation() {
    let f = fixture(|_| Step::ok(&[]), &["nmap.exe"]);
    let t = vals(&[("target", "does-not-exist.example")]);
    let e = f.runner.start(&nmap(), "quick-scan", &t, &confirm("does-not-exist.example", &[])).await.unwrap_err();
    assert!(e.to_string().contains("out of scope"), "{e}");
    assert!(f.exec.calls().is_empty());
}

#[tokio::test]
async fn a_redirect_out_of_scope_is_stopped_before_anything_is_sent_there() {
    let f = fixture(|_| Step::ok(&[]), &["zap.exe"]);
    let zap = ToolId::new("zap").unwrap();
    let start = "http://127.0.0.1:18080/redirect?to=http://93.184.216.34/";
    f.redirects.0.lock().unwrap().insert(start.into(), "http://93.184.216.34/".into());
    let v = vals(&[("url", start)]);
    let e = f.runner.start(&zap, "quick-scan", &v, &none()).await.unwrap_err();
    assert!(e.to_string().contains("redirects"), "{e}");
    let p = f.runner.preview(&zap, "quick-scan", &v, &none()).await.unwrap_err();
    assert!(p.to_string().contains("redirects"), "preview refuses too: {p}");
    // Also a chain: local -> local -> public.
    let a = "http://127.0.0.1:18080/a";
    f.redirects.0.lock().unwrap().insert(a.into(), "/b".into());
    f.redirects.0.lock().unwrap().insert("http://127.0.0.1:18080/b".into(), "http://93.184.216.34/x".into());
    assert!(f.runner.start(&zap, "quick-scan", &vals(&[("url", a)]), &none()).await.is_err());
    // A redirect that stays local is fine.
    let c = "http://127.0.0.1:18080/c";
    f.redirects.0.lock().unwrap().insert(c.into(), "http://127.0.0.1:18080/admin/".into());
    assert!(f.runner.start(&zap, "quick-scan", &vals(&[("url", c)]), &none()).await.is_ok());
}

#[tokio::test]
async fn a_missing_program_is_explained_and_creates_no_job() {
    let f = fixture(|_| Step::ok(&[]), &[]);
    let e = f.runner.start(&nmap(), "quick-scan", &HashMap::new(), &none()).await.unwrap_err();
    assert!(matches!(e, AppError::PrerequisiteMissing(_)), "{e:?}");
    assert!(e.to_string().contains("Install Nmap"), "{e}");
    assert!(f.store.list_jobs(5).unwrap().is_empty());
    let p = f.runner.preview(&nmap(), "quick-scan", &HashMap::new(), &none()).await.unwrap();
    assert!(p.notes.iter().any(|n| n.contains("not installed yet")));
}

#[tokio::test]
async fn the_preview_describes_the_run_without_starting_it() {
    let f = fixture(|_| Step::ok(&[]), &["nmap.exe"]);
    let p = f.runner.preview(&nmap(), "service-scan", &vals(&[("ports", "80,443")]), &none()).await.unwrap();
    assert_eq!(p.command.args, ["-sV", "--version-light", "-p", "80,443", "127.0.0.1"]);
    assert!(p.command.display.contains("-sV"));
    assert_eq!(p.network_activity, crate::domain::manifest::NetworkActivity::ProbesTargets);
    assert_eq!(p.timeout_secs, 900);
    assert!(p.expected_output.contains("service"));
    assert!(p.notes.iter().any(|n| n.contains("never asks for administrator")));
    assert!(p.scope.is_some() && p.container.is_none());
    assert!(f.exec.calls().is_empty() && f.store.list_jobs(5).unwrap().is_empty());
}

#[tokio::test]
async fn runs_are_never_elevated_and_container_profiles_are_left_to_the_container_runner() {
    let f = fixture(|_| Step::ok(&[]), &["nmap.exe"]);
    let op = f.runner.start(&nmap(), "quick-scan", &HashMap::new(), &none()).await.unwrap();
    f.runner.wait(op, WAIT).await.unwrap();
    assert!(!f.store.get_job(op).unwrap().allow_elevation);
    let sem = ToolId::new("semgrep").unwrap();
    assert!(f.runner.start(&sem, "scan-local", &HashMap::new(), &none()).await.is_err());
}

#[tokio::test]
async fn old_history_and_output_are_purged_by_the_retention_settings() {
    let f = fixture(|_| Step::ok(&["Nmap done"]), &["nmap.exe"]);
    let op = f.runner.start(&nmap(), "quick-scan", &HashMap::new(), &none()).await.unwrap();
    f.runner.wait(op, WAIT).await.unwrap();
    // Fresh data is kept by the defaults.
    let r = f.support.purge_expired().unwrap();
    assert_eq!((r.runs_deleted, r.output_lines_deleted), (0, 0));
    assert!(!f.store.job_output(op, 10).unwrap().is_empty());
    // "0 days of output": the output goes at the next purge; the history record stays.
    let mut s = f.settings.get();
    s.retention.output_days = 0;
    f.settings.update(s).unwrap();
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let r = f.support.purge_expired().unwrap();
    assert!(r.output_lines_deleted > 0);
    assert!(f.store.job_output(op, 10).unwrap().is_empty());
    assert!(f.store.get_run(op).is_ok());
}

#[tokio::test]
async fn an_ipv4_mapped_address_is_judged_by_the_address_inside_it() {
    let f = fixture(|_| Step::ok(&[]), &["nmap.exe"]);
    let e = f.runner.start(&nmap(), "quick-scan", &vals(&[("target", "::ffff:8.8.8.8")]), &none()).await.unwrap_err();
    assert!(e.to_string().contains("confirm"), "a mapped public address is public: {e}");
    assert!(f.runner.start(&nmap(), "quick-scan", &vals(&[("target", "::ffff:127.0.0.1")]), &none()).await.is_ok(), "a mapped loopback address is loopback");
}

/// Everything real except the tool process: the practice server, the real redirect probe and the real run support.
#[tokio::test]
async fn end_to_end_a_real_redirect_from_the_practice_server_is_stopped_by_scope() {
    use crate::adapters::system_net::HttpRedirectProbe;
    use crate::ports::LabManager;
    let f = fixture(|_| Step::ok(&[]), &["zap.exe"]);
    let labs_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("mount-tests").join(format!("e2e-{}", uuid::Uuid::new_v4().simple()));
    let labs = Arc::new(PracticeLabs::new(&labs_dir, 0));
    labs.start(&crate::domain::LabId::new("practice-web").unwrap()).await.unwrap();
    let port = labs.bound_addr().unwrap().port();
    let support = Arc::new(RunSupport {
        settings: f.settings.clone(),
        labs: labs.clone(),
        history: f.store.clone(),
        resolver: Arc::new(SharedDns(f.dns.clone())),
        probe: Arc::new(HttpRedirectProbe),
        runs_dir: f.support.runs_dir.clone(),
    });
    let runner = NativeToolRunner::new(
        f.runner.inner.registry.clone(),
        f.exec.clone(),
        f.runner.inner.locator.clone(),
        f.store.clone(),
        f.runner.inner.activity.clone(),
        support,
        None,
    );
    let zap = ToolId::new("zap").unwrap();

    // The lab redirects to a public address: refused, and the mock process never started.
    let evil = format!("http://127.0.0.1:{port}/redirect?to=http%3A%2F%2F93.184.216.34%2F");
    let e = runner.start(&zap, "quick-scan", &vals(&[("url", &evil)]), &none()).await.unwrap_err();
    assert!(e.to_string().contains("redirects") && e.to_string().contains("93.184.216.34"), "{e}");
    // The same lab redirecting to one of its own pages is fine.
    let ok = format!("http://127.0.0.1:{port}/redirect?to=%2Fadmin%2F");
    let op = runner.start(&zap, "quick-scan", &vals(&[("url", &ok)]), &none()).await.unwrap();
    runner.wait(op, WAIT).await.unwrap();
    assert_eq!(f.exec.calls().len(), 1, "only the allowed run reached the process");
    // The default URL placeholder resolves to the running lab.
    let p = runner.preview(&zap, "quick-scan", &HashMap::new(), &none()).await.unwrap();
    assert!(p.command.args.iter().any(|a| a == &format!("http://127.0.0.1:{port}/")), "{:?}", p.command.args);
}

#[tokio::test]
async fn cleaning_up_stale_run_folders_never_follows_a_junction() {
    let f = fixture(|_| Step::ok(&[]), &["nmap.exe"]);
    let runs = f.support.runs_dir.clone();
    std::fs::create_dir_all(&runs).unwrap();
    // Something precious outside the runs folder, and a junction to it that is named like a run id.
    let precious = tempfile::tempdir().unwrap();
    std::fs::write(precious.path().join("keep.txt"), "do not delete").unwrap();
    let junction = runs.join(uuid::Uuid::new_v4().to_string());
    let made = std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(&junction).arg(precious.path()).output();
    if !made.map(|o| o.status.success()).unwrap_or(false) {
        eprintln!("skipped: could not create a junction");
        return;
    }
    // A genuinely stale run folder is removed; an in-history one and unrelated names are kept.
    let stale = runs.join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(stale.join("report.html"), "x").unwrap();
    let other = runs.join("not-a-run-id");
    std::fs::create_dir_all(&other).unwrap();
    f.support.remove_stale_run_dirs();
    assert!(precious.path().join("keep.txt").exists(), "the junction target must be untouched");
    assert!(!stale.exists(), "a stale run folder is removed");
    assert!(other.exists(), "folders that are not run ids are never touched");
    let _ = std::process::Command::new("cmd").args(["/C", "rmdir"]).arg(&junction).output();
}

#[tokio::test]
async fn shutdown_asks_every_active_run_to_stop_and_leaves_a_truthful_record() {
    let f = fixture(|_| Step::hang(&["scanning..."]), &["nmap.exe", "trivy.exe"]);
    let op = f.runner.start(&nmap(), "quick-scan", &HashMap::new(), &none()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(f.runner.active_count(), 1);
    assert_eq!(f.runner.cancel_all(), 1);
    let job = f.runner.wait(op, WAIT).await.unwrap();
    assert_eq!(job.phase, JobPhase::Cancelled);
    assert_eq!(f.store.get_run(op).unwrap().outcome, Some(RunOutcome::Cancelled));
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(f.runner.active_count(), 0);
    assert_eq!(f.runner.cancel_all(), 0);
}
