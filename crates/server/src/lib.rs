//! The homedit HTTP server.
//!
//! [`app`] builds the axum router: the REST API under `/api` (see [`api`]),
//! login under `/api/auth` (from `homedit-auth`), live events over SSE,
//! custom icon files under `/icons/custom` (and `/icons`), uploaded
//! background images under `/images`, `/healthz`, `/readyz` and
//! `/metrics` (from `homedit-ops`), and the embedded React UI for
//! everything else (see [`assets`]). The Krifmaz kit's shared routes (health, What's new, notices, bug
//! reports, diagnostics, settings) come from [`kit`].
//!
//! [`AppState`] is shared by all handlers. It holds the database, the auth
//! state (users, sessions, the secret box), the widget registry, the
//! in-memory status cache with its broadcast channel
//! ([`status::StatusHub`]), metrics, and the data folder.

/// This build's version: `HOMEDIT_VERSION` at build time (the release sets it), else Cargo.toml's.
pub const VERSION: &str = match option_env!("HOMEDIT_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};

pub mod api;
pub mod assets;
pub mod auth;
pub mod discovery;
pub mod docs;
pub mod error;
pub mod export;
pub mod icons;
pub mod images;
pub mod import;
pub mod info;
pub mod kit;
pub mod scan;
pub mod schedule;
pub mod secrets;
pub mod status;
pub mod view;
pub mod widgets;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use axum::Router;
use axum::extract::FromRef;
use axum::http::{HeaderName, HeaderValue};
use homedit_auth::{AuthConfig, AuthState, secrets::SecretBox};
use homedit_db::Db;
use homedit_ops::{Metrics, OpsState, ReadinessCheck};
use homedit_widgets::{ContextOptions, WidgetContext, WidgetRegistry};
use tower_http::compression::CompressionLayer;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;

pub use error::ApiError;

/// State shared by every request handler. Cheap to clone.
#[derive(Clone)]
pub struct AppState {
    /// Database handle.
    pub db: Db,
    /// Users, sessions, CSRF and the secret box.
    pub auth: AuthState,
    /// Latest status per service, history, and the live event channel.
    pub status: Arc<status::StatusHub>,
    /// Service widget implementations with their cache.
    pub widgets: Arc<WidgetRegistry>,
    /// HTTP client and SSRF guard for widget requests.
    pub widget_ctx: WidgetContext,
    /// Prometheus metrics.
    pub metrics: Metrics,
    /// The port scanner and its single running job.
    pub scans: Arc<scan::ScanJobs>,
    /// Cache and CPU sampler for header (info) widgets.
    pub info: Arc<info::InfoState>,
    /// Root of the data folder (database, uploads, secret key).
    pub data_dir: PathBuf,
    /// The Krifmaz kit's state: settings, What's new, notices, bug reports (see [`kit`]).
    pub kit: Arc<kit::Kit>,
}

impl FromRef<AppState> for AuthState {
    fn from_ref(s: &AppState) -> AuthState {
        s.auth.clone()
    }
}

impl FromRef<AppState> for Metrics {
    fn from_ref(s: &AppState) -> Metrics {
        s.metrics.clone()
    }
}

impl AppState {
    /// Opens (creating if needed) the data folder and database at
    /// `data_dir`, sets up auth from the environment (`HOMEDIT_AUTH_*`,
    /// `HOMEDIT_SECRET_KEY` or `secret.key` in the data folder), and loads
    /// recent status history into memory.
    pub async fn open(data_dir: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(data_dir.join("icons"))
            .with_context(|| format!("cannot create data folder {}", data_dir.display()))?;
        let db = Db::open(&data_dir.join("homedit.db"))
            .await
            .context("cannot open database")?;
        let kit = kit::Kit::open(data_dir, |name| std::env::var(name).ok()).await?;
        let config = kit.auth_config().context("invalid auth settings")?;
        let secret_box = SecretBox::from_env_or_file(&data_dir.join("secret.key"))
            .context("cannot load the secret key")?;
        Self::build(db, data_dir.to_owned(), config, secret_box, kit).await
    }

    /// Builds the state around an existing database with auth turned off
    /// and a random secret key (used by tests).
    pub async fn with_db(db: Db, data_dir: PathBuf) -> anyhow::Result<Self> {
        let key = SecretBox::generate_key();
        let secret_box = SecretBox::from_base64(&key)?;
        let kit = kit::Kit::memory().await?;
        Self::build(db, data_dir, AuthConfig::default(), secret_box, kit).await
    }

    /// Builds the state from its parts.
    pub async fn build(
        db: Db,
        data_dir: PathBuf,
        config: AuthConfig,
        secret_box: SecretBox,
        kit: kit::Kit,
    ) -> anyhow::Result<Self> {
        homedit_auth::schema::init(db.pool())
            .await
            .context("cannot create auth tables")?;
        let auth = AuthState::new(db.pool().clone(), config, secret_box).await?;
        let hub = status::StatusHub::default();
        hub.load(db.recent_samples(status::HISTORY_LEN as i64).await?);
        let widget_ctx = WidgetContext::new(ContextOptions::default())?;
        let state = Self {
            db,
            auth,
            status: Arc::new(hub),
            widgets: Arc::new(WidgetRegistry::builtin()),
            widget_ctx,
            metrics: Metrics::new(crate::VERSION),
            scans: Arc::new(scan::ScanJobs::from_lookup(kit::lookup(&kit.settings))?),
            info: Arc::new(info::InfoState::default()),
            data_dir,
            kit: Arc::new(kit),
        };
        secrets::seal_stored(&state).await?;
        icons::store_inline_icons_stored(&state).await?;
        Ok(state)
    }

    /// Folder that holds uploaded icons.
    pub fn icon_dir(&self) -> PathBuf {
        self.data_dir.join("icons")
    }

    /// Folder that holds uploaded background images.
    pub fn image_dir(&self) -> PathBuf {
        self.data_dir.join("images")
    }
}

/// Builds the complete router.
pub fn app(state: AppState) -> Router {
    let pool = state.db.pool().clone();
    let ops = OpsState::new(state.metrics.clone())
        .with_version(crate::VERSION)
        .with_check(ReadinessCheck::new("database", move || {
            let pool = pool.clone();
            async move {
                sqlx::query("SELECT 1")
                    .execute(&pool)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
        }))
        .with_metrics_token(
            Some(state.kit.settings.str("metrics_token")).filter(|t| !t.trim().is_empty()),
        );
    Router::new()
        .nest(
            "/api/auth",
            homedit_auth::routes::router().with_state(state.auth.clone()),
        )
        .merge(kit::router())
        .nest("/api", api::router())
        .nest("/icons/custom", icons::serve_router(&state.icon_dir()))
        // gethomepage serves `public/icons` at `/icons/`; serving the same
        // folder there makes imported `/icons/x.png` references work.
        .nest("/icons", icons::serve_router(&state.icon_dir()))
        .nest("/images", images::router(&state))
        .fallback(assets::serve)
        .layer(axum::middleware::from_fn_with_state(
            state.auth.clone(),
            homedit_auth::middleware::auth_middleware,
        ))
        .merge(homedit_ops::router(ops))
        .layer(axum::middleware::from_fn_with_state(
            state.metrics.clone(),
            homedit_ops::track_http,
        ))
        .layer(CompressionLayer::new())
        // Marks every answer as homedit's own, so the web app tells a proxy's error page (a 502 while the
        // server restarts) from the server's answers (the kit's update and offline rules read it).
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("x-homedit"),
            HeaderValue::from_static("1"),
        ))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
