//! Types for Docker containers and the services discovered from them.

use std::collections::BTreeMap;

use homedit_import::{ImportWarning, Service};
use serde::{Deserialize, Serialize};

/// One container, as returned by `GET /containers/json`. Only the fields
/// homedit uses are decoded; the rest are ignored.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Container {
    /// The full container ID.
    pub id: String,
    /// Container names, each with a leading `/` as Docker reports them.
    #[serde(default)]
    pub names: Vec<String>,
    /// The image the container was created from.
    #[serde(default)]
    pub image: String,
    /// Machine-readable state: `running`, `exited`, `paused`, `restarting`,
    /// `created`, `removing` or `dead`.
    #[serde(default)]
    pub state: String,
    /// Human-readable status, such as `Up 3 hours (healthy)`.
    #[serde(default)]
    pub status: String,
    /// Container labels.
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
}

impl Container {
    /// The container's primary name without the leading `/`, or the short
    /// ID if it has no name.
    pub fn name(&self) -> &str {
        self.names
            .first()
            .map(|n| n.trim_start_matches('/'))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| &self.id[..self.id.len().min(12)])
    }

    /// The container's state as an enum.
    pub fn state(&self) -> ContainerState {
        ContainerState::parse(&self.state)
    }

    /// The health reported in the status text, if the image has a health
    /// check.
    pub fn health(&self) -> Option<Health> {
        if self.status.contains("(healthy)") {
            Some(Health::Healthy)
        } else if self.status.contains("(unhealthy)") {
            Some(Health::Unhealthy)
        } else if self.status.contains("(health: starting)") {
            Some(Health::Starting)
        } else {
            None
        }
    }
}

/// A container's lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContainerState {
    /// Running.
    Running,
    /// Paused.
    Paused,
    /// Restarting.
    Restarting,
    /// Created but never started.
    Created,
    /// Stopped.
    Exited,
    /// Being removed.
    Removing,
    /// Dead (failed removal).
    Dead,
    /// Any state this version does not know.
    Unknown,
}

impl ContainerState {
    /// Parses Docker's state string.
    pub fn parse(state: &str) -> Self {
        match state {
            "running" => Self::Running,
            "paused" => Self::Paused,
            "restarting" => Self::Restarting,
            "created" => Self::Created,
            "exited" => Self::Exited,
            "removing" => Self::Removing,
            "dead" => Self::Dead,
            _ => Self::Unknown,
        }
    }

    /// True when the container is running (paused does not count).
    pub fn is_up(self) -> bool {
        self == Self::Running
    }
}

/// Health check result from the container status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Health {
    /// The health check passes.
    Healthy,
    /// The health check fails.
    Unhealthy,
    /// The health check has not finished its first run.
    Starting,
}

/// A service card built from a container's labels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiscoveredService {
    /// The group to put the card in (`homepage.group`).
    pub group: String,
    /// The card, in the same shape the YAML importer produces.
    pub service: Service,
    /// The ID of the container the labels came from.
    pub container_id: String,
    /// The name of the container the labels came from.
    pub container_name: String,
    /// The container's state when it was discovered.
    pub state: ContainerState,
}

/// The result of discovery over a set of containers.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Discovery {
    /// Services found, sorted by group, then weight, then name.
    pub services: Vec<DiscoveredService>,
    /// Problems with labels (bad keys, missing group or name).
    pub warnings: Vec<ImportWarning>,
}
