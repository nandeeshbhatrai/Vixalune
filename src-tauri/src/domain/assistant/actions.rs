//! The only things the assistant can ask the application to do. A closed set with typed arguments: there is no "run a command"
//! action and no free-form argument list. A request is checked here for shape, then again by the executor against the registry,
//! and what it is allowed to do without a person's click is decided by [`Action::risk`], never by the model.

use super::packages::{valid_package_name, valid_version, Ecosystem};
use super::vuln::valid_vuln_id;
use crate::domain::manifest::{Category, ParamValue};
use crate::domain::{AppError, AppResult, ToolId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    SearchCatalog {
        query: String,
        #[serde(default)]
        category: Option<Category>,
        #[serde(default)]
        limit: Option<u8>,
    },
    GetToolDetails {
        tool_id: String,
    },
    CheckPrerequisites {
        tool_id: String,
    },
    GetInstallationStatus {
        tool_id: String,
    },
    /// Describes a run (command, scope, network use) without starting it. Never executes.
    PrepareToolRun {
        tool_id: String,
        profile_id: String,
        #[serde(default)]
        values: HashMap<String, ParamValue>,
    },
    /// Reads the stored output of a finished run the user names. Sending it to a model is a separate, explicit consent.
    ExplainScanResult {
        run_id: String,
    },
    QuerySupportedVulnerabilitySources {
        ecosystem: String,
        package: String,
        #[serde(default)]
        version: Option<String>,
        #[serde(default)]
        vulnerability_id: Option<String>,
    },
}

/// How much trust an action needs before it may run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    /// Reads the application's own catalog and records. May run without asking.
    ReadOnly,
    /// Sends the package name and version to a public vulnerability database. Needs the user's standing consent.
    ExternalLookup,
    /// Validates and describes a run. Changes nothing, but a run itself needs the user's explicit start.
    Prepares,
    /// Reads output from a past run and may send part of it to the model provider. Needs a click that names the run.
    ReadsLocalData,
}

impl Action {
    pub fn name(&self) -> &'static str {
        match self {
            Self::SearchCatalog { .. } => "search_catalog",
            Self::GetToolDetails { .. } => "get_tool_details",
            Self::CheckPrerequisites { .. } => "check_prerequisites",
            Self::GetInstallationStatus { .. } => "get_installation_status",
            Self::PrepareToolRun { .. } => "prepare_tool_run",
            Self::ExplainScanResult { .. } => "explain_scan_result",
            Self::QuerySupportedVulnerabilitySources { .. } => "query_supported_vulnerability_sources",
        }
    }

    pub fn risk(&self) -> Risk {
        match self {
            Self::SearchCatalog { .. } | Self::GetToolDetails { .. } | Self::CheckPrerequisites { .. } | Self::GetInstallationStatus { .. } => Risk::ReadOnly,
            Self::QuerySupportedVulnerabilitySources { .. } => Risk::ExternalLookup,
            Self::PrepareToolRun { .. } => Risk::Prepares,
            Self::ExplainScanResult { .. } => Risk::ReadsLocalData,
        }
    }

    /// Strict structural validation. Does not consult the registry or the file system.
    pub fn validate(&self) -> AppResult<()> {
        let bad = |m: &str| Err(AppError::Validation(m.to_string()));
        match self {
            Self::SearchCatalog { query, limit, .. } => {
                if query.trim().is_empty() || query.chars().count() > 200 || query.chars().any(|c| c.is_control()) {
                    return bad("the search text must be 1 to 200 characters without control characters");
                }
                if limit.is_some_and(|l| l == 0 || l > 10) {
                    return bad("the search limit must be 1 to 10");
                }
            }
            Self::GetToolDetails { tool_id } | Self::CheckPrerequisites { tool_id } | Self::GetInstallationStatus { tool_id } => {
                ToolId::new(tool_id.clone())?;
            }
            Self::PrepareToolRun { tool_id, profile_id, values } => {
                ToolId::new(tool_id.clone())?;
                if profile_id.is_empty() || profile_id.len() > 63 || !profile_id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
                    return bad("the profile id is not valid");
                }
                if values.len() > 24 {
                    return bad("too many parameters");
                }
                for (k, v) in values {
                    if k.is_empty() || k.len() > 40 || !k.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-') {
                        return bad("a parameter name is not valid");
                    }
                    if let ParamValue::Text(t) = v {
                        if t.chars().count() > 1000 || t.contains('\0') {
                            return bad("a parameter value is too long or contains invalid characters");
                        }
                    }
                }
            }
            Self::ExplainScanResult { run_id } => {
                if uuid::Uuid::parse_str(run_id).is_err() {
                    return bad("the run id is not valid");
                }
            }
            Self::QuerySupportedVulnerabilitySources { ecosystem, package, version, vulnerability_id } => {
                if vulnerability_id.is_none() {
                    if Ecosystem::parse(ecosystem).is_none() {
                        return Err(AppError::Unsupported(format!("the ecosystem '{}' is not covered by the vulnerability sources Vixalune can query. Supported: {}", ecosystem.chars().take(30).collect::<String>(), super::packages::supported_ecosystem_names().join(", "))));
                    }
                    if !valid_package_name(package) {
                        return bad("the package name is not valid");
                    }
                    match version {
                        None => return bad("a version is required: without it the result would not say whether your version is affected"),
                        Some(v) if !valid_version(v) => return bad("the version is not valid"),
                        _ => {}
                    }
                }
                if let Some(id) = vulnerability_id {
                    if !valid_vuln_id(id) {
                        return bad("the vulnerability identifier is not valid");
                    }
                }
            }
        }
        Ok(())
    }
}

/// An action the model suggested that passed validation. It is only a suggestion: nothing runs until the person chooses it in the
/// interface, and the executor checks the request again at that moment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProposedAction {
    pub action: Action,
    pub risk: Risk,
    /// Written by the application from the validated arguments, not by the model.
    pub summary: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn arguments_are_typed_and_unknown_fields_are_rejected() {
        let ok: Action = serde_json::from_value(json!({"action": "get_tool_details", "tool_id": "trivy"})).unwrap();
        assert!(ok.validate().is_ok());
        for bad in [
            json!({"action": "run_command", "command": "calc.exe"}),
            json!({"action": "get_tool_details", "tool_id": "trivy", "shell": "cmd"}),
            json!({"action": "prepare_tool_run", "tool_id": "trivy", "profile_id": "scan-folder", "args": ["--evil"]}),
            json!({"action": "search_catalog"}),
        ] {
            assert!(serde_json::from_value::<Action>(bad.clone()).is_err(), "{bad}");
        }
    }

    #[test]
    fn identifiers_and_values_are_validated_before_anything_else_sees_them() {
        let a = |v: serde_json::Value| serde_json::from_value::<Action>(v).unwrap().validate();
        assert!(a(json!({"action": "get_tool_details", "tool_id": "../etc"})).is_err());
        assert!(a(json!({"action": "get_tool_details", "tool_id": "Trivy; calc"})).is_err());
        assert!(a(json!({"action": "prepare_tool_run", "tool_id": "trivy", "profile_id": "scan folder", "values": {}})).is_err());
        assert!(a(json!({"action": "prepare_tool_run", "tool_id": "trivy", "profile_id": "scan-folder", "values": {"folder": "x\u{0000}y"}})).is_err());
        assert!(a(json!({"action": "explain_scan_result", "run_id": "not-a-uuid"})).is_err());
        assert!(a(json!({"action": "search_catalog", "query": "x", "limit": 50})).is_err());
        assert!(a(json!({"action": "query_supported_vulnerability_sources", "ecosystem": "npm", "package": "lodash"})).is_err(), "no version");
        assert!(a(json!({"action": "query_supported_vulnerability_sources", "ecosystem": "npm", "package": "lodash; id", "version": "1.0.0"})).is_err());
        assert!(a(json!({"action": "query_supported_vulnerability_sources", "ecosystem": "apt", "package": "openssl", "version": "1.0.0"})).is_err());
        assert!(a(json!({"action": "query_supported_vulnerability_sources", "ecosystem": "npm", "package": "lodash", "version": "4.17.15"})).is_ok());
        assert!(a(json!({"action": "query_supported_vulnerability_sources", "ecosystem": "npm", "package": "x", "vulnerability_id": "CVE-2021-23337"})).is_ok());
    }

    #[test]
    fn risk_is_decided_by_the_action_not_by_the_caller() {
        let r = |v: serde_json::Value| serde_json::from_value::<Action>(v).unwrap().risk();
        assert_eq!(r(json!({"action": "search_catalog", "query": "x"})), Risk::ReadOnly);
        assert_eq!(r(json!({"action": "query_supported_vulnerability_sources", "ecosystem": "npm", "package": "x", "version": "1.0.0"})), Risk::ExternalLookup);
        assert_eq!(r(json!({"action": "prepare_tool_run", "tool_id": "trivy", "profile_id": "p"})), Risk::Prepares);
        assert_eq!(r(json!({"action": "explain_scan_result", "run_id": "2f0e6a2c-3b0a-4d11-8d3e-6f9d3d2c9a11"})), Risk::ReadsLocalData);
    }
}
