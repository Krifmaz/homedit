//! Read-only Docker discovery for homedit.
//!
//! This crate lists containers through the Docker Engine API and turns
//! gethomepage-compatible `homepage.*` labels into service cards, so a
//! dashboard can fill itself from the containers already running.
//!
//! * [`DockerClient`] talks to a Unix socket or a TCP socket proxy and only
//!   ever sends `GET` requests. Mount the socket read-only
//!   (`/var/run/docker.sock:/var/run/docker.sock:ro`), or better, point it at
//!   a socket proxy that only allows `GET /containers`.
//! * [`discover`] turns a list of [`Container`]s into [`DiscoveredService`]s.
//!   It is a pure function, so it is easy to test and to run on cached
//!   container lists.
//!
//! ```
//! use std::collections::BTreeMap;
//! use homedit_discovery::{discover, Container, DiscoverOptions};
//!
//! let container = Container {
//!     id: "abc".into(),
//!     names: vec!["/sonarr".into()],
//!     state: "running".into(),
//!     labels: BTreeMap::from([
//!         ("homepage.group".into(), "Media".into()),
//!         ("homepage.name".into(), "Sonarr".into()),
//!     ]),
//!     ..Container::default()
//! };
//! let found = discover(&[container], &DiscoverOptions::for_server("local"));
//! assert_eq!(found.services[0].service.name, "Sonarr");
//! assert_eq!(found.services[0].service.container.as_deref(), Some("sonarr"));
//! ```

mod client;
mod labels;
pub mod model;

use std::time::Duration;

use homedit_import::ImportWarning;
use serde_json::Value;

pub use client::{DockerClient, DockerEndpoint, MAX_BODY_BYTES};
pub use labels::{MAX_INDEX, PREFIX};
pub use model::{Container, ContainerState, DiscoveredService, Discovery, Health};

/// Errors from talking to Docker.
#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    /// The socket or host could not be reached.
    #[error("cannot connect to docker at {0}: {1}")]
    Connect(String, #[source] std::io::Error),
    /// The HTTP exchange failed.
    #[error("docker http error: {0}")]
    Http(#[source] hyper::Error),
    /// Reading the response body failed or it was too large.
    #[error("docker response body: {0}")]
    Body(String),
    /// Docker answered with an error status.
    #[error("docker returned status {0}: {1}")]
    Status(u16, String),
    /// The response was not the JSON we expected.
    #[error("cannot decode docker response: {0}")]
    Decode(#[source] serde_json::Error),
    /// The request took too long.
    #[error("docker request timed out after {0:?}")]
    Timeout(Duration),
    /// A request path was not valid.
    #[error("invalid request path: {0}")]
    InvalidPath(String),
}

/// Options for [`discover`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiscoverOptions {
    /// The Docker server's name (as in `docker.yaml`). Used as the card's
    /// `server` when the labels do not set one, so status and stats work.
    pub server: Option<String>,
    /// The gethomepage instance name. `homepage.instance.<name>.*` labels
    /// for this name override the plain labels.
    pub instance: Option<String>,
    /// Include containers that are not running.
    pub include_stopped: bool,
}

impl DiscoverOptions {
    /// Options for the Docker server called `server`, running containers
    /// only.
    pub fn for_server(server: impl Into<String>) -> Self {
        Self {
            server: Some(server.into()),
            ..Self::default()
        }
    }
}

/// Turns container labels into service cards.
///
/// A container produces a card when its labels set both `homepage.group`
/// and `homepage.name` (as in gethomepage). A container with only one of
/// them is reported in [`Discovery::warnings`]. `server` and `container`
/// default to the Docker server's name and the container's name. Results
/// are sorted by group, then `homepage.weight`, then name.
pub fn discover(containers: &[Container], options: &DiscoverOptions) -> Discovery {
    let mut out = Discovery::default();
    for container in containers {
        if !options.include_stopped && !container.state().is_up() {
            continue;
        }
        let has_labels = container.labels.keys().any(|k| k.starts_with(PREFIX));
        if !has_labels {
            continue;
        }
        let warn = |out: &mut Discovery, message: String| {
            out.warnings.push(ImportWarning {
                file: "docker labels".into(),
                path: container.name().to_string(),
                message,
            });
        };
        let (mut fields, problems) =
            labels::fields_from_labels(&container.labels, options.instance.as_deref());
        for problem in problems {
            warn(&mut out, problem);
        }
        let group = take_text(&mut fields, "group");
        let name = take_text(&mut fields, "name");
        let (Some(group), Some(name)) = (group, name) else {
            warn(
                &mut out,
                "has homepage labels but no `homepage.group` and `homepage.name`; skipped".into(),
            );
            continue;
        };
        if !fields.contains_key("container") {
            fields.insert("container".into(), Value::String(container.name().into()));
        }
        if let (Some(server), false) = (&options.server, fields.contains_key("server")) {
            fields.insert("server".into(), Value::String(server.clone()));
        }
        let (service, warnings) = homedit_import::service_from_fields(name, fields);
        out.warnings.extend(warnings);
        out.services.push(DiscoveredService {
            group,
            service,
            container_id: container.id.clone(),
            container_name: container.name().to_string(),
            state: container.state(),
        });
    }
    out.services.sort_by(|a, b| {
        a.group
            .cmp(&b.group)
            .then(
                a.service
                    .weight
                    .unwrap_or(0)
                    .cmp(&b.service.weight.unwrap_or(0)),
            )
            .then(a.service.name.cmp(&b.service.name))
    });
    out
}

fn take_text(fields: &mut serde_json::Map<String, Value>, key: &str) -> Option<String> {
    match fields.remove(key)? {
        Value::String(s) if !s.trim().is_empty() => Some(s),
        _ => None,
    }
}

/// Lists containers through `client` and runs [`discover`] on them.
pub async fn discover_from(
    client: &DockerClient,
    options: &DiscoverOptions,
) -> Result<Discovery, DiscoveryError> {
    let containers = client.list_containers(options.include_stopped).await?;
    Ok(discover(&containers, options))
}

impl DiscoveredService {
    /// The card as a [`homedit_core::Service`], ready to add to a group.
    pub fn to_core_service(&self) -> homedit_core::Service {
        homedit_import::to_core_service(self.service.clone())
    }
}
