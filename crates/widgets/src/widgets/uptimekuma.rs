//! Uptime Kuma: monitors up and down on a public status page.
//!
//! Uptime Kuma has no API-key protected stats endpoint; instead the widget
//! reads a published status page, which needs no credentials:
//!
//! - `GET /api/status-page/{slug}` lists the monitors, grouped,
//! - `GET /api/status-page/heartbeat/{slug}` holds recent heartbeats per
//!   monitor id and the 24 hour uptime ratios (keyed `"{id}_24"`).

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use reqwest::Method;
use serde::Deserialize;
use url::Url;

use crate::config::WidgetConfig;
use crate::context::WidgetContext;
use crate::data::{ListItem, Stat, StatValue, WidgetData};
use crate::error::WidgetError;
use crate::schema::{FieldSpec, FieldType, WidgetSchema};
use crate::widget::Widget;

/// Heartbeat status of a monitor that is down.
pub const STATUS_DOWN: u8 = 0;
/// Heartbeat status of a monitor that is up.
pub const STATUS_UP: u8 = 1;
/// Heartbeat status of a monitor that is pending (retrying).
pub const STATUS_PENDING: u8 = 2;
/// Heartbeat status of a monitor under maintenance.
pub const STATUS_MAINTENANCE: u8 = 3;

/// Uptime Kuma: monitors up and down and the average 24 hour uptime of one
/// status page.
#[derive(Debug, Default, Clone, Copy)]
pub struct UptimeKuma;

/// Response of `GET /api/status-page/{slug}`, only the fields used.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusPage {
    /// Monitor groups shown on the page.
    #[serde(default)]
    pub public_group_list: Vec<MonitorGroup>,
}

/// A group of monitors on a status page.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorGroup {
    /// Group name.
    #[serde(default)]
    pub name: String,
    /// Monitors in the group.
    #[serde(default)]
    pub monitor_list: Vec<Monitor>,
}

/// A monitor listed on a status page.
#[derive(Debug, Default, Deserialize)]
pub struct Monitor {
    /// Monitor id, the key into the heartbeat and uptime maps.
    #[serde(default)]
    pub id: u64,
    /// Display name.
    #[serde(default)]
    pub name: String,
}

/// Response of `GET /api/status-page/heartbeat/{slug}`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Heartbeats {
    /// Recent heartbeats per monitor id (as a string).
    #[serde(default)]
    pub heartbeat_list: HashMap<String, Vec<Heartbeat>>,
    /// Uptime ratios (0 to 1) keyed `"{id}_24"` for the last 24 hours.
    #[serde(default)]
    pub uptime_list: HashMap<String, f64>,
}

/// One heartbeat of a monitor.
#[derive(Debug, Default, Deserialize)]
pub struct Heartbeat {
    /// [`STATUS_DOWN`], [`STATUS_UP`], [`STATUS_PENDING`] or
    /// [`STATUS_MAINTENANCE`].
    #[serde(default)]
    pub status: u8,
    /// Time of the check, `YYYY-MM-DD HH:MM:SS.mmm` (sorts as text).
    #[serde(default)]
    pub time: String,
    /// Message of the check, e.g. an error.
    #[serde(default)]
    pub msg: Option<String>,
    /// Response time in milliseconds, if measured.
    #[serde(default)]
    pub ping: Option<f64>,
}

/// Combines a status page and its heartbeats into widget data.
///
/// A monitor's state is taken from its most recent heartbeat; monitors
/// with no heartbeat yet count as neither up nor down. The uptime is the
/// average of the 24 hour ratios of the listed monitors that have one.
/// Monitors that are down are listed as items, with the last check
/// message as subtitle.
pub fn parse_status(page: &StatusPage, beats: &Heartbeats) -> WidgetData {
    let mut seen = HashSet::new();
    let mut up = 0u64;
    let mut down = 0u64;
    let mut uptime_sum = 0.0;
    let mut uptime_n = 0u32;
    let mut items = Vec::new();

    let monitors = page
        .public_group_list
        .iter()
        .flat_map(|g| g.monitor_list.iter());
    for monitor in monitors {
        if !seen.insert(monitor.id) {
            continue;
        }
        let id = monitor.id.to_string();
        let latest = beats
            .heartbeat_list
            .get(&id)
            .and_then(|list| list.iter().max_by(|a, b| a.time.cmp(&b.time)));
        match latest.map(|b| b.status) {
            Some(STATUS_UP) => up += 1,
            Some(STATUS_DOWN) => {
                down += 1;
                let mut item = ListItem::new(monitor.name.clone());
                if let Some(msg) = latest
                    .and_then(|b| b.msg.as_deref())
                    .filter(|m| !m.is_empty())
                {
                    item = item.subtitle(msg);
                }
                items.push(item);
            }
            _ => {}
        }
        if let Some(ratio) = beats.uptime_list.get(&format!("{id}_24")) {
            uptime_sum += ratio;
            uptime_n += 1;
        }
    }

    let uptime = if uptime_n == 0 {
        StatValue::Text("-".into())
    } else {
        StatValue::Percent(uptime_sum / f64::from(uptime_n) * 100.0)
    };
    WidgetData::new(vec![
        Stat::count("up", "Up", up),
        Stat::count("down", "Down", down),
        Stat::new("uptime", "Uptime", uptime),
    ])
    .with_items(items)
}

/// Builds `{base}/{prefix}/{slug}` with the slug percent-encoded as a
/// single path segment.
fn slug_url(cfg: &WidgetConfig, prefix: &str, slug: &str) -> Result<Url, WidgetError> {
    let mut url = cfg.endpoint(prefix)?;
    url.path_segments_mut()
        .map_err(|()| WidgetError::InvalidConfig("`url` cannot have a path".into()))?
        .pop_if_empty()
        .push(slug);
    Ok(url)
}

#[async_trait]
impl Widget for UptimeKuma {
    fn kind(&self) -> &'static str {
        "uptimekuma"
    }

    fn schema(&self) -> WidgetSchema {
        WidgetSchema::new(
            "uptimekuma",
            "Uptime Kuma",
            "Monitors up and down and average uptime of a status page.",
        )
        .field(
            FieldSpec::new("slug", "Status page slug", FieldType::Text)
                .required()
                .help(
                    "The last part of the status page address, e.g. `homelab` for /status/homelab.",
                ),
        )
    }

    async fn fetch(
        &self,
        ctx: &WidgetContext,
        cfg: &WidgetConfig,
    ) -> Result<WidgetData, WidgetError> {
        let slug = cfg.require_str("slug")?;
        let page_req =
            ctx.request_url(cfg, Method::GET, slug_url(cfg, "/api/status-page", slug)?)?;
        let beats_req = ctx.request_url(
            cfg,
            Method::GET,
            slug_url(cfg, "/api/status-page/heartbeat", slug)?,
        )?;
        let (page, beats) = tokio::try_join!(
            ctx.send_json::<StatusPage>(page_req),
            ctx.send_json::<Heartbeats>(beats_req),
        )?;
        Ok(parse_status(&page, &beats))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::fixture;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn parses_recorded_status_page() {
        let page: StatusPage =
            serde_json::from_value(fixture("uptimekuma/status_page.json")).unwrap();
        let beats: Heartbeats =
            serde_json::from_value(fixture("uptimekuma/heartbeat.json")).unwrap();
        let d = parse_status(&page, &beats);
        assert_eq!(d.stat("up").unwrap().value, StatValue::Number(2.0));
        assert_eq!(d.stat("down").unwrap().value, StatValue::Number(1.0));
        match d.stat("uptime").unwrap().value {
            StatValue::Percent(p) => assert!((p - 96.25).abs() < 1e-9, "{p}"),
            ref v => panic!("{v:?}"),
        }
        assert_eq!(d.items.len(), 1);
        assert_eq!(d.items[0].title, "Jellyfin");
        assert_eq!(
            d.items[0].subtitle.as_deref(),
            Some("connect ECONNREFUSED 192.168.1.40:8096")
        );
    }

    #[test]
    fn empty_page_has_no_uptime() {
        let d = parse_status(&StatusPage::default(), &Heartbeats::default());
        assert_eq!(d.stat("uptime").unwrap().value, StatValue::Text("-".into()));
    }

    #[test]
    fn slug_is_one_encoded_segment() {
        let cfg = WidgetConfig::default().with("url", "http://kuma.lan:3001/");
        let url = slug_url(&cfg, "/api/status-page", "home lab/../x").unwrap();
        assert_eq!(
            url.as_str(),
            "http://kuma.lan:3001/api/status-page/home%20lab%2F..%2Fx"
        );
    }

    #[tokio::test]
    async fn fetches_status_page_and_heartbeats() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/status-page/home%20lab"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture("uptimekuma/status_page.json")),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/status-page/heartbeat/home%20lab"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture("uptimekuma/heartbeat.json")),
            )
            .expect(1)
            .mount(&server)
            .await;
        let ctx = WidgetContext::with_defaults().unwrap();
        let cfg = WidgetConfig::default()
            .with("url", server.uri())
            .with("slug", "home lab");
        let d = UptimeKuma.fetch(&ctx, &cfg).await.unwrap();
        assert_eq!(d.stat("down").unwrap().value, StatValue::Number(1.0));
    }
}
