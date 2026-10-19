//! Learning mode: a guided tutorial per tool. Content is bundled TOML that is validated against the tool's manifest
//! when it is loaded, so a tutorial can never point at a profile, parameter, lab or link that does not exist.

use super::manifest::ToolManifest;
use super::{AppError, AppResult, ToolId};
use serde::{Deserialize, Serialize};

pub const LAB_IDS: &[&str] = &["practice-web", "sample-code"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TutorialStep {
    pub title: String,
    pub body: String,
    /// The launch profile this step uses, if any (the UI offers to open it).
    #[serde(default)]
    pub profile: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParameterNote {
    pub profile: String,
    pub name: String,
    pub explanation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExampleOutput {
    pub profile: String,
    /// Illustrative output, not a recording: real output differs by version and machine.
    pub output: String,
    pub interpretation: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommonError {
    pub symptom: String,
    pub cause: String,
    pub fix: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exercise {
    pub title: String,
    /// Which practice lab it uses (`practice-web` or `sample-code`).
    pub lab: String,
    pub profile: String,
    pub goal: String,
    pub steps: Vec<String>,
    pub expected: String,
    #[serde(default)]
    pub stretch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocLink {
    pub label: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tutorial {
    pub tool_id: ToolId,
    pub title: String,
    pub what_it_does: String,
    pub problem_solved: String,
    pub requirements: Vec<String>,
    /// Always shown first and cannot be empty.
    pub authorization_warning: String,
    pub first_run: Vec<TutorialStep>,
    pub parameters: Vec<ParameterNote>,
    pub example: ExampleOutput,
    pub common_errors: Vec<CommonError>,
    pub exercise: Exercise,
    pub links: Vec<DocLink>,
}

fn bad(m: impl Into<String>) -> AppError {
    AppError::Validation(format!("tutorial: {}", m.into()))
}

fn text(label: &str, s: &str, max: usize) -> AppResult<()> {
    if s.trim().is_empty() {
        return Err(bad(format!("{label} is empty")));
    }
    if s.len() > max {
        return Err(bad(format!("{label} is longer than {max} characters")));
    }
    if s.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
        return Err(bad(format!("{label} contains control characters")));
    }
    Ok(())
}

fn host_of(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let auth = rest.split(['/', '?', '#']).next()?;
    if auth.is_empty() || auth.contains('@') || auth.contains(':') {
        return None;
    }
    Some(auth.to_ascii_lowercase())
}

fn same_site(host: &str, official: &str) -> bool {
    let Some(o) = host_of(official) else { return false };
    let strip = |h: &str| h.strip_prefix("www.").unwrap_or(h).to_string();
    let (h, o) = (strip(host), strip(&o));
    h == o || h.ends_with(&format!(".{o}"))
}

impl Tutorial {
    /// Check the tutorial against the manifest it teaches.
    pub fn validate(&self, m: &ToolManifest) -> AppResult<()> {
        if self.tool_id != m.id {
            return Err(bad(format!("it is for '{}' but was checked against '{}'", self.tool_id, m.id)));
        }
        text("title", &self.title, 120)?;
        text("what_it_does", &self.what_it_does, 1200)?;
        text("problem_solved", &self.problem_solved, 1200)?;
        text("authorization_warning", &self.authorization_warning, 800)?;
        if !self.authorization_warning.to_lowercase().contains("permission") && !self.authorization_warning.to_lowercase().contains("authoris") {
            return Err(bad("the authorization warning must mention permission or authorisation"));
        }
        if self.requirements.is_empty() || self.requirements.len() > 10 {
            return Err(bad("list 1-10 requirements"));
        }
        for r in &self.requirements {
            text("a requirement", r, 400)?;
        }
        if !(3..=8).contains(&self.first_run.len()) {
            return Err(bad("the first-run walkthrough needs 3-8 steps"));
        }
        let has_profile = |p: &str| m.profile(p).is_some();
        for s in &self.first_run {
            text("a step title", &s.title, 120)?;
            text("a step body", &s.body, 1200)?;
            if let Some(p) = &s.profile {
                if !has_profile(p) {
                    return Err(bad(format!("a step refers to unknown profile '{p}'")));
                }
            }
        }
        if self.parameters.is_empty() {
            return Err(bad("explain at least one parameter"));
        }
        for n in &self.parameters {
            let Some(p) = m.profile(&n.profile) else { return Err(bad(format!("a parameter note refers to unknown profile '{}'", n.profile))) };
            if !p.parameters.iter().any(|d| d.name == n.name) {
                return Err(bad(format!("profile '{}' has no parameter '{}'", n.profile, n.name)));
            }
            text("a parameter explanation", &n.explanation, 800)?;
        }
        if !has_profile(&self.example.profile) {
            return Err(bad("the example refers to an unknown profile"));
        }
        text("example output", &self.example.output, 4000)?;
        if self.example.interpretation.is_empty() {
            return Err(bad("the example needs an interpretation"));
        }
        for i in &self.example.interpretation {
            text("an interpretation line", i, 500)?;
        }
        if self.common_errors.len() < 2 || self.common_errors.len() > 12 {
            return Err(bad("list 2-12 common errors"));
        }
        for e in &self.common_errors {
            text("an error symptom", &e.symptom, 300)?;
            text("an error cause", &e.cause, 500)?;
            text("an error fix", &e.fix, 500)?;
        }
        let x = &self.exercise;
        text("exercise title", &x.title, 120)?;
        text("exercise goal", &x.goal, 600)?;
        text("exercise expectation", &x.expected, 800)?;
        if !LAB_IDS.contains(&x.lab.as_str()) {
            return Err(bad(format!("the exercise uses unknown lab '{}'", x.lab)));
        }
        if !has_profile(&x.profile) {
            return Err(bad("the exercise refers to an unknown profile"));
        }
        if x.steps.is_empty() || x.steps.len() > 10 {
            return Err(bad("the exercise needs 1-10 steps"));
        }
        for s in &x.steps {
            text("an exercise step", s, 600)?;
        }
        if self.links.is_empty() || self.links.len() > 8 {
            return Err(bad("list 1-8 official links"));
        }
        let officials = [Some(m.homepage.as_str()), Some(m.documentation.as_str()), m.source_repository.as_deref()];
        for l in &self.links {
            text("a link label", &l.label, 120)?;
            let host = host_of(&l.url).ok_or_else(|| bad(format!("link '{}' must be a plain https address", l.url)))?;
            if !officials.iter().flatten().any(|o| same_site(&host, o)) {
                return Err(bad(format!("link '{}' is not on the tool's official sites", l.url)));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_and_site_matching_is_strict() {
        assert_eq!(host_of("https://nmap.org/book/man.html").as_deref(), Some("nmap.org"));
        assert_eq!(host_of("http://nmap.org"), None);
        assert_eq!(host_of("https://user@nmap.org/"), None);
        assert_eq!(host_of("https://nmap.org:8443/"), None);
        assert!(same_site("docs.nmap.org", "https://nmap.org"));
        assert!(same_site("zaproxy.org", "https://www.zaproxy.org"));
        assert!(!same_site("evilnmap.org", "https://nmap.org"));
        assert!(!same_site("nmap.org.evil.example", "https://nmap.org"));
    }
}
