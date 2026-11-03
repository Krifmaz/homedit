//! Portainer: container counts of one environment.
//!
//! Authenticates with an access token in the `X-API-Key` header and reads
//! the Docker container list through Portainer's Docker proxy
//! (`/api/endpoints/{env}/docker/containers/json`).

use async_trait::async_trait;
use serde::Deserialize;

use crate::config::WidgetConfig;
use crate::context::WidgetContext;
use crate::data::WidgetData;
use crate::error::WidgetError;
use crate::schema::{FieldSpec, FieldType, WidgetSchema};
use crate::widget::Widget;
use crate::widgets::api_key_field;
use crate::widgets::docker::{ContainerSummary, count_containers, count_stats};

/// Portainer widget.
#[derive(Debug, Default, Clone, Copy)]
pub struct Portainer;

/// `GET /api/status` (only the field used).
#[derive(Debug, Default, Deserialize)]
pub struct PortainerStatus {
    /// Portainer version, e.g. `2.21.4`.
    #[serde(default, rename = "Version")]
    pub version: String,
}

/// Parses an environment's container list into running, stopped and total
/// counts.
pub fn parse_portainer(containers: &[ContainerSummary]) -> WidgetData {
    WidgetData::new(count_stats(count_containers(containers)))
}

/// The environment id as a path segment.
fn env_id(cfg: &WidgetConfig) -> Result<u64, WidgetError> {
    match cfg.f64("env") {
        Some(n) if n >= 0.0 && n.fract() == 0.0 => Ok(n as u64),
        Some(_) => Err(WidgetError::InvalidConfig(
            "`env` must be a whole number".into(),
        )),
        None => Err(WidgetError::InvalidConfig("`env` is required".into())),
    }
}

#[async_trait]
impl Widget for Portainer {
    fn kind(&self) -> &'static str {
        "portainer"
    }

    fn schema(&self) -> WidgetSchema {
        WidgetSchema::new(
            "portainer",
            "Portainer",
            "Running, stopped and total containers of one environment.",
        )
        .field(api_key_field(
            "Access token from My account → Access tokens.",
        ))
        .field(
            FieldSpec::new(
                "env",
                "Environment ID",
                FieldType::Number {
                    min: Some(0.0),
                    max: None,
                },
            )
            .required()
            .help("The number in the environment's URL, e.g. 2 in #!/2/docker/dashboard."),
        )
    }

    async fn fetch(
        &self,
        ctx: &WidgetContext,
        cfg: &WidgetConfig,
    ) -> Result<WidgetData, WidgetError> {
        let key = cfg.require_str("key")?;
        let env = env_id(cfg)?;
        let list: Vec<ContainerSummary> = ctx
            .get_json(
                cfg,
                &format!("/api/endpoints/{env}/docker/containers/json?all=1"),
                &[("X-API-Key", key)],
            )
            .await?;
        Ok(parse_portainer(&list))
    }

    async fn test_connection(
        &self,
        ctx: &WidgetContext,
        cfg: &WidgetConfig,
    ) -> Result<String, WidgetError> {
        // `/api/status` is public, so also run the authenticated fetch to
        // check the key and the environment id.
        self.fetch(ctx, cfg).await?;
        let key = cfg.require_str("key")?;
        let s: PortainerStatus = ctx
            .get_json(cfg, "/api/status", &[("X-API-Key", key)])
            .await?;
        Ok(format!("Connected to Portainer {}", s.version)
            .trim()
            .to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::StatValue;
    use crate::widgets::fixture;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn num(d: &WidgetData, key: &str) -> f64 {
        match d.stat(key).unwrap().value {
            StatValue::Number(n) => n,
            ref v => panic!("{key}: {v:?}"),
        }
    }

    #[test]
    fn parses_recorded_containers() {
        let list: Vec<ContainerSummary> =
            serde_json::from_value(fixture("portainer/containers.json")).unwrap();
        let d = parse_portainer(&list);
        assert_eq!(num(&d, "running"), 2.0);
        assert_eq!(num(&d, "stopped"), 2.0);
        assert_eq!(num(&d, "total"), 5.0);
    }

    #[test]
    fn env_must_be_whole() {
        let cfg = WidgetConfig::default().with("env", 2);
        assert_eq!(env_id(&cfg).unwrap(), 2);
        assert!(env_id(&WidgetConfig::default().with("env", 1.5)).is_err());
        assert!(env_id(&WidgetConfig::default()).is_err());
    }

    #[tokio::test]
    async fn fetch_sends_api_key() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/endpoints/2/docker/containers/json"))
            .and(query_param("all", "1"))
            .and(header("x-api-key", "ptr_s3cret"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture("portainer/containers.json")),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/status"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture("portainer/status.json")),
            )
            .mount(&server)
            .await;
        let ctx = WidgetContext::with_defaults().unwrap();
        let cfg = WidgetConfig::default()
            .with("url", server.uri())
            .with("key", "ptr_s3cret")
            .with("env", 2);
        let d = Portainer.fetch(&ctx, &cfg).await.unwrap();
        assert_eq!(num(&d, "total"), 5.0);
        let msg = Portainer.test_connection(&ctx, &cfg).await.unwrap();
        assert_eq!(msg, "Connected to Portainer 2.21.4");

        // Any other key is rejected, as Portainer does.
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(401))
            .with_priority(10)
            .mount(&server)
            .await;
        let wrong = cfg.clone().with("key", "nope");
        let err = Portainer.fetch(&ctx, &wrong).await.unwrap_err();
        assert!(matches!(err, WidgetError::Auth), "{err:?}");
    }
}
