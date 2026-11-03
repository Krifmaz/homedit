//! qBittorrent: transfer speeds, torrent counts and active downloads.
//!
//! Uses the WebUI API v2. When a username is configured the widget logs in
//! with `POST /api/v2/auth/login` and sends the returned session cookie on
//! the following calls. Without a username it assumes the homedit server is
//! on qBittorrent's authentication whitelist ("Bypass authentication for
//! clients in whitelisted IP subnets").

use async_trait::async_trait;
use reqwest::header::{CONTENT_TYPE, COOKIE, REFERER, SET_COOKIE};
use reqwest::{Method, RequestBuilder};
use serde::Deserialize;

use crate::config::WidgetConfig;
use crate::context::WidgetContext;
use crate::data::{ListItem, Stat, StatValue, WidgetData};
use crate::error::WidgetError;
use crate::schema::WidgetSchema;
use crate::widget::Widget;
use crate::widgets::{password_field, username_field};

/// Largest number of active downloads listed as items.
const MAX_ITEMS: usize = 10;

/// Torrent states counted as leeching (downloading or about to).
pub const LEECHING_STATES: &[&str] = &[
    "downloading",
    "stalledDL",
    "metaDL",
    "forcedDL",
    "queuedDL",
    "checkingDL",
];

/// Torrent states counted as seeding.
pub const SEEDING_STATES: &[&str] = &["uploading", "stalledUP", "forcedUP", "queuedUP"];

/// qBittorrent widget.
#[derive(Debug, Default, Clone, Copy)]
pub struct QBittorrent;

/// `GET /api/v2/transfer/info` (only the fields used).
#[derive(Debug, Default, Deserialize)]
pub struct TransferInfo {
    /// Global download rate in bytes per second.
    #[serde(default)]
    pub dl_info_speed: f64,
    /// Global upload rate in bytes per second.
    #[serde(default)]
    pub up_info_speed: f64,
}

/// One entry of `GET /api/v2/torrents/info` (only the fields used).
#[derive(Debug, Default, Deserialize)]
pub struct Torrent {
    /// Torrent name.
    #[serde(default)]
    pub name: String,
    /// Progress from 0 to 1.
    #[serde(default)]
    pub progress: f64,
    /// State word such as `downloading` or `stalledUP`.
    #[serde(default)]
    pub state: String,
    /// Current download rate in bytes per second.
    #[serde(default)]
    pub dlspeed: f64,
}

/// Parses transfer info and the torrent list into widget data.
///
/// Stats: download and upload rate, leeching, seeding and total torrent
/// counts. Items: torrents in a leeching state, with their progress.
pub fn parse_qbittorrent(transfer: &TransferInfo, torrents: &[Torrent]) -> WidgetData {
    let leeching: Vec<&Torrent> = torrents
        .iter()
        .filter(|t| LEECHING_STATES.contains(&t.state.as_str()))
        .collect();
    let seeding = torrents
        .iter()
        .filter(|t| SEEDING_STATES.contains(&t.state.as_str()))
        .count();
    let items = leeching
        .iter()
        .take(MAX_ITEMS)
        .map(|t| {
            ListItem::new(t.name.clone())
                .subtitle(t.state.clone())
                .progress(t.progress * 100.0)
        })
        .collect();
    WidgetData::new(vec![
        Stat::new(
            "download",
            "Download",
            StatValue::Rate(transfer.dl_info_speed),
        ),
        Stat::new("upload", "Upload", StatValue::Rate(transfer.up_info_speed)),
        Stat::count("leeching", "Leeching", leeching.len() as u64),
        Stat::count("seeding", "Seeding", seeding as u64),
        Stat::count("torrents", "Torrents", torrents.len() as u64),
    ])
    .with_items(items)
}

/// Extracts the session cookie (`name=value`) from one `Set-Cookie` header
/// value. qBittorrent names it `SID`; newer releases may use a name
/// starting with `QBT_SID`, which is accepted too.
pub fn parse_session_cookie(set_cookie: &str) -> Option<String> {
    let pair = set_cookie.split(';').next()?.trim();
    let (name, value) = pair.split_once('=')?;
    let name = name.trim();
    let is_session = name == "SID" || name.starts_with("QBT_SID");
    (is_session && !value.trim().is_empty()).then(|| format!("{name}={}", value.trim()))
}

/// Builds the urlencoded login form body.
fn login_body(username: &str, password: &str) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .append_pair("username", username)
        .append_pair("password", password)
        .finish()
}

/// Logs in if a username is configured, returning the cookie header value
/// to send on later requests.
async fn login(ctx: &WidgetContext, cfg: &WidgetConfig) -> Result<Option<String>, WidgetError> {
    let Some(username) = cfg.str("username") else {
        return Ok(None);
    };
    let password = cfg.str("password").unwrap_or_default();
    let base = cfg.base_url()?;
    let req = ctx
        .request(cfg, Method::POST, "/api/v2/auth/login")?
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        // qBittorrent rejects logins whose Referer/Origin does not match its
        // own host (CSRF protection).
        .header(REFERER, base.as_str())
        .body(login_body(username, password));
    let mut resp = ctx.send(req).await?;
    let cookie = resp
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(parse_session_cookie);
    // The answer is a short word ("Ok." or "Fails."); read at most a few
    // KiB of it.
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        body.extend_from_slice(&chunk);
        if body.len() > 4096 {
            break;
        }
    }
    if String::from_utf8_lossy(&body).trim() == "Fails." {
        return Err(WidgetError::Auth);
    }
    cookie
        .map(Some)
        .ok_or_else(|| WidgetError::Parse("login succeeded but no session cookie was set".into()))
}

/// Starts a `GET` with the session cookie, if any.
fn get(
    ctx: &WidgetContext,
    cfg: &WidgetConfig,
    path: &str,
    cookie: Option<&str>,
) -> Result<RequestBuilder, WidgetError> {
    let mut req = ctx.request(cfg, Method::GET, path)?;
    if let Some(cookie) = cookie {
        req = req.header(COOKIE, cookie);
    }
    Ok(req)
}

#[async_trait]
impl Widget for QBittorrent {
    fn kind(&self) -> &'static str {
        "qbittorrent"
    }

    fn schema(&self) -> WidgetSchema {
        WidgetSchema::new(
            "qbittorrent",
            "qBittorrent",
            "Download and upload speed, leeching and seeding torrents.",
        )
        .field(username_field(false).help(
            "WebUI username. Leave empty if homedit is on qBittorrent's authentication whitelist.",
        ))
        .field(password_field(false).help("WebUI password."))
    }

    async fn fetch(
        &self,
        ctx: &WidgetContext,
        cfg: &WidgetConfig,
    ) -> Result<WidgetData, WidgetError> {
        let cookie = login(ctx, cfg).await?;
        let cookie = cookie.as_deref();
        let (transfer, torrents) = tokio::try_join!(
            ctx.send_json::<TransferInfo>(get(ctx, cfg, "/api/v2/transfer/info", cookie)?),
            ctx.send_json::<Vec<Torrent>>(get(ctx, cfg, "/api/v2/torrents/info", cookie)?),
        )?;
        Ok(parse_qbittorrent(&transfer, &torrents))
    }

    async fn test_connection(
        &self,
        ctx: &WidgetContext,
        cfg: &WidgetConfig,
    ) -> Result<String, WidgetError> {
        let cookie = login(ctx, cfg).await?;
        let body = ctx
            .send_bytes(get(ctx, cfg, "/api/v2/app/version", cookie.as_deref())?)
            .await?;
        let version = String::from_utf8_lossy(&body).trim().to_owned();
        Ok(format!("Connected to qBittorrent {version}")
            .trim()
            .to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::fixture;
    use wiremock::matchers::{body_string, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn value(d: &WidgetData, key: &str) -> StatValue {
        d.stat(key).unwrap().value.clone()
    }

    #[test]
    fn parses_recorded_responses() {
        let transfer: TransferInfo =
            serde_json::from_value(fixture("qbittorrent/transfer_info.json")).unwrap();
        let torrents: Vec<Torrent> =
            serde_json::from_value(fixture("qbittorrent/torrents_info.json")).unwrap();
        let d = parse_qbittorrent(&transfer, &torrents);
        assert_eq!(value(&d, "download"), StatValue::Rate(5_242_880.0));
        assert_eq!(value(&d, "upload"), StatValue::Rate(131_072.0));
        assert_eq!(value(&d, "leeching"), StatValue::Number(3.0));
        assert_eq!(value(&d, "seeding"), StatValue::Number(2.0));
        assert_eq!(value(&d, "torrents"), StatValue::Number(6.0));
        assert_eq!(d.items.len(), 3);
        assert_eq!(d.items[0].title, "debian-12.7.0-amd64-netinst.iso");
        assert_eq!(d.items[0].progress, Some(42.5));
        assert_eq!(d.items[0].subtitle.as_deref(), Some("downloading"));
    }

    #[test]
    fn extracts_session_cookie() {
        assert_eq!(
            parse_session_cookie("SID=abc123; HttpOnly; SameSite=Strict; path=/").as_deref(),
            Some("SID=abc123")
        );
        assert_eq!(
            parse_session_cookie("QBT_SID_8080=xyz; HttpOnly").as_deref(),
            Some("QBT_SID_8080=xyz")
        );
        assert_eq!(parse_session_cookie("other=1; path=/"), None);
        assert_eq!(parse_session_cookie("SID=; path=/"), None);
    }

    #[test]
    fn encodes_login_form() {
        assert_eq!(
            login_body("admin", "p&ss w=rd"),
            "username=admin&password=p%26ss+w%3Drd"
        );
    }

    #[tokio::test]
    async fn fetch_logs_in_and_sends_cookie() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v2/auth/login"))
            .and(header("content-type", "application/x-www-form-urlencoded"))
            .and(body_string("username=admin&password=secret"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("set-cookie", "SID=s3ss10n; HttpOnly; path=/")
                    .set_body_string("Ok."),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v2/transfer/info"))
            .and(header("cookie", "SID=s3ss10n"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture("qbittorrent/transfer_info.json")),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v2/torrents/info"))
            .and(header("cookie", "SID=s3ss10n"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture("qbittorrent/torrents_info.json")),
            )
            .mount(&server)
            .await;

        let ctx = WidgetContext::with_defaults().unwrap();
        let cfg = WidgetConfig::default()
            .with("url", server.uri())
            .with("username", "admin")
            .with("password", "secret");
        let d = QBittorrent.fetch(&ctx, &cfg).await.unwrap();
        assert_eq!(value(&d, "torrents"), StatValue::Number(6.0));
    }

    #[tokio::test]
    async fn rejected_login_is_auth_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v2/auth/login"))
            .respond_with(ResponseTemplate::new(200).set_body_string("Fails."))
            .mount(&server)
            .await;
        let ctx = WidgetContext::with_defaults().unwrap();
        let cfg = WidgetConfig::default()
            .with("url", server.uri())
            .with("username", "admin")
            .with("password", "wrong");
        let err = QBittorrent.fetch(&ctx, &cfg).await.unwrap_err();
        assert!(matches!(err, WidgetError::Auth), "{err:?}");
    }

    #[tokio::test]
    async fn whitelisted_skips_login_and_tests_version() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/app/version"))
            .respond_with(ResponseTemplate::new(200).set_body_string("v5.0.1"))
            .mount(&server)
            .await;
        let ctx = WidgetContext::with_defaults().unwrap();
        let cfg = WidgetConfig::default().with("url", server.uri());
        let msg = QBittorrent.test_connection(&ctx, &cfg).await.unwrap();
        assert_eq!(msg, "Connected to qBittorrent v5.0.1");
    }
}
