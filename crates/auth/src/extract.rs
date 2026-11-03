//! Axum extractors for the current caller.
//!
//! [`middleware::auth_middleware`](crate::middleware::auth_middleware) puts a
//! [`CurrentUser`] into the request extensions; these extractors read it. If
//! the middleware is missing they fail closed with a 500.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use serde::{Deserialize, Serialize};

use crate::client_ip::ClientInfo;
use crate::error::AuthError;
use crate::role::{GroupVisibility, Role, UserSource};

/// A signed-in user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthUser {
    /// Row id in `auth_users`; `None` for reverse-proxy users.
    pub id: Option<i64>,
    /// Login name.
    pub username: String,
    /// Role.
    pub role: Role,
    /// How the user signed in.
    pub source: UserSource,
}

/// Session details needed by logout and CSRF checks.
#[derive(Debug, Clone)]
pub(crate) struct SessionRef {
    pub(crate) token_hash: Vec<u8>,
}

/// The caller of the current request. Always extractable once the auth
/// middleware ran.
#[derive(Debug, Clone)]
pub struct CurrentUser {
    /// The signed-in user, or `None` for anonymous visitors (and for
    /// everyone when auth is disabled).
    pub user: Option<AuthUser>,
    /// Effective role: the user's role, the anonymous role, or `Admin` when
    /// auth is disabled. `None` means "may see nothing".
    pub role: Option<Role>,
    /// Whether login is enabled at all.
    pub auth_enabled: bool,
    /// The CSRF token to hand to the frontend.
    pub csrf_token: String,
    /// Connection facts (client IP, HTTPS, host).
    pub client: ClientInfo,
    pub(crate) session: Option<SessionRef>,
}

impl CurrentUser {
    /// Whether the caller is an admin.
    pub fn is_admin(&self) -> bool {
        self.role == Some(Role::Admin)
    }

    /// Whether the caller may see the dashboard at all.
    pub fn can_view_dashboard(&self) -> bool {
        self.role.is_some()
    }

    /// Whether the caller may see groups marked `hidden_from_viewers`.
    /// Only admins can.
    pub fn can_view_hidden(&self) -> bool {
        self.is_admin()
    }

    /// Whether the caller may see a group with this visibility.
    pub fn can_view(&self, visibility: GroupVisibility) -> bool {
        self.role.is_some_and(|r| r >= visibility.min_role())
    }

    /// Convenience for the `groups.hidden_from_viewers` flag.
    pub fn can_view_group(&self, hidden_from_viewers: bool) -> bool {
        if hidden_from_viewers {
            self.can_view_hidden()
        } else {
            self.can_view_dashboard()
        }
    }

    /// Name to record as the author of a change (e.g. in config history).
    pub fn author(&self) -> String {
        match &self.user {
            Some(u) => u.username.clone(),
            None if !self.auth_enabled => "admin".into(),
            None => "anonymous".into(),
        }
    }

    /// Fails with 401/403 unless the caller has at least `role`.
    pub fn require(&self, role: Role) -> Result<(), AuthError> {
        match self.role {
            Some(r) if r >= role => Ok(()),
            _ if self.user.is_none() && self.auth_enabled => Err(AuthError::Unauthenticated),
            _ => Err(AuthError::Forbidden),
        }
    }
}

impl<S: Send + Sync> FromRequestParts<S> for CurrentUser {
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<CurrentUser>()
            .cloned()
            .ok_or_else(|| AuthError::Internal("auth middleware is not installed".into()))
    }
}

/// Extracts the caller and rejects with 401/403 unless they may view the
/// dashboard.
#[derive(Debug, Clone)]
pub struct RequireViewer(pub CurrentUser);

impl<S: Send + Sync> FromRequestParts<S> for RequireViewer {
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let user = CurrentUser::from_request_parts(parts, state).await?;
        user.require(Role::Viewer)?;
        Ok(Self(user))
    }
}

/// Extracts the caller and rejects with 401/403 unless they are an admin.
/// Use on every editing, user-management and scanning route.
#[derive(Debug, Clone)]
pub struct RequireAdmin(pub CurrentUser);

impl<S: Send + Sync> FromRequestParts<S> for RequireAdmin {
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let user = CurrentUser::from_request_parts(parts, state).await?;
        user.require(Role::Admin)?;
        Ok(Self(user))
    }
}
