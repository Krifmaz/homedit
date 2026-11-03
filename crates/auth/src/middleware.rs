//! The auth middleware.
//!
//! Install [`auth_middleware`] once, around every route (API, pages and
//! assets), with `axum::middleware::from_fn_with_state(auth_state, auth_middleware)`.
//! It
//!
//! 1. resolves the caller (proxy headers from a trusted proxy, then the
//!    session cookie, then the anonymous role) into a
//!    [`CurrentUser`] request extension;
//! 2. enforces CSRF checks on state-changing methods (see [`crate::csrf`]);
//! 3. sets the CSRF cookie when the browser does not have one yet, and
//!    clears stale session cookies.
//!
//! [`resolve_user`] and [`csrf`] are the two halves, for apps that want to
//! place them separately; `resolve_user` must run first (be the outer
//! layer).

use axum::extract::{Request, State};
use axum::http::header::SET_COOKIE;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::csrf::{CSRF_COOKIE, check_origin, check_token, is_safe_method};
use crate::error::AuthError;
use crate::extract::CurrentUser;
use crate::state::AuthState;
use crate::util::set_cookie;

/// Resolve the caller and enforce CSRF in one layer.
pub async fn auth_middleware(State(state): State<AuthState>, req: Request, next: Next) -> Response {
    resolve_inner(state, req, next, true).await
}

/// Only resolves the caller (no CSRF checks). Pair with [`csrf`].
pub async fn resolve_user(State(state): State<AuthState>, req: Request, next: Next) -> Response {
    resolve_inner(state, req, next, false).await
}

/// Only the CSRF checks. Needs [`resolve_user`] to have run first.
pub async fn csrf(State(state): State<AuthState>, req: Request, next: Next) -> Response {
    let Some(user) = req.extensions().get::<CurrentUser>() else {
        return AuthError::Internal("csrf layer runs before resolve_user".into()).into_response();
    };
    if let Err(e) = check_csrf(&state, &req, user) {
        return e.into_response();
    }
    next.run(req).await
}

fn check_csrf(state: &AuthState, req: &Request, user: &CurrentUser) -> Result<(), AuthError> {
    if is_safe_method(req.method()) {
        return Ok(());
    }
    check_origin(req.headers(), &user.client, &state.config().allowed_origins)?;
    check_token(req.headers(), &user.csrf_token)
}

async fn resolve_inner(state: AuthState, req: Request, next: Next, with_csrf: bool) -> Response {
    let (mut parts, body) = req.into_parts();
    let resolved = match state.resolve(&parts).await {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
    let secure = state.cookie_secure(&resolved.user.client);
    parts.extensions.insert(resolved.user.clone());
    let req = Request::from_parts(parts, body);

    let mut res = match with_csrf.then(|| check_csrf(&state, &req, &resolved.user)) {
        Some(Err(e)) => e.into_response(),
        _ => next.run(req).await,
    };

    if resolved.set_csrf_cookie {
        res.headers_mut().append(
            SET_COOKIE,
            set_cookie(
                CSRF_COOKIE,
                &resolved.user.csrf_token,
                None,
                secure,
                cookie::SameSite::Strict,
            ),
        );
    }
    if resolved.clear_session_cookie
        && !res.headers().get_all(SET_COOKIE).iter().any(|v| {
            v.to_str()
                .is_ok_and(|s| s.starts_with(&format!("{}=", state.config().session.cookie_name)))
        })
    {
        res.headers_mut().append(
            SET_COOKIE,
            set_cookie(
                &state.config().session.cookie_name,
                "",
                Some(0),
                secure,
                cookie::SameSite::Lax,
            ),
        );
    }
    res
}
