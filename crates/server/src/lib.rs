//! The homedit HTTP server.
//!
//! [`app`] builds the axum router: the REST API under `/api` (see [`api`]),
//! live events over SSE, custom icon files under `/icons/custom`, and the
//! embedded React UI for everything else (see [`assets`]).
//!
//! [`AppState`] is shared by all handlers. It holds the database, the
//! in-memory status cache with its broadcast channel ([`status::StatusHub`]),
//! and the data folder.

pub mod api;
pub mod assets;
pub mod auth;
pub mod error;
pub mod icons;
pub mod status;
pub mod view;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use axum::Router;
use homedit_db::Db;
use tower_http::compression::CompressionLayer;
use tower_http::trace::TraceLayer;

pub use error::ApiError;

/// State shared by every request handler. Cheap to clone.
#[derive(Clone)]
pub struct AppState {
    /// Database handle.
    pub db: Db,
    /// Latest status per service, history, and the live event channel.
    pub status: Arc<status::StatusHub>,
    /// Root of the data folder (database, uploads).
    pub data_dir: PathBuf,
}

impl AppState {
    /// Opens (creating if needed) the data folder and database at
    /// `data_dir`, and loads recent status history into memory.
    pub async fn open(data_dir: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(data_dir.join("icons"))
            .with_context(|| format!("cannot create data folder {}", data_dir.display()))?;
        let db = Db::open(&data_dir.join("homedit.db"))
            .await
            .context("cannot open database")?;
        Self::with_db(db, data_dir.to_owned()).await
    }

    /// Builds the state around an existing database (used by tests).
    pub async fn with_db(db: Db, data_dir: PathBuf) -> anyhow::Result<Self> {
        let hub = status::StatusHub::default();
        hub.load(db.recent_samples(status::HISTORY_LEN as i64).await?);
        Ok(Self {
            db,
            status: Arc::new(hub),
            data_dir,
        })
    }

    /// Folder that holds uploaded icons.
    pub fn icon_dir(&self) -> PathBuf {
        self.data_dir.join("icons")
    }
}

/// Builds the complete router.
pub fn app(state: AppState) -> Router {
    Router::new()
        .nest("/api", api::router())
        .nest("/icons/custom", icons::serve_router(&state.icon_dir()))
        .fallback(assets::serve)
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
