//! Intentionally vulnerable local lab definitions. Labs are container-based, pinned by
//! digest, and published only on the loopback interface.

use super::ids::LabId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LabDefinition {
    pub id: LabId,
    pub name: String,
    pub description: String,
    pub image: String,
    pub digest: String,
    /// Container port -> host port. Always bound to 127.0.0.1.
    pub ports: Vec<LabPort>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabPort {
    pub container: u16,
    pub host: u16,
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
