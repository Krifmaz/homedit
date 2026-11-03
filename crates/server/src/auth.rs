//! Request identity.
//!
//! This is a placeholder until the `homedit-auth` crate is wired in. With
//! no login configured every visitor is treated as an admin, which matches a
//! dashboard on a trusted LAN with authentication turned off. The types
//! here keep the handlers' signatures stable: swapping in the real
//! extractors does not change any handler body.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use serde::Serialize;

use crate::ApiError;

/// Role of the current visitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Can view the dashboard.
    Viewer,
    /// Can view, edit and scan.
    Admin,
}

/// The visitor making a request.
#[derive(Debug, Clone)]
pub struct CurrentUser {
    /// Display name, `None` for anonymous visitors.
    pub name: Option<String>,
    /// Effective role.
    pub role: Role,
}

impl CurrentUser {
    /// Whether this visitor may see groups hidden from viewers.
    pub fn can_view_hidden(&self) -> bool {
        self.role == Role::Admin
    }
}

impl<S: Send + Sync> FromRequestParts<S> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(_parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self {
            name: None,
            role: Role::Admin,
        })
    }
}

/// Extractor that only succeeds for admins.
#[derive(Debug, Clone)]
pub struct RequireAdmin(pub CurrentUser);

impl<S: Send + Sync> FromRequestParts<S> for RequireAdmin {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let user = CurrentUser::from_request_parts(parts, state).await?;
        if user.role == Role::Admin {
            Ok(Self(user))
        } else {
            Err(ApiError::new(
                axum::http::StatusCode::FORBIDDEN,
                "admin role required",
            ))
        }
    }
}
