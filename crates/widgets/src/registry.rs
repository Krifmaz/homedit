//! The widget catalogue and the per-instance response cache.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::Mutex;

use crate::config::WidgetConfig;
use crate::context::WidgetContext;
use crate::data::{TestReport, WidgetData};
use crate::error::WidgetError;
use crate::schema::WidgetSchema;
use crate::widget::Widget;

/// All known widget kinds, plus a cache of their latest results.
///
/// The cache is keyed by a caller-chosen instance id (for example the
/// service card id). Concurrent requests for the same instance share one
/// upstream fetch, and results are reused for the widget's
/// [`Widget::cache_ttl`]. Changing an instance's config invalidates its
/// cache entry.
pub struct WidgetRegistry {
    widgets: BTreeMap<&'static str, Arc<dyn Widget>>,
    cache: Mutex<HashMap<String, Arc<Mutex<Option<CacheEntry>>>>>,
}

struct CacheEntry {
    config: WidgetConfig,
    fetched: Instant,
    data: Arc<WidgetData>,
}

impl std::fmt::Debug for WidgetRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WidgetRegistry")
            .field("kinds", &self.widgets.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl Default for WidgetRegistry {
    fn default() -> Self {
        Self::builtin()
    }
}

impl WidgetRegistry {
    /// An empty registry.
    pub fn empty() -> Self {
        Self {
            widgets: BTreeMap::new(),
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// A registry with every built-in widget.
    pub fn builtin() -> Self {
        let mut reg = Self::empty();
        for widget in crate::widgets::all() {
            reg.register(widget);
        }
        reg
    }

    /// Adds (or replaces) a widget.
    pub fn register(&mut self, widget: Arc<dyn Widget>) {
        self.widgets.insert(widget.kind(), widget);
    }

    /// Looks up a widget by kind.
    pub fn get(&self, kind: &str) -> Result<&Arc<dyn Widget>, WidgetError> {
        self.widgets
            .get(kind)
            .ok_or_else(|| WidgetError::UnknownKind(kind.into()))
    }

    /// The registered kinds, sorted.
    pub fn kinds(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.widgets.keys().copied()
    }

    /// The schemas of all widgets, sorted by kind, for the editor's widget
    /// picker.
    pub fn schemas(&self) -> Vec<WidgetSchema> {
        self.widgets.values().map(|w| w.schema()).collect()
    }

    /// Validates and normalises a config: fills defaults, then checks it
    /// against the widget's schema.
    pub fn prepare(&self, kind: &str, cfg: WidgetConfig) -> Result<WidgetConfig, WidgetError> {
        let widget = self.get(kind)?;
        let schema = widget.schema();
        let cfg = cfg.with_defaults(&schema);
        cfg.validate(&schema)?;
        Ok(cfg)
    }

    /// Fetches fresh data, bypassing the cache.
    pub async fn fetch(
        &self,
        ctx: &WidgetContext,
        kind: &str,
        cfg: WidgetConfig,
    ) -> Result<WidgetData, WidgetError> {
        let cfg = self.prepare(kind, cfg)?;
        self.get(kind)?.fetch(ctx, &cfg).await
    }

    /// Fetches data for a dashboard instance, using the cache.
    pub async fn fetch_cached(
        &self,
        ctx: &WidgetContext,
        instance_id: &str,
        kind: &str,
        cfg: WidgetConfig,
    ) -> Result<Arc<WidgetData>, WidgetError> {
        let widget = self.get(kind)?.clone();
        let cfg = self.prepare(kind, cfg)?;
        let slot = {
            let mut cache = self.cache.lock().await;
            cache
                .entry(format!("{kind}\u{0}{instance_id}"))
                .or_default()
                .clone()
        };
        // Holding the per-instance lock across the fetch coalesces
        // concurrent requests into one upstream call.
        let mut entry = slot.lock().await;
        if let Some(e) = entry.as_ref()
            && e.config == cfg
            && e.fetched.elapsed() < widget.cache_ttl()
        {
            return Ok(e.data.clone());
        }
        let data = Arc::new(widget.fetch(ctx, &cfg).await?);
        *entry = Some(CacheEntry {
            config: cfg,
            fetched: Instant::now(),
            data: data.clone(),
        });
        Ok(data)
    }

    /// Fetches data for a service card whose widget block is stored in the
    /// gethomepage shape (`{"type": .., "url": .., ...}`), using the cache
    /// keyed by the service id.
    pub async fn fetch_service(
        &self,
        ctx: &WidgetContext,
        service_id: i64,
        widget: &serde_json::Value,
    ) -> Result<Arc<WidgetData>, WidgetError> {
        let (kind, cfg) = WidgetConfig::from_homepage(widget)?;
        self.fetch_cached(ctx, &service_id.to_string(), &kind, cfg)
            .await
    }

    /// Drops the cached result of one instance (for example after it was
    /// deleted).
    pub async fn invalidate(&self, instance_id: &str) {
        let suffix = format!("\u{0}{instance_id}");
        self.cache.lock().await.retain(|k, _| !k.ends_with(&suffix));
    }

    /// Runs the widget's connection test, for the editor's "test connection"
    /// button. Invalid configs and failures are reported in the
    /// [`TestReport`], not as errors.
    pub async fn test_connection(
        &self,
        ctx: &WidgetContext,
        kind: &str,
        cfg: WidgetConfig,
    ) -> TestReport {
        let started = Instant::now();
        let result = async {
            let widget = self.get(kind)?;
            let cfg = self.prepare(kind, cfg)?;
            widget.test_connection(ctx, &cfg).await
        }
        .await;
        let latency_ms = started.elapsed().as_millis() as u64;
        match result {
            Ok(message) => TestReport {
                ok: true,
                message,
                latency_ms,
            },
            Err(e) => TestReport {
                ok: false,
                message: e.to_string(),
                latency_ms,
            },
        }
    }
}
