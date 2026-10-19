//! Shared pieces of launching a tool, used by every runner: building the validated command, checking scope
//! (with a fresh name resolution and plain-http redirect hops), checking file-system parameters, and keeping the
//! run history.

use crate::domain::manifest::{BuildContext, BuiltCommand, NetworkActivity, ParamValue, PathKind, RunSpec};
use crate::domain::runs::{LaunchDefaults, RunOutcome, RunRecord};
use crate::domain::scope::{assess_target, parse_target, Locality, NameResolver, RedirectHop, ScopeAck, ScopeReport, TargetForm, Verdict};
use crate::domain::{AppError, AppResult, OperationId};
use crate::ports::{LabInfo, RedirectProbe, RunHistory, SettingsProvider};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const MAX_REDIRECT_HOPS: usize = 5;

pub struct RunSupport {
    pub settings: Arc<dyn SettingsProvider>,
    pub labs: Arc<dyn LabInfo>,
    pub history: Arc<dyn RunHistory>,
    pub resolver: Arc<dyn NameResolver>,
    pub probe: Arc<dyn RedirectProbe>,
    /// Parent of the per-run folders (`<runs_dir>/<operation id>`).
    pub runs_dir: PathBuf,
}

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn bad(m: impl Into<String>) -> AppError {
    AppError::Validation(m.into())
}

impl RunSupport {
    pub fn run_dir(&self, op: OperationId) -> PathBuf {
        self.runs_dir.join(op.to_string())
    }

    /// Validate every parameter and build the argument array. Lab placeholders are replaced here.
    pub fn build(&self, profile: &RunSpec, values: &HashMap<String, ParamValue>, run_dir: &Path) -> AppResult<BuiltCommand> {
        let ctx = BuildContext { run_dir: Some(run_dir.display().to_string()), placeholders: self.labs.placeholders() };
        crate::domain::manifest::build_command_with(profile, values, &ctx)
    }

    /// Form defaults for a profile. Values that are not lab placeholders pass through unchanged.
    pub fn defaults(&self, profile: &RunSpec) -> LaunchDefaults {
        let ph = self.labs.placeholders();
        let resolve = |s: &str| ph.get(s).cloned().unwrap_or_else(|| s.to_string());
        let parameters = profile
            .parameters
            .iter()
            .filter_map(|p| {
                p.default.as_ref().map(|d| {
                    let v = match d {
                        ParamValue::Text(t) => ParamValue::Text(resolve(t)),
                        other => other.clone(),
                    };
                    (p.name.clone(), v)
                })
            })
            .collect();
        let mounts = profile.suggested_mounts.iter().map(|m| (m.name.clone(), resolve(&m.value))).collect();
        LaunchDefaults { parameters, mounts }
    }

    /// Classify every network target and follow plain-http redirects hop by hop. A hop is only contacted while
    /// everything before it was allowed without confirmation, so a redirect can never make Vixalune talk to an
    /// out-of-scope host.
    pub async fn assess_scope(&self, built: &BuiltCommand) -> AppResult<Option<ScopeReport>> {
        if built.targets.is_empty() {
            return Ok(None);
        }
        let settings = self.settings.get();
        let policy = settings.policy()?;
        let mut assessed = Vec::new();
        for (param, form, canonical) in &built.targets {
            let parsed = parse_target(canonical, *form)?;
            assessed.push(assess_target(param, &parsed, *form, &policy, self.resolver.as_ref()));
        }
        let mut report = ScopeReport::build(assessed, settings.scope.lab_targets.clone());

        let mut hops = Vec::new();
        for t in report.targets.iter().filter(|t| t.form == TargetForm::Url && t.verdict == Verdict::Allowed && t.canonical.starts_with("http://")) {
            let mut current = t.canonical.clone();
            for _ in 0..MAX_REDIRECT_HOPS {
                let Some(next) = self.probe.head(&current).await? else { break };
                match parse_target(&next, TargetForm::Url) {
                    Ok(p) => {
                        let a = assess_target("redirect", &p, TargetForm::Url, &policy, self.resolver.as_ref());
                        hops.push(RedirectHop { from: current.clone(), to: p.canonical.clone(), locality: a.locality });
                        if a.verdict != Verdict::Allowed || !p.canonical.starts_with("http://") {
                            break;
                        }
                        current = p.canonical;
                    }
                    Err(_) => {
                        // A destination that cannot be understood is treated as out of scope.
                        hops.push(RedirectHop { from: current.clone(), to: next, locality: Locality::Unspecified });
                        break;
                    }
                }
            }
        }
        report.redirects = hops;
        Ok(Some(report))
    }

    /// Re-assess and enforce. Run this at start time (not only at preview) so nothing can change in between.
    pub async fn enforce_scope(&self, built: &BuiltCommand, ack: Option<&ScopeAck>) -> AppResult<Option<ScopeReport>> {
        let report = self.assess_scope(built).await?;
        if let Some(r) = &report {
            r.check(ack)?;
        }
        Ok(report)
    }

    pub fn scope_summary(report: Option<&ScopeReport>, activity: NetworkActivity) -> String {
        match report {
            None => match activity {
                NetworkActivity::DownloadsData => "no hosts targeted (downloads public data)".into(),
                _ => "no network targets".into(),
            },
            Some(r) => match r.verdict {
                Verdict::Allowed => {
                    if r.targets.iter().all(|t| t.locality == Locality::Loopback) {
                        "this computer (loopback)".into()
                    } else {
                        "your configured lab".into()
                    }
                }
                Verdict::NeedsConfirmation => {
                    if r.targets.iter().any(|t| t.locality == Locality::Public) {
                        "confirmed: public address".into()
                    } else {
                        "confirmed: private network outside the lab list".into()
                    }
                }
                Verdict::Blocked => "blocked".into(),
            },
        }
    }

    /// Check file-system parameters against the real file system. Returns the canonical paths by parameter.
    pub fn check_paths(&self, built: &BuiltCommand, home: Option<&Path>) -> AppResult<Vec<(String, String)>> {
        let mut out = Vec::new();
        for (param, kind, must_exist, path) in &built.paths {
            let p = Path::new(path);
            if !p.exists() {
                if *must_exist {
                    return Err(bad(format!("'{param}': '{path}' does not exist")));
                }
                continue;
            }
            match kind {
                PathKind::Directory => {
                    out.push((param.clone(), crate::adapters::container_runner::canonical_mount(path, home).map_err(|e| bad(format!("'{param}': {e}")))?));
                }
                PathKind::File => {
                    let canon = std::fs::canonicalize(p).map_err(|e| bad(format!("'{param}': '{path}' cannot be used: {e}")))?;
                    if !canon.is_file() {
                        return Err(bad(format!("'{param}': '{path}' is not a file")));
                    }
                    let s = canon.to_string_lossy().to_string();
                    if s.starts_with("\\\\?\\UNC\\") {
                        return Err(bad(format!("'{param}': network files are not allowed")));
                    }
                    out.push((param.clone(), s.strip_prefix("\\\\?\\").unwrap_or(&s).to_string()));
                }
            }
        }
        Ok(out)
    }

    // -------------------------------------------------------------- history

    pub fn record_start(&self, r: &RunRecord) {
        if let Err(e) = self.history.put_run(r) {
            tracing::warn!("could not record run {}: {e}", r.id);
        }
    }

    pub fn record_finish(&self, op: OperationId, exit_code: Option<i32>, outcome: RunOutcome) {
        let result = self.history.get_run(op).and_then(|mut r| {
            let finished = now();
            r.duration_ms = chrono::DateTime::parse_from_rfc3339(&r.started_at)
                .ok()
                .zip(chrono::DateTime::parse_from_rfc3339(&finished).ok())
                .map(|(a, b)| (b - a).num_milliseconds().max(0) as u64);
            r.finished_at = Some(finished);
            r.exit_code = exit_code;
            r.outcome = Some(outcome);
            self.history.put_run(&r)
        });
        if let Err(e) = result {
            tracing::warn!("could not finish run record {op}: {e}");
        }
    }

    /// Apply the retention settings: delete expired history and output.
    pub fn purge_expired(&self) -> AppResult<crate::domain::runs::PurgeReport> {
        let s = self.settings.get();
        let cutoff = |days: u32| (chrono::Utc::now() - chrono::Duration::days(i64::from(days))).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        self.history.purge(&cutoff(s.retention.history_days), &cutoff(s.retention.output_days))
    }

    /// Run folders of runs whose history is gone are removed too (they may hold result files).
    pub fn remove_stale_run_dirs(&self) {
        let keep: std::collections::HashSet<String> = self.history.list_runs(10_000).map(|v| v.into_iter().map(|r| r.id.to_string()).collect()).unwrap_or_default();
        let Ok(rd) = std::fs::read_dir(&self.runs_dir) else { return };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if e.path().is_dir() && uuid::Uuid::parse_str(&name).is_ok() && !keep.contains(&name) {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
}
