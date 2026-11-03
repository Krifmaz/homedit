//! Authentication, authorization and secret handling for homedit.
//!
//! This crate is self-contained: it owns its SQLite tables (all prefixed with
//! `auth_`), exposes axum middleware, extractors and a router, and contains the
//! cryptographic and network-safety building blocks the rest of the server
//! uses.
//!
//! # Pieces
//!
//! | Module | What it does |
//! |---|---|
//! | [`config`] | [`AuthConfig`]: which login methods are on, cookies, rate limits. |
//! | [`schema`] | Creates and upgrades the `auth_*` tables. |
//! | [`password`] | argon2id hashing and verification. |
//! | [`users`] | Local and OIDC user records. |
//! | [`sessions`] | Server-side sessions keyed by a hashed random token. |
//! | [`ratelimit`] | Failed-login throttling per client IP and per username. |
//! | [`proxy`] | Trusted reverse-proxy header authentication. |
//! | [`oidc`] | OpenID Connect authorization-code login with PKCE. |
//! | [`csrf`] | Origin / Fetch-Metadata checks and per-session CSRF tokens. |
//! | [`middleware`] | [`middleware::auth_middleware`], which resolves the caller and enforces CSRF. |
//! | [`extract`] | [`CurrentUser`], [`RequireViewer`] and [`RequireAdmin`] extractors. |
//! | [`routes`] | `/api/auth/*` endpoints (login, logout, setup, OIDC, user admin). |
//! | [`secrets`] | AES-256-GCM encrypted secrets, masked as `••••` when serialized. |
//! | [`ssrf`] | An allowlist guard and a `reqwest` client for outbound widget calls. |
//!
//! # Wiring it into an axum app
//!
//! ```no_run
//! use std::net::SocketAddr;
//! use axum::{Router, routing::get, extract::FromRef};
//! use homedit_auth::{AuthConfig, AuthState, RequireAdmin, middleware::auth_middleware, secrets::SecretBox};
//!
//! #[derive(Clone)]
//! struct AppState { auth: AuthState }
//! impl FromRef<AppState> for AuthState {
//!     fn from_ref(s: &AppState) -> AuthState { s.auth.clone() }
//! }
//!
//! # async fn run(pool: sqlx::SqlitePool) -> Result<(), Box<dyn std::error::Error>> {
//! homedit_auth::schema::init(&pool).await?;
//! let auth = AuthState::new(pool, AuthConfig::from_env()?, SecretBox::from_env()?).await?;
//! let app = Router::new()
//!     .route("/api/admin-only", get(|_: RequireAdmin| async { "hi admin" }))
//!     .nest("/api/auth", homedit_auth::routes::router().with_state(auth.clone()))
//!     .layer(axum::middleware::from_fn_with_state(auth.clone(), auth_middleware))
//!     .with_state(AppState { auth });
//! let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
//! axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await?;
//! # Ok(()) }
//! ```
//!
//! The connect-info make-service is required: rate limiting and the trusted
//! proxy checks need the peer address. Without it every request is treated as
//! coming from an unknown, untrusted peer.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod client_ip;
pub mod config;
pub mod csrf;
pub mod error;
pub mod extract;
pub mod middleware;
pub mod oidc;
pub mod password;
pub mod proxy;
pub mod ratelimit;
pub mod role;
pub mod routes;
pub mod schema;
pub mod secrets;
pub mod sessions;
pub mod ssrf;
pub mod state;
pub mod users;

mod util;

pub use config::AuthConfig;
pub use error::AuthError;
pub use extract::{AuthUser, CurrentUser, RequireAdmin, RequireViewer};
pub use role::{GroupVisibility, Role, UserSource};
pub use state::AuthState;
