//! Docker Engine API over HTTP: container counts, or the state of one
//! container.
//!
//! The widget talks to an Engine API exposed over TCP, typically a
//! read-only docker-socket-proxy (`http://socket-proxy:2375`). Unix sockets
//! are handled by Docker discovery, not by this widget, because widget
//! requests always go through the HTTP [`WidgetContext`] and its host
//! allow-list.

use async_trait::async_trait;
use reqwest::Method;
use serde::Deserialize;

use crate::config::WidgetConfig;
use crate::context::WidgetContext;
use crate::data::{ListItem, Stat, StatValue, WidgetData};
use crate::error::WidgetError;
use crate::schema::{FieldSpec, FieldType, WidgetSchema};
use crate::widget::Widget;

/// Docker widget.
#[derive(Debug, Default, Clone, Copy)]
pub struct Docker;

/// One entry of `GET /containers/json` (only the fields used). Also used
/// by the Portainer widget, which proxies the same endpoint.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ContainerSummary {
    /// Names, each with a leading `/`.
    #[serde(default)]
    pub names: Vec<String>,
    /// Machine state: `created`, `running`, `paused`, `restarting`,
    /// `removing`, `exited` or `dead`.
    #[serde(default)]
    pub state: String,
    /// Human readable status, e.g. `Up 3 hours (healthy)`.
    #[serde(default)]
    pub status: String,
}

impl ContainerSummary {
    /// The first name without its leading `/`.
    pub fn name(&self) -> &str {
        self.names
            .first()
            .map(|n| n.trim_start_matches('/'))
            .unwrap_or_default()
    }
}

/// Running, stopped and total container counts.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ContainerCounts {
    /// Containers in state `running`.
    pub running: u64,
    /// Containers in state `exited`, `created` or `dead`.
    pub stopped: u64,
    /// All containers.
    pub total: u64,
}

/// Counts containers by state.
pub fn count_containers(containers: &[ContainerSummary]) -> ContainerCounts {
    let is = |states: &[&str]| {
        containers
            .iter()
            .filter(|c| states.contains(&c.state.as_str()))
            .count() as u64
    };
    ContainerCounts {
        running: is(&["running"]),
        stopped: is(&["exited", "created", "dead"]),
        total: containers.len() as u64,
    }
}

/// The count stats shared by the Docker and Portainer widgets.
pub fn count_stats(counts: ContainerCounts) -> Vec<Stat> {
    vec![
        Stat::count("running", "Running", counts.running),
        Stat::count("stopped", "Stopped", counts.stopped),
        Stat::count("total", "Total", counts.total),
    ]
}

/// Parses the container list into counts plus one item per container.
pub fn parse_containers(containers: &[ContainerSummary]) -> WidgetData {
    let items = containers
        .iter()
        .map(|c| {
            let subtitle = if c.status.is_empty() {
                &c.state
            } else {
                &c.status
            };
            ListItem::new(c.name()).subtitle(subtitle.clone())
        })
        .collect();
    WidgetData::new(count_stats(count_containers(containers))).with_items(items)
}

/// `GET /containers/{id}/json` (only the fields used).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ContainerInspect {
    /// Runtime state.
    #[serde(default)]
    pub state: ContainerState,
}

/// The `State` object of a container.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ContainerState {
    /// `running`, `exited`, ...
    #[serde(default)]
    pub status: String,
    /// RFC 3339 start time.
    #[serde(default)]
    pub started_at: String,
    /// Health check result, present only if the image defines one.
    #[serde(default)]
    pub health: Option<Health>,
}

/// The `State.Health` object.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Health {
    /// `starting`, `healthy` or `unhealthy`.
    #[serde(default)]
    pub status: String,
}

/// Parses one container's inspect output into status, start time and
/// (if present) health stats.
pub fn parse_inspect(inspect: &ContainerInspect) -> WidgetData {
    let state = &inspect.state;
    let mut stats = vec![Stat::new(
        "status",
        "Status",
        StatValue::Text(state.status.clone()),
    )];
    // Docker reports "0001-01-01T00:00:00Z" for containers never started.
    if !state.started_at.is_empty() && !state.started_at.starts_with("0001-") {
        // Trim fractional seconds: 2024-11-02T09:14:03.123456789Z → …03Z.
        let started = match state.started_at.split_once('.') {
            Some((head, _)) => format!("{head}Z"),
            None => state.started_at.clone(),
        };
        stats.push(Stat::new("started", "Started", StatValue::Text(started)));
    }
    if let Some(health) = state.health.as_ref().filter(|h| !h.status.is_empty()) {
        stats.push(Stat::new(
            "health",
            "Health",
            StatValue::Text(health.status.clone()),
        ));
    }
    WidgetData::new(stats)
}

/// `GET /version` (only the field used).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct EngineVersion {
    /// Engine version, e.g. `27.3.1`.
    #[serde(default)]
    pub version: String,
}

#[async_trait]
impl Widget for Docker {
    fn kind(&self) -> &'static str {
        "docker"
    }

    fn schema(&self) -> WidgetSchema {
        let mut schema = WidgetSchema::new(
            "docker",
            "Docker",
            "Running and stopped containers, or the state of one container.",
        )
        .field(
            FieldSpec::new("container", "Container", FieldType::Text)
                .help("Show only this container (name or ID). Leave empty for all containers."),
        );
        schema.fields[0].help = "Docker Engine API over HTTP, e.g. a read-only \
            docker-socket-proxy at http://socket-proxy:2375. Unix sockets are \
            handled by Docker discovery, not this widget."
            .into();
        schema
    }

    async fn fetch(
        &self,
        ctx: &WidgetContext,
        cfg: &WidgetConfig,
    ) -> Result<WidgetData, WidgetError> {
        match cfg.str("container") {
            Some(name) => {
                let mut url = cfg.endpoint("/containers")?;
                url.path_segments_mut()
                    .map_err(|()| WidgetError::InvalidConfig("`url` cannot be a base".into()))?
                    .push(name)
                    .push("json");
                let inspect: ContainerInspect = ctx
                    .send_json(ctx.request_url(cfg, Method::GET, url)?)
                    .await?;
                Ok(parse_inspect(&inspect))
            }
            None => {
                let list: Vec<ContainerSummary> =
                    ctx.get_json(cfg, "/containers/json?all=1", &[]).await?;
                Ok(parse_containers(&list))
            }
        }
    }

    async fn test_connection(
        &self,
        ctx: &WidgetContext,
        cfg: &WidgetConfig,
    ) -> Result<String, WidgetError> {
        let v: EngineVersion = ctx.get_json(cfg, "/version", &[]).await?;
        Ok(format!("Connected to Docker {}", v.version)
            .trim()
            .to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::fixture;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn value(d: &WidgetData, key: &str) -> StatValue {
        d.stat(key).unwrap().value.clone()
    }

    #[test]
    fn parses_container_list() {
        let list: Vec<ContainerSummary> =
            serde_json::from_value(fixture("docker/containers.json")).unwrap();
        assert_eq!(
            count_containers(&list),
            ContainerCounts {
                running: 3,
                stopped: 2,
                total: 6
            }
        );
        let d = parse_containers(&list);
        assert_eq!(value(&d, "running"), StatValue::Number(3.0));
        assert_eq!(d.items.len(), 6);
        assert_eq!(d.items[0].title, "jellyfin");
        assert_eq!(d.items[0].subtitle.as_deref(), Some("Up 3 days (healthy)"));
    }

    #[test]
    fn parses_inspect() {
        let inspect: ContainerInspect =
            serde_json::from_value(fixture("docker/container_inspect.json")).unwrap();
        let d = parse_inspect(&inspect);
        assert_eq!(value(&d, "status"), StatValue::Text("running".into()));
        assert_eq!(
            value(&d, "started"),
            StatValue::Text("2024-10-30T08:12:45Z".into())
        );
        assert_eq!(value(&d, "health"), StatValue::Text("healthy".into()));

        let never: ContainerInspect = serde_json::from_value(serde_json::json!({
            "State": {"Status": "created", "StartedAt": "0001-01-01T00:00:00Z"}
        }))
        .unwrap();
        let d = parse_inspect(&never);
        assert_eq!(d.stats.len(), 1);
    }

    #[tokio::test]
    async fn fetch_lists_and_inspects() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/containers/json"))
            .and(query_param("all", "1"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture("docker/containers.json")),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/containers/my%20app/json"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture("docker/container_inspect.json")),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/version"))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture("docker/version.json")))
            .mount(&server)
            .await;
        let ctx = WidgetContext::with_defaults().unwrap();
        let cfg = WidgetConfig::default().with("url", server.uri());
        let d = Docker.fetch(&ctx, &cfg).await.unwrap();
        assert_eq!(value(&d, "total"), StatValue::Number(6.0));

        let one = cfg.clone().with("container", "my app");
        let d = Docker.fetch(&ctx, &one).await.unwrap();
        assert_eq!(value(&d, "health"), StatValue::Text("healthy".into()));

        let msg = Docker.test_connection(&ctx, &cfg).await.unwrap();
        assert_eq!(msg, "Connected to Docker 27.3.1");
    }

    #[tokio::test]
    async fn container_name_cannot_escape_path() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/containers/..%2Fversion/json"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let ctx = WidgetContext::with_defaults().unwrap();
        let cfg = WidgetConfig::default()
            .with("url", server.uri())
            .with("container", "../version");
        let err = Docker.fetch(&ctx, &cfg).await.unwrap_err();
        assert!(
            matches!(err, WidgetError::Status { status: 404 }),
            "{err:?}"
        );
    }
}
