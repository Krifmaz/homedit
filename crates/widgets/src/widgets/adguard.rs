//! AdGuard Home: DNS queries, blocked and filtered queries, and latency.
//!
//! AdGuard Home's control API uses HTTP basic authentication with the
//! admin username and password.

use async_trait::async_trait;
use reqwest::Method;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::config::WidgetConfig;
use crate::context::WidgetContext;
use crate::data::{Stat, StatValue, WidgetData};
use crate::error::WidgetError;
use crate::schema::WidgetSchema;
use crate::widget::Widget;
use crate::widgets::{password_field, username_field};

/// AdGuard Home: queries, blocked, filtered and average processing time.
#[derive(Debug, Default, Clone, Copy)]
pub struct AdGuard;

/// Response of `GET /control/stats`, only the fields used.
#[derive(Debug, Default, Deserialize)]
pub struct AdGuardStats {
    /// DNS queries in the statistics window.
    #[serde(default)]
    pub num_dns_queries: f64,
    /// Queries blocked by filter lists.
    #[serde(default)]
    pub num_blocked_filtering: f64,
    /// Queries blocked by safe browsing.
    #[serde(default)]
    pub num_replaced_safebrowsing: f64,
    /// Queries blocked by parental control.
    #[serde(default)]
    pub num_replaced_parental: f64,
    /// Average processing time of a query, in seconds.
    #[serde(default)]
    pub avg_processing_time: f64,
}

/// Response of `GET /control/status`, only the fields used.
#[derive(Debug, Default, Deserialize)]
pub struct AdGuardStatus {
    /// AdGuard Home version, e.g. `v0.107.57`.
    #[serde(default)]
    pub version: String,
}

/// Turns AdGuard Home's statistics into widget data.
///
/// `filtered` is the sum of safe browsing and parental control
/// replacements; latency is converted from seconds to milliseconds.
pub fn parse_stats(s: &AdGuardStats) -> WidgetData {
    WidgetData::new(vec![
        Stat::new("queries", "Queries", StatValue::Number(s.num_dns_queries)),
        Stat::new(
            "blocked",
            "Blocked",
            StatValue::Number(s.num_blocked_filtering),
        ),
        Stat::new(
            "filtered",
            "Filtered",
            StatValue::Number(s.num_replaced_safebrowsing + s.num_replaced_parental),
        ),
        Stat::new(
            "latency",
            "Latency",
            StatValue::Millis(s.avg_processing_time * 1000.0),
        ),
    ])
}

async fn get<T: DeserializeOwned>(
    ctx: &WidgetContext,
    cfg: &WidgetConfig,
    path: &str,
) -> Result<T, WidgetError> {
    let mut req = ctx.request(cfg, Method::GET, path)?;
    if let Some(user) = cfg.str("username") {
        req = req.basic_auth(user, cfg.str("password"));
    }
    ctx.send_json(req).await
}

#[async_trait]
impl Widget for AdGuard {
    fn kind(&self) -> &'static str {
        "adguard"
    }

    fn schema(&self) -> WidgetSchema {
        WidgetSchema::new(
            "adguard",
            "AdGuard Home",
            "DNS queries, blocked and filtered queries, and average latency.",
        )
        .field(username_field(false).help("AdGuard Home admin user."))
        .field(password_field(false))
    }

    async fn fetch(
        &self,
        ctx: &WidgetContext,
        cfg: &WidgetConfig,
    ) -> Result<WidgetData, WidgetError> {
        let stats: AdGuardStats = get(ctx, cfg, "/control/stats").await?;
        Ok(parse_stats(&stats))
    }

    async fn test_connection(
        &self,
        ctx: &WidgetContext,
        cfg: &WidgetConfig,
    ) -> Result<String, WidgetError> {
        let status: AdGuardStatus = get(ctx, cfg, "/control/status").await?;
        Ok(format!("Connected to AdGuard Home {}", status.version)
            .trim()
            .to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::fixture;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn parses_recorded_stats() {
        let s: AdGuardStats = serde_json::from_value(fixture("adguard/stats.json")).unwrap();
        let d = parse_stats(&s);
        assert_eq!(d.stat("queries").unwrap().value, StatValue::Number(26834.0));
        assert_eq!(d.stat("blocked").unwrap().value, StatValue::Number(3418.0));
        assert_eq!(d.stat("filtered").unwrap().value, StatValue::Number(30.0));
        match d.stat("latency").unwrap().value {
            StatValue::Millis(ms) => assert!((ms - 12.5).abs() < 1e-9),
            ref v => panic!("{v:?}"),
        }
    }

    #[test]
    fn parses_recorded_status() {
        let s: AdGuardStatus = serde_json::from_value(fixture("adguard/status.json")).unwrap();
        assert_eq!(s.version, "v0.107.57");
    }

    #[tokio::test]
    async fn sends_basic_auth() {
        let server = MockServer::start().await;
        // "admin:hunter2" in base64.
        let auth = "Basic YWRtaW46aHVudGVyMg==";
        Mock::given(method("GET"))
            .and(path("/control/stats"))
            .and(header("Authorization", auth))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture("adguard/stats.json")))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/control/status"))
            .and(header("Authorization", auth))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture("adguard/status.json")))
            .expect(1)
            .mount(&server)
            .await;
        let ctx = WidgetContext::with_defaults().unwrap();
        let cfg = WidgetConfig::default()
            .with("url", server.uri())
            .with("username", "admin")
            .with("password", "hunter2");
        let d = AdGuard.fetch(&ctx, &cfg).await.unwrap();
        assert_eq!(d.stat("blocked").unwrap().value, StatValue::Number(3418.0));
        let msg = AdGuard.test_connection(&ctx, &cfg).await.unwrap();
        assert_eq!(msg, "Connected to AdGuard Home v0.107.57");
    }

    #[tokio::test]
    async fn wrong_password_is_auth_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let ctx = WidgetContext::with_defaults().unwrap();
        let cfg = WidgetConfig::default()
            .with("url", server.uri())
            .with("username", "admin")
            .with("password", "bad");
        assert!(matches!(
            AdGuard.fetch(&ctx, &cfg).await,
            Err(WidgetError::Auth)
        ));
    }
}
