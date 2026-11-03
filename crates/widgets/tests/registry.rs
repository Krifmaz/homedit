//! End-to-end tests of the registry against a local mock service.

use homedit_widgets::{WidgetConfig, WidgetContext, WidgetError, WidgetRegistry};
use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fixture(p: &str) -> serde_json::Value {
    let full = format!("{}/tests/fixtures/{p}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(full).unwrap()).unwrap()
}

async fn mock_sonarr() -> MockServer {
    let server = MockServer::start().await;
    for (p, f) in [
        ("/api/v3/series", "sonarr/series.json"),
        ("/api/v3/wanted/missing", "sonarr/wanted_missing.json"),
        ("/api/v3/queue", "sonarr/queue.json"),
    ] {
        Mock::given(method("GET"))
            .and(path(p))
            .and(header("X-Api-Key", "secret"))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture(f)))
            .expect(1)
            .mount(&server)
            .await;
    }
    server
}

#[tokio::test]
async fn fetch_service_uses_cache_and_coalesces() {
    let server = mock_sonarr().await;
    let reg = WidgetRegistry::builtin();
    let ctx = WidgetContext::with_defaults().unwrap();
    let widget = json!({"type": "sonarr", "url": server.uri(), "key": "secret"});
    // Concurrent and repeated calls hit the service once (`expect(1)`).
    let (a, b) = tokio::join!(
        reg.fetch_service(&ctx, 7, &widget),
        reg.fetch_service(&ctx, 7, &widget)
    );
    assert_eq!(a.unwrap(), b.unwrap());
    let again = reg.fetch_service(&ctx, 7, &widget).await.unwrap();
    assert!(again.stat("queued").is_some());
}

#[tokio::test]
async fn bad_key_reports_auth_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let reg = WidgetRegistry::builtin();
    let ctx = WidgetContext::with_defaults().unwrap();
    let cfg = WidgetConfig::default()
        .with("url", server.uri())
        .with("key", "wrong");
    let err = reg.fetch(&ctx, "sonarr", cfg.clone()).await.unwrap_err();
    assert!(matches!(err, WidgetError::Auth));
    let report = reg.test_connection(&ctx, "sonarr", cfg).await;
    assert!(!report.ok);
    assert!(!report.message.contains("wrong"));
}

#[tokio::test]
async fn redirects_are_not_followed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("Location", "http://169.254.169.254/"),
        )
        .mount(&server)
        .await;
    let reg = WidgetRegistry::builtin();
    let ctx = WidgetContext::with_defaults().unwrap();
    let cfg = WidgetConfig::default()
        .with("url", server.uri())
        .with("key", "k");
    let err = reg.fetch(&ctx, "radarr", cfg).await.unwrap_err();
    assert!(matches!(err, WidgetError::Status { status: 302 }));
}

#[tokio::test]
async fn missing_required_field_is_rejected_before_any_request() {
    let reg = WidgetRegistry::builtin();
    let ctx = WidgetContext::with_defaults().unwrap();
    let cfg = WidgetConfig::default().with("url", "http://127.0.0.1:9");
    let err = reg.fetch(&ctx, "sonarr", cfg).await.unwrap_err();
    assert!(matches!(err, WidgetError::InvalidConfig(_)));
}

#[test]
fn every_schema_starts_with_url_and_matches_kind() {
    let reg = WidgetRegistry::builtin();
    for kind in reg.kinds() {
        let schema = reg.get(kind).unwrap().schema();
        assert_eq!(schema.kind, kind);
        assert_eq!(schema.fields[0].key, "url");
        assert_eq!(kind, kind.to_ascii_lowercase());
    }
}
