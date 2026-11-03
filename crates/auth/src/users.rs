//! User accounts stored in `auth_users`.
//!
//! Local users have an argon2id password hash. OIDC users are created on
//! their first sign-in and keyed by `issuer|subject`. Reverse-proxy users are
//! never stored.

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::error::AuthError;
use crate::role::{Role, UserSource};
use crate::util::now_unix;

/// A stored user, without the password hash.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct User {
    /// Row id.
    pub id: i64,
    /// Login name (unique, case-insensitive).
    pub username: String,
    /// Role.
    pub role: Role,
    /// Where the account comes from.
    pub source: UserSource,
    /// Disabled accounts cannot sign in and lose their sessions.
    pub disabled: bool,
    /// Creation time, Unix seconds.
    pub created_at: i64,
    /// Last successful sign-in, Unix seconds.
    pub last_login_at: Option<i64>,
}

macro_rules! user_select {
    ($rest:literal) => {
        concat!("SELECT id, username, role, source, disabled, created_at, last_login_at FROM auth_users ", $rest)
    };
}

/// Data access for `auth_users`.
#[derive(Debug, Clone)]
pub struct UserStore {
    pool: SqlitePool,
}

/// Fields to change on an existing user; `None` leaves a field alone.
#[derive(Debug, Default, Clone)]
pub struct UserUpdate {
    /// New role.
    pub role: Option<Role>,
    /// New password hash (already hashed).
    pub password_hash: Option<String>,
    /// Enable or disable.
    pub disabled: Option<bool>,
}

impl UserStore {
    /// Wraps a pool. The `auth_*` tables must exist (see [`crate::schema::init`]).
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Number of users.
    pub async fn count(&self) -> Result<i64, AuthError> {
        Ok(sqlx::query_scalar("SELECT COUNT(*) FROM auth_users")
            .fetch_one(&self.pool)
            .await?)
    }

    /// Number of enabled admins.
    pub async fn count_admins(&self) -> Result<i64, AuthError> {
        Ok(sqlx::query_scalar(
            "SELECT COUNT(*) FROM auth_users WHERE role = 'admin' AND disabled = 0",
        )
        .fetch_one(&self.pool)
        .await?)
    }

    /// All users ordered by name.
    pub async fn list(&self) -> Result<Vec<User>, AuthError> {
        Ok(
            sqlx::query_as(user_select!("ORDER BY username COLLATE NOCASE"))
                .fetch_all(&self.pool)
                .await?,
        )
    }

    /// One user by id.
    pub async fn get(&self, id: i64) -> Result<Option<User>, AuthError> {
        Ok(sqlx::query_as(user_select!("WHERE id = ?"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// A user by name (case-insensitive) together with the password hash.
    pub async fn find_for_login(
        &self,
        username: &str,
    ) -> Result<Option<(User, Option<String>)>, AuthError> {
        let row: Option<(i64, Option<String>)> = sqlx::query_as(
            "SELECT id, password_hash FROM auth_users WHERE username = ? AND source = 'local'",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;
        match row {
            Some((id, hash)) => Ok(self.get(id).await?.map(|u| (u, hash))),
            None => Ok(None),
        }
    }

    /// The password hash of a local user.
    pub async fn password_hash(&self, id: i64) -> Result<Option<String>, AuthError> {
        Ok(
            sqlx::query_scalar("SELECT password_hash FROM auth_users WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?
                .flatten(),
        )
    }

    /// Creates a local user. `password_hash` must come from
    /// [`crate::password::hash_password`].
    pub async fn create_local(
        &self,
        username: &str,
        password_hash: &str,
        role: Role,
    ) -> Result<User, AuthError> {
        validate_username(username)?;
        let now = now_unix();
        let res = sqlx::query(
            "INSERT INTO auth_users (username, password_hash, role, source, created_at, updated_at)
             VALUES (?, ?, ?, 'local', ?, ?)",
        )
        .bind(username)
        .bind(password_hash)
        .bind(role)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await;
        let id = match res {
            Ok(r) => r.last_insert_rowid(),
            Err(e) if is_unique_violation(&e) => {
                return Err(AuthError::Conflict(format!(
                    "username `{username}` is taken"
                )));
            }
            Err(e) => return Err(e.into()),
        };
        self.get(id).await?.ok_or(AuthError::NotFound)
    }

    /// Finds or creates the user for an OIDC identity, updating its role to
    /// what the provider's groups say.
    pub async fn upsert_oidc(
        &self,
        external_id: &str,
        username: &str,
        role: Role,
    ) -> Result<User, AuthError> {
        let now = now_unix();
        let existing: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM auth_users WHERE source = 'oidc' AND external_id = ?",
        )
        .bind(external_id)
        .fetch_optional(&self.pool)
        .await?;
        let id = match existing {
            Some(id) => {
                sqlx::query("UPDATE auth_users SET role = ?, updated_at = ? WHERE id = ?")
                    .bind(role)
                    .bind(now)
                    .bind(id)
                    .execute(&self.pool)
                    .await?;
                id
            }
            None => {
                validate_username(username)?;
                let res = sqlx::query(
                    "INSERT INTO auth_users (username, role, source, external_id, created_at, updated_at)
                     VALUES (?, ?, 'oidc', ?, ?, ?)",
                )
                .bind(username)
                .bind(role)
                .bind(external_id)
                .bind(now)
                .bind(now)
                .execute(&self.pool)
                .await;
                match res {
                    Ok(r) => r.last_insert_rowid(),
                    Err(e) if is_unique_violation(&e) => {
                        return Err(AuthError::Conflict(format!(
                            "username `{username}` already belongs to another account"
                        )));
                    }
                    Err(e) => return Err(e.into()),
                }
            }
        };
        self.get(id).await?.ok_or(AuthError::NotFound)
    }

    /// Applies `update`. Refuses to demote or disable the last enabled admin.
    pub async fn update(&self, id: i64, update: UserUpdate) -> Result<User, AuthError> {
        let user = self.get(id).await?.ok_or(AuthError::NotFound)?;
        let loses_admin = user.role == Role::Admin
            && !user.disabled
            && (update.role.is_some_and(|r| r != Role::Admin) || update.disabled == Some(true));
        if loses_admin && self.count_admins().await? <= 1 {
            return Err(AuthError::Conflict("cannot remove the last admin".into()));
        }
        if update.password_hash.is_some() && user.source != UserSource::Local {
            return Err(AuthError::BadRequest(
                "only local users have passwords".into(),
            ));
        }
        let now = now_unix();
        let mut tx = self.pool.begin().await?;
        if let Some(role) = update.role {
            sqlx::query("UPDATE auth_users SET role = ?, updated_at = ? WHERE id = ?")
                .bind(role)
                .bind(now)
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        if let Some(hash) = &update.password_hash {
            sqlx::query("UPDATE auth_users SET password_hash = ?, updated_at = ? WHERE id = ?")
                .bind(hash)
                .bind(now)
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        if let Some(disabled) = update.disabled {
            sqlx::query("UPDATE auth_users SET disabled = ?, updated_at = ? WHERE id = ?")
                .bind(disabled)
                .bind(now)
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        // Any credential or privilege change ends existing sessions.
        if update.password_hash.is_some()
            || update.disabled == Some(true)
            || update.role.is_some_and(|r| r != user.role)
        {
            sqlx::query("DELETE FROM auth_sessions WHERE user_id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        self.get(id).await?.ok_or(AuthError::NotFound)
    }

    /// Deletes a user and their sessions. Refuses to delete the last admin.
    pub async fn delete(&self, id: i64) -> Result<(), AuthError> {
        let user = self.get(id).await?.ok_or(AuthError::NotFound)?;
        if user.role == Role::Admin && !user.disabled && self.count_admins().await? <= 1 {
            return Err(AuthError::Conflict("cannot delete the last admin".into()));
        }
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM auth_sessions WHERE user_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM auth_users WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Records a successful sign-in.
    pub async fn touch_login(&self, id: i64) -> Result<(), AuthError> {
        sqlx::query("UPDATE auth_users SET last_login_at = ? WHERE id = ?")
            .bind(now_unix())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

/// Usernames are 1 to 64 printable characters without surrounding spaces.
pub fn validate_username(name: &str) -> Result<(), AuthError> {
    let ok = !name.is_empty()
        && name.chars().count() <= 64
        && name.trim() == name
        && !name.chars().any(char::is_control);
    if ok {
        Ok(())
    } else {
        Err(AuthError::BadRequest(
            "username must be 1-64 printable characters".into(),
        ))
    }
}

fn is_unique_violation(e: &sqlx::Error) -> bool {
    matches!(e, sqlx::Error::Database(d) if d.is_unique_violation())
}
