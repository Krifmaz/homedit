//! Pi-hole: DNS queries, blocked queries and blocklist size.
//!
//! Two API generations are supported:
//!
//! - **v6** (`version: "6"`, the default): the REST API under `/api`. When a
//!   password is configured, the widget logs in with `POST /api/auth`,
//!   reads `GET /api/stats/summary` with the `X-FTL-SID` session header and
//!   then logs out again with `DELETE /api/auth`, so polling does not pile
//!   up sessions on the Pi-hole (which limits concurrent sessions).
//! - **v5** (`version: "5"`): the legacy `GET /admin/api.php?summaryRaw`
//!   endpoint with the API token in the `auth` query parameter.

use async_trait::async_trait;
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::config::WidgetConfig;
use crate::context::WidgetContext;
use crate::data::{Stat, StatValue, WidgetData};
use crate::error::WidgetError;
use crate::schema::{FieldSpec, FieldType, WidgetSchema};
use crate::widget::Widget;

/// Header carrying the v6 session id.
const SID_HEADER: &str = "X-FTL-SID";

/// Pi-hole (v5 or v6): total queries, blocked queries, block percentage
/// and the number of blocked domains.
#[derive(Debug, Default, Clone, Copy)]
pub struct PiHole;

// ------------------------------------------------------------------- v6

/// Response of `POST /api/auth` (v6).
#[derive(Debug, Default, Deserialize)]
pub struct AuthResponse {
    /// The session that was (or was not) created.
    #[serde(default)]
    pub session: Session,
}

/// The `session` object of a v6 auth response.
#[derive(Debug, Default, Deserialize)]
pub struct Session {
    /// Whether the login succeeded (also `true` when no password is set).
    #[serde(default)]
    pub valid: bool,
    /// Session id to send in the `X-FTL-SID` header; absent when the
    /// Pi-hole has no password.
    #[serde(default)]
    pub sid: Option<String>,
}

/// Response of `GET /api/stats/summary` (v6), only the fields used.
#[derive(Debug, Default, Deserialize)]
pub struct SummaryV6 {
    /// Query counters.
    #[serde(default)]
    pub queries: QueriesV6,
    /// Blocklist information.
    #[serde(default)]
    pub gravity: GravityV6,
}

/// The `queries` object of a v6 summary.
#[derive(Debug, Default, Deserialize)]
pub struct QueriesV6 {
    /// Queries in the last 24 hours.
    #[serde(default)]
    pub total: f64,
    /// Blocked queries in the last 24 hours.
    #[serde(default)]
    pub blocked: f64,
    /// Share of blocked queries, 0 to 100.
    #[serde(default)]
    pub percent_blocked: f64,
}

/// The `gravity` object of a v6 summary.
#[derive(Debug, Default, Deserialize)]
pub struct GravityV6 {
    /// Number of domains on the blocklists.
    #[serde(default)]
    pub domains_being_blocked: f64,
}

/// Extracts the session id from a v6 login response.
///
/// Returns `Ok(None)` when the login is valid but needs no session (no
/// password set on the Pi-hole) and [`WidgetError::Auth`] when the login
/// was rejected.
pub fn parse_auth(resp: &AuthResponse) -> Result<Option<String>, WidgetError> {
    if !resp.session.valid {
        return Err(WidgetError::Auth);
    }
    Ok(resp.session.sid.clone().filter(|s| !s.is_empty()))
}

/// Turns a v6 summary into widget data.
pub fn parse_summary_v6(s: &SummaryV6) -> WidgetData {
    stats(
        s.queries.total,
        s.queries.blocked,
        s.queries.percent_blocked,
        s.gravity.domains_being_blocked,
    )
}

// ------------------------------------------------------------------- v5

/// Response of `GET /admin/api.php?summaryRaw` (v5), only the fields used.
#[derive(Debug, Default, Deserialize)]
pub struct SummaryV5 {
    /// Queries today.
    #[serde(default)]
    pub dns_queries_today: f64,
    /// Blocked queries today.
    #[serde(default)]
    pub ads_blocked_today: f64,
    /// Share of blocked queries today, 0 to 100.
    #[serde(default)]
    pub ads_percentage_today: f64,
    /// Number of domains on the blocklists.
    #[serde(default)]
    pub domains_being_blocked: f64,
}

/// Parses a raw v5 summary response.
///
/// Pi-hole v5 answers an unauthenticated or wrongly authenticated request
/// with an empty JSON array instead of an HTTP error, which is reported as
/// [`WidgetError::Auth`].
pub fn parse_summary_v5(value: Value) -> Result<WidgetData, WidgetError> {
    if value.is_array() {
        return Err(WidgetError::Auth);
    }
    let s: SummaryV5 = serde_json::from_value(value)?;
    Ok(stats(
        s.dns_queries_today,
        s.ads_blocked_today,
        s.ads_percentage_today,
        s.domains_being_blocked,
    ))
}

fn stats(queries: f64, blocked: f64, percent: f64, gravity: f64) -> WidgetData {
    WidgetData::new(vec![
        Stat::new("queries", "Queries", StatValue::Number(queries)),
        Stat::new("blocked", "Blocked", StatValue::Number(blocked)),
        Stat::new("blocked_percent", "Blocked %", StatValue::Percent(percent)),
        Stat::new("gravity", "Blocklist", StatValue::Number(gravity)),
    ])
}

// ---------------------------------------------------------------- fetch

async fn fetch_v6(ctx: &WidgetContext, cfg: &WidgetConfig) -> Result<WidgetData, WidgetError> {
    let sid = match cfg.str("key") {
        Some(password) => {
            let req = ctx
                .request(cfg, Method::POST, "/api/auth")?
                .json(&json!({ "password": password }));
            let auth: AuthResponse = ctx.send_json(req).await?;
            parse_auth(&auth)?
        }
        None => None,
    };

    let mut req = ctx.request(cfg, Method::GET, "/api/stats/summary")?;
    if let Some(sid) = &sid {
        req = req.header(SID_HEADER, sid);
    }
    let result = ctx.send_json::<SummaryV6>(req).await;

    if let Some(sid) = &sid {
        // Best effort: a failed logout only means the session expires on
        // its own.
        if let Ok(req) = ctx.request(cfg, Method::DELETE, "/api/auth") {
            let _ = ctx.send(req.header(SID_HEADER, sid)).await;
        }
    }
    Ok(parse_summary_v6(&result?))
}

async fn fetch_v5(ctx: &WidgetContext, cfg: &WidgetConfig) -> Result<WidgetData, WidgetError> {
    let mut path = String::from("/admin/api.php?summaryRaw");
    if let Some(token) = cfg.str("key") {
        let encoded: String = url::form_urlencoded::byte_serialize(token.as_bytes()).collect();
        path.push_str("&auth=");
        path.push_str(&encoded);
    }
    let value: Value = ctx.get_json(cfg, &path, &[]).await?;
    parse_summary_v5(value)
}

#[async_trait]
impl Widget for PiHole {
    fn kind(&self) -> &'static str {
        "pihole"
    }

    fn schema(&self) -> WidgetSchema {
        WidgetSchema::new(
            "pihole",
            "Pi-hole",
            "DNS queries, blocked queries and blocklist size.",
        )
        .field(
            FieldSpec::new("version", "API version", FieldType::select(&["6", "5"]))
                .default_value("6")
                .help("Pi-hole 6 and later use the new REST API; pick 5 for older installs."),
        )
        .field(
            FieldSpec::new("key", "Password or API token", FieldType::Secret).help(
                "v6: an app password (Settings → Web interface / API → Configure app password). \
                 v5: the API token (Settings → API). Leave empty if the Pi-hole has no password.",
            ),
        )
    }

    async fn fetch(
        &self,
        ctx: &WidgetContext,
        cfg: &WidgetConfig,
    ) -> Result<WidgetData, WidgetError> {
        match cfg.str("version").unwrap_or("6") {
            "5" => fetch_v5(ctx, cfg).await,
            _ => fetch_v6(ctx, cfg).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::fixture;
    use wiremock::matchers::{body_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn parses_v6_summary() {
        let s: SummaryV6 = serde_json::from_value(fixture("pihole/summary_v6.json")).unwrap();
        let d = parse_summary_v6(&s);
        assert_eq!(d.stat("queries").unwrap().value, StatValue::Number(48213.0));
        assert_eq!(d.stat("blocked").unwrap().value, StatValue::Number(9172.0));
        assert_eq!(
            d.stat("blocked_percent").unwrap().value,
            StatValue::Percent(19.023914)
        );
        assert_eq!(
            d.stat("gravity").unwrap().value,
            StatValue::Number(186512.0)
        );
    }

    #[test]
    fn parses_v6_auth() {
        let ok: AuthResponse = serde_json::from_value(fixture("pihole/auth_v6.json")).unwrap();
        assert_eq!(
            parse_auth(&ok).unwrap().as_deref(),
            Some("vFA+EP4MQ5JJvJg+3Q2Jnw=")
        );
        let bad: AuthResponse =
            serde_json::from_value(fixture("pihole/auth_v6_invalid.json")).unwrap();
        assert!(matches!(parse_auth(&bad), Err(WidgetError::Auth)));
        let open: AuthResponse =
            serde_json::from_value(json!({"session": {"valid": true, "sid": null}})).unwrap();
        assert_eq!(parse_auth(&open).unwrap(), None);
    }

    #[test]
    fn parses_v5_summary() {
        let d = parse_summary_v5(fixture("pihole/summary_raw_v5.json")).unwrap();
        assert_eq!(d.stat("queries").unwrap().value, StatValue::Number(31544.0));
        assert_eq!(d.stat("blocked").unwrap().value, StatValue::Number(4127.0));
        assert_eq!(
            d.stat("blocked_percent").unwrap().value,
            StatValue::Percent(13.083312)
        );
        assert_eq!(
            d.stat("gravity").unwrap().value,
            StatValue::Number(121987.0)
        );
        assert!(matches!(
            parse_summary_v5(json!([])),
            Err(WidgetError::Auth)
        ));
    }

    #[tokio::test]
    async fn v6_logs_in_reads_summary_and_logs_out() {
        let server = MockServer::start().await;
        let sid = "vFA+EP4MQ5JJvJg+3Q2Jnw=";
        Mock::given(method("POST"))
            .and(path("/api/auth"))
            .and(body_json(json!({"password": "app-pass"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture("pihole/auth_v6.json")))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/stats/summary"))
            .and(header(SID_HEADER, sid))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture("pihole/summary_v6.json")),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/api/auth"))
            .and(header(SID_HEADER, sid))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        let ctx = WidgetContext::with_defaults().unwrap();
        let cfg = WidgetConfig::default()
            .with("url", server.uri())
            .with("key", "app-pass")
            .with_defaults(&PiHole.schema());
        let d = PiHole.fetch(&ctx, &cfg).await.unwrap();
        assert_eq!(d.stat("queries").unwrap().value, StatValue::Number(48213.0));
    }

    #[tokio::test]
    async fn v6_wrong_password_is_auth_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/auth"))
            .respond_with(
                ResponseTemplate::new(401).set_body_json(fixture("pihole/auth_v6_invalid.json")),
            )
            .mount(&server)
            .await;
        let ctx = WidgetContext::with_defaults().unwrap();
        let cfg = WidgetConfig::default()
            .with("url", server.uri())
            .with("key", "nope");
        let err = PiHole.fetch(&ctx, &cfg).await.unwrap_err();
        assert!(matches!(err, WidgetError::Auth));
    }

    #[tokio::test]
    async fn v5_sends_encoded_token() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/admin/api.php"))
            .and(query_param("auth", "a b&c"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture("pihole/summary_raw_v5.json")),
            )
            .expect(1)
            .mount(&server)
            .await;
        let ctx = WidgetContext::with_defaults().unwrap();
        let cfg = WidgetConfig::default()
            .with("url", server.uri())
            .with("version", "5")
            .with("key", "a b&c");
        let d = PiHole.fetch(&ctx, &cfg).await.unwrap();
        assert_eq!(d.stat("blocked").unwrap().value, StatValue::Number(4127.0));
    }
}
