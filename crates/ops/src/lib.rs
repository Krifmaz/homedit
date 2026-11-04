//! Health checks and Prometheus metrics for homedit.
//!
//! The server merges [`router`] at the root of its axum app and wraps the
//! whole app with [`track_http`]:
//!
//! ```no_run
//! use axum::{Router, middleware, routing::get};
//! use homedit_ops::{OpsState, Metrics, ReadinessCheck};
//!
//! # async fn run() {
//! let metrics = Metrics::default();
//! let ops = OpsState::new(metrics.clone())
//!     .with_check(ReadinessCheck::new("database", || async { Ok(()) }));
//! let app: Router = Router::new()
//!     .route("/", get(|| async { "hello" }))
//!     .merge(homedit_ops::router(ops))
//!     .layer(middleware::from_fn_with_state(metrics, homedit_ops::track_http));
//! # }
//! ```
//!
//! Endpoints:
//!
//! * `GET /healthz`: liveness. Always `200 {"status":"ok"}` while the
//!   process serves requests. Used by the Docker `HEALTHCHECK`.
//! * `GET /readyz`: readiness. Runs every [`ReadinessCheck`] (such as a
//!   database ping) and answers `503` if any fails.
//! * `GET /metrics`: Prometheus metrics. When a token is set with
//!   [`OpsState::with_metrics_token`] (the server reads
//!   `HOMEDIT_METRICS_TOKEN`), requests must send
//!   `Authorization: Bearer <token>`.
//!
//! [`probe`] is a tiny HTTP client for `homedit healthcheck`, so the
//! container image needs no `curl` or shell.

/// This build's version: `HOMEDIT_VERSION` at build time (the release sets it), else Cargo.toml's.
pub const VERSION: &str = match option_env!("HOMEDIT_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};

pub mod metrics;
mod probe;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{MatchedPath, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

pub use metrics::{Metrics, WidgetOutcome};
pub use probe::{probe, probe_addr_from_listen};

/// How long one readiness check may take before it counts as failed.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(3);

type CheckFuture = Pin<Box<dyn Future<Output = Result<(), String>> + Send>>;

/// A named readiness check, such as "database" or "docker".
#[derive(Clone)]
pub struct ReadinessCheck {
    name: String,
    run: Arc<dyn Fn() -> CheckFuture + Send + Sync>,
}

impl std::fmt::Debug for ReadinessCheck {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReadinessCheck")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl ReadinessCheck {
    /// Creates a check from an async closure that returns `Ok(())` when the
    /// dependency is ready, or an error message.
    pub fn new<F, Fut>(name: impl Into<String>, check: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        Self {
            name: name.into(),
            run: Arc::new(move || Box::pin(check())),
        }
    }

    /// The check's name.
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// State for the ops routes.
#[derive(Debug, Clone)]
pub struct OpsState {
    metrics: Metrics,
    checks: Vec<ReadinessCheck>,
    metrics_token: Option<Arc<str>>,
    version: &'static str,
}

impl OpsState {
    /// Creates the state with no readiness checks and an open `/metrics`.
    pub fn new(metrics: Metrics) -> Self {
        Self {
            metrics,
            checks: Vec::new(),
            metrics_token: None,
            version: VERSION,
        }
    }

    /// Adds a readiness check run by `/readyz`.
    #[must_use]
    pub fn with_check(mut self, check: ReadinessCheck) -> Self {
        self.checks.push(check);
        self
    }

    /// Requires `Authorization: Bearer <token>` on `/metrics`. An empty
    /// token leaves the endpoint open.
    #[must_use]
    pub fn with_metrics_token(mut self, token: Option<String>) -> Self {
        self.metrics_token = token.filter(|t| !t.is_empty()).map(Arc::from);
        self
    }

    /// Sets the version reported by `/healthz` (defaults to this crate's).
    #[must_use]
    pub fn with_version(mut self, version: &'static str) -> Self {
        self.version = version;
        self
    }

    /// The metrics registry.
    pub fn metrics(&self) -> &Metrics {
        &self.metrics
    }
}

/// The `/healthz`, `/readyz` and `/metrics` routes.
pub fn router<S>(state: OpsState) -> Router<S> {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/metrics", get(metrics_handler))
        .with_state(state)
}

/// Body of `/healthz`.
#[derive(Debug, Serialize)]
struct Health {
    status: &'static str,
    version: &'static str,
}

async fn healthz(State(state): State<OpsState>) -> impl IntoResponse {
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(Health {
            status: "ok",
            version: state.version,
        }),
    )
}

/// One check's result in `/readyz`.
#[derive(Debug, Serialize)]
struct CheckResult {
    name: String,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    millis: u64,
}

/// Body of `/readyz`.
#[derive(Debug, Serialize)]
struct Ready {
    status: &'static str,
    checks: Vec<CheckResult>,
}

async fn readyz(State(state): State<OpsState>) -> Response {
    let runs = state.checks.iter().map(|check| async move {
        let started = Instant::now();
        let result = match tokio::time::timeout(CHECK_TIMEOUT, (check.run)()).await {
            Ok(result) => result,
            Err(_) => Err(format!("timed out after {CHECK_TIMEOUT:?}")),
        };
        CheckResult {
            name: check.name.clone(),
            ok: result.is_ok(),
            error: result.err(),
            millis: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        }
    });
    let checks = futures::future::join_all(runs).await;
    let ready = checks.iter().all(|c| c.ok);
    let status = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    for failed in checks.iter().filter(|c| !c.ok) {
        tracing::warn!(check = %failed.name, error = ?failed.error, "readiness check failed");
    }
    (
        status,
        [(header::CACHE_CONTROL, "no-store")],
        Json(Ready {
            status: if ready { "ready" } else { "not ready" },
            checks,
        }),
    )
        .into_response()
}

/// Compares two byte strings in time that depends only on their lengths.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn authorized(headers: &HeaderMap, token: &str) -> bool {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|given| constant_time_eq(given.trim().as_bytes(), token.as_bytes()))
}

async fn metrics_handler(State(state): State<OpsState>, headers: HeaderMap) -> Response {
    if let Some(token) = &state.metrics_token
        && !authorized(&headers, token)
    {
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
            "metrics token required\n",
        )
            .into_response();
    }
    (
        [
            (header::CONTENT_TYPE, metrics::CONTENT_TYPE),
            (header::CACHE_CONTROL, "no-store"),
        ],
        state.metrics.render(),
    )
        .into_response()
}

/// Axum middleware that records every request in [`Metrics`].
///
/// The `route` label is the matched route template, or `unmatched` for
/// requests that hit no route (such as SPA fallbacks and 404s), so random
/// paths cannot create new series. Requests to `/metrics` and `/healthz`
/// are not recorded, to keep scrapes and health checks out of the numbers.
pub async fn track_http(State(metrics): State<Metrics>, request: Request, next: Next) -> Response {
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map_or_else(|| "unmatched".to_string(), |p| p.as_str().to_string());
    if route == "/metrics" || route == "/healthz" {
        return next.run(request).await;
    }
    let method = request.method().as_str().to_string();
    let started = Instant::now();
    let response = next.run(request).await;
    metrics.record_http(
        &method,
        &route,
        response.status().as_u16(),
        started.elapsed().as_secs_f64(),
    );
    response
}
