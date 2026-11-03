//! Tests for the ops routes and the HTTP metrics middleware.

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::{Router, middleware, routing::get};
use homedit_ops::{Metrics, OpsState, ReadinessCheck, probe, router, track_http};
use http_body_util::BodyExt;
use tower::ServiceExt;

fn app(ops: OpsState) -> Router {
    let metrics = ops.metrics().clone();
    Router::new()
        .route("/api/services/{id}", get(|| async { "svc" }))
        .merge(router(ops))
        .layer(middleware::from_fn_with_state(metrics, track_http))
}

async fn call(app: &Router, uri: &str, auth: Option<&str>) -> (StatusCode, String) {
    let mut request = Request::get(uri);
    if let Some(auth) = auth {
        request = request.header(header::AUTHORIZATION, auth);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

#[tokio::test]
async fn healthz_and_readyz() {
    let ok = OpsState::new(Metrics::default())
        .with_check(ReadinessCheck::new("database", || async { Ok(()) }));
    let app_ok = app(ok);
    let (status, body) = call(&app_ok, "/healthz", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(r#""status":"ok""#));
    let (status, body) = call(&app_ok, "/readyz", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let failing = OpsState::new(Metrics::default())
        .with_check(ReadinessCheck::new("database", || async { Ok(()) }))
        .with_check(ReadinessCheck::new("docker", || async {
            Err("socket missing".to_string())
        }));
    let (status, body) = call(&app(failing), "/readyz", None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(body.contains("socket missing"), "{body}");
}

#[tokio::test]
async fn middleware_uses_route_templates() {
    let ops = OpsState::new(Metrics::default());
    let app = app(ops);
    call(&app, "/api/services/1", None).await;
    call(&app, "/api/services/2", None).await;
    call(&app, "/random/path", None).await;
    let (status, text) = call(&app, "/metrics", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        text.contains(
            r#"homedit_http_requests_total{method="GET",route="/api/services/{id}",status="200"} 2"#
        ),
        "{text}"
    );
    assert!(text.contains(r#"route="unmatched",status="404""#), "{text}");
    assert!(!text.contains("/api/services/1"));
    assert!(!text.contains(r#"route="/metrics""#));
}

#[tokio::test]
async fn metrics_token_is_enforced() {
    let ops = OpsState::new(Metrics::default()).with_metrics_token(Some("s3cret".into()));
    let app = app(ops);
    assert_eq!(
        call(&app, "/metrics", None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(&app, "/metrics", Some("Bearer wrong")).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(&app, "/metrics", Some("Bearer s3cret")).await.0,
        StatusCode::OK
    );
    // Health stays open.
    assert_eq!(call(&app, "/healthz", None).await.0, StatusCode::OK);
}

#[tokio::test]
async fn probe_talks_to_a_real_server() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = app(OpsState::new(Metrics::default()));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    probe(addr, Duration::from_secs(2)).await.unwrap();

    // A closed port fails.
    let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let closed_addr = closed.local_addr().unwrap();
    drop(closed);
    assert!(probe(closed_addr, Duration::from_secs(2)).await.is_err());
}
