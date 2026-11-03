//! End-to-end tests of the HTTP API against an in-memory database.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use homedit::{AppState, app};
use homedit_core::{Dashboard, Group, Service};
use homedit_db::Db;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

async fn state() -> (AppState, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open_in_memory().await.unwrap();
    let mut svc = Service::new("Sonarr", Some("http://sonarr:8989"));
    svc.icon = Some("sonarr.png".into());
    svc.site_monitor = Some("http://sonarr:8989".into());
    svc.widget = Some(json!({"type":"sonarr","url":"http://sonarr:8989","key":"TOPSECRET"}));
    let dash = Dashboard {
        groups: vec![Group {
            name: "Media".into(),
            services: vec![svc],
            ..Group::default()
        }],
        ..Dashboard::default()
    };
    db.save_dashboard(&dash, None, None, None).await.unwrap();
    let st = AppState::with_db(db, dir.path().to_owned()).await.unwrap();
    (st, dir)
}

async fn call(
    st: &AppState,
    method: &str,
    uri: &str,
    body: Option<Value>,
    if_match: Option<&str>,
) -> (StatusCode, String) {
    let mut req = Request::builder().method(method).uri(uri);
    if let Some(v) = if_match {
        req = req.header(header::IF_MATCH, v);
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
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn dashboard_masks_secrets() {
    let (st, _d) = state().await;
    let (code, body) = call(&st, "GET", "/api/dashboard", None, None).await;
    assert_eq!(code, StatusCode::OK);
    assert!(!body.contains("TOPSECRET"));
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["version"], 1);
    let item = &v["groups"][0]["items"][0];
    assert_eq!(item["name"], "Sonarr");
    assert_eq!(item["iconKind"], "image");
    assert_eq!(item["check"]["kind"], "http");
    assert_eq!(item["widget"]["type"], "sonarr");
}

#[tokio::test]
async fn index_inlines_data() {
    let (st, _d) = state().await;
    let (code, body) = call(&st, "GET", "/some/client/route", None, None).await;
    assert_eq!(code, StatusCode::OK);
    assert!(body.contains(r#"<script id="homedit-data" type="application/json">"#));
    assert!(body.contains("Sonarr"));
    assert!(!body.contains("TOPSECRET"));
    // Missing asset files are 404, not the SPA page.
    let (code, _) = call(&st, "GET", "/assets/missing.js", None, None).await;
    assert_eq!(code, StatusCode::NOT_FOUND);
    let (code, _) = call(&st, "GET", "/api/nope", None, None).await;
    assert_eq!(code, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn config_edit_cycle() {
    let (st, _d) = state().await;
    let (code, body) = call(&st, "GET", "/api/config", None, None).await;
    assert_eq!(code, StatusCode::OK);
    let mut doc: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        doc["groups"][0]["items"][0]["widget"]["fields"]["key"],
        json!({"$secret":"set"})
    );
    assert_eq!(doc["groups"][0]["items"][0]["icon"], "sonarr.png");

    // Rename, keep the secret, add a card with a TCP check.
    doc["groups"][0]["items"][0]["name"] = json!("Sonarr 4K");
    doc["groups"][0]["items"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"NAS","check":{"kind":"tcp","target":"10.0.0.2:22"}}));
    let (code, body) = call(&st, "PUT", "/api/config", Some(doc.clone()), Some("1")).await;
    assert_eq!(code, StatusCode::OK, "{body}");
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["version"], 2);

    let stored = st.db.load_dashboard().await.unwrap();
    let s = &stored.groups[0].services;
    assert_eq!(s[0].name, "Sonarr 4K");
    assert_eq!(s[0].widget.as_ref().unwrap()["key"], "TOPSECRET");
    assert_eq!(s[1].ping.as_deref(), Some("10.0.0.2:22"));

    // Stale version is refused.
    let (code, _) = call(&st, "PUT", "/api/config", Some(doc.clone()), Some("1")).await;
    assert_eq!(code, StatusCode::CONFLICT);

    // Invalid document is refused with 422.
    let mut bad = doc.clone();
    bad["settings"]["theme"] = json!("neon");
    let (code, _) = call(&st, "PUT", "/api/config", Some(bad), Some("\"2\"")).await;
    assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);

    // History and restore.
    let (_, body) = call(&st, "GET", "/api/history", None, None).await;
    let h: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(h[0]["version"], 2);
    assert_eq!(h[1]["version"], 1);
    let (code, body) = call(&st, "POST", "/api/history/1/restore", None, None).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["version"], 3);
    let stored = st.db.load_dashboard().await.unwrap();
    assert_eq!(stored.groups[0].services[0].name, "Sonarr");
    assert_eq!(stored.groups[0].services.len(), 1);
    let (code, _) = call(&st, "GET", "/api/history/99", None, None).await;
    assert_eq!(code, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn status_endpoint_reports_samples() {
    let (st, _d) = state().await;
    let id = st.db.load_dashboard().await.unwrap().groups[0].services[0]
        .id
        .unwrap();
    st.status.record(homedit_core::StatusSample {
        service_id: id,
        checked_at: 42,
        ok: true,
        status_code: Some(200),
        latency_ms: Some(12),
        error: None,
    });
    let (code, body) = call(&st, "GET", "/api/status", None, None).await;
    assert_eq!(code, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).unwrap();
    let s = &v[id.to_string()];
    assert_eq!(s["up"], true);
    assert_eq!(s["code"], 200);
    assert_eq!(s["latencyMs"], 12);
    assert_eq!(s["history"][0], json!({"t":42,"up":true,"ms":12}));
}

#[tokio::test]
async fn icon_upload_and_search() {
    let (st, _d) = state().await;
    let boundary = "XBOUNDARY";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"My NAS.png\"\r\n\
         Content-Type: image/png\r\n\r\nPNGDATA\r\n--{boundary}--\r\n"
    );
    let req = Request::post("/api/icons")
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();
    let resp = app(st.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v: Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(v["name"], "custom/my-nas.png");
    assert_eq!(v["url"], "/icons/custom/my-nas.png");

    let resp = app(st.clone())
        .oneshot(
            Request::get("/icons/custom/my-nas.png")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(
        resp.headers()[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .contains("sandbox")
    );

    let (_, body) = call(&st, "GET", "/api/icons?q=nas", None, None).await;
    let hits: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(hits[0]["name"], "custom/my-nas.png");
    assert!(
        hits.as_array()
            .unwrap()
            .iter()
            .any(|h| h["source"] == "selfhst")
    );
}
