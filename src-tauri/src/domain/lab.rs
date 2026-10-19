//! Learning labs: practice targets that are safe by construction and run only on this computer.

use super::ids::LabId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabKind {
    /// A deliberately vulnerable web application served by Vixalune on 127.0.0.1.
    WebApp,
    /// A folder of small, intentionally insecure sample files for code-analysis exercises. Never executed.
    SampleProject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabDefinition {
    pub id: LabId,
    pub name: String,
    pub description: String,
    pub kind: LabKind,
    /// For web labs: the address once started (always loopback).
    pub address: Option<String>,
    /// For sample projects: the folder on disk.
    pub folder: Option<String>,
    /// What is deliberately wrong, so learners know what to look for.
    pub weaknesses: Vec<String>,
    pub warning: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabState {
    Stopped,
    Starting,
    Running,
    Stopping,
    Failed,
}

/// A lab with its current state, as shown in Learning Labs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabStatus {
    #[serde(flatten)]
    pub lab: LabDefinition,
    pub state: LabState,
}
