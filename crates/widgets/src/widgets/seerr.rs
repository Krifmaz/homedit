//! Overseerr, Jellyseerr and Seerr: media request counts.
//!
//! The three apps share one API (Jellyseerr is a fork of Overseerr, and
//! Seerr is the merged successor), so they share one implementation and
//! differ only in kind and display name. Requests authenticate with the
//! `X-Api-Key` header.

use async_trait::async_trait;
use serde::Deserialize;

use crate::config::WidgetConfig;
use crate::context::WidgetContext;
use crate::data::{Stat, WidgetData};
use crate::error::WidgetError;
use crate::schema::WidgetSchema;
use crate::widget::Widget;
use crate::widgets::api_key_field;

const KEY_HELP: &str = "Settings → General → API Key.";

/// Response of `GET /api/v1/request/count`, only the fields used.
#[derive(Debug, Default, Deserialize)]
pub struct RequestCount {
    /// All requests.
    #[serde(default)]
    pub total: u64,
    /// Requests waiting for approval.
    #[serde(default)]
    pub pending: u64,
    /// Approved requests.
    #[serde(default)]
    pub approved: u64,
    /// Declined requests.
    #[serde(default)]
    pub declined: u64,
    /// Approved requests not yet available.
    #[serde(default)]
    pub processing: u64,
    /// Requests whose media is available.
    #[serde(default)]
    pub available: u64,
}

/// Response of `GET /api/v1/status`, only the fields used.
#[derive(Debug, Default, Deserialize)]
pub struct SeerrStatus {
    /// Application version.
    #[serde(default)]
    pub version: String,
}

/// Turns request counts into widget data.
pub fn parse_request_count(c: &RequestCount) -> WidgetData {
    WidgetData::new(vec![
        Stat::count("pending", "Pending", c.pending),
        Stat::count("approved", "Approved", c.approved),
        Stat::count("processing", "Processing", c.processing),
        Stat::count("available", "Available", c.available),
        Stat::count("total", "Total", c.total),
        Stat::count("declined", "Declined", c.declined),
    ])
}

fn schema(kind: &str, name: &str) -> WidgetSchema {
    WidgetSchema::new(
        kind,
        name,
        "Pending, approved, processing and available media requests.",
    )
    .field(api_key_field(KEY_HELP))
}

async fn fetch(ctx: &WidgetContext, cfg: &WidgetConfig) -> Result<WidgetData, WidgetError> {
    let key = cfg.require_str("key")?;
    let counts: RequestCount = ctx
        .get_json(cfg, "/api/v1/request/count", &[("X-Api-Key", key)])
        .await?;
    Ok(parse_request_count(&counts))
}

async fn test_connection(
    ctx: &WidgetContext,
    cfg: &WidgetConfig,
    name: &str,
) -> Result<String, WidgetError> {
    let key = cfg.require_str("key")?;
    // `/status` is public, so also hit an authenticated endpoint to check
    // the key.
    let headers = [("X-Api-Key", key)];
    let (status, _) = tokio::try_join!(
        ctx.get_json::<SeerrStatus>(cfg, "/api/v1/status", &headers),
        ctx.get_json::<RequestCount>(cfg, "/api/v1/request/count", &headers),
    )?;
    Ok(format!("Connected to {name} {}", status.version)
        .trim()
        .to_owned())
}

macro_rules! seerr_widget {
    ($(#[$doc:meta])* $ty:ident, $kind:literal, $name:literal) => {
        $(#[$doc])*
        #[derive(Debug, Default, Clone, Copy)]
        pub struct $ty;

        #[async_trait]
        impl Widget for $ty {
            fn kind(&self) -> &'static str {
                $kind
            }

            fn schema(&self) -> WidgetSchema {
                schema($kind, $name)
            }

            async fn fetch(
                &self,
                ctx: &WidgetContext,
                cfg: &WidgetConfig,
            ) -> Result<WidgetData, WidgetError> {
                fetch(ctx, cfg).await
            }

            async fn test_connection(
                &self,
                ctx: &WidgetContext,
                cfg: &WidgetConfig,
            ) -> Result<String, WidgetError> {
                test_connection(ctx, cfg, $name).await
            }
        }
    };
}

seerr_widget!(
    /// Overseerr: media request counts.
    Overseerr,
    "overseerr",
    "Overseerr"
);
seerr_widget!(
    /// Jellyseerr: media request counts.
    Jellyseerr,
    "jellyseerr",
    "Jellyseerr"
);
seerr_widget!(
    /// Seerr: media request counts.
    Seerr,
    "seerr",
    "Seerr"
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::StatValue;
    use crate::widgets::fixture;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn parses_recorded_counts() {
        let c: RequestCount = serde_json::from_value(fixture("seerr/request_count.json")).unwrap();
        let d = parse_request_count(&c);
        let keys: Vec<_> = d.stats.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "pending",
                "approved",
                "processing",
                "available",
                "total",
                "declined"
            ]
        );
        assert_eq!(d.stat("pending").unwrap().value, StatValue::Number(4.0));
        assert_eq!(d.stat("available").unwrap().value, StatValue::Number(171.0));
        assert_eq!(d.stat("total").unwrap().value, StatValue::Number(214.0));
    }

    #[test]
    fn kinds_share_schema_shape() {
        for w in [&Overseerr as &dyn Widget, &Jellyseerr, &Seerr] {
            let s = w.schema();
            assert_eq!(s.kind, w.kind());
            assert_eq!(s.fields[1].key, "key");
        }
    }

    #[tokio::test]
    async fn sends_api_key() {
        let server = MockServer::start().await;
        for (p, f) in [
            ("/api/v1/request/count", "seerr/request_count.json"),
            ("/api/v1/status", "seerr/status.json"),
        ] {
            Mock::given(method("GET"))
                .and(path(p))
                .and(header("X-Api-Key", "k3y"))
                .respond_with(ResponseTemplate::new(200).set_body_json(fixture(f)))
                .mount(&server)
                .await;
        }
        let ctx = WidgetContext::with_defaults().unwrap();
        let cfg = WidgetConfig::default()
            .with("url", server.uri())
            .with("key", "k3y");
        let d = Jellyseerr.fetch(&ctx, &cfg).await.unwrap();
        assert_eq!(d.stat("approved").unwrap().value, StatValue::Number(187.0));
        let msg = Overseerr.test_connection(&ctx, &cfg).await.unwrap();
        assert_eq!(msg, "Connected to Overseerr 2.7.3");
    }
}
