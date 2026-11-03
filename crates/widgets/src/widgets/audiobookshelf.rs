//! Audiobookshelf: book and podcast counts and total listening time.
//!
//! Authenticates with an API token sent as `Authorization: Bearer <key>`.
//! The library list comes from `GET /api/libraries`, and each library's
//! totals from `GET /api/libraries/{id}/stats`.

use async_trait::async_trait;
use serde::Deserialize;

use crate::config::WidgetConfig;
use crate::context::WidgetContext;
use crate::data::{Stat, StatValue, WidgetData};
use crate::error::WidgetError;
use crate::schema::WidgetSchema;
use crate::widget::Widget;
use crate::widgets::api_key_field;

/// Audiobookshelf widget.
#[derive(Debug, Default, Clone, Copy)]
pub struct Audiobookshelf;

/// `GET /api/libraries`.
#[derive(Debug, Default, Deserialize)]
pub struct Libraries {
    /// All libraries the token can see.
    #[serde(default)]
    pub libraries: Vec<Library>,
}

/// One library (only the fields used).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Library {
    /// Library id, used in the stats path.
    #[serde(default)]
    pub id: String,
    /// Display name.
    #[serde(default)]
    pub name: String,
    /// `book` or `podcast`.
    #[serde(default)]
    pub media_type: String,
}

/// `GET /api/libraries/{id}/stats` (only the fields used).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryStats {
    /// Number of items (books or podcasts).
    #[serde(default)]
    pub total_items: u64,
    /// Total duration of all items in seconds.
    #[serde(default)]
    pub total_duration: f64,
}

/// Sums library stats by media type into widget data.
pub fn parse_audiobookshelf(libraries: &[(Library, LibraryStats)]) -> WidgetData {
    let (mut books, mut book_secs, mut podcasts, mut podcast_secs) = (0, 0.0, 0, 0.0);
    for (lib, stats) in libraries {
        if lib.media_type == "podcast" {
            podcasts += stats.total_items;
            podcast_secs += stats.total_duration;
        } else {
            books += stats.total_items;
            book_secs += stats.total_duration;
        }
    }
    WidgetData::new(vec![
        Stat::count("podcasts", "Podcasts", podcasts),
        Stat::new(
            "podcastsDuration",
            "Podcast time",
            StatValue::Duration(podcast_secs),
        ),
        Stat::count("books", "Books", books),
        Stat::new("booksDuration", "Book time", StatValue::Duration(book_secs)),
    ])
}

/// Accepts only ids that are safe to put in a URL path (Audiobookshelf uses
/// UUIDs or short alphanumeric ids).
fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[async_trait]
impl Widget for Audiobookshelf {
    fn kind(&self) -> &'static str {
        "audiobookshelf"
    }

    fn schema(&self) -> WidgetSchema {
        WidgetSchema::new(
            "audiobookshelf",
            "Audiobookshelf",
            "Books and podcasts with their total listening time.",
        )
        .field(api_key_field(
            "Settings → Users → your user → API token (or an API key on 2.26+).",
        ))
    }

    async fn fetch(
        &self,
        ctx: &WidgetContext,
        cfg: &WidgetConfig,
    ) -> Result<WidgetData, WidgetError> {
        let auth = format!("Bearer {}", cfg.require_str("key")?);
        let headers = [("Authorization", auth.as_str())];
        let libs: Libraries = ctx.get_json(cfg, "/api/libraries", &headers).await?;
        let mut out = Vec::new();
        for lib in libs.libraries.into_iter().filter(|l| safe_id(&l.id)) {
            let stats: LibraryStats = ctx
                .get_json(cfg, &format!("/api/libraries/{}/stats", lib.id), &headers)
                .await?;
            out.push((lib, stats));
        }
        Ok(parse_audiobookshelf(&out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::fixture;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn value(d: &WidgetData, key: &str) -> StatValue {
        d.stat(key).unwrap().value.clone()
    }

    fn recorded() -> Vec<(Library, LibraryStats)> {
        let libs: Libraries =
            serde_json::from_value(fixture("audiobookshelf/libraries.json")).unwrap();
        libs.libraries
            .into_iter()
            .map(|l| {
                let f = format!("audiobookshelf/stats_{}.json", l.media_type);
                (l, serde_json::from_value(fixture(&f)).unwrap())
            })
            .collect()
    }

    #[test]
    fn parses_recorded_responses() {
        let d = parse_audiobookshelf(&recorded());
        assert_eq!(value(&d, "books"), StatValue::Number(212.0));
        assert_eq!(value(&d, "booksDuration"), StatValue::Duration(7_524_311.5));
        assert_eq!(value(&d, "podcasts"), StatValue::Number(6.0));
        assert_eq!(
            value(&d, "podcastsDuration"),
            StatValue::Duration(1_036_800.0)
        );
    }

    #[test]
    fn rejects_unsafe_ids() {
        assert!(safe_id("lib_c1u6t4p45c35rf0nzd"));
        assert!(safe_id("8a3f6c2e-0b2d-4f61-9a7e-31c9a0f1d2b4"));
        assert!(!safe_id("../../api/users"));
        assert!(!safe_id(""));
    }

    #[tokio::test]
    async fn fetches_with_bearer_token() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/libraries"))
            .and(header("Authorization", "Bearer abs-token"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture("audiobookshelf/libraries.json")),
            )
            .mount(&server)
            .await;
        for (id, kind) in [
            ("lib_c1u6t4p45c35rf0nzd", "book"),
            ("lib_p9w2a7nq0k3m1x8yzt", "podcast"),
        ] {
            Mock::given(method("GET"))
                .and(path(format!("/api/libraries/{id}/stats")))
                .and(header("Authorization", "Bearer abs-token"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(fixture(&format!("audiobookshelf/stats_{kind}.json"))),
                )
                .mount(&server)
                .await;
        }
        let ctx = WidgetContext::with_defaults().unwrap();
        let cfg = WidgetConfig::default()
            .with("url", server.uri())
            .with("key", "abs-token");
        let d = Audiobookshelf.fetch(&ctx, &cfg).await.unwrap();
        assert_eq!(value(&d, "books"), StatValue::Number(212.0));
        let bad = cfg.clone().with("key", "wrong");
        assert!(Audiobookshelf.fetch(&ctx, &bad).await.is_err());
    }
}
