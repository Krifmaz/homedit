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

/// Gets a CSRF token and its cookie, as the browser does via `/api/me`.
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
    if_match: Option<&str>,
) -> (StatusCode, String) {
    let mut req = Request::builder().method(method).uri(uri);
    if method != "GET" {
        let (token, cookie) = csrf(st).await;
        req = req
            .header("x-csrf-token", token)
            .header(header::COOKIE, cookie);
    }
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
    assert_eq!(v["version"], 2, "version 2: secrets sealed at startup");
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
    let (code, body) = call(&st, "PUT", "/api/config", Some(doc.clone()), Some("2")).await;
    assert_eq!(code, StatusCode::OK, "{body}");
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["version"], 3);

    let stored = st.db.load_dashboard().await.unwrap();
    let s = &stored.groups[0].services;
    assert_eq!(s[0].name, "Sonarr 4K");
    let key = s[0].widget.as_ref().unwrap()["key"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        key.starts_with("enc:v1:"),
        "secret is encrypted at rest: {key}"
    );
    assert_eq!(s[1].ping.as_deref(), Some("10.0.0.2:22"));

    // Stale version is refused.
    let (code, _) = call(&st, "PUT", "/api/config", Some(doc.clone()), Some("2")).await;
    assert_eq!(code, StatusCode::CONFLICT);

    // Invalid document is refused with 422.
    let mut bad = doc.clone();
    bad["settings"]["theme"] = json!("neon");
    let (code, _) = call(&st, "PUT", "/api/config", Some(bad), Some("\"3\"")).await;
    assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);

    // History and restore.
    let (_, body) = call(&st, "GET", "/api/history", None, None).await;
    let h: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(h[0]["version"], 3);
    assert_eq!(h[1]["version"], 2);
    let (code, body) = call(&st, "POST", "/api/history/2/restore", None, None).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["version"], 4);
    let stored = st.db.load_dashboard().await.unwrap();
    assert_eq!(stored.groups[0].services[0].name, "Sonarr");
    assert_eq!(stored.groups[0].services.len(), 1);

    // Mutations without the CSRF token are refused.
    let req = Request::put("/api/config")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(doc.to_string()))
        .unwrap();
    let resp = app(st.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

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
    let (token, cookie) = csrf(&st).await;
    let req = Request::post("/api/icons")
        .header("x-csrf-token", token)
        .header(header::COOKIE, cookie)
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

#[tokio::test]
async fn image_upload_list_and_serve() {
    let (st, _d) = state().await;
    let boundary = "XBOUNDARY";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"My Wall.JPG\"\r\n\
         Content-Type: image/jpeg\r\n\r\nJPEGDATA\r\n--{boundary}--\r\n"
    );
    let (token, cookie) = csrf(&st).await;
    let req = Request::post("/api/images")
        .header("x-csrf-token", token)
        .header(header::COOKIE, cookie)
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
    assert_eq!(v["url"], "/images/my-wall.jpg");

    let resp = app(st.clone())
        .oneshot(
            Request::get("/images/my-wall.jpg")
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

    let (_, body) = call(&st, "GET", "/api/images", None, None).await;
    let list: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        list,
        json!([{"name": "my-wall.jpg", "url": "/images/my-wall.jpg"}])
    );

    // gethomepage-style `/icons/x.png` references are served from the icon folder.
    std::fs::create_dir_all(st.icon_dir()).unwrap();
    std::fs::write(st.icon_dir().join("nas.png"), b"PNG").unwrap();
    for path in ["/icons/nas.png", "/icons/custom/nas.png"] {
        let resp = app(st.clone())
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "{path}");
    }
}

#[tokio::test]
async fn ops_routes_and_catalog() {
    let (st, _d) = state().await;
    let (code, _) = call(&st, "GET", "/healthz", None, None).await;
    assert_eq!(code, StatusCode::OK);
    let (code, body) = call(&st, "GET", "/readyz", None, None).await;
    assert_eq!(code, StatusCode::OK, "{body}");
    let (code, body) = call(&st, "GET", "/metrics", None, None).await;
    assert_eq!(code, StatusCode::OK);
    assert!(body.contains("homedit"), "{body}");
    let (code, body) = call(&st, "GET", "/api/widgets/catalog", None, None).await;
    assert_eq!(code, StatusCode::OK);
    let cat: Value = serde_json::from_str(&body).unwrap();
    assert!(
        cat.as_array()
            .unwrap()
            .iter()
            .any(|w| w["type"] == "jellyfin")
    );
}

#[tokio::test]
async fn widget_data_reports_upstream_error() {
    let (st, _d) = state().await;
    let id = st.db.load_dashboard().await.unwrap().groups[0].services[0]
        .id
        .unwrap();
    // sonarr:8989 does not resolve in tests, so the widget reports an error
    // instead of failing the request.
    let (code, body) = call(&st, "GET", &format!("/api/widgets/{id}/data"), None, None).await;
    assert_eq!(code, StatusCode::OK, "{body}");
    let v: Value = serde_json::from_str(&body).unwrap();
    assert!(v["error"].is_string(), "{body}");
    assert!(!body.contains("TOPSECRET"));
    let (code, _) = call(&st, "GET", "/api/widgets/9999/data", None, None).await;
    assert_eq!(code, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn import_upload_replaces_dashboard() {
    let (st, _d) = state().await;
    let (token, cookie) = csrf(&st).await;
    let boundary = "B0UND";
    let services = "- Media:\n    - Jellyfin:\n        href: http://10.0.0.5:8096\n        icon: jellyfin.png\n        widget:\n          type: jellyfin\n          url: http://10.0.0.5:8096\n          key: JKEY\n";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"services.yaml\"\r\n\r\n{services}\r\n--{boundary}--\r\n"
    );
    let req = Request::post("/api/import")
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
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    let d = st.db.load_dashboard().await.unwrap();
    let s = &d.groups[0].services[0];
    assert_eq!(s.name, "Jellyfin");
    let key = s.widget.as_ref().unwrap()["key"].as_str().unwrap();
    assert!(key.starts_with("enc:v1:"), "imported secret is encrypted");
}

async fn upload(st: &AppState, filename: &str, text: &str) -> (StatusCode, String) {
    let (token, cookie) = csrf(st).await;
    let boundary = "B0UND";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n\r\n{text}\r\n--{boundary}--\r\n"
    );
    let req = Request::post("/api/import")
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
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn export_and_restore_round_trip() {
    let (st, _d) = state().await;
    let (code, plain) = call(&st, "GET", "/api/export", None, None).await;
    assert_eq!(code, StatusCode::OK, "{plain}");
    assert!(plain.contains("Sonarr") && !plain.contains("TOPSECRET") && !plain.contains("enc:"));
    let (code, full) = call(&st, "GET", "/api/export?secrets=true", None, None).await;
    assert_eq!(code, StatusCode::OK);
    assert!(full.contains("TOPSECRET"), "{full}");

    let edited = full.replace("Sonarr", "Sonarr 4K");
    let (code, body) = upload(&st, "homedit.yaml", &edited).await;
    assert_eq!(code, StatusCode::OK, "{body}");
    let d = st.db.load_dashboard().await.unwrap();
    let s = &d.groups[0].services[0];
    assert_eq!(s.name, "Sonarr 4K");
    let key = s.widget.as_ref().unwrap()["key"].as_str().unwrap();
    assert!(key.starts_with("enc:v1:"), "restored secret is encrypted");

    let (code, _) = upload(&st, "homedit.yaml", "groups: [1, 2").await;
    assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn info_widget_data() {
    let (st, _d) = state().await;
    let mut d = st.db.load_dashboard().await.unwrap();
    for kind in ["datetime", "resources", "openmeteo", "docker"] {
        d.info_widgets.push(homedit_core::InfoWidget {
            kind: kind.into(),
            ..Default::default()
        });
    }
    let v = st.db.current_version().await.unwrap();
    let (_, d) = st.db.save_dashboard(&d, Some(v), None, None).await.unwrap();
    let id = |k: &str| {
        d.info_widgets
            .iter()
            .find(|w| w.kind == k)
            .unwrap()
            .id
            .unwrap()
    };

    let (code, body) = call(
        &st,
        "GET",
        &format!("/api/info/{}", id("datetime")),
        None,
        None,
    )
    .await;
    assert_eq!((code, body.as_str()), (StatusCode::OK, "{}"));
    // No coordinates and no Docker host: both report a readable error.
    for k in ["openmeteo", "docker"] {
        let (code, body) = call(&st, "GET", &format!("/api/info/{}", id(k)), None, None).await;
        assert_eq!(code, StatusCode::BAD_GATEWAY, "{k}: {body}");
    }
    if cfg!(target_os = "linux") {
        let (code, body) = call(
            &st,
            "GET",
            &format!("/api/info/{}", id("resources")),
            None,
            None,
        )
        .await;
        assert_eq!(code, StatusCode::OK, "{body}");
        let v: Value = serde_json::from_str(&body).unwrap();
        assert!(v["memTotal"].as_u64().unwrap() > 0, "{v}");
    }
    let (code, _) = call(&st, "GET", "/api/info/99999", None, None).await;
    assert_eq!(code, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn scan_schedule_round_trip() {
    let (st, _d) = state().await;
    let (code, body) = call(&st, "GET", "/api/scan/schedule", None, None).await;
    assert_eq!(code, StatusCode::OK, "{body}");
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["enabled"], false);
    assert_eq!(v["request"]["ports"]["kind"], "catalogue");

    let bad = json!({"enabled": true, "intervalHours": 6, "request": {"targets": []}});
    let (code, _) = call(&st, "PUT", "/api/scan/schedule", Some(bad), None).await;
    assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
    let bad = json!({"enabled": false, "intervalHours": 0, "request": {"targets": []}});
    let (code, _) = call(&st, "PUT", "/api/scan/schedule", Some(bad), None).await;
    assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);

    let good = json!({"enabled": true, "intervalHours": 6,
        "request": {"targets": ["10.9.8.0/30"], "ports": {"kind": "list", "ports": "80"}}});
    let (code, body) = call(&st, "PUT", "/api/scan/schedule", Some(good), None).await;
    assert_eq!(code, StatusCode::OK, "{body}");
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["nextRunAt"], 0, "never ran, so it is due now: {v}");
    let s = homedit::schedule::load(&st).await.unwrap();
    assert!(s.enabled && s.interval_hours == 6 && s.request.targets.len() == 1);

    // One tick starts the scan and pushes the next run out by the interval.
    let id = homedit::schedule::tick(&st).await.unwrap();
    assert!(id.is_some());
    let s = homedit::schedule::load(&st).await.unwrap();
    let last = s.last_run_at.unwrap();
    assert!(!s.is_due(last + 6 * 3_600_000 - 1));
}

#[tokio::test]
async fn scan_networks_are_listed() {
    let (st, _d) = state().await;
    let (code, body) = call(&st, "GET", "/api/scan/networks", None, None).await;
    assert_eq!(code, StatusCode::OK, "{body}");
    let v: Value = serde_json::from_str(&body).unwrap();
    for n in v.as_array().unwrap() {
        let cidr: scanner::Target = n["cidr"].as_str().unwrap().parse().unwrap();
        let addr: std::net::IpAddr = n["address"].as_str().unwrap().parse().unwrap();
        let scanner::Target::Network(net) = cidr else {
            panic!("{n}")
        };
        assert!(net.contains(&addr), "{n}");
        assert!(n["docker"].is_boolean());
    }
}

#[tokio::test]
async fn api_docs_are_served() {
    let (st, _d) = state().await;
    let (code, yaml) = call(&st, "GET", "/api/openapi.yaml", None, None).await;
    assert_eq!(code, StatusCode::OK);
    assert!(yaml.starts_with("openapi: 3.1.0"));
    let (code, html) = call(&st, "GET", "/api/docs", None, None).await;
    assert_eq!(code, StatusCode::OK);
    assert!(html.contains("<title>homedit API</title>"), "{html}");
}
