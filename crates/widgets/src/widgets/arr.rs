//! The *arr family: Sonarr, Radarr and Prowlarr.
//!
//! All three authenticate with an `X-Api-Key` header and expose
//! `system/status`, which is used for the connection test.

use async_trait::async_trait;
use serde::Deserialize;

use crate::config::WidgetConfig;
use crate::context::WidgetContext;
use crate::data::{Stat, WidgetData};
use crate::error::WidgetError;
use crate::schema::WidgetSchema;
use crate::widget::Widget;
use crate::widgets::api_key_field;

const KEY_HELP: &str = "Settings → General → Security → API Key.";

/// `GET {api}/system/status`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemStatus {
    /// Application name, e.g. `Sonarr`.
    #[serde(default)]
    pub app_name: String,
    /// Application version.
    #[serde(default)]
    pub version: String,
}

/// A paged list response; only the total is used.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Paged {
    /// Number of records across all pages.
    #[serde(default)]
    pub total_records: u64,
}

async fn get<T: serde::de::DeserializeOwned>(
    ctx: &WidgetContext,
    cfg: &WidgetConfig,
    path: &str,
) -> Result<T, WidgetError> {
    let key = cfg.require_str("key")?;
    ctx.get_json(cfg, path, &[("X-Api-Key", key)]).await
}

async fn status(ctx: &WidgetContext, cfg: &WidgetConfig, api: &str) -> Result<String, WidgetError> {
    let s: SystemStatus = get(ctx, cfg, &format!("{api}/system/status")).await?;
    Ok(format!("Connected to {} {}", s.app_name, s.version)
        .trim()
        .to_owned())
}

// ---------------------------------------------------------------- Sonarr

/// Sonarr: series count, missing episodes and download queue.
#[derive(Debug, Default, Clone, Copy)]
pub struct Sonarr;

/// Parses Sonarr's series list, missing list and queue into widget data.
pub fn parse_sonarr(series: &[serde_json::Value], missing: &Paged, queue: &Paged) -> WidgetData {
    WidgetData::new(vec![
        Stat::count("wanted", "Wanted", missing.total_records),
        Stat::count("queued", "Queued", queue.total_records),
        Stat::count("series", "Series", series.len() as u64),
    ])
}

#[async_trait]
impl Widget for Sonarr {
    fn kind(&self) -> &'static str {
        "sonarr"
    }

    fn schema(&self) -> WidgetSchema {
        WidgetSchema::new(
            "sonarr",
            "Sonarr",
            "Missing episodes, queue and series count.",
        )
        .field(api_key_field(KEY_HELP))
    }

    async fn fetch(
        &self,
        ctx: &WidgetContext,
        cfg: &WidgetConfig,
    ) -> Result<WidgetData, WidgetError> {
        let (series, missing, queue) = tokio::try_join!(
            get::<Vec<serde_json::Value>>(ctx, cfg, "/api/v3/series"),
            get::<Paged>(ctx, cfg, "/api/v3/wanted/missing?page=1&pageSize=1"),
            get::<Paged>(ctx, cfg, "/api/v3/queue?page=1&pageSize=1"),
        )?;
        Ok(parse_sonarr(&series, &missing, &queue))
    }

    async fn test_connection(
        &self,
        ctx: &WidgetContext,
        cfg: &WidgetConfig,
    ) -> Result<String, WidgetError> {
        status(ctx, cfg, "/api/v3").await
    }
}

// ---------------------------------------------------------------- Radarr

/// Radarr: movie counts and download queue.
#[derive(Debug, Default, Clone, Copy)]
pub struct Radarr;

/// One entry of `GET /api/v3/movie` (only the fields used).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RadarrMovie {
    /// Whether a file is on disk.
    #[serde(default)]
    pub has_file: bool,
    /// Whether the movie is monitored.
    #[serde(default)]
    pub monitored: bool,
    /// Whether the movie is released and can be searched.
    #[serde(default)]
    pub is_available: bool,
}

/// `GET /api/v3/queue/status`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueStatus {
    /// Total items in the queue.
    #[serde(default)]
    pub total_count: u64,
}

/// Parses Radarr's movie list and queue status into widget data.
pub fn parse_radarr(movies: &[RadarrMovie], queue: &QueueStatus) -> WidgetData {
    let wanted = movies
        .iter()
        .filter(|m| m.monitored && !m.has_file && m.is_available)
        .count() as u64;
    let missing = movies.iter().filter(|m| m.monitored && !m.has_file).count() as u64;
    WidgetData::new(vec![
        Stat::count("wanted", "Wanted", wanted),
        Stat::count("missing", "Missing", missing),
        Stat::count("queued", "Queued", queue.total_count),
        Stat::count("movies", "Movies", movies.len() as u64),
    ])
}

#[async_trait]
impl Widget for Radarr {
    fn kind(&self) -> &'static str {
        "radarr"
    }

    fn schema(&self) -> WidgetSchema {
        WidgetSchema::new(
            "radarr",
            "Radarr",
            "Wanted and missing movies, queue and movie count.",
        )
        .field(api_key_field(KEY_HELP))
    }

    async fn fetch(
        &self,
        ctx: &WidgetContext,
        cfg: &WidgetConfig,
    ) -> Result<WidgetData, WidgetError> {
        let (movies, queue) = tokio::try_join!(
            get::<Vec<RadarrMovie>>(ctx, cfg, "/api/v3/movie"),
            get::<QueueStatus>(ctx, cfg, "/api/v3/queue/status"),
        )?;
        Ok(parse_radarr(&movies, &queue))
    }

    async fn test_connection(
        &self,
        ctx: &WidgetContext,
        cfg: &WidgetConfig,
    ) -> Result<String, WidgetError> {
        status(ctx, cfg, "/api/v3").await
    }
}

// -------------------------------------------------------------- Prowlarr

/// Prowlarr: indexer count and grab/query totals.
#[derive(Debug, Default, Clone, Copy)]
pub struct Prowlarr;

/// One entry of `GET /api/v1/indexer` (only the fields used).
#[derive(Debug, Deserialize)]
pub struct ProwlarrIndexer {
    /// Whether the indexer is enabled.
    #[serde(default)]
    pub enable: bool,
}

/// `GET /api/v1/indexerstats`.
#[derive(Debug, Deserialize)]
pub struct IndexerStats {
    /// Per-indexer counters.
    #[serde(default)]
    pub indexers: Vec<IndexerStat>,
}

/// Counters of one indexer.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexerStat {
    /// Successful grabs.
    #[serde(default)]
    pub number_of_grabs: u64,
    /// Queries sent.
    #[serde(default)]
    pub number_of_queries: u64,
    /// Failed grabs.
    #[serde(default)]
    pub number_of_failed_grabs: u64,
    /// Failed queries.
    #[serde(default)]
    pub number_of_failed_queries: u64,
}

/// Parses Prowlarr's indexers and indexer stats into widget data.
pub fn parse_prowlarr(indexers: &[ProwlarrIndexer], stats: &IndexerStats) -> WidgetData {
    let sum = |f: fn(&IndexerStat) -> u64| stats.indexers.iter().map(f).sum::<u64>();
    WidgetData::new(vec![
        Stat::count(
            "indexers",
            "Indexers",
            indexers.iter().filter(|i| i.enable).count() as u64,
        ),
        Stat::count("grabs", "Grabs", sum(|s| s.number_of_grabs)),
        Stat::count("queries", "Queries", sum(|s| s.number_of_queries)),
        Stat::count(
            "failures",
            "Failures",
            sum(|s| s.number_of_failed_grabs + s.number_of_failed_queries),
        ),
    ])
}

#[async_trait]
impl Widget for Prowlarr {
    fn kind(&self) -> &'static str {
        "prowlarr"
    }

    fn schema(&self) -> WidgetSchema {
        WidgetSchema::new(
            "prowlarr",
            "Prowlarr",
            "Enabled indexers and grab, query and failure totals.",
        )
        .field(api_key_field(KEY_HELP))
    }

    async fn fetch(
        &self,
        ctx: &WidgetContext,
        cfg: &WidgetConfig,
    ) -> Result<WidgetData, WidgetError> {
        let (indexers, stats) = tokio::try_join!(
            get::<Vec<ProwlarrIndexer>>(ctx, cfg, "/api/v1/indexer"),
            get::<IndexerStats>(ctx, cfg, "/api/v1/indexerstats"),
        )?;
        Ok(parse_prowlarr(&indexers, &stats))
    }

    async fn test_connection(
        &self,
        ctx: &WidgetContext,
        cfg: &WidgetConfig,
    ) -> Result<String, WidgetError> {
        status(ctx, cfg, "/api/v1").await
    }
}

// ---------------------------------------------------------------- Lidarr

/// Lidarr: missing albums, download queue and artist count.
#[derive(Debug, Default, Clone, Copy)]
pub struct Lidarr;

/// Parses Lidarr's artist list, missing list and queue status into widget
/// data.
pub fn parse_lidarr(
    artists: &[serde_json::Value],
    missing: &Paged,
    queue: &QueueStatus,
) -> WidgetData {
    WidgetData::new(vec![
        Stat::count("wanted", "Wanted", missing.total_records),
        Stat::count("queued", "Queued", queue.total_count),
        Stat::count("artists", "Artists", artists.len() as u64),
    ])
}

#[async_trait]
impl Widget for Lidarr {
    fn kind(&self) -> &'static str {
        "lidarr"
    }

    fn schema(&self) -> WidgetSchema {
        WidgetSchema::new(
            "lidarr",
            "Lidarr",
            "Missing albums, queue and artist count.",
        )
        .field(api_key_field(KEY_HELP))
    }

    async fn fetch(
        &self,
        ctx: &WidgetContext,
        cfg: &WidgetConfig,
    ) -> Result<WidgetData, WidgetError> {
        let (artists, missing, queue) = tokio::try_join!(
            get::<Vec<serde_json::Value>>(ctx, cfg, "/api/v1/artist"),
            get::<Paged>(ctx, cfg, "/api/v1/wanted/missing?page=1&pageSize=1"),
            get::<QueueStatus>(ctx, cfg, "/api/v1/queue/status"),
        )?;
        Ok(parse_lidarr(&artists, &missing, &queue))
    }

    async fn test_connection(
        &self,
        ctx: &WidgetContext,
        cfg: &WidgetConfig,
    ) -> Result<String, WidgetError> {
        status(ctx, cfg, "/api/v1").await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::StatValue;
    use crate::widgets::fixture;

    fn num(d: &WidgetData, key: &str) -> f64 {
        match d.stat(key).unwrap().value {
            StatValue::Number(n) => n,
            ref v => panic!("{key}: {v:?}"),
        }
    }

    #[test]
    fn sonarr_parses_recorded_responses() {
        let series: Vec<serde_json::Value> =
            serde_json::from_value(fixture("sonarr/series.json")).unwrap();
        let missing: Paged = serde_json::from_value(fixture("sonarr/wanted_missing.json")).unwrap();
        let queue: Paged = serde_json::from_value(fixture("sonarr/queue.json")).unwrap();
        let d = parse_sonarr(&series, &missing, &queue);
        assert_eq!(num(&d, "series"), 2.0);
        assert_eq!(num(&d, "wanted"), 37.0);
        assert_eq!(num(&d, "queued"), 3.0);
    }

    #[test]
    fn radarr_parses_recorded_responses() {
        let movies: Vec<RadarrMovie> =
            serde_json::from_value(fixture("radarr/movie.json")).unwrap();
        let queue: QueueStatus =
            serde_json::from_value(fixture("radarr/queue_status.json")).unwrap();
        let d = parse_radarr(&movies, &queue);
        assert_eq!(num(&d, "movies"), 3.0);
        assert_eq!(num(&d, "missing"), 2.0);
        assert_eq!(num(&d, "wanted"), 1.0);
        assert_eq!(num(&d, "queued"), 1.0);
    }

    #[test]
    fn prowlarr_parses_recorded_responses() {
        let idx: Vec<ProwlarrIndexer> =
            serde_json::from_value(fixture("prowlarr/indexer.json")).unwrap();
        let stats: IndexerStats =
            serde_json::from_value(fixture("prowlarr/indexerstats.json")).unwrap();
        let d = parse_prowlarr(&idx, &stats);
        assert_eq!(num(&d, "indexers"), 1.0);
        assert_eq!(num(&d, "grabs"), 15.0);
        assert_eq!(num(&d, "queries"), 1290.0);
        assert_eq!(num(&d, "failures"), 7.0);
    }

    #[test]
    fn lidarr_parses_recorded_responses() {
        let artists: Vec<serde_json::Value> =
            serde_json::from_value(fixture("lidarr/artist.json")).unwrap();
        let missing: Paged = serde_json::from_value(fixture("lidarr/wanted_missing.json")).unwrap();
        let queue: QueueStatus =
            serde_json::from_value(fixture("lidarr/queue_status.json")).unwrap();
        let d = parse_lidarr(&artists, &missing, &queue);
        assert_eq!(num(&d, "artists"), 2.0);
        assert_eq!(num(&d, "wanted"), 14.0);
        assert_eq!(num(&d, "queued"), 2.0);
    }

    #[tokio::test]
    async fn lidarr_fetches_with_api_key() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        for (p, f) in [
            ("/api/v1/artist", "lidarr/artist.json"),
            ("/api/v1/wanted/missing", "lidarr/wanted_missing.json"),
            ("/api/v1/queue/status", "lidarr/queue_status.json"),
        ] {
            Mock::given(method("GET"))
                .and(path(p))
                .and(header("X-Api-Key", "lid"))
                .respond_with(ResponseTemplate::new(200).set_body_json(fixture(f)))
                .mount(&server)
                .await;
        }
        let ctx = WidgetContext::with_defaults().unwrap();
        let cfg = WidgetConfig::default()
            .with("url", server.uri())
            .with("key", "lid");
        let d = Lidarr.fetch(&ctx, &cfg).await.unwrap();
        assert_eq!(num(&d, "queued"), 2.0);
        let bad = WidgetConfig::default()
            .with("url", server.uri())
            .with("key", "nope");
        assert!(Lidarr.fetch(&ctx, &bad).await.is_err());
    }
}
