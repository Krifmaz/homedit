//! The homedit HTTP server.
//!
//! [`app`] builds the axum router: the REST API under `/api` (see [`api`]),
//! login under `/api/auth` (from `homedit-auth`), live events over SSE,
//! custom icon files under `/icons/custom`, `/healthz`, `/readyz` and
//! `/metrics` (from `homedit-ops`), and the embedded React UI for
//! everything else (see [`assets`]).
//!
//! [`AppState`] is shared by all handlers. It holds the database, the auth
//! state (users, sessions, the secret box), the widget registry, the
//! in-memory status cache with its broadcast channel
//! ([`status::StatusHub`]), metrics, and the data folder.

pub mod api;
pub mod assets;
pub mod auth;
pub mod discovery;
pub mod error;
pub mod export;
pub mod icons;
pub mod import;
pub mod info;
pub mod scan;
pub mod secrets;
pub mod status;
pub mod view;
pub mod widgets;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use axum::Router;
use axum::extract::FromRef;
use homedit_auth::{AuthConfig, AuthState, secrets::SecretBox};
use homedit_db::Db;
use homedit_ops::{Metrics, OpsState, ReadinessCheck};
use homedit_widgets::{ContextOptions, WidgetContext, WidgetRegistry};
use tower_http::compression::CompressionLayer;
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
        let config = AuthConfig::from_env().context("invalid auth settings")?;
        let secret_box = SecretBox::from_env_or_file(&data_dir.join("secret.key"))
            .context("cannot load the secret key")?;
        Self::build(db, data_dir.to_owned(), config, secret_box).await
    }

    /// Builds the state around an existing database with auth turned off
    /// and a random secret key (used by tests).
    pub async fn with_db(db: Db, data_dir: PathBuf) -> anyhow::Result<Self> {
        let key = SecretBox::generate_key();
        let secret_box = SecretBox::from_base64(&key)?;
        Self::build(db, data_dir, AuthConfig::default(), secret_box).await
    }

    /// Builds the state from its parts.
    pub async fn build(
        db: Db,
        data_dir: PathBuf,
        config: AuthConfig,
        secret_box: SecretBox,
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
            metrics: Metrics::new(env!("CARGO_PKG_VERSION")),
            scans: Arc::new(scan::ScanJobs::from_env()?),
            info: Arc::new(info::InfoState::default()),
            data_dir,
        };
        secrets::seal_stored(&state).await?;
        Ok(state)
    }

    /// Folder that holds uploaded icons.
    pub fn icon_dir(&self) -> PathBuf {
        self.data_dir.join("icons")
    }
}

/// Builds the complete router.
pub fn app(state: AppState) -> Router {
    let pool = state.db.pool().clone();
    let ops = OpsState::new(state.metrics.clone())
        .with_version(env!("CARGO_PKG_VERSION"))
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
        .with_metrics_token(std::env::var("HOMEDIT_METRICS_TOKEN").ok());
    Router::new()
        .nest(
            "/api/auth",
            homedit_auth::routes::router().with_state(state.auth.clone()),
        )
        .nest("/api", api::router())
        .nest("/icons/custom", icons::serve_router(&state.icon_dir()))
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
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
