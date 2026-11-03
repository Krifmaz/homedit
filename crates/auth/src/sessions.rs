//! Server-side sessions.
//!
//! The browser holds a random 256-bit token in an `HttpOnly`,
//! `SameSite=Lax` cookie. The database stores only the token's SHA-256, plus
//! a per-session CSRF token that the frontend sends back in `X-CSRF-Token`.

use sqlx::SqlitePool;

use crate::error::AuthError;
use crate::users::User;
use crate::util::{now_unix, random_token, sha256};

/// A newly created session. `token` goes into the cookie and is never
/// stored.
#[derive(Debug, Clone)]
pub struct NewSession {
    /// Value for the session cookie.
    pub token: String,
    /// CSRF token bound to this session.
    pub csrf_token: String,
    /// Expiry, Unix seconds.
    pub expires_at: i64,
}

/// A session looked up from a cookie.
#[derive(Debug, Clone)]
pub struct ActiveSession {
    /// SHA-256 of the cookie token (the primary key).
    pub token_hash: Vec<u8>,
    /// CSRF token bound to this session.
    pub csrf_token: String,
    /// The signed-in user.
    pub user: User,
}

/// Data access for `auth_sessions`.
#[derive(Debug, Clone)]
pub struct SessionStore {
    pool: SqlitePool,
    ttl_secs: i64,
}

impl SessionStore {
    /// Sessions last `ttl_secs` from creation.
    pub fn new(pool: SqlitePool, ttl_secs: u64) -> Self {
        Self {
            pool,
            ttl_secs: ttl_secs.min(i64::MAX as u64) as i64,
        }
    }

    /// Starts a session for `user_id`. Also sweeps expired sessions.
    pub async fn create(
        &self,
        user_id: i64,
        ip: Option<&str>,
        user_agent: Option<&str>,
    ) -> Result<NewSession, AuthError> {
        let now = now_unix();
        let token = random_token(32);
        let csrf_token = random_token(32);
        let expires_at = now.saturating_add(self.ttl_secs);
        sqlx::query("DELETE FROM auth_sessions WHERE expires_at <= ?")
            .bind(now)
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "INSERT INTO auth_sessions (token_hash, user_id, csrf_token, created_at, expires_at, last_seen_at, ip, user_agent)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(sha256(&token))
        .bind(user_id)
        .bind(&csrf_token)
        .bind(now)
        .bind(expires_at)
        .bind(now)
        .bind(ip)
        .bind(user_agent.map(|s| s.chars().take(256).collect::<String>()))
        .execute(&self.pool)
        .await?;
        Ok(NewSession {
            token,
            csrf_token,
            expires_at,
        })
    }

    /// Resolves a cookie token. Returns `None` for unknown or expired
    /// sessions and for disabled users.
    pub async fn lookup(&self, token: &str) -> Result<Option<ActiveSession>, AuthError> {
        if token.is_empty() || token.len() > 128 {
            return Ok(None);
        }
        let now = now_unix();
        let hash = sha256(token);
        let row: Option<(String, i64, i64)> = sqlx::query_as(
            "SELECT s.csrf_token, s.user_id, s.last_seen_at FROM auth_sessions s
             JOIN auth_users u ON u.id = s.user_id
             WHERE s.token_hash = ? AND s.expires_at > ? AND u.disabled = 0",
        )
        .bind(&hash)
        .bind(now)
        .fetch_optional(&self.pool)
        .await?;
        let Some((csrf_token, user_id, last_seen)) = row else {
            return Ok(None);
        };
        let user: Option<User> = sqlx::query_as(
            "SELECT id, username, role, source, disabled, created_at, last_login_at FROM auth_users WHERE id = ?",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some(user) = user else { return Ok(None) };
        // Cheap activity tracking: at most one write per minute per session.
        if now - last_seen >= 60 {
            sqlx::query("UPDATE auth_sessions SET last_seen_at = ? WHERE token_hash = ?")
                .bind(now)
                .bind(&hash)
                .execute(&self.pool)
                .await?;
        }
        Ok(Some(ActiveSession {
            token_hash: hash,
            csrf_token,
            user,
        }))
    }

    /// Ends one session.
    pub async fn delete(&self, token_hash: &[u8]) -> Result<(), AuthError> {
        sqlx::query("DELETE FROM auth_sessions WHERE token_hash = ?")
            .bind(token_hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Ends every session of a user.
    pub async fn delete_for_user(&self, user_id: i64) -> Result<(), AuthError> {
        sqlx::query("DELETE FROM auth_sessions WHERE user_id = ?")
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
