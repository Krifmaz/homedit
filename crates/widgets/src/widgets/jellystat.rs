//! Jellystat: plays per library type over the last N days.
//!
//! Authenticates with an API key sent as `x-api-token`. Data comes from
//! `/stats/getViewsByLibraryType`, which answers with play counts keyed by
//! library type (`Audio`, `Movie`, `Series`, `Other`). Newer Jellystat
//! releases serve it as `GET ?days=N`; older ones only as `POST` with a JSON
//! body `{"days": N}`, so the widget falls back to `POST` when `GET` is
//! refused.

use async_trait::async_trait;
use reqwest::Method;
use serde::Deserialize;

use crate::config::WidgetConfig;
use crate::context::WidgetContext;
use crate::data::{Stat, WidgetData};
use crate::error::WidgetError;
use crate::schema::{FieldSpec, FieldType, WidgetSchema};
use crate::widget::Widget;
use crate::widgets::api_key_field;

/// Default reporting window in days.
pub const DEFAULT_DAYS: u64 = 30;

/// Jellystat widget.
#[derive(Debug, Default, Clone, Copy)]
pub struct Jellystat;

/// `getViewsByLibraryType` response.
#[derive(Debug, Default, Deserialize)]
pub struct ViewsByType {
    /// Plays from music libraries.
    #[serde(default, rename = "Audio")]
    pub audio: u64,
    /// Plays from movie libraries.
    #[serde(default, rename = "Movie")]
    pub movie: u64,
    /// Plays from TV libraries.
    #[serde(default, rename = "Series")]
    pub series: u64,
    /// Plays from other libraries.
    #[serde(default, rename = "Other")]
    pub other: u64,
}

/// Turns the play counts into widget data.
pub fn parse_jellystat(v: &ViewsByType) -> WidgetData {
    WidgetData::new(vec![
        Stat::count("songs", "Songs", v.audio),
        Stat::count("movies", "Movies", v.movie),
        Stat::count("episodes", "Episodes", v.series),
        Stat::count("other", "Other", v.other),
    ])
}

#[async_trait]
impl Widget for Jellystat {
    fn kind(&self) -> &'static str {
        "jellystat"
    }

    fn schema(&self) -> WidgetSchema {
        WidgetSchema::new(
            "jellystat",
            "Jellystat",
            "Plays of songs, movies and episodes over the last days.",
        )
        .field(api_key_field("Jellystat → Settings → API Keys."))
        .field(
            FieldSpec::new(
                "days",
                "Days",
                FieldType::Number {
                    min: Some(1.0),
                    max: Some(3650.0),
                },
            )
            .default_value(DEFAULT_DAYS)
            .help("Reporting window in days."),
        )
    }

    async fn fetch(
        &self,
        ctx: &WidgetContext,
        cfg: &WidgetConfig,
    ) -> Result<WidgetData, WidgetError> {
        let key = cfg.require_str("key")?;
        let days = cfg.f64("days").map_or(DEFAULT_DAYS, |d| d.round() as u64);
        let get = ctx
            .request(
                cfg,
                Method::GET,
                &format!("/stats/getViewsByLibraryType?days={days}"),
            )?
            .header("x-api-token", key);
        let views: ViewsByType = match ctx.send_json(get).await {
            Err(WidgetError::Status { status: 404 | 405 }) => {
                let post = ctx
                    .request(cfg, Method::POST, "/stats/getViewsByLibraryType")?
                    .header("x-api-token", key)
                    .header("Content-Type", "application/json")
                    .body(serde_json::json!({ "days": days }).to_string());
                ctx.send_json(post).await?
            }
            other => other?,
        };
        Ok(parse_jellystat(&views))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::StatValue;
    use crate::widgets::fixture;
    use wiremock::matchers::{body_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn parses_recorded_response() {
        let v: ViewsByType =
            serde_json::from_value(fixture("jellystat/views_by_library_type.json")).unwrap();
        let d = parse_jellystat(&v);
        assert_eq!(d.stat("songs").unwrap().value, StatValue::Number(184.0));
        assert_eq!(d.stat("movies").unwrap().value, StatValue::Number(23.0));
        assert_eq!(d.stat("episodes").unwrap().value, StatValue::Number(97.0));
        assert_eq!(d.stat("other").unwrap().value, StatValue::Number(0.0));
    }

    #[tokio::test]
    async fn uses_get_with_days() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/stats/getViewsByLibraryType"))
            .and(query_param("days", "7"))
            .and(header("x-api-token", "js"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(fixture("jellystat/views_by_library_type.json")),
            )
            .mount(&server)
            .await;
        let ctx = WidgetContext::with_defaults().unwrap();
        let cfg = WidgetConfig::default()
            .with("url", server.uri())
            .with("key", "js")
            .with("days", 7);
        let d = Jellystat.fetch(&ctx, &cfg).await.unwrap();
        assert_eq!(d.stat("movies").unwrap().value, StatValue::Number(23.0));
    }

    #[tokio::test]
    async fn falls_back_to_post_on_older_servers() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/stats/getViewsByLibraryType"))
            .and(header("x-api-token", "js"))
            .and(body_json(serde_json::json!({"days": 30})))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(fixture("jellystat/views_by_library_type.json")),
            )
            .mount(&server)
            .await;
        let ctx = WidgetContext::with_defaults().unwrap();
        let cfg = WidgetConfig::default()
            .with("url", server.uri())
            .with("key", "js");
        let d = Jellystat.fetch(&ctx, &cfg).await.unwrap();
        assert_eq!(d.stat("songs").unwrap().value, StatValue::Number(184.0));
    }

    #[tokio::test]
    async fn bad_key_is_auth_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let ctx = WidgetContext::with_defaults().unwrap();
        let cfg = WidgetConfig::default()
            .with("url", server.uri())
            .with("key", "bad");
        let err = Jellystat.fetch(&ctx, &cfg).await.unwrap_err();
        assert!(matches!(err, WidgetError::Auth));
    }
}
