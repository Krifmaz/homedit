//! Immich: users, photos, videos and storage used.
//!
//! Immich authenticates with an `x-api-key` header. The statistics endpoint
//! needs a key created by an admin user. Immich renamed its server routes in
//! release 1.118, so the widget has a `version` setting:
//!
//! | version | statistics                       | connection test             |
//! |---------|----------------------------------|-----------------------------|
//! | `2`     | `GET /api/server/statistics`     | `GET /api/server/about`     |
//! | `1`     | `GET /api/server-info/statistics`| `GET /api/server-info/version` |
//!
//! Both statistics routes return the same shape.

use async_trait::async_trait;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::config::WidgetConfig;
use crate::context::WidgetContext;
use crate::data::{Stat, StatValue, WidgetData};
use crate::error::WidgetError;
use crate::schema::{FieldSpec, FieldType, WidgetSchema};
use crate::widget::Widget;
use crate::widgets::api_key_field;

/// Header that carries the API key.
pub const KEY_HEADER: &str = "x-api-key";

/// Which generation of the Immich API to talk to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiVersion {
    /// Servers older than 1.118 (`/api/server-info/...`).
    V1,
    /// Servers from 1.118 on (`/api/server/...`).
    V2,
}

impl ApiVersion {
    /// Reads the `version` field; anything other than `1` means [`Self::V2`].
    pub fn from_config(cfg: &WidgetConfig) -> Self {
        match cfg.str("version") {
            Some("1") => Self::V1,
            _ => Self::V2,
        }
    }

    /// Path of the statistics endpoint.
    pub fn statistics_path(self) -> &'static str {
        match self {
            Self::V1 => "/api/server-info/statistics",
            Self::V2 => "/api/server/statistics",
        }
    }

    /// Path of the endpoint used by the connection test.
    pub fn test_path(self) -> &'static str {
        match self {
            Self::V1 => "/api/server-info/version",
            Self::V2 => "/api/server/about",
        }
    }
}

/// `GET /api/server/statistics` (v2) or `/api/server-info/statistics` (v1),
/// only the fields used.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatistics {
    /// Number of photos across all users.
    #[serde(default)]
    pub photos: u64,
    /// Number of videos across all users.
    #[serde(default)]
    pub videos: u64,
    /// Storage used, in bytes.
    #[serde(default)]
    pub usage: f64,
    /// Per-user breakdown; its length is the number of users.
    #[serde(default)]
    pub usage_by_user: Vec<serde_json::Value>,
}

/// Builds the widget data from the server statistics.
pub fn parse_statistics(stats: &ServerStatistics) -> WidgetData {
    WidgetData::new(vec![
        Stat::count("users", "Users", stats.usage_by_user.len() as u64),
        Stat::count("photos", "Photos", stats.photos),
        Stat::count("videos", "Videos", stats.videos),
        Stat::new("storage", "Storage", StatValue::Bytes(stats.usage)),
    ])
}

async fn get<T: DeserializeOwned>(
    ctx: &WidgetContext,
    cfg: &WidgetConfig,
    path: &str,
) -> Result<T, WidgetError> {
    let key = cfg.require_str("key")?;
    ctx.get_json(cfg, path, &[(KEY_HEADER, key)]).await
}

/// Immich: users, photos, videos and storage used.
#[derive(Debug, Default, Clone, Copy)]
pub struct Immich;

#[async_trait]
impl Widget for Immich {
    fn kind(&self) -> &'static str {
        "immich"
    }

    fn schema(&self) -> WidgetSchema {
        WidgetSchema::new(
            "immich",
            "Immich",
            "Number of users, photos and videos, and storage used.",
        )
        .field(api_key_field(
            "Account Settings → API Keys. The key must belong to an admin user.",
        ))
        .field(
            FieldSpec::new("version", "API version", FieldType::select(&["1", "2"]))
                .default_value("2")
                .help("Use 1 for Immich servers older than 1.118."),
        )
    }

    async fn fetch(
        &self,
        ctx: &WidgetContext,
        cfg: &WidgetConfig,
    ) -> Result<WidgetData, WidgetError> {
        let version = ApiVersion::from_config(cfg);
        let stats: ServerStatistics = get(ctx, cfg, version.statistics_path()).await?;
        Ok(parse_statistics(&stats))
    }

    async fn test_connection(
        &self,
        ctx: &WidgetContext,
        cfg: &WidgetConfig,
    ) -> Result<String, WidgetError> {
        let version = ApiVersion::from_config(cfg);
        let _: serde_json::Value = get(ctx, cfg, version.test_path()).await?;
        Ok("Connected to Immich".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::fixture;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn value(d: &WidgetData, key: &str) -> StatValue {
        d.stat(key).unwrap().value.clone()
    }

    #[test]
    fn parses_v2_statistics() {
        let s: ServerStatistics =
            serde_json::from_value(fixture("immich/statistics.json")).unwrap();
        let d = parse_statistics(&s);
        assert_eq!(value(&d, "users"), StatValue::Number(3.0));
        assert_eq!(value(&d, "photos"), StatValue::Number(48213.0));
        assert_eq!(value(&d, "videos"), StatValue::Number(2391.0));
        assert_eq!(value(&d, "storage"), StatValue::Bytes(412_316_860_416.0));
    }

    #[test]
    fn parses_v1_statistics() {
        let s: ServerStatistics =
            serde_json::from_value(fixture("immich/statistics_v1.json")).unwrap();
        let d = parse_statistics(&s);
        assert_eq!(value(&d, "users"), StatValue::Number(1.0));
        assert_eq!(value(&d, "photos"), StatValue::Number(1204.0));
        assert_eq!(value(&d, "storage"), StatValue::Bytes(9_876_543_210.0));
    }

    #[test]
    fn version_defaults_to_v2() {
        let cfg = WidgetConfig::default();
        assert_eq!(ApiVersion::from_config(&cfg), ApiVersion::V2);
        let cfg = cfg.with("version", "1");
        assert_eq!(ApiVersion::from_config(&cfg), ApiVersion::V1);
        let filled = WidgetConfig::default().with_defaults(&Immich.schema());
        assert_eq!(filled.str("version"), Some("2"));
    }

    async fn mount(server: &MockServer, p: &str, file: &str) {
        Mock::given(method("GET"))
            .and(path(p))
            .and(header(KEY_HEADER, "imm-key"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture(&format!("immich/{file}"))),
            )
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn fetch_end_to_end_both_versions() {
        let server = MockServer::start().await;
        mount(&server, "/api/server/statistics", "statistics.json").await;
        mount(&server, "/api/server/about", "about.json").await;
        mount(&server, "/api/server-info/statistics", "statistics_v1.json").await;
        Mock::given(method("GET"))
            .and(path("/api/server-info/version"))
            .and(header(KEY_HEADER, "imm-key"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"major": 1, "minor": 106, "patch": 4})),
            )
            .mount(&server)
            .await;

        let ctx = WidgetContext::with_defaults().unwrap();
        let v2 = WidgetConfig::default()
            .with("url", server.uri())
            .with("key", "imm-key")
            .with("version", "2");
        let d = Immich.fetch(&ctx, &v2).await.unwrap();
        assert_eq!(value(&d, "photos"), StatValue::Number(48213.0));
        assert_eq!(
            Immich.test_connection(&ctx, &v2).await.unwrap(),
            "Connected to Immich"
        );

        let v1 = v2.clone().with("version", "1");
        let d = Immich.fetch(&ctx, &v1).await.unwrap();
        assert_eq!(value(&d, "photos"), StatValue::Number(1204.0));
        assert!(Immich.test_connection(&ctx, &v1).await.is_ok());

        let bad = v2.with("key", "wrong");
        assert!(Immich.fetch(&ctx, &bad).await.is_err());
    }
}
