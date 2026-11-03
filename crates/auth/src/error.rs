//! The error type returned by handlers and extractors in this crate.

use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::json;

/// Everything that can go wrong in authentication and authorization.
///
/// Converts into a JSON response `{"error": "<code>", "message": "..."}` with
/// a matching HTTP status. Internal errors are logged and returned without
/// detail.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// No valid credentials were presented, and anonymous access is off.
    #[error("authentication required")]
    Unauthenticated,
    /// Wrong username or password (deliberately not saying which).
    #[error("invalid username or password")]
    InvalidCredentials,
    /// The caller is signed in but lacks the role for this action.
    #[error("forbidden")]
    Forbidden,
    /// The request failed the CSRF checks.
    #[error("CSRF check failed: {0}")]
    Csrf(&'static str),
    /// Too many failed logins; retry after the given number of seconds.
    #[error("too many attempts, retry in {0} seconds")]
    RateLimited(u64),
    /// The request body or parameters are invalid.
    #[error("{0}")]
    BadRequest(String),
    /// The request conflicts with existing state (e.g. duplicate username).
    #[error("{0}")]
    Conflict(String),
    /// The target does not exist.
    #[error("not found")]
    NotFound,
    /// The endpoint is disabled by configuration.
    #[error("{0} is not enabled")]
    Disabled(&'static str),
    /// OIDC provider or protocol failure.
    #[error("OIDC error: {0}")]
    Oidc(String),
    /// Database error.
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    /// Any other server-side failure.
    #[error("internal error: {0}")]
    Internal(String),
}

impl AuthError {
    /// HTTP status code for this error.
    pub fn status(&self) -> StatusCode {
        match self {
            AuthError::Unauthenticated | AuthError::InvalidCredentials => StatusCode::UNAUTHORIZED,
            AuthError::Forbidden | AuthError::Csrf(_) => StatusCode::FORBIDDEN,
            AuthError::RateLimited(_) => StatusCode::TOO_MANY_REQUESTS,
            AuthError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AuthError::Conflict(_) => StatusCode::CONFLICT,
            AuthError::NotFound | AuthError::Disabled(_) => StatusCode::NOT_FOUND,
            AuthError::Oidc(_) => StatusCode::BAD_GATEWAY,
            AuthError::Db(_) | AuthError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Short machine-readable code.
    pub fn code(&self) -> &'static str {
        match self {
            AuthError::Unauthenticated => "unauthenticated",
            AuthError::InvalidCredentials => "invalid_credentials",
            AuthError::Forbidden => "forbidden",
            AuthError::Csrf(_) => "csrf",
            AuthError::RateLimited(_) => "rate_limited",
            AuthError::BadRequest(_) => "bad_request",
            AuthError::Conflict(_) => "conflict",
            AuthError::NotFound => "not_found",
            AuthError::Disabled(_) => "disabled",
            AuthError::Oidc(_) => "oidc",
            AuthError::Db(_) | AuthError::Internal(_) => "internal",
        }
    }
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let status = self.status();
        let message = match &self {
            AuthError::Db(e) => {
                tracing::error!(error = %e, "auth database error");
                "internal error".to_owned()
            }
            AuthError::Internal(e) => {
                tracing::error!(error = %e, "auth internal error");
                "internal error".to_owned()
            }
            AuthError::Oidc(e) => {
                tracing::warn!(error = %e, "OIDC login failed");
                "single sign-on failed".to_owned()
            }
            other => other.to_string(),
        };
        let mut res = (
            status,
            Json(json!({ "error": self.code(), "message": message })),
        )
            .into_response();
        if let AuthError::RateLimited(secs) = self
            && let Ok(v) = HeaderValue::from_str(&secs.to_string())
        {
            res.headers_mut().insert(header::RETRY_AFTER, v);
        }
        res
    }
}
