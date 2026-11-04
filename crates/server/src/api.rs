//! The REST API under `/api`.
//!
//! | Method | Path                              | Role   | Purpose                         |
//! |--------|-----------------------------------|--------|---------------------------------|
//! | GET    | `/api/me`                         | any    | Who am I, CSRF token            |
//! | GET    | `/api/dashboard`                  | any    | Dashboard view for rendering    |
//! | GET    | `/api/status`                     | any    | Latest status and history       |
//! | GET    | `/api/events`                     | any    | SSE: `status` and `config`      |
//! | GET    | `/api/config`                     | admin  | Editable document               |
//! | PUT    | `/api/config`                     | admin  | Save (with `If-Match: version`) |
//! | GET    | `/api/history`                    | admin  | Saved versions, newest first    |
//! | GET    | `/api/history/{version}`          | admin  | One saved version               |
//! | POST   | `/api/history/{version}/restore`  | admin  | Restore a saved version         |
//! | GET    | `/api/icons?q=`                   | admin  | Icon search                     |
//! | POST   | `/api/icons`                      | admin  | Icon upload (multipart `file`)  |
//! | GET    | `/api/images`                     | admin  | Uploaded background images      |
//! | POST   | `/api/images`                     | admin  | Image upload (multipart `file`) |
//!
//! JSON shapes are documented in [`crate::view`].

use std::convert::Infallible;
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post};
use futures::Stream;
use serde::{Deserialize, Serialize};
use tokio_stream::StreamExt as _;
use tokio_stream::wrappers::BroadcastStream;

use crate::auth::{RequireAdmin, RequireViewer};
use crate::status::LiveEvent;
use crate::view::{self, DocView, Mode};
use crate::{
    ApiError, AppState, docs, export, icons, images, import, info, scan, schedule, secrets, widgets,
};

/// Number of history entries returned by `/api/history`.
pub const HISTORY_LIMIT: i64 = 200;

/// Builds the `/api` router.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/me", get(homedit_auth::routes::me))
        .route("/dashboard", get(dashboard))
        .route("/status", get(status))
        .route("/events", get(events))
        .route("/config", get(get_config).put(put_config))
        .route("/history", get(history))
        .route("/history/{version}", get(history_get))
        .route("/history/{version}/restore", post(history_restore))
        .route("/icons", get(icons::search).post(icons::upload))
        .route("/images", images::api_route())
        .route("/widgets/catalog", get(widgets::catalog))
        .route("/widgets/test", post(widgets::test))
        .route("/widgets/{id}/data", get(widgets::data))
        .route("/import", post(import::upload))
        .route("/export", get(export::download))
        .route("/info/{id}", get(info::data))
        .route("/scan", get(scan::current).post(scan::start))
        .route("/scan/history", get(scan::history))
        .route("/scan/schedule", get(schedule::get).put(schedule::put))
        .route("/openapi.yaml", get(docs::spec))
        .route("/docs", get(docs::page))
        .fallback(|| async { ApiError::not_found() })
}

async fn dashboard(
    State(state): State<AppState>,
    RequireViewer(user): RequireViewer,
) -> Result<Json<DocView>, ApiError> {
    let (version, dash) = state.db.load_versioned().await?;
    Ok(Json(view::to_view(
        &dash,
        version,
        Mode::Dashboard {
            show_hidden: user.can_view_hidden(),
        },
    )))
}

async fn status(
    State(state): State<AppState>,
    RequireViewer(user): RequireViewer,
) -> Result<Json<serde_json::Value>, ApiError> {
    let mut snap = state.status.snapshot();
    if !user.can_view_hidden() {
        // Don't reveal status of cards the viewer cannot see.
        let dash = state.db.load_dashboard().await?;
        let visible: std::collections::HashSet<i64> =
            view::to_view(&dash, 0, Mode::Dashboard { show_hidden: false })
                .groups
                .iter()
                .flat_map(visible_ids)
                .collect();
        snap.retain(|id, _| visible.contains(id));
    }
    let map: serde_json::Map<String, serde_json::Value> = snap
        .into_iter()
        .map(|(id, s)| (id.to_string(), serde_json::to_value(s).unwrap_or_default()))
        .collect();
    Ok(Json(serde_json::Value::Object(map)))
}

fn visible_ids(g: &view::GroupView) -> Vec<i64> {
    let mut ids: Vec<i64> = g.items.iter().filter_map(|s| s.id).collect();
    ids.extend(g.groups.iter().flat_map(visible_ids));
    ids
}

async fn events(
    State(state): State<AppState>,
    RequireViewer(user): RequireViewer,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let visible: Option<std::collections::HashSet<i64>> = if user.can_view_hidden() {
        None
    } else {
        let dash = state.db.load_dashboard().await.unwrap_or_default();
        Some(
            view::to_view(&dash, 0, Mode::Dashboard { show_hidden: false })
                .groups
                .iter()
                .flat_map(visible_ids)
                .collect(),
        )
    };
    let is_admin = user.is_admin();
    let stream = BroadcastStream::new(state.status.subscribe()).filter_map(move |ev| {
        // A lagging client just misses some events; the next ones arrive.
        let ev = ev.ok()?;
        if let (LiveEvent::Status { service_id, .. }, Some(v)) = (&ev, &visible)
            && !v.contains(service_id)
        {
            return None;
        }
        // Scan progress is for admins only.
        if matches!(ev, LiveEvent::Scan { .. }) && !is_admin {
            return None;
        }
        let mut data = serde_json::to_value(&ev).ok()?;
        if let Some(o) = data.as_object_mut() {
            o.remove("event");
        }
        Some(Ok(Event::default().event(ev.name()).data(data.to_string())))
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(20)))
}

async fn get_config(
    RequireAdmin(_): RequireAdmin,
    State(state): State<AppState>,
) -> Result<(HeaderMap, Json<DocView>), ApiError> {
    let (version, dash) = state.db.load_versioned().await?;
    let mut h = HeaderMap::new();
    if let Ok(v) = version.to_string().parse() {
        h.insert(header::ETAG, v);
    }
    Ok((h, Json(view::to_view(&dash, version, Mode::Config))))
}

/// Response of a save or restore.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Saved {
    /// The new version.
    pub version: i64,
}

/// Reads the expected version from `If-Match` (`3`, `"3"` or `W/"3"`).
fn if_match(headers: &HeaderMap) -> Result<Option<i64>, ApiError> {
    let Some(v) = headers.get(header::IF_MATCH) else {
        return Ok(None);
    };
    let s = v
        .to_str()
        .map_err(|_| ApiError::bad_request("bad If-Match header"))?
        .trim()
        .trim_start_matches("W/")
        .trim_matches('"');
    s.parse()
        .map(Some)
        .map_err(|_| ApiError::bad_request("If-Match must be a version number"))
}

async fn put_config(
    RequireAdmin(admin): RequireAdmin,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(doc): Json<DocView>,
) -> Result<Json<Saved>, ApiError> {
    let expected = if_match(&headers)?;
    let stored = state.db.load_dashboard().await?;
    let mut dash = view::from_view(&doc, &stored)
        .map_err(|e| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;
    secrets::seal(&mut dash, state.auth.secrets());
    let (version, _) = state
        .db
        .save_dashboard(&dash, expected, Some(&admin.author()), Some("edited"))
        .await?;
    state.status.publish(LiveEvent::Config { version });
    Ok(Json(Saved { version }))
}

async fn history(
    RequireAdmin(_): RequireAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<homedit_db::VersionInfo>>, ApiError> {
    Ok(Json(state.db.history(HISTORY_LIMIT).await?))
}

async fn history_get(
    RequireAdmin(_): RequireAdmin,
    State(state): State<AppState>,
    Path(version): Path<i64>,
) -> Result<Json<DocView>, ApiError> {
    let dash = state.db.version_snapshot(version).await?;
    Ok(Json(view::to_view(&dash, version, Mode::Config)))
}

async fn history_restore(
    RequireAdmin(admin): RequireAdmin,
    State(state): State<AppState>,
    Path(version): Path<i64>,
) -> Result<Json<Saved>, ApiError> {
    let dash = state.db.version_snapshot(version).await?;
    let summary = format!("restored version {version}");
    let (new_version, _) = state
        .db
        .save_dashboard(&dash, None, Some(&admin.author()), Some(&summary))
        .await?;
    state.status.publish(LiveEvent::Config {
        version: new_version,
    });
    Ok(Json(Saved {
        version: new_version,
    }))
}
