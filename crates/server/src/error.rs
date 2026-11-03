//! API error type: a status code and a message, sent as `{"error": "..."}`.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use homedit_db::DbError;

/// An error returned by an API handler.
#[derive(Debug)]
pub struct ApiError {
    /// HTTP status to send.
    pub status: StatusCode,
    /// Message for the client. Never contains secrets or internal details
    /// for 5xx errors; those are logged instead.
    pub message: String,
}

impl ApiError {
    /// Builds an error with a status and message.
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    /// 400 Bad Request.
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    /// 404 Not Found.
    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "not found")
    }

    /// 500 Internal Server Error; logs the cause.
    pub fn internal(cause: impl std::fmt::Display) -> Self {
        tracing::error!(%cause, "internal error");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
    }
}

impl From<DbError> for ApiError {
    fn from(e: DbError) -> Self {
        match e {
            DbError::Conflict { current, .. } => Self::new(
                StatusCode::CONFLICT,
                format!("the dashboard was changed elsewhere; current version is {current}"),
            ),
            DbError::NotFound => Self::not_found(),
            other => Self::internal(other),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({ "error": self.message })),
        )
            .into_response()
    }
}
