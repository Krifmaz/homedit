//! Error type shared by every widget.

use thiserror::Error;

/// Everything that can go wrong while configuring or fetching a widget.
///
/// Errors are designed to be shown to an admin in the editor (for example
/// after pressing "test connection"), so messages never contain secrets.
#[derive(Debug, Error)]
pub enum WidgetError {
    /// No widget with this kind is registered.
    #[error("unknown widget kind `{0}`")]
    UnknownKind(String),

    /// The configuration does not match the widget's schema.
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),

    /// The request targets a host that is not configured for this widget
    /// (server-side request forgery guard).
    #[error("request to host `{0}` is not allowed for this widget")]
    HostNotAllowed(String),

    /// The request could not be completed (DNS, connect, TLS, timeout).
    #[error("request failed: {0}")]
    Http(String),

    /// The service answered with a non-success HTTP status.
    #[error("service returned HTTP {status}")]
    Status {
        /// The HTTP status code.
        status: u16,
    },

    /// The service rejected the credentials.
    #[error("authentication failed (check the API key, token or password)")]
    Auth,

    /// The response body could not be understood.
    #[error("unexpected response: {0}")]
    Parse(String),
}

impl WidgetError {
    /// Builds a [`WidgetError::Parse`] from any displayable error.
    pub fn parse(err: impl std::fmt::Display) -> Self {
        Self::Parse(err.to_string())
    }
}

impl From<reqwest::Error> for WidgetError {
    fn from(err: reqwest::Error) -> Self {
        if let Some(status) = err.status() {
            return status_error(status.as_u16());
        }
        // `without_url` keeps query strings (which may hold API keys) out of
        // the message.
        let err = err.without_url();
        if err.is_timeout() {
            Self::Http("timed out".into())
        } else if err.is_decode() {
            Self::Parse(err.to_string())
        } else {
            Self::Http(err.to_string())
        }
    }
}

impl From<serde_json::Error> for WidgetError {
    fn from(err: serde_json::Error) -> Self {
        Self::Parse(err.to_string())
    }
}

/// Maps an HTTP status to the matching error, treating 401 and 403 as
/// [`WidgetError::Auth`].
pub(crate) fn status_error(status: u16) -> WidgetError {
    match status {
        401 | 403 => WidgetError::Auth,
        status => WidgetError::Status { status },
    }
}
