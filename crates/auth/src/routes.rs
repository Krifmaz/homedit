//! HTTP endpoints, meant to be nested under `/api/auth`.
//!
//! | Method & path | Who | What |
//! |---|---|---|
//! | `GET /me` | anyone | Caller, role, CSRF token, enabled login methods. Also mount [`me`] at `/api/me`. |
//! | `POST /login` | anyone | `{username, password}` → session cookie. Rate limited. |
//! | `POST /logout` | anyone | Ends the session. |
//! | `POST /setup` | anyone, once | Creates the first admin while no users exist. |
//! | `POST /password` | local users | `{currentPassword, newPassword}`. |
//! | `GET /oidc/login?returnTo=/path` | anyone | Redirects to the identity provider. |
//! | `GET /oidc/callback` | provider | Finishes SSO and redirects back. |
//! | `GET /users` | admin | List users. |
//! | `POST /users` | admin | `{username, password, role}`. |
//! | `PATCH /users/{id}` | admin | `{role?, password?, disabled?}`. |
//! | `DELETE /users/{id}` | admin | Delete a user. |
//!
//! All bodies are JSON with camelCase keys. State-changing requests need the
//! `X-CSRF-Token` header (see [`crate::csrf`]).

use axum::extract::{Path, Query, State};
use axum::http::header::{LOCATION, SET_COOKIE, USER_AGENT};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::AuthError;
use crate::extract::{CurrentUser, RequireAdmin};
use crate::oidc::sanitize_return_to;
use crate::password::{hash_password_async, validate_new_password, verify_password_async};
use crate::ratelimit::LoginLimiter;
use crate::role::{Role, UserSource};
use crate::state::AuthState;
use crate::users::{User, UserUpdate};
use crate::util::{ct_eq, get_cookie, set_cookie};

/// Cookie pinning an OIDC login to the browser that started it.
const OIDC_STATE_COOKIE: &str = "homedit_oidc_state";

/// The `/api/auth` router.
pub fn router() -> Router<AuthState> {
    Router::new()
        .route("/me", get(me))
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/setup", post(setup))
        .route("/password", post(change_password))
        .route("/oidc/login", get(oidc_login))
        .route("/oidc/callback", get(oidc_callback))
        .route("/users", get(list_users).post(create_user))
        .route("/users/{id}", patch(update_user).delete(delete_user))
}

/// The user part of [`MeResponse`].
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeUser {
    /// Display / login name.
    pub name: String,
    /// Row id (absent for proxy users).
    pub id: Option<i64>,
    /// How they signed in.
    pub source: UserSource,
}

/// Which login methods the login page should offer.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginMethods {
    /// Username/password form.
    pub local: bool,
    /// SSO button label, when OIDC is configured.
    pub oidc: Option<String>,
    /// Whether a reverse proxy handles login.
    pub proxy: bool,
    /// Where to send the browser to log out at the proxy.
    pub proxy_logout_url: Option<String>,
}

/// Response of `GET /api/me`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeResponse {
    /// The signed-in user, if any.
    pub user: Option<MeUser>,
    /// Effective role, `null` when the caller may see nothing.
    pub role: Option<Role>,
    /// Whether login is enabled.
    pub auth_enabled: bool,
    /// Token for the `X-CSRF-Token` header.
    pub csrf_token: String,
    /// Login methods.
    pub methods: LoginMethods,
    /// `true` while no users exist and first-run setup is allowed.
    pub setup_required: bool,
}

/// `GET /api/me`: who am I, and what may I do. Also mount this at
/// `/api/me` for the frontend contract.
pub async fn me(
    State(state): State<AuthState>,
    user: CurrentUser,
) -> Result<Json<MeResponse>, AuthError> {
    let cfg = state.config();
    let setup_required = cfg.enabled
        && cfg.local.enabled
        && cfg.local.allow_setup
        && state.users().count().await? == 0;
    Ok(Json(MeResponse {
        user: user.user.as_ref().map(|u| MeUser {
            name: u.username.clone(),
            id: u.id,
            source: u.source,
        }),
        role: user.role,
        auth_enabled: cfg.enabled,
        csrf_token: user.csrf_token.clone(),
        methods: LoginMethods {
            local: cfg.enabled && cfg.local.enabled,
            oidc: state.oidc().map(|o| o.config().button_label.clone()),
            proxy: cfg.enabled && cfg.proxy.is_some(),
            proxy_logout_url: cfg.proxy.as_ref().and_then(|p| p.logout_url.clone()),
        },
        setup_required,
    }))
}

/// Body of `POST /login` and `POST /setup`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginRequest {
    /// Username.
    pub username: String,
    /// Password.
    pub password: String,
}

/// Response of a successful login.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginResponse {
    /// The user.
    pub user: User,
    /// The new session's CSRF token.
    pub csrf_token: String,
}

async fn start_session(
    state: &AuthState,
    caller: &CurrentUser,
    headers: &HeaderMap,
    user: User,
) -> Result<Response, AuthError> {
    if let Some(old) = &caller.session {
        state.sessions().delete(&old.token_hash).await?;
    }
    state.users().touch_login(user.id).await?;
    let ip = caller.client.ip.map(|i| i.to_string());
    let ua = headers.get(USER_AGENT).and_then(|v| v.to_str().ok());
    let session = state.sessions().create(user.id, ip.as_deref(), ua).await?;
    let cookie = set_cookie(
        &state.config().session.cookie_name,
        &session.token,
        Some(state.config().session.ttl_secs.min(i64::MAX as u64) as i64),
        state.cookie_secure(&caller.client),
        cookie::SameSite::Lax,
    );
    let mut res = Json(LoginResponse {
        user,
        csrf_token: session.csrf_token,
    })
    .into_response();
    res.headers_mut().append(SET_COOKIE, cookie);
    Ok(res)
}

fn require_local(state: &AuthState) -> Result<(), AuthError> {
    let cfg = state.config();
    if !cfg.enabled {
        return Err(AuthError::Disabled("login"));
    }
    if !cfg.local.enabled {
        return Err(AuthError::Disabled("password login"));
    }
    Ok(())
}

async fn login(
    State(state): State<AuthState>,
    caller: CurrentUser,
    headers: HeaderMap,
    Json(body): Json<LoginRequest>,
) -> Result<Response, AuthError> {
    require_local(&state)?;
    let username = body.username.trim();
    let keys = LoginLimiter::keys(caller.client.ip, username);
    if let Some(wait) = state.limiter().check(&keys) {
        return Err(AuthError::RateLimited(wait));
    }
    let found = state.users().find_for_login(username).await?;
    let (user, hash) = match found {
        Some((u, h)) if !u.disabled => (Some(u), h),
        _ => (None, None),
    };
    let ok = verify_password_async(body.password, hash).await;
    match (ok, user) {
        (true, Some(user)) => {
            state.limiter().record_success(&keys[1]);
            tracing::info!(username = %user.username, ip = ?caller.client.ip, "login");
            start_session(&state, &caller, &headers, user).await
        }
        _ => {
            state.limiter().record_failure(&keys);
            tracing::warn!(username, ip = ?caller.client.ip, "failed login");
            Err(AuthError::InvalidCredentials)
        }
    }
}

async fn logout(
    State(state): State<AuthState>,
    caller: CurrentUser,
) -> Result<Response, AuthError> {
    if let Some(s) = &caller.session {
        state.sessions().delete(&s.token_hash).await?;
    }
    let logout_url = state
        .config()
        .proxy
        .as_ref()
        .and_then(|p| p.logout_url.clone());
    let mut res = Json(json!({ "ok": true, "logoutUrl": logout_url })).into_response();
    res.headers_mut().append(
        SET_COOKIE,
        set_cookie(
            &state.config().session.cookie_name,
            "",
            Some(0),
            state.cookie_secure(&caller.client),
            cookie::SameSite::Lax,
        ),
    );
    Ok(res)
}

async fn setup(
    State(state): State<AuthState>,
    caller: CurrentUser,
    headers: HeaderMap,
    Json(body): Json<LoginRequest>,
) -> Result<Response, AuthError> {
    require_local(&state)?;
    if !state.config().local.allow_setup {
        return Err(AuthError::Disabled("setup"));
    }
    let _guard = state.setup_lock().lock().await;
    if state.users().count().await? > 0 {
        return Err(AuthError::Conflict("setup is already done".into()));
    }
    validate_new_password(&body.password, state.config().local.min_password_length)?;
    let hash = hash_password_async(body.password).await?;
    let user = state
        .users()
        .create_local(body.username.trim(), &hash, Role::Admin)
        .await?;
    tracing::info!(username = %user.username, "first admin created via setup");
    start_session(&state, &caller, &headers, user).await
}

/// Body of `POST /password`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangePasswordRequest {
    /// Current password.
    pub current_password: String,
    /// New password.
    pub new_password: String,
}

async fn change_password(
    State(state): State<AuthState>,
    caller: CurrentUser,
    headers: HeaderMap,
    Json(body): Json<ChangePasswordRequest>,
) -> Result<Response, AuthError> {
    require_local(&state)?;
    let Some(id) = caller
        .user
        .as_ref()
        .filter(|_| caller.is_local())
        .and_then(|u| u.id)
    else {
        return Err(AuthError::Forbidden);
    };
    let username = caller
        .user
        .as_ref()
        .map(|u| u.username.as_str())
        .unwrap_or_default();
    let keys = LoginLimiter::keys(caller.client.ip, username);
    if let Some(wait) = state.limiter().check(&keys) {
        return Err(AuthError::RateLimited(wait));
    }
    let hash = state.users().password_hash(id).await?;
    if !verify_password_async(body.current_password, hash).await {
        state.limiter().record_failure(&keys);
        return Err(AuthError::InvalidCredentials);
    }
    validate_new_password(&body.new_password, state.config().local.min_password_length)?;
    let new_hash = hash_password_async(body.new_password).await?;
    // This ends every session of the user, including this one...
    let user = state
        .users()
        .update(
            id,
            UserUpdate {
                password_hash: Some(new_hash),
                ..Default::default()
            },
        )
        .await?;
    // ...so start a fresh one for the caller.
    let caller = CurrentUser {
        session: None,
        ..caller
    };
    start_session(&state, &caller, &headers, user).await
}

/// Query of `GET /oidc/login`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OidcLoginQuery {
    /// Path to return to after login.
    pub return_to: Option<String>,
}

fn redirect(to: &str, cookies: Vec<HeaderValue>) -> Response {
    let mut res = StatusCode::SEE_OTHER.into_response();
    if let Ok(v) = HeaderValue::from_str(to) {
        res.headers_mut().insert(LOCATION, v);
    }
    for c in cookies {
        res.headers_mut().append(SET_COOKIE, c);
    }
    res
}

async fn oidc_login(
    State(state): State<AuthState>,
    caller: CurrentUser,
    Query(q): Query<OidcLoginQuery>,
) -> Result<Response, AuthError> {
    let oidc = state.oidc().ok_or(AuthError::Disabled("single sign-on"))?;
    let (url, csrf_state) = oidc
        .begin(sanitize_return_to(q.return_to.as_deref()))
        .await?;
    let cookie = set_cookie(
        OIDC_STATE_COOKIE,
        &csrf_state,
        Some(600),
        state.cookie_secure(&caller.client),
        cookie::SameSite::Lax,
    );
    Ok(redirect(url.as_str(), vec![cookie]))
}

/// Query of `GET /oidc/callback`.
#[derive(Debug, Deserialize)]
pub struct OidcCallbackQuery {
    /// Authorization code.
    pub code: Option<String>,
    /// State echoed by the provider.
    pub state: Option<String>,
    /// Error code from the provider.
    pub error: Option<String>,
}

async fn oidc_callback(
    State(state): State<AuthState>,
    caller: CurrentUser,
    headers: HeaderMap,
    Query(q): Query<OidcCallbackQuery>,
) -> Response {
    let secure = state.cookie_secure(&caller.client);
    let clear_state = set_cookie(
        OIDC_STATE_COOKIE,
        "",
        Some(0),
        secure,
        cookie::SameSite::Lax,
    );
    match oidc_finish(&state, &caller, &headers, q).await {
        Ok((session_cookie, return_to)) => redirect(&return_to, vec![clear_state, session_cookie]),
        Err(e) => {
            let code = e.code();
            if !matches!(e, AuthError::Oidc(_)) {
                tracing::warn!(error = %e, "OIDC login refused");
            } else {
                let _ = e.into_response(); // logs the detail
            }
            redirect(&format!("/?authError={code}"), vec![clear_state])
        }
    }
}

async fn oidc_finish(
    state: &AuthState,
    caller: &CurrentUser,
    headers: &HeaderMap,
    q: OidcCallbackQuery,
) -> Result<(HeaderValue, String), AuthError> {
    let oidc = state.oidc().ok_or(AuthError::Disabled("single sign-on"))?;
    if let Some(err) = q.error {
        return Err(AuthError::Oidc(format!("provider returned error `{err}`")));
    }
    let (Some(code), Some(st)) = (q.code, q.state) else {
        return Err(AuthError::BadRequest("missing code or state".into()));
    };
    let pinned = get_cookie(headers, OIDC_STATE_COOKIE).unwrap_or_default();
    if !ct_eq(&pinned, &st) {
        return Err(AuthError::Csrf("SSO state does not match this browser"));
    }
    let (identity, return_to) = oidc.finish(&st, code).await?;
    let role = oidc.role_for(&identity.groups)?;
    let user = state
        .users()
        .upsert_oidc(&identity.external_id, &identity.username, role)
        .await?;
    if user.disabled {
        return Err(AuthError::Forbidden);
    }
    tracing::info!(username = %user.username, role = %user.role, "SSO login");
    let res = start_session(state, caller, headers, user).await?;
    let cookie = res
        .headers()
        .get(SET_COOKIE)
        .cloned()
        .ok_or_else(|| AuthError::Internal("session cookie missing".into()))?;
    Ok((cookie, return_to))
}

async fn list_users(
    State(state): State<AuthState>,
    _: RequireAdmin,
) -> Result<Json<Vec<User>>, AuthError> {
    Ok(Json(state.users().list().await?))
}

/// Body of `POST /users`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateUserRequest {
    /// Username.
    pub username: String,
    /// Initial password.
    pub password: String,
    /// Role (default viewer).
    #[serde(default = "viewer")]
    pub role: Role,
}

fn viewer() -> Role {
    Role::Viewer
}

async fn create_user(
    State(state): State<AuthState>,
    RequireAdmin(admin): RequireAdmin,
    Json(body): Json<CreateUserRequest>,
) -> Result<(StatusCode, Json<User>), AuthError> {
    validate_new_password(&body.password, state.config().local.min_password_length)?;
    let hash = hash_password_async(body.password).await?;
    let user = state
        .users()
        .create_local(body.username.trim(), &hash, body.role)
        .await?;
    tracing::info!(by = %admin.author(), username = %user.username, role = %user.role, "user created");
    Ok((StatusCode::CREATED, Json(user)))
}

/// Body of `PATCH /users/{id}`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateUserRequest {
    /// New role.
    pub role: Option<Role>,
    /// New password (local users only).
    pub password: Option<String>,
    /// Disable or enable.
    pub disabled: Option<bool>,
}

async fn update_user(
    State(state): State<AuthState>,
    RequireAdmin(admin): RequireAdmin,
    Path(id): Path<i64>,
    Json(body): Json<UpdateUserRequest>,
) -> Result<Json<User>, AuthError> {
    let password_hash = match body.password {
        Some(p) => {
            validate_new_password(&p, state.config().local.min_password_length)?;
            Some(hash_password_async(p).await?)
        }
        None => None,
    };
    let user = state
        .users()
        .update(
            id,
            UserUpdate {
                role: body.role,
                password_hash,
                disabled: body.disabled,
            },
        )
        .await?;
    tracing::info!(by = %admin.author(), username = %user.username, "user updated");
    Ok(Json(user))
}

async fn delete_user(
    State(state): State<AuthState>,
    RequireAdmin(admin): RequireAdmin,
    Path(id): Path<i64>,
) -> Result<StatusCode, AuthError> {
    state.users().delete(id).await?;
    tracing::info!(by = %admin.author(), id, "user deleted");
    Ok(StatusCode::NO_CONTENT)
}
