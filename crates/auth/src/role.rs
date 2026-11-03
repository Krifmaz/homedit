//! Roles, user sources and per-group visibility.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// What a caller may do.
///
/// Roles are ordered: `Viewer < Admin`, so `role >= Role::Viewer` means
/// "may at least view".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// May see the dashboard (minus groups hidden from viewers).
    Viewer,
    /// May additionally edit the dashboard, manage users and run scans.
    Admin,
}

impl Role {
    /// The lowercase name stored in the database and used in config.
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Viewer => "viewer",
            Role::Admin => "admin",
        }
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Role {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "viewer" => Ok(Role::Viewer),
            "admin" => Ok(Role::Admin),
            other => Err(format!("unknown role `{other}` (expected viewer or admin)")),
        }
    }
}

/// Where a user account comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserSource {
    /// Username and password stored in homedit (argon2id hash).
    Local,
    /// Signed in through an OpenID Connect provider.
    Oidc,
    /// Identified by headers from a trusted reverse proxy. Not stored.
    Proxy,
}

impl UserSource {
    /// The lowercase name stored in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            UserSource::Local => "local",
            UserSource::Oidc => "oidc",
            UserSource::Proxy => "proxy",
        }
    }
}

impl FromStr for UserSource {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "local" => Ok(UserSource::Local),
            "oidc" => Ok(UserSource::Oidc),
            "proxy" => Ok(UserSource::Proxy),
            other => Err(format!("unknown user source `{other}`")),
        }
    }
}

/// Who may see a dashboard group.
///
/// Stored as TEXT (`everyone` / `admin`) in the dashboard's `groups` table,
/// which belongs to the main server crate. Use
/// [`CurrentUser::can_view`](crate::CurrentUser::can_view) to filter groups.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupVisibility {
    /// Everyone who may see the dashboard at all.
    #[default]
    Everyone,
    /// Only admins. Hidden from viewers and anonymous visitors.
    Admin,
}

impl GroupVisibility {
    /// The lowercase name stored in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            GroupVisibility::Everyone => "everyone",
            GroupVisibility::Admin => "admin",
        }
    }

    /// The minimum role that may see a group with this visibility.
    pub fn min_role(self) -> Role {
        match self {
            GroupVisibility::Everyone => Role::Viewer,
            GroupVisibility::Admin => Role::Admin,
        }
    }
}

impl FromStr for GroupVisibility {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "everyone" | "" => Ok(GroupVisibility::Everyone),
            "admin" | "admins" => Ok(GroupVisibility::Admin),
            other => Err(format!("unknown group visibility `{other}`")),
        }
    }
}

macro_rules! sqlx_text_enum {
    ($t:ty) => {
        impl sqlx::Type<sqlx::Sqlite> for $t {
            fn type_info() -> sqlx::sqlite::SqliteTypeInfo {
                <str as sqlx::Type<sqlx::Sqlite>>::type_info()
            }
            fn compatible(ty: &sqlx::sqlite::SqliteTypeInfo) -> bool {
                <str as sqlx::Type<sqlx::Sqlite>>::compatible(ty)
            }
        }

        impl<'q> sqlx::Encode<'q, sqlx::Sqlite> for $t {
            fn encode_by_ref(
                &self,
                buf: &mut <sqlx::Sqlite as sqlx::Database>::ArgumentBuffer,
            ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
                <&str as sqlx::Encode<'q, sqlx::Sqlite>>::encode_by_ref(&self.as_str(), buf)
            }
        }

        impl<'r> sqlx::Decode<'r, sqlx::Sqlite> for $t {
            fn decode(
                value: sqlx::sqlite::SqliteValueRef<'r>,
            ) -> Result<Self, sqlx::error::BoxDynError> {
                let s = <&str as sqlx::Decode<'r, sqlx::Sqlite>>::decode(value)?;
                Ok(s.parse()?)
            }
        }
    };
}

sqlx_text_enum!(Role);
sqlx_text_enum!(UserSource);
sqlx_text_enum!(GroupVisibility);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_are_ordered() {
        assert!(Role::Admin > Role::Viewer);
        assert_eq!("ADMIN".parse::<Role>().unwrap(), Role::Admin);
        assert!("root".parse::<Role>().is_err());
    }

    #[test]
    fn visibility_parses() {
        assert_eq!(
            "".parse::<GroupVisibility>().unwrap(),
            GroupVisibility::Everyone
        );
        assert_eq!(
            "admin".parse::<GroupVisibility>().unwrap().min_role(),
            Role::Admin
        );
    }
}
