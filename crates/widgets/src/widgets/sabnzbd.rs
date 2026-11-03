//! SABnzbd: download rate, remaining size and time, and the queue.
//!
//! Authenticates with the `apikey` query parameter. SABnzbd answers bad
//! keys with HTTP 200 and a body like
//! `{"status": false, "error": "API Key Incorrect"}`, which is mapped to
//! [`WidgetError::Auth`].

use async_trait::async_trait;
use reqwest::Method;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::config::WidgetConfig;
use crate::context::WidgetContext;
use crate::data::{ListItem, Stat, StatValue, WidgetData};
use crate::error::WidgetError;
use crate::schema::WidgetSchema;
use crate::widget::Widget;
use crate::widgets::api_key_field;

/// Largest number of queue slots listed as items.
const MAX_ITEMS: usize = 10;

/// SABnzbd widget.
#[derive(Debug, Default, Clone, Copy)]
pub struct Sabnzbd;

/// `GET /api?mode=queue&output=json`.
#[derive(Debug, Default, Deserialize)]
pub struct QueueResponse {
    /// The queue.
    #[serde(default)]
    pub queue: Queue,
}

/// The `queue` object (only the fields used). SABnzbd sends most numbers
/// as strings; both forms are accepted.
#[derive(Debug, Default, Deserialize)]
pub struct Queue {
    /// Overall state, e.g. `Downloading`, `Paused` or `Idle`.
    #[serde(default)]
    pub status: String,
    /// Current rate in KiB per second.
    #[serde(default, deserialize_with = "lenient_f64")]
    pub kbpersec: f64,
    /// Remaining size in MiB.
    #[serde(default, deserialize_with = "lenient_f64")]
    pub mbleft: f64,
    /// Number of jobs in the queue.
    #[serde(default, deserialize_with = "lenient_f64")]
    pub noofslots: f64,
    /// Estimated time left, as `H:MM:SS` (or `D:HH:MM:SS`).
    #[serde(default)]
    pub timeleft: String,
    /// The queued jobs.
    #[serde(default)]
    pub slots: Vec<Slot>,
}

/// One queued job.
#[derive(Debug, Default, Deserialize)]
pub struct Slot {
    /// Job name.
    #[serde(default)]
    pub filename: String,
    /// Progress from 0 to 100.
    #[serde(default, deserialize_with = "lenient_f64")]
    pub percentage: f64,
    /// Job state, e.g. `Downloading` or `Queued`.
    #[serde(default)]
    pub status: String,
}

/// Accepts a JSON number, a numeric string or `null` (as 0).
fn lenient_f64<'de, D: Deserializer<'de>>(de: D) -> Result<f64, D::Error> {
    Ok(match Value::deserialize(de)? {
        Value::Number(n) => n.as_f64().unwrap_or_default(),
        Value::String(s) => s.trim().parse().unwrap_or_default(),
        _ => 0.0,
    })
}

/// Parses a SABnzbd time like `1:02:05` or `2:03:04:05` (days first) into
/// seconds. Returns `None` for anything else.
pub fn parse_timeleft(s: &str) -> Option<f64> {
    let parts: Vec<u64> = s
        .trim()
        .split(':')
        .map(|p| p.trim().parse().ok())
        .collect::<Option<_>>()?;
    let multipliers = [1, 60, 3600, 86_400];
    if parts.is_empty() || parts.len() > multipliers.len() {
        return None;
    }
    let secs = parts
        .iter()
        .rev()
        .zip(multipliers)
        .map(|(v, m)| v * m)
        .sum::<u64>();
    Some(secs as f64)
}

/// Maps SABnzbd's in-band error body to an error, if the body is one.
pub fn check_error(body: &Value) -> Result<(), WidgetError> {
    if body.get("status").and_then(Value::as_bool) != Some(false) {
        return Ok(());
    }
    let msg = body
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("unknown error");
    if msg.to_ascii_lowercase().contains("api key") {
        Err(WidgetError::Auth)
    } else {
        Err(WidgetError::Parse(format!("SABnzbd: {msg}")))
    }
}

/// Parses a queue response body into widget data.
///
/// Stats: rate (bytes/s), remaining size (bytes), queue length, time left
/// (seconds) and status. Items: the first queued jobs with their progress.
pub fn parse_sabnzbd(body: Value) -> Result<WidgetData, WidgetError> {
    check_error(&body)?;
    let QueueResponse { queue } = serde_json::from_value(body)?;
    let mut stats = vec![
        Stat::new("rate", "Rate", StatValue::Rate(queue.kbpersec * 1024.0)),
        Stat::new(
            "remaining",
            "Remaining",
            StatValue::Bytes(queue.mbleft * 1024.0 * 1024.0),
        ),
        Stat::count("queue", "Queue", queue.noofslots.max(0.0) as u64),
    ];
    if let Some(secs) = parse_timeleft(&queue.timeleft) {
        stats.push(Stat::new(
            "timeleft",
            "Time left",
            StatValue::Duration(secs),
        ));
    }
    if !queue.status.is_empty() {
        stats.push(Stat::new(
            "status",
            "Status",
            StatValue::Text(queue.status.clone()),
        ));
    }
    let items = queue
        .slots
        .iter()
        .take(MAX_ITEMS)
        .map(|s| {
            let item = ListItem::new(s.filename.clone()).progress(s.percentage);
            if s.status.is_empty() {
                item
            } else {
                item.subtitle(s.status.clone())
            }
        })
        .collect();
    Ok(WidgetData::new(stats).with_items(items))
}

/// Builds `/api?mode=<mode>&output=json&apikey=<key>` with the key
/// percent-encoded.
fn api_path(mode: &str, key: &str) -> String {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("mode", mode)
        .append_pair("output", "json")
        .append_pair("apikey", key)
        .finish();
    format!("/api?{query}")
}

async fn call(ctx: &WidgetContext, cfg: &WidgetConfig, mode: &str) -> Result<Value, WidgetError> {
    let key = cfg.require_str("key")?;
    let req = ctx.request(cfg, Method::GET, &api_path(mode, key))?;
    let body: Value = ctx.send_json(req).await?;
    check_error(&body)?;
    Ok(body)
}

#[async_trait]
impl Widget for Sabnzbd {
    fn kind(&self) -> &'static str {
        "sabnzbd"
    }

    fn schema(&self) -> WidgetSchema {
        WidgetSchema::new(
            "sabnzbd",
            "SABnzbd",
            "Download rate, remaining size, time left and the queue.",
        )
        .field(api_key_field("Config → General → Security → API Key."))
    }

    async fn fetch(
        &self,
        ctx: &WidgetContext,
        cfg: &WidgetConfig,
    ) -> Result<WidgetData, WidgetError> {
        parse_sabnzbd(call(ctx, cfg, "queue").await?)
    }

    async fn test_connection(
        &self,
        ctx: &WidgetContext,
        cfg: &WidgetConfig,
    ) -> Result<String, WidgetError> {
        let body = call(ctx, cfg, "version").await?;
        let version = body.get("version").and_then(Value::as_str).unwrap_or("");
        Ok(format!("Connected to SABnzbd {version}").trim().to_owned())
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
    fn parses_recorded_queue() {
        let d = parse_sabnzbd(fixture("sabnzbd/queue.json")).unwrap();
        assert_eq!(value(&d, "rate"), StatValue::Rate(10_485_760.0));
        assert_eq!(
            value(&d, "remaining"),
            StatValue::Bytes(3584.5 * 1024.0 * 1024.0)
        );
        assert_eq!(value(&d, "queue"), StatValue::Number(3.0));
        assert_eq!(value(&d, "timeleft"), StatValue::Duration(3725.0));
        assert_eq!(value(&d, "status"), StatValue::Text("Downloading".into()));
        assert_eq!(d.items.len(), 3);
        assert_eq!(d.items[0].title, "Big.Buck.Bunny.2008.1080p.BluRay.x264");
        assert_eq!(d.items[0].progress, Some(71.0));
    }

    #[test]
    fn api_key_error_is_auth() {
        let err = parse_sabnzbd(fixture("sabnzbd/error_apikey.json")).unwrap_err();
        assert!(matches!(err, WidgetError::Auth), "{err:?}");
    }

    #[test]
    fn parses_time_left() {
        assert_eq!(parse_timeleft("0:00:00"), Some(0.0));
        assert_eq!(parse_timeleft("1:02:05"), Some(3725.0));
        assert_eq!(parse_timeleft("1:00:00:01"), Some(86_401.0));
        assert_eq!(parse_timeleft(""), None);
        assert_eq!(parse_timeleft("soon"), None);
    }

    #[test]
    fn encodes_key() {
        assert_eq!(
            api_path("queue", "a&b c"),
            "/api?mode=queue&output=json&apikey=a%26b+c"
        );
    }

    #[tokio::test]
    async fn fetch_sends_api_key() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api"))
            .and(query_param("mode", "queue"))
            .and(query_param("apikey", "k3y&1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture("sabnzbd/queue.json")))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api"))
            .and(query_param("mode", "version"))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture("sabnzbd/version.json")))
            .mount(&server)
            .await;
        let ctx = WidgetContext::with_defaults().unwrap();
        let cfg = WidgetConfig::default()
            .with("url", server.uri())
            .with("key", "k3y&1");
        let d = Sabnzbd.fetch(&ctx, &cfg).await.unwrap();
        assert_eq!(value(&d, "queue"), StatValue::Number(3.0));
        let msg = Sabnzbd.test_connection(&ctx, &cfg).await.unwrap();
        assert_eq!(msg, "Connected to SABnzbd 4.3.3");
    }

    #[tokio::test]
    async fn wrong_key_is_auth() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture("sabnzbd/error_apikey.json")),
            )
            .mount(&server)
            .await;
        let ctx = WidgetContext::with_defaults().unwrap();
        let cfg = WidgetConfig::default()
            .with("url", server.uri())
            .with("key", "bad");
        let err = Sabnzbd.test_connection(&ctx, &cfg).await.unwrap_err();
        assert!(matches!(err, WidgetError::Auth), "{err:?}");
    }
}
