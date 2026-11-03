//! Plex Media Server: active streams and library sizes.
//!
//! Plex authenticates with an `X-Plex-Token` header and answers in JSON when
//! asked with `Accept: application/json` (which [`WidgetContext::send_json`]
//! always sends). Every response is wrapped in a `MediaContainer` object.
//!
//! Endpoints used:
//!
//! - `GET /status/sessions`: what is playing right now,
//! - `GET /library/sections`: the libraries and their types,
//! - `GET /library/sections/{key}/all?X-Plex-Container-Size=0`: one
//!   library's item count (`totalSize`) without any items,
//! - `GET /identity`: server version, for the connection test.

use async_trait::async_trait;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tokio::task::JoinSet;

use crate::config::WidgetConfig;
use crate::context::WidgetContext;
use crate::data::{ListItem, Stat, WidgetData};
use crate::error::WidgetError;
use crate::schema::{FieldSpec, FieldType, WidgetSchema};
use crate::widget::Widget;

/// Header that carries the Plex token.
pub const TOKEN_HEADER: &str = "X-Plex-Token";

/// Plex's `type` value for an album, used to count albums rather than
/// artists in music libraries.
const ALBUM_TYPE: u32 = 9;

/// The `{"MediaContainer": ...}` wrapper around every Plex JSON response.
#[derive(Debug, Default, Deserialize)]
pub struct Envelope<T> {
    /// The wrapped container.
    #[serde(rename = "MediaContainer", default)]
    pub media_container: T,
}

/// `GET /status/sessions`.
#[derive(Debug, Default, Deserialize)]
pub struct Sessions {
    /// Number of active sessions.
    #[serde(default)]
    pub size: u64,
    /// One entry per playing item.
    #[serde(rename = "Metadata", default)]
    pub metadata: Vec<SessionItem>,
}

/// One playing item of [`Sessions`] (only the fields used).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionItem {
    /// Item kind: `movie`, `episode`, `track`, `clip`, ...
    #[serde(rename = "type", default)]
    pub kind: String,
    /// Title of the item itself (episode or track name for those kinds).
    #[serde(default)]
    pub title: String,
    /// Show name for episodes, artist for tracks.
    #[serde(default)]
    pub grandparent_title: String,
    /// Playback position in milliseconds.
    #[serde(default)]
    pub view_offset: f64,
    /// Length of the item in milliseconds.
    #[serde(default)]
    pub duration: f64,
    /// The account that is watching.
    #[serde(rename = "User", default)]
    pub user: Option<PlexUser>,
}

/// The `User` object of a [`SessionItem`].
#[derive(Debug, Default, Deserialize)]
pub struct PlexUser {
    /// Display name of the account.
    #[serde(default)]
    pub title: String,
}

/// `GET /library/sections`.
#[derive(Debug, Default, Deserialize)]
pub struct Sections {
    /// The libraries.
    #[serde(rename = "Directory", default)]
    pub directory: Vec<Section>,
}

/// One library of [`Sections`].
#[derive(Debug, Default, Deserialize)]
pub struct Section {
    /// Section id used in `/library/sections/{key}/...` paths.
    #[serde(default)]
    pub key: String,
    /// Library type: `movie`, `show`, `artist` or `photo`.
    #[serde(rename = "type", default)]
    pub kind: String,
    /// Library name.
    #[serde(default)]
    pub title: String,
}

/// `GET /library/sections/{key}/all` with a container size of zero.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SectionTotal {
    /// Number of items in the library (of the requested type).
    #[serde(default)]
    pub total_size: u64,
}

/// `GET /identity`.
#[derive(Debug, Default, Deserialize)]
pub struct Identity {
    /// Server version, e.g. `1.41.2.9200-c6bbc1b53`.
    #[serde(default)]
    pub version: String,
}

/// What a library counts towards in the widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryKind {
    /// A movie library.
    Movies,
    /// A TV show library (counted in shows, not episodes).
    Shows,
    /// A music library (counted in albums).
    Music,
}

impl LibraryKind {
    /// Maps a Plex section type to a kind; photo and other libraries are
    /// not counted.
    pub fn from_section_type(kind: &str) -> Option<Self> {
        match kind {
            "movie" => Some(Self::Movies),
            "show" => Some(Self::Shows),
            "artist" => Some(Self::Music),
            _ => None,
        }
    }

    /// Path and query that return this library's item count without items.
    pub fn count_path(self, section_key: &str) -> String {
        let type_filter = match self {
            Self::Music => format!("type={ALBUM_TYPE}&"),
            Self::Movies | Self::Shows => String::new(),
        };
        format!(
            "/library/sections/{section_key}/all?{type_filter}X-Plex-Container-Start=0&X-Plex-Container-Size=0"
        )
    }
}

/// Item counts summed over all libraries.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LibraryTotals {
    /// Movies across all movie libraries.
    pub movies: u64,
    /// Shows across all TV libraries.
    pub shows: u64,
    /// Albums across all music libraries.
    pub albums: u64,
}

impl LibraryTotals {
    /// Adds one library's count.
    pub fn add(&mut self, kind: LibraryKind, count: u64) {
        match kind {
            LibraryKind::Movies => self.movies += count,
            LibraryKind::Shows => self.shows += count,
            LibraryKind::Music => self.albums += count,
        }
    }
}

/// Picks the libraries that are counted, with their section keys.
///
/// Keys that are not purely alphanumeric are skipped, so a misbehaving
/// server cannot make the widget request arbitrary paths.
pub fn parse_sections(sections: &Sections) -> Vec<(String, LibraryKind)> {
    sections
        .directory
        .iter()
        .filter(|s| !s.key.is_empty() && s.key.chars().all(|c| c.is_ascii_alphanumeric()))
        .filter_map(|s| LibraryKind::from_section_type(&s.kind).map(|k| (s.key.clone(), k)))
        .collect()
}

/// Turns the active sessions into a stream count and "now playing" rows.
pub fn parse_sessions(sessions: &Sessions) -> (u64, Vec<ListItem>) {
    let items: Vec<ListItem> = sessions
        .metadata
        .iter()
        .map(|m| {
            let title = if matches!(m.kind.as_str(), "episode" | "track")
                && !m.grandparent_title.is_empty()
            {
                format!("{} - {}", m.grandparent_title, m.title)
            } else {
                m.title.clone()
            };
            let mut item = ListItem::new(title);
            if let Some(user) = m.user.as_ref().filter(|u| !u.title.is_empty()) {
                item = item.subtitle(user.title.clone());
            }
            if m.duration > 0.0 {
                item = item.progress(m.view_offset / m.duration * 100.0);
            }
            item
        })
        .collect();
    let streams = sessions.size.max(items.len() as u64);
    (streams, items)
}

/// Builds the widget data from the sessions and the library totals.
pub fn parse_plex(sessions: &Sessions, totals: &LibraryTotals) -> WidgetData {
    let (streams, items) = parse_sessions(sessions);
    WidgetData::new(vec![
        Stat::count("streams", "Streams", streams),
        Stat::count("movies", "Movies", totals.movies),
        Stat::count("shows", "Shows", totals.shows),
        Stat::count("albums", "Albums", totals.albums),
    ])
    .with_items(items)
}

/// Plex Media Server: active streams, now playing, and movie, show and
/// album counts.
#[derive(Debug, Default, Clone, Copy)]
pub struct Plex;

async fn get<T: DeserializeOwned + Default>(
    ctx: &WidgetContext,
    cfg: &WidgetConfig,
    path: &str,
) -> Result<T, WidgetError> {
    let token = cfg.require_str("key")?;
    let env: Envelope<T> = ctx.get_json(cfg, path, &[(TOKEN_HEADER, token)]).await?;
    Ok(env.media_container)
}

async fn library_totals(
    ctx: &WidgetContext,
    cfg: &WidgetConfig,
    libraries: Vec<(String, LibraryKind)>,
) -> Result<LibraryTotals, WidgetError> {
    let mut set = JoinSet::new();
    for (key, kind) in libraries {
        let (ctx, cfg) = (ctx.clone(), cfg.clone());
        set.spawn(async move {
            let total: SectionTotal = get(&ctx, &cfg, &kind.count_path(&key)).await?;
            Ok::<_, WidgetError>((kind, total.total_size))
        });
    }
    let mut totals = LibraryTotals::default();
    while let Some(joined) = set.join_next().await {
        let (kind, count) = joined.map_err(|e| WidgetError::Http(e.to_string()))??;
        totals.add(kind, count);
    }
    Ok(totals)
}

#[async_trait]
impl Widget for Plex {
    fn kind(&self) -> &'static str {
        "plex"
    }

    fn schema(&self) -> WidgetSchema {
        WidgetSchema::new(
            "plex",
            "Plex",
            "Active streams, what is playing, and movie, show and album counts.",
        )
        .field(
            FieldSpec::new("key", "Plex token", FieldType::Secret)
                .required()
                .help("The X-Plex-Token of a server admin account (see Plex's support article \"Finding an authentication token\")."),
        )
    }

    async fn fetch(
        &self,
        ctx: &WidgetContext,
        cfg: &WidgetConfig,
    ) -> Result<WidgetData, WidgetError> {
        let (sessions, sections) = tokio::try_join!(
            get::<Sessions>(ctx, cfg, "/status/sessions"),
            get::<Sections>(ctx, cfg, "/library/sections"),
        )?;
        let totals = library_totals(ctx, cfg, parse_sections(&sections)).await?;
        Ok(parse_plex(&sessions, &totals))
    }

    async fn test_connection(
        &self,
        ctx: &WidgetContext,
        cfg: &WidgetConfig,
    ) -> Result<String, WidgetError> {
        let id: Identity = get(ctx, cfg, "/identity").await?;
        Ok(format!("Connected to Plex {}", id.version)
            .trim()
            .to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::StatValue;
    use crate::widgets::fixture;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn num(d: &WidgetData, key: &str) -> f64 {
        match d.stat(key).unwrap().value {
            StatValue::Number(n) => n,
            ref v => panic!("{key}: {v:?}"),
        }
    }

    fn load<T: DeserializeOwned + Default>(name: &str) -> T {
        let env: Envelope<T> = serde_json::from_value(fixture(&format!("plex/{name}"))).unwrap();
        env.media_container
    }

    #[test]
    fn parses_sessions() {
        let sessions: Sessions = load("sessions.json");
        let (streams, items) = parse_sessions(&sessions);
        assert_eq!(streams, 2);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "Severance - Who Is Alive?");
        assert_eq!(items[0].subtitle.as_deref(), Some("alice"));
        assert_eq!(items[0].progress, Some(50.0));
        assert_eq!(items[1].title, "Dune: Part Two");
        assert_eq!(items[1].subtitle.as_deref(), Some("bob"));
        assert_eq!(items[1].progress, Some(25.0));
    }

    #[test]
    fn parses_sections_and_skips_photos() {
        let sections: Sections = load("sections.json");
        assert_eq!(
            parse_sections(&sections),
            vec![
                ("1".to_owned(), LibraryKind::Movies),
                ("2".to_owned(), LibraryKind::Shows),
                ("3".to_owned(), LibraryKind::Music),
            ]
        );
    }

    #[test]
    fn rejects_suspicious_section_keys() {
        let sections: Sections = serde_json::from_value(serde_json::json!({
            "Directory": [{"key": "../../admin", "type": "movie"}, {"key": "", "type": "show"}]
        }))
        .unwrap();
        assert!(parse_sections(&sections).is_empty());
    }

    #[test]
    fn parses_totals_into_stats() {
        let mut totals = LibraryTotals::default();
        totals.add(
            LibraryKind::Movies,
            load::<SectionTotal>("section_movies_all.json").total_size,
        );
        totals.add(
            LibraryKind::Shows,
            load::<SectionTotal>("section_shows_all.json").total_size,
        );
        totals.add(
            LibraryKind::Music,
            load::<SectionTotal>("section_music_albums.json").total_size,
        );
        let d = parse_plex(&load("sessions.json"), &totals);
        assert_eq!(num(&d, "streams"), 2.0);
        assert_eq!(num(&d, "movies"), 1342.0);
        assert_eq!(num(&d, "shows"), 187.0);
        assert_eq!(num(&d, "albums"), 905.0);
        assert_eq!(d.items.len(), 2);
    }

    #[test]
    fn empty_sessions_container() {
        let sessions: Sessions = load_str(r#"{"MediaContainer": {"size": 0}}"#);
        let (streams, items) = parse_sessions(&sessions);
        assert_eq!(streams, 0);
        assert!(items.is_empty());
    }

    fn load_str<T: DeserializeOwned + Default>(s: &str) -> T {
        serde_json::from_str::<Envelope<T>>(s)
            .unwrap()
            .media_container
    }

    async fn mount(server: &MockServer, p: &str, file: &str) {
        Mock::given(method("GET"))
            .and(path(p))
            .and(header(TOKEN_HEADER, "tok3n"))
            .and(header("Accept", "application/json"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture(&format!("plex/{file}"))),
            )
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn fetch_end_to_end_sends_token() {
        let server = MockServer::start().await;
        mount(&server, "/status/sessions", "sessions.json").await;
        mount(&server, "/library/sections", "sections.json").await;
        mount(
            &server,
            "/library/sections/1/all",
            "section_movies_all.json",
        )
        .await;
        mount(&server, "/library/sections/2/all", "section_shows_all.json").await;
        Mock::given(method("GET"))
            .and(path("/library/sections/3/all"))
            .and(query_param("type", "9"))
            .and(query_param("X-Plex-Container-Size", "0"))
            .and(header(TOKEN_HEADER, "tok3n"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture("plex/section_music_albums.json")),
            )
            .mount(&server)
            .await;
        mount(&server, "/identity", "identity.json").await;

        let ctx = WidgetContext::with_defaults().unwrap();
        let cfg = WidgetConfig::default()
            .with("url", server.uri())
            .with("key", "tok3n");
        let d = Plex.fetch(&ctx, &cfg).await.unwrap();
        assert_eq!(num(&d, "streams"), 2.0);
        assert_eq!(num(&d, "movies"), 1342.0);
        assert_eq!(num(&d, "shows"), 187.0);
        assert_eq!(num(&d, "albums"), 905.0);

        let msg = Plex.test_connection(&ctx, &cfg).await.unwrap();
        assert_eq!(msg, "Connected to Plex 1.41.2.9200-c6bbc1b53");

        let bad = cfg.clone().with("key", "wrong");
        assert!(Plex.fetch(&ctx, &bad).await.is_err());
    }
}
