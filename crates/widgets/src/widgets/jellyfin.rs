//! Jellyfin and Emby: active streams and library counts.
//!
//! Jellyfin is a fork of Emby and both still share the API used here, so
//! [`Jellyfin`] and [`Emby`] are thin wrappers around the same code. Both
//! authenticate with an `X-Emby-Token` header carrying an API key.
//!
//! Endpoints used:
//!
//! - `GET /Sessions`: connected clients; those with a `NowPlayingItem` are
//!   streaming,
//! - `GET /Items/Counts`: movie, series, episode and song totals,
//! - `GET /System/Info/Public`: server name and version, for the
//!   connection test (works without a key, but the key is sent anyway).

use async_trait::async_trait;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::config::WidgetConfig;
use crate::context::WidgetContext;
use crate::data::{ListItem, Stat, WidgetData};
use crate::error::WidgetError;
use crate::schema::WidgetSchema;
use crate::widget::Widget;
use crate::widgets::api_key_field;

/// Header that carries the API key.
pub const TOKEN_HEADER: &str = "X-Emby-Token";

/// One entry of `GET /Sessions` (only the fields used).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Session {
    /// Name of the signed-in user.
    #[serde(default)]
    pub user_name: String,
    /// What the client is playing, if anything.
    #[serde(default)]
    pub now_playing_item: Option<NowPlayingItem>,
    /// Playback position and pause state.
    #[serde(default)]
    pub play_state: Option<PlayState>,
}

/// The item a [`Session`] is playing.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct NowPlayingItem {
    /// Item name (episode name for episodes).
    #[serde(default)]
    pub name: String,
    /// Series name, set for episodes.
    #[serde(default)]
    pub series_name: Option<String>,
    /// Item type: `Movie`, `Episode`, `Audio`, ...
    #[serde(rename = "Type", default)]
    pub kind: String,
    /// Length in ticks (100 ns units).
    #[serde(default)]
    pub run_time_ticks: Option<f64>,
}

/// The `PlayState` of a [`Session`].
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlayState {
    /// Position in ticks (100 ns units).
    #[serde(default)]
    pub position_ticks: Option<f64>,
    /// Whether playback is paused.
    #[serde(default)]
    pub is_paused: bool,
}

/// `GET /Items/Counts` (only the fields used).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ItemCounts {
    /// Number of movies.
    #[serde(default)]
    pub movie_count: u64,
    /// Number of series.
    #[serde(default)]
    pub series_count: u64,
    /// Number of episodes.
    #[serde(default)]
    pub episode_count: u64,
    /// Number of songs.
    #[serde(default)]
    pub song_count: u64,
}

/// `GET /System/Info/Public` (only the fields used).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PublicSystemInfo {
    /// Friendly server name.
    #[serde(default)]
    pub server_name: String,
    /// Server version.
    #[serde(default)]
    pub version: String,
}

/// Turns the session list into "now playing" rows, one per streaming
/// session. Idle sessions are skipped.
pub fn parse_sessions(sessions: &[Session]) -> Vec<ListItem> {
    sessions
        .iter()
        .filter_map(|s| {
            let item = s.now_playing_item.as_ref()?;
            let title = match item.series_name.as_deref() {
                Some(series) if item.kind == "Episode" && !series.is_empty() => {
                    format!("{series} - {}", item.name)
                }
                _ => item.name.clone(),
            };
            let mut row = ListItem::new(title);
            if !s.user_name.is_empty() {
                row = row.subtitle(s.user_name.clone());
            }
            let position = s.play_state.as_ref().and_then(|p| p.position_ticks);
            if let (Some(pos), Some(len)) = (position, item.run_time_ticks)
                && len > 0.0
            {
                row = row.progress(pos / len * 100.0);
            }
            Some(row)
        })
        .collect()
}

/// Builds the widget data from the sessions and item counts.
pub fn parse_jellyfin(sessions: &[Session], counts: &ItemCounts) -> WidgetData {
    let items = parse_sessions(sessions);
    WidgetData::new(vec![
        Stat::count("streams", "Streams", items.len() as u64),
        Stat::count("movies", "Movies", counts.movie_count),
        Stat::count("series", "Series", counts.series_count),
        Stat::count("episodes", "Episodes", counts.episode_count),
        Stat::count("songs", "Songs", counts.song_count),
    ])
    .with_items(items)
}

async fn get<T: DeserializeOwned>(
    ctx: &WidgetContext,
    cfg: &WidgetConfig,
    path: &str,
) -> Result<T, WidgetError> {
    let key = cfg.require_str("key")?;
    ctx.get_json(cfg, path, &[(TOKEN_HEADER, key)]).await
}

fn schema(kind: &str, name: &str) -> WidgetSchema {
    WidgetSchema::new(
        kind,
        name,
        "Active streams, what is playing, and movie, series, episode and song counts.",
    )
    .field(api_key_field(
        "Dashboard → Advanced → API Keys → create a key for homedit.",
    ))
}

async fn fetch(ctx: &WidgetContext, cfg: &WidgetConfig) -> Result<WidgetData, WidgetError> {
    let (sessions, counts) = tokio::try_join!(
        get::<Vec<Session>>(ctx, cfg, "/Sessions"),
        get::<ItemCounts>(ctx, cfg, "/Items/Counts"),
    )?;
    Ok(parse_jellyfin(&sessions, &counts))
}

async fn test_connection(
    ctx: &WidgetContext,
    cfg: &WidgetConfig,
    name: &str,
) -> Result<String, WidgetError> {
    let info: PublicSystemInfo = get(ctx, cfg, "/System/Info/Public").await?;
    let mut msg = format!("Connected to {name}");
    if !info.server_name.is_empty() {
        msg.push_str(&format!(" server \"{}\"", info.server_name));
    }
    if !info.version.is_empty() {
        msg.push_str(&format!(", version {}", info.version));
    }
    Ok(msg)
}

/// Jellyfin: active streams, now playing and library counts.
#[derive(Debug, Default, Clone, Copy)]
pub struct Jellyfin;

#[async_trait]
impl Widget for Jellyfin {
    fn kind(&self) -> &'static str {
        "jellyfin"
    }

    fn schema(&self) -> WidgetSchema {
        schema("jellyfin", "Jellyfin")
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
        test_connection(ctx, cfg, "Jellyfin").await
    }
}

/// Emby: active streams, now playing and library counts (same API as
/// Jellyfin).
#[derive(Debug, Default, Clone, Copy)]
pub struct Emby;

#[async_trait]
impl Widget for Emby {
    fn kind(&self) -> &'static str {
        "emby"
    }

    fn schema(&self) -> WidgetSchema {
        schema("emby", "Emby")
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
        test_connection(ctx, cfg, "Emby").await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::StatValue;
    use crate::widgets::fixture;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn num(d: &WidgetData, key: &str) -> f64 {
        match d.stat(key).unwrap().value {
            StatValue::Number(n) => n,
            ref v => panic!("{key}: {v:?}"),
        }
    }

    fn sessions() -> Vec<Session> {
        serde_json::from_value(fixture("jellyfin/sessions.json")).unwrap()
    }

    fn counts() -> ItemCounts {
        serde_json::from_value(fixture("jellyfin/items_counts.json")).unwrap()
    }

    #[test]
    fn parses_now_playing() {
        let items = parse_sessions(&sessions());
        assert_eq!(items.len(), 2, "idle session is skipped");
        assert_eq!(items[0].title, "Game of Thrones - The Rains of Castamere");
        assert_eq!(items[0].subtitle.as_deref(), Some("alice"));
        assert_eq!(items[0].progress, Some(50.0));
        assert_eq!(items[1].title, "Arrival");
        assert_eq!(items[1].subtitle.as_deref(), Some("bob"));
        let p = items[1].progress.unwrap();
        assert!((p - 12.931).abs() < 0.01, "{p}");
    }

    #[test]
    fn parses_counts() {
        let d = parse_jellyfin(&sessions(), &counts());
        assert_eq!(num(&d, "streams"), 2.0);
        assert_eq!(num(&d, "movies"), 812.0);
        assert_eq!(num(&d, "series"), 143.0);
        assert_eq!(num(&d, "episodes"), 6021.0);
        assert_eq!(num(&d, "songs"), 10422.0);
    }

    #[test]
    fn parses_public_info() {
        let info: PublicSystemInfo =
            serde_json::from_value(fixture("jellyfin/system_info_public.json")).unwrap();
        assert_eq!(info.server_name, "media-box");
        assert_eq!(info.version, "10.10.7");
    }

    async fn server() -> MockServer {
        let server = MockServer::start().await;
        for (p, file) in [
            ("/Sessions", "sessions.json"),
            ("/Items/Counts", "items_counts.json"),
            ("/System/Info/Public", "system_info_public.json"),
        ] {
            Mock::given(method("GET"))
                .and(path(p))
                .and(header(TOKEN_HEADER, "jf-key"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(fixture(&format!("jellyfin/{file}"))),
                )
                .mount(&server)
                .await;
        }
        server
    }

    #[tokio::test]
    async fn fetch_end_to_end_sends_token() {
        let server = server().await;
        let ctx = WidgetContext::with_defaults().unwrap();
        let cfg = WidgetConfig::default()
            .with("url", server.uri())
            .with("key", "jf-key");
        for widget in [&Jellyfin as &dyn Widget, &Emby] {
            let d = widget.fetch(&ctx, &cfg).await.unwrap();
            assert_eq!(num(&d, "streams"), 2.0);
            assert_eq!(num(&d, "movies"), 812.0);
            assert_eq!(d.items.len(), 2);
        }
        let msg = Jellyfin.test_connection(&ctx, &cfg).await.unwrap();
        assert_eq!(
            msg,
            "Connected to Jellyfin server \"media-box\", version 10.10.7"
        );
        let bad = cfg.clone().with("key", "nope");
        assert!(Jellyfin.fetch(&ctx, &bad).await.is_err());
    }
}
