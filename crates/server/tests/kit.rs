//! The Krifmaz kit's routes on homedit: health, What's new, preferences, notices, diagnostics, settings
//! and bug reports, with sign-in off (every visitor is the admin) and through homedit's CSRF check.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use homedit::{AppState, app};
use homedit_db::Db;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

async fn state() -> (AppState, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open_in_memory().await.unwrap();
    let st = AppState::with_db(db, dir.path().to_owned()).await.unwrap();
    (st, dir)
}

/// The CSRF token and its cookie, as the browser gets them from `/api/me`.
async fn csrf(st: &AppState) -> (String, String) {
    let resp = app(st.clone())
        .oneshot(Request::get("/api/me").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let cookie = resp
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .map(|c| c.split(';').next().unwrap_or_default().to_owned())
        .collect::<Vec<_>>()
        .join("; ");
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let me: Value = serde_json::from_slice(&bytes).unwrap();
    (me["csrfToken"].as_str().unwrap().to_owned(), cookie)
}

async fn call(
    st: &AppState,
    method: &str,
    uri: &str,
    body: Option<Value>,
    with_csrf: bool,
) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri);
    if with_csrf {
        let (token, cookie) = csrf(st).await;
        req = req
            .header("x-csrf-token", token)
            .header(header::COOKIE, cookie);
    }
    let req = match body {
        Some(b) => req
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(b.to_string())),
        None => req.body(Body::empty()),
    }
    .unwrap();
    let resp = app(st.clone()).oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn health_answers_the_version() {
    let (st, _d) = state().await;
    let (s, v) = call(&st, "GET", "/api/health", None, false).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["ok"], json!(true));
    assert_eq!(v["version"], json!(env!("CARGO_PKG_VERSION")));
}

#[tokio::test]
async fn whats_new_lists_the_release_notes_and_the_switch_turns_the_popup_off() {
    let (st, _d) = state().await;
    let (s, v) = call(&st, "GET", "/api/changelog", None, false).await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        v["releases"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["version"] == "0.1.0"),
        "{v}"
    );
    let (s, _) = call(
        &st,
        "PUT",
        "/api/auth/prefs",
        Some(json!({ "whats_new_popup": false })),
        true,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (_, v) = call(&st, "GET", "/api/changelog/new", None, false).await;
    assert_eq!(v["popup"], json!(false));
    assert_eq!(v["show"], json!(false));
}

#[tokio::test]
async fn changes_without_the_csrf_token_are_refused() {
    let (st, _d) = state().await;
    let (s, _) = call(
        &st,
        "PUT",
        "/api/auth/prefs",
        Some(json!({ "whats_new_popup": false })),
        false,
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = call(
        &st,
        "PUT",
        "/api/settings",
        Some(json!({ "auth_enabled": true })),
        false,
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn settings_schema_and_values() {
    let (st, _d) = state().await;
    let (s, v) = call(&st, "GET", "/api/settings/schema", None, false).await;
    assert_eq!(s, StatusCode::OK);
    let text = v.to_string();
    assert!(
        text.contains("Require sign-in") && text.contains("Docker address"),
        "{text}"
    );
    let (s, v) = call(&st, "GET", "/api/settings", None, false).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["auth_enabled"], json!(false));
    assert_eq!(v["scan_rate"], json!(300));
}

#[tokio::test]
async fn a_save_that_would_not_start_is_refused() {
    let (st, _d) = state().await;
    let (s, v) = call(
        &st,
        "PUT",
        "/api/settings",
        Some(json!({ "oidc_issuer": "https://id.example.com" })),
        true,
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["detail"].as_str().unwrap().contains("Client ID"), "{v}");
    let (s, v) = call(
        &st,
        "PUT",
        "/api/settings",
        Some(json!({ "docker_host": "docker.sock" })),
        true,
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    let (s, v) = call(
        &st,
        "PUT",
        "/api/settings",
        Some(json!({ "scan_rate": 50, "auth_enabled": true })),
        true,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["scan_rate"], json!(50));
    assert!(v["_warnings"].to_string().contains("Restart"), "{v}");
}

#[tokio::test]
async fn diagnostics_and_notices() {
    let (st, _d) = state().await;
    let (s, v) = call(&st, "GET", "/api/diagnostics", None, false).await;
    assert_eq!(s, StatusCode::OK);
    let names: Vec<&str> = v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["name"].as_str())
        .collect();
    for n in [
        "Data folder",
        "Database",
        "Settings and reports database",
        "Secret key",
        "Sign-in",
    ] {
        assert!(names.contains(&n), "{names:?}");
    }
    let (s, v) = call(&st, "POST", "/api/diagnostics/docker/test", None, true).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["ok"], json!(false));
    let (s, v) = call(&st, "GET", "/api/notices", None, false).await;
    assert_eq!(s, StatusCode::OK);
    let ids: Vec<&str> = v["notices"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|n| n["id"].as_str())
        .collect();
    assert_eq!(ids, ["data", "docker"]);
}

#[tokio::test]
async fn bug_reports_are_kept() {
    let (st, _d) = state().await;
    let (token, cookie) = csrf(&st).await;
    let boundary = "XBOUNDARY";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nCards jump\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"body\"\r\n\r\nThey move when I drag.\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"kind\"\r\n\r\nbug\r\n--{boundary}--\r\n"
    );
    let req = Request::post("/api/bug-reports")
        .header("x-csrf-token", token)
        .header(header::COOKIE, cookie)
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();
    let resp = app(st.clone()).oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    assert!(
        status.is_success(),
        "{status} {}",
        String::from_utf8_lossy(&bytes)
    );
    let (s, v) = call(&st, "GET", "/api/bug-reports", None, false).await;
    assert_eq!(s, StatusCode::OK);
    assert!(v.to_string().contains("Cards jump"), "{v}");
}

#[tokio::test]
async fn answers_are_marked_as_homedit() {
    let (st, _d) = state().await;
    let resp = app(st)
        .oneshot(Request::get("/api/nope").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(
        resp.headers().get("x-homedit").map(|v| v.to_str().unwrap()),
        Some("1")
    );
}
