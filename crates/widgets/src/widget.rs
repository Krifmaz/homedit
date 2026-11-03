//! The [`Widget`] trait every service widget implements.

use std::time::Duration;

use async_trait::async_trait;

use crate::config::WidgetConfig;
use crate::context::WidgetContext;
use crate::data::WidgetData;
use crate::error::WidgetError;
use crate::schema::WidgetSchema;

/// Default time a widget result is reused before fetching again.
pub const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(10);

/// A service widget: a small module that knows how to talk to one kind of
/// self-hosted app and turn its API responses into [`WidgetData`].
///
/// Implementations are stateless; per-instance settings come in through
/// [`WidgetConfig`] and all network access goes through [`WidgetContext`],
/// which enforces timeouts and the host allow-list. Widgets keep their
/// response parsing in pure functions so it can be tested against recorded
/// API responses.
#[async_trait]
pub trait Widget: Send + Sync {
    /// Stable identifier used in configs, e.g. `sonarr`.
    fn kind(&self) -> &'static str;

    /// The settings form for this widget.
    fn schema(&self) -> WidgetSchema;

    /// Fetches live data from the service. The config has already been
    /// validated against [`Widget::schema`] and has defaults filled in.
    async fn fetch(
        &self,
        ctx: &WidgetContext,
        cfg: &WidgetConfig,
    ) -> Result<WidgetData, WidgetError>;

    /// Checks that the service is reachable and the credentials work,
    /// returning a short success message. The default runs
    /// [`Widget::fetch`].
    async fn test_connection(
        &self,
        ctx: &WidgetContext,
        cfg: &WidgetConfig,
    ) -> Result<String, WidgetError> {
        let data = self.fetch(ctx, cfg).await?;
        Ok(format!("Connected, {} stats received", data.stats.len()))
    }

    /// How long a result may be served from cache.
    fn cache_ttl(&self) -> Duration {
        DEFAULT_CACHE_TTL
    }
}
