//! slskd (Soulseek daemon): connection state, transfers and shares.
//!
//! Authenticates with an API key sent as `X-API-Key`. Uses
//! `GET /api/v0/application` for server, version and share state, and
//! `GET /api/v0/transfers/{downloads,uploads}` for transfers, which slskd
//! groups by user and then by directory.

use async_trait::async_trait;
use serde::Deserialize;

use crate::config::WidgetConfig;
use crate::context::WidgetContext;
use crate::data::{ListItem, Stat, StatValue, WidgetData};
use crate::error::WidgetError;
use crate::schema::WidgetSchema;
use crate::widget::Widget;
use crate::widgets::api_key_field;

/// Most active downloads listed as items.
const MAX_ITEMS: usize = 10;

/// slskd widget.
#[derive(Debug, Default, Clone, Copy)]
pub struct Slskd;

/// `GET /api/v0/application` (only the fields used).
#[derive(Debug, Default, Deserialize)]
pub struct Application {
    /// Soulseek server connection.
    #[serde(default)]
    pub server: ServerState,
    /// Installed and latest version.
    #[serde(default)]
    pub version: VersionState,
    /// Share scan totals.
    #[serde(default)]
    pub shares: ShareState,
}

/// Soulseek server connection state.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerState {
    /// Connected to the Soulseek network.
    #[serde(default)]
    pub is_connected: bool,
    /// Logged in on the Soulseek network.
    #[serde(default)]
    pub is_logged_in: bool,
}

/// Version state.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionState {
    /// Installed version.
    #[serde(default)]
    pub current: String,
    /// Whether a newer release exists (`null` when not checked).
    #[serde(default)]
    pub is_update_available: Option<bool>,
}

/// Share totals.
#[derive(Debug, Default, Deserialize)]
pub struct ShareState {
    /// Shared directories.
    #[serde(default)]
    pub directories: u64,
    /// Shared files.
    #[serde(default)]
    pub files: u64,
}

/// Transfers of one remote user.
#[derive(Debug, Default, Deserialize)]
pub struct UserTransfers {
    /// The remote user.
    #[serde(default)]
    pub username: String,
    /// Transfers grouped by directory.
    #[serde(default)]
    pub directories: Vec<DirectoryTransfers>,
}

/// Transfers in one directory.
#[derive(Debug, Default, Deserialize)]
pub struct DirectoryTransfers {
    /// The files being transferred.
    #[serde(default)]
    pub files: Vec<Transfer>,
}

/// One file transfer (only the fields used).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transfer {
    /// Remote path of the file.
    #[serde(default)]
    pub filename: String,
    /// State flags such as `Queued, Remotely`, `InProgress` or
    /// `Completed, Succeeded`.
    #[serde(default)]
    pub state: String,
    /// Progress from 0 to 100.
    #[serde(default)]
    pub percent_complete: f64,
    /// Average speed in bytes per second.
    #[serde(default)]
    pub average_speed: f64,
}

impl Transfer {
    /// Whether the transfer is still queued or running.
    pub fn is_active(&self) -> bool {
        !self.state.starts_with("Completed")
    }

    /// Whether bytes are moving right now.
    pub fn is_in_progress(&self) -> bool {
        self.state.contains("InProgress")
    }
}

fn files(users: &[UserTransfers]) -> impl Iterator<Item = (&str, &Transfer)> {
    users.iter().flat_map(|u| {
        u.directories
            .iter()
            .flat_map(|d| d.files.iter())
            .map(move |f| (u.username.as_str(), f))
    })
}

/// File name without the remote directory (slskd paths use `\` or `/`).
fn base_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

/// Turns the application state and transfer lists into widget data.
pub fn parse_slskd(
    app: &Application,
    downloads: &[UserTransfers],
    uploads: &[UserTransfers],
) -> WidgetData {
    let status = if app.server.is_connected && app.server.is_logged_in {
        "Connected"
    } else if app.server.is_connected {
        "Not logged in"
    } else {
        "Disconnected"
    };
    let update = match app.version.is_update_available {
        Some(true) => "Update available",
        Some(false) => "Up to date",
        None => "Unknown",
    };
    let active_downloads = files(downloads).filter(|(_, f)| f.is_active()).count();
    let active_uploads = files(uploads).filter(|(_, f)| f.is_active()).count();
    let speed: f64 = files(downloads)
        .filter(|(_, f)| f.is_in_progress())
        .map(|(_, f)| f.average_speed)
        .sum();
    let items = files(downloads)
        .filter(|(_, f)| f.is_in_progress())
        .take(MAX_ITEMS)
        .map(|(user, f)| {
            ListItem::new(base_name(&f.filename))
                .subtitle(user)
                .progress(f.percent_complete)
        })
        .collect();
    WidgetData::new(vec![
        Stat::new("slskStatus", "Status", StatValue::Text(status.into())),
        Stat::new("updateStatus", "Update", StatValue::Text(update.into())),
        Stat::count("downloads", "Downloads", active_downloads as u64),
        Stat::count("uploads", "Uploads", active_uploads as u64),
        Stat::new("downloadSpeed", "Speed", StatValue::Rate(speed)),
        Stat::count("sharedFiles", "Shared files", app.shares.files),
    ])
    .with_items(items)
}

#[async_trait]
impl Widget for Slskd {
    fn kind(&self) -> &'static str {
        "slskd"
    }

    fn schema(&self) -> WidgetSchema {
        WidgetSchema::new(
            "slskd",
            "slskd",
            "Soulseek connection, active transfers and shared files.",
        )
        .field(api_key_field(
            "An API key from slskd.yml (web.authentication.api_keys).",
        ))
    }

    async fn fetch(
        &self,
        ctx: &WidgetContext,
        cfg: &WidgetConfig,
    ) -> Result<WidgetData, WidgetError> {
        let key = cfg.require_str("key")?;
        let h = [("X-API-Key", key)];
        let (app, downloads, uploads) = tokio::try_join!(
            ctx.get_json::<Application>(cfg, "/api/v0/application", &h),
            ctx.get_json::<Vec<UserTransfers>>(cfg, "/api/v0/transfers/downloads", &h),
            ctx.get_json::<Vec<UserTransfers>>(cfg, "/api/v0/transfers/uploads", &h),
        )?;
        Ok(parse_slskd(&app, &downloads, &uploads))
    }

    async fn test_connection(
        &self,
        ctx: &WidgetContext,
        cfg: &WidgetConfig,
    ) -> Result<String, WidgetError> {
        let key = cfg.require_str("key")?;
        let app: Application = ctx
            .get_json(cfg, "/api/v0/application", &[("X-API-Key", key)])
            .await?;
        Ok(format!("Connected to slskd {}", app.version.current)
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

    fn recorded() -> (Application, Vec<UserTransfers>, Vec<UserTransfers>) {
        (
            serde_json::from_value(fixture("slskd/application.json")).unwrap(),
            serde_json::from_value(fixture("slskd/downloads.json")).unwrap(),
            serde_json::from_value(fixture("slskd/uploads.json")).unwrap(),
        )
    }

    #[test]
    fn parses_recorded_responses() {
        let (app, down, up) = recorded();
        let d = parse_slskd(&app, &down, &up);
        let v = |k: &str| d.stat(k).unwrap().value.clone();
        assert_eq!(v("slskStatus"), StatValue::Text("Connected".into()));
        assert_eq!(
            v("updateStatus"),
            StatValue::Text("Update available".into())
        );
        assert_eq!(v("downloads"), StatValue::Number(3.0));
        assert_eq!(v("uploads"), StatValue::Number(1.0));
        assert_eq!(v("downloadSpeed"), StatValue::Rate(1_572_864.0));
        assert_eq!(v("sharedFiles"), StatValue::Number(48213.0));
        assert_eq!(d.items.len(), 2);
        assert_eq!(d.items[0].title, "01 - Opening.flac");
        assert_eq!(d.items[0].subtitle.as_deref(), Some("peer_one"));
    }

    #[test]
    fn reports_disconnected() {
        let app: Application = serde_json::from_value(
            serde_json::json!({"server": {"isConnected": false, "isLoggedIn": false}}),
        )
        .unwrap();
        let d = parse_slskd(&app, &[], &[]);
        assert_eq!(
            d.stat("slskStatus").unwrap().value,
            StatValue::Text("Disconnected".into())
        );
    }

    #[tokio::test]
    async fn fetches_with_api_key() {
        let server = MockServer::start().await;
        for (p, f) in [
            ("/api/v0/application", "slskd/application.json"),
            ("/api/v0/transfers/downloads", "slskd/downloads.json"),
            ("/api/v0/transfers/uploads", "slskd/uploads.json"),
        ] {
            Mock::given(method("GET"))
                .and(path(p))
                .and(header("X-API-Key", "sk"))
                .respond_with(ResponseTemplate::new(200).set_body_json(fixture(f)))
                .mount(&server)
                .await;
        }
        let ctx = WidgetContext::with_defaults().unwrap();
        let cfg = WidgetConfig::default()
            .with("url", server.uri())
            .with("key", "sk");
        let d = Slskd.fetch(&ctx, &cfg).await.unwrap();
        assert_eq!(d.stat("uploads").unwrap().value, StatValue::Number(1.0));
        let msg = Slskd.test_connection(&ctx, &cfg).await.unwrap();
        assert!(msg.contains("0.24.1"));
        let bad = cfg.clone().with("key", "nope");
        assert!(Slskd.fetch(&ctx, &bad).await.is_err());
    }
}
