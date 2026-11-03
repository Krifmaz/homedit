//! The `auth_*` tables.
//!
//! The auth crate manages its own tables with a private version counter
//! (`auth_schema_version`) so it never interferes with the server's sqlx
//! migrations. Call [`init`] once at startup; it is idempotent and upgrades
//! older layouts in place.

use sqlx::SqlitePool;

/// Ordered schema steps. Step `n` (1-based) upgrades version `n - 1` to `n`.
/// Never edit a released step; append a new one.
const STEPS: &[&str] = &[
    // 1: users, sessions, secrets
    r#"
    CREATE TABLE auth_users (
        id            INTEGER PRIMARY KEY AUTOINCREMENT,
        username      TEXT    NOT NULL UNIQUE COLLATE NOCASE,
        password_hash TEXT,
        role          TEXT    NOT NULL CHECK (role IN ('viewer', 'admin')),
        source        TEXT    NOT NULL DEFAULT 'local' CHECK (source IN ('local', 'oidc')),
        external_id   TEXT,
        disabled      INTEGER NOT NULL DEFAULT 0,
        created_at    INTEGER NOT NULL,
        updated_at    INTEGER NOT NULL,
        last_login_at INTEGER
    );
    CREATE UNIQUE INDEX auth_users_external ON auth_users (source, external_id) WHERE external_id IS NOT NULL;

    CREATE TABLE auth_sessions (
        token_hash   BLOB    PRIMARY KEY,
        user_id      INTEGER NOT NULL REFERENCES auth_users (id) ON DELETE CASCADE,
        csrf_token   TEXT    NOT NULL,
        created_at   INTEGER NOT NULL,
        expires_at   INTEGER NOT NULL,
        last_seen_at INTEGER NOT NULL,
        ip           TEXT,
        user_agent   TEXT
    );
    CREATE INDEX auth_sessions_user ON auth_sessions (user_id);
    CREATE INDEX auth_sessions_expires ON auth_sessions (expires_at);

    CREATE TABLE auth_secrets (
        name       TEXT    PRIMARY KEY,
        ciphertext TEXT    NOT NULL,
        updated_at INTEGER NOT NULL
    );
    "#,
];

/// Creates or upgrades the `auth_*` tables.
pub async fn init(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS auth_schema_version (version INTEGER NOT NULL)")
        .execute(&mut *tx)
        .await?;
    let current: Option<i64> =
        sqlx::query_scalar("SELECT version FROM auth_schema_version LIMIT 1")
            .fetch_optional(&mut *tx)
            .await?;
    let current = current.unwrap_or(0) as usize;
    if current > STEPS.len() {
        tracing::warn!(
            current,
            known = STEPS.len(),
            "auth schema is newer than this binary"
        );
    }
    for (i, step) in STEPS.iter().enumerate().skip(current) {
        sqlx::raw_sql(*step).execute(&mut *tx).await?;
        tracing::info!(version = i + 1, "applied auth schema step");
    }
    if current < STEPS.len() {
        sqlx::query("DELETE FROM auth_schema_version")
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO auth_schema_version (version) VALUES (?)")
            .bind(STEPS.len() as i64)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await
}

/// The schema version this binary creates.
pub fn latest_version() -> usize {
    STEPS.len()
}
