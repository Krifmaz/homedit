//! End-to-end tests of the middleware, extractors and routes through a real
//! axum router and an in-memory SQLite database.

use std::net::SocketAddr;

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, StatusCode, header};
use axum::routing::{get, post};
use homedit_auth::config::{ProxyConfig, RateLimitConfig};
use homedit_auth::middleware::auth_middleware;
use homedit_auth::secrets::SecretBox;
use homedit_auth::{AuthConfig, AuthState, CurrentUser, RequireAdmin, RequireViewer, Role};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::sqlite::SqlitePoolOptions;
use tower::ServiceExt;

async fn app(cfg: AuthConfig) -> (Router, AuthState) {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    homedit_auth::schema::init(&pool).await.unwrap();
    homedit_auth::schema::init(&pool).await.unwrap(); // idempotent
    let state = AuthState::new(pool, cfg, SecretBox::from_key(&[1; 32]))
        .await
        .unwrap();
    let router = Router::new()
        .route(
            "/api/dashboard",
            get(|u: RequireViewer| async move { format!("hidden:{}", u.0.can_view_group(true)) }),
        )
        .route(
            "/api/config",
            post(|RequireAdmin(u): RequireAdmin| async move { u.author() }),
        )
        .route(
            "/api/whoami",
            get(|u: CurrentUser| async move { format!("{:?}", u.role) }),
        )
        .route("/api/me", get(homedit_auth::routes::me))
        .nest("/api/auth", homedit_auth::routes::router())
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .with_state(state.clone());
    (router, state)
}

/// A tiny cookie-jar client.
struct Client {
    router: Router,
    cookies: Vec<(String, String)>,
    csrf: String,
    peer: SocketAddr,
}

struct Res {
    status: StatusCode,
    body: Value,
    headers: axum::http::HeaderMap,
}

impl Client {
    fn new(router: Router) -> Self {
        Self {
            router,
            cookies: vec![],
            csrf: String::new(),
            peer: "192.168.1.50:5555".parse().unwrap(),
        }
    }

    async fn send(
        &mut self,
        method: Method,
        uri: &str,
        body: Option<Value>,
        extra: &[(&str, &str)],
    ) -> Res {
        let mut b = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::HOST, "dash.lan");
        if !self.cookies.is_empty() {
            let c = self
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; ");
            b = b.header(header::COOKIE, c);
        }
        if !self.csrf.is_empty() {
            b = b.header("x-csrf-token", &self.csrf);
        }
        for (k, v) in extra {
            b = b.header(*k, *v);
        }
        let body = match body {
            Some(v) => {
                b = b.header(header::CONTENT_TYPE, "application/json");
                Body::from(v.to_string())
            }
            None => Body::empty(),
        };
        let mut req = b.body(body).unwrap();
        req.extensions_mut().insert(ConnectInfo(self.peer));
        let res = self.router.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let headers = res.headers().clone();
        for v in headers.get_all(header::SET_COOKIE) {
            let s = v.to_str().unwrap();
            let pair = s.split(';').next().unwrap();
            let (k, val) = pair.split_once('=').unwrap();
            self.cookies.retain(|(n, _)| n != k);
            if !s.contains("Max-Age=0") {
                self.cookies.push((k.to_string(), val.to_string()));
            }
        }
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let body = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()));
        Res {
            status,
            body,
            headers,
        }
    }

    async fn refresh_csrf(&mut self) -> Value {
        let me = self.send(Method::GET, "/api/me", None, &[]).await.body;
        self.csrf = me["csrfToken"].as_str().unwrap().to_string();
        me
    }
}

fn enabled() -> AuthConfig {
    AuthConfig {
        enabled: true,
        ..AuthConfig::default()
    }
}

#[tokio::test]
async fn disabled_auth_is_admin_but_still_csrf_protected() {
    let (router, _) = app(AuthConfig::default()).await;
    let mut c = Client::new(router);
    let me = c.refresh_csrf().await;
    assert_eq!(me["role"], "admin");
    assert_eq!(me["authEnabled"], false);
    assert!(c.cookies.iter().any(|(k, _)| k == "homedit_csrf"));

    let token = std::mem::take(&mut c.csrf);
    assert_eq!(
        c.send(Method::POST, "/api/config", None, &[]).await.status,
        StatusCode::FORBIDDEN
    );
    c.csrf = token;
    let ok = c.send(Method::POST, "/api/config", None, &[]).await;
    assert_eq!(ok.status, StatusCode::OK);
    assert_eq!(ok.body, "admin");

    // Cross-origin with a valid token is still refused.
    let r = c
        .send(
            Method::POST,
            "/api/config",
            None,
            &[("origin", "https://evil.example")],
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert_eq!(r.body["error"], "csrf");
}

#[tokio::test]
async fn setup_login_roles_and_users() {
    let (router, _) = app(AuthConfig {
        anonymous_role: None,
        ..enabled()
    })
    .await;
    let mut c = Client::new(router);
    let me = c.refresh_csrf().await;
    assert_eq!(me["setupRequired"], true);
    assert_eq!(me["role"], Value::Null);
    assert_eq!(
        c.send(Method::GET, "/api/dashboard", None, &[])
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );

    // First-run setup creates an admin and logs in.
    let r = c
        .send(
            Method::POST,
            "/api/auth/setup",
            Some(json!({"username": "root", "password": "very secret pw"})),
            &[],
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    let cookie = r
        .headers
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_owned())
        .find(|s| s.starts_with("homedit_session="))
        .unwrap();
    assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Lax"));
    c.csrf = r.body["csrfToken"].as_str().unwrap().into();
    assert_eq!(
        c.send(Method::GET, "/api/dashboard", None, &[]).await.body,
        "hidden:true"
    );
    assert_eq!(
        c.send(Method::POST, "/api/config", None, &[]).await.body,
        "root"
    );

    // Setup only works once.
    let again = c
        .send(
            Method::POST,
            "/api/auth/setup",
            Some(json!({"username": "x", "password": "very secret pw"})),
            &[],
        )
        .await;
    assert_eq!(again.status, StatusCode::CONFLICT);

    // Admin creates a viewer.
    let r = c
        .send(
            Method::POST,
            "/api/auth/users",
            Some(json!({"username": "kid", "password": "another secret", "role": "viewer"})),
            &[],
        )
        .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    let kid_id = r.body["id"].as_i64().unwrap();
    assert!(r.body.get("passwordHash").is_none());

    // Short passwords are refused.
    let r = c
        .send(
            Method::POST,
            "/api/auth/users",
            Some(json!({"username": "y", "password": "short"})),
            &[],
        )
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);

    // Can't remove the last admin.
    let users = c.send(Method::GET, "/api/auth/users", None, &[]).await.body;
    let root_id = users
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["username"] == "root")
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    let r = c
        .send(
            Method::PATCH,
            &format!("/api/auth/users/{root_id}"),
            Some(json!({"role": "viewer"})),
            &[],
        )
        .await;
    assert_eq!(r.status, StatusCode::CONFLICT);

    // Viewer logs in in another browser.
    let mut v = Client::new(c.router.clone());
    v.refresh_csrf().await;
    let r = v
        .send(
            Method::POST,
            "/api/auth/login",
            Some(json!({"username": "KID", "password": "another secret"})),
            &[],
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    v.csrf = r.body["csrfToken"].as_str().unwrap().into();
    assert_eq!(
        v.send(Method::GET, "/api/dashboard", None, &[]).await.body,
        "hidden:false"
    );
    assert_eq!(
        v.send(Method::POST, "/api/config", None, &[]).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        v.send(Method::GET, "/api/auth/users", None, &[])
            .await
            .status,
        StatusCode::FORBIDDEN
    );

    // Disabling the viewer ends their session.
    let r = c
        .send(
            Method::PATCH,
            &format!("/api/auth/users/{kid_id}"),
            Some(json!({"disabled": true})),
            &[],
        )
        .await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(
        v.send(Method::GET, "/api/dashboard", None, &[])
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );

    // Logout ends the admin session.
    assert_eq!(
        c.send(Method::POST, "/api/auth/logout", None, &[])
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        c.send(Method::GET, "/api/whoami", None, &[]).await.body,
        "None"
    );
}

#[tokio::test]
async fn login_is_rate_limited() {
    let cfg = AuthConfig {
        login_rate_limit: RateLimitConfig {
            max_failures: 3,
            window_secs: 60,
            lockout_secs: 60,
        },
        ..enabled()
    };
    let (router, state) = app(cfg).await;
    state
        .bootstrap_admin(Some("admin".into()), Some("correct password".into()))
        .await
        .unwrap();
    let mut c = Client::new(router);
    c.refresh_csrf().await;
    for _ in 0..3 {
        let r = c
            .send(
                Method::POST,
                "/api/auth/login",
                Some(json!({"username": "admin", "password": "nope"})),
                &[],
            )
            .await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    }
    let r = c
        .send(
            Method::POST,
            "/api/auth/login",
            Some(json!({"username": "admin", "password": "correct password"})),
            &[],
        )
        .await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(r.headers.get(header::RETRY_AFTER).is_some());
}

#[tokio::test]
async fn login_needs_csrf_token() {
    let (router, state) = app(enabled()).await;
    state
        .bootstrap_admin(Some("admin".into()), Some("correct password".into()))
        .await
        .unwrap();
    let mut c = Client::new(router);
    let r = c
        .send(
            Method::POST,
            "/api/auth/login",
            Some(json!({"username": "admin", "password": "correct password"})),
            &[],
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn proxy_headers_only_from_trusted_peers() {
    let cfg = AuthConfig {
        anonymous_role: Some(Role::Viewer),
        proxy: Some(ProxyConfig {
            admin_group: Some("admins".into()),
            ..ProxyConfig::default()
        }),
        trusted_proxies: vec!["172.16.0.0/12".parse().unwrap()],
        ..enabled()
    };
    let (router, _) = app(cfg).await;

    let mut direct = Client::new(router.clone());
    let r = direct
        .send(
            Method::GET,
            "/api/whoami",
            None,
            &[("remote-user", "eve"), ("remote-groups", "admins")],
        )
        .await;
    assert_eq!(
        r.body, "Some(Viewer)",
        "forged headers from LAN are ignored"
    );

    let mut proxied = Client::new(router);
    proxied.peer = "172.18.0.2:4000".parse().unwrap();
    let hdrs = [("remote-user", "alice"), ("remote-groups", "users,admins")];
    let r = proxied.send(Method::GET, "/api/me", None, &hdrs).await;
    assert_eq!(r.body["user"]["name"], "alice");
    assert_eq!(r.body["role"], "admin");
    proxied.csrf = r.body["csrfToken"].as_str().unwrap().into();
    assert_eq!(
        proxied
            .send(Method::POST, "/api/config", None, &hdrs)
            .await
            .body,
        "alice"
    );
}

#[tokio::test]
async fn stale_session_cookie_is_cleared() {
    let (router, _) = app(enabled()).await;
    let mut c = Client::new(router);
    c.cookies.push(("homedit_session".into(), "bogus".into()));
    let r = c.send(Method::GET, "/api/whoami", None, &[]).await;
    assert!(
        r.headers
            .get_all(header::SET_COOKIE)
            .iter()
            .any(|v| v.to_str().unwrap().starts_with("homedit_session=;"))
    );
    assert!(!c.cookies.iter().any(|(k, _)| k == "homedit_session"));
}

#[tokio::test]
async fn secret_store_round_trip() {
    let (_, state) = app(enabled()).await;
    let store = state.secret_store();
    store
        .put("service:1:key", &homedit_auth::secrets::Secret::new("abc"))
        .await
        .unwrap();
    assert_eq!(
        store.get("service:1:key").await.unwrap().unwrap().expose(),
        "abc"
    );
    let raw: String = sqlx::query_scalar("SELECT ciphertext FROM auth_secrets")
        .fetch_one(state.pool())
        .await
        .unwrap();
    assert!(!raw.contains("abc"));
    store
        .apply("service:1:key", &homedit_auth::secrets::SecretInput::Keep)
        .await
        .unwrap();
    assert!(store.exists("service:1:key").await.unwrap());
    store
        .apply("service:1:key", &homedit_auth::secrets::SecretInput::Clear)
        .await
        .unwrap();
    assert!(store.get("service:1:key").await.unwrap().is_none());
}
