//! Password hashing with argon2id.
//!
//! Hashes use the `argon2` crate's defaults (argon2id, 19 MiB, 2 passes,
//! 1 lane), which match the OWASP recommendation, and are stored in PHC
//! string form so parameters can be raised later without breaking old
//! hashes. Hashing is CPU-heavy, so the async wrappers run it on the blocking
//! thread pool.

use argon2::Argon2;
use argon2::password_hash::{
    PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng,
};

use crate::error::AuthError;

/// Hashes `password` into a PHC string (`$argon2id$v=19$...`).
pub fn hash_password(password: &str) -> Result<String, AuthError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| AuthError::Internal(format!("argon2 hash: {e}")))
}

/// Checks `password` against a PHC `hash`. Returns `false` for a wrong
/// password or an unparsable hash.
pub fn verify_password(password: &str, hash: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

/// [`hash_password`] on the blocking pool.
pub async fn hash_password_async(password: String) -> Result<String, AuthError> {
    tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .map_err(|e| AuthError::Internal(e.to_string()))?
}

/// [`verify_password`] on the blocking pool.
///
/// When `hash` is `None` (unknown user, or an account without a password) a
/// dummy hash is verified anyway, so response time does not reveal whether a
/// username exists. The result is then always `false`.
pub async fn verify_password_async(password: String, hash: Option<String>) -> bool {
    tokio::task::spawn_blocking(move || match hash {
        Some(h) => verify_password(&password, &h),
        None => {
            let _ = verify_password(&password, dummy_hash());
            false
        }
    })
    .await
    .unwrap_or(false)
}

fn dummy_hash() -> &'static str {
    static DUMMY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    DUMMY.get_or_init(|| hash_password("not-a-real-password").unwrap_or_default())
}

/// Checks a new password against the minimum length and an upper bound
/// (to keep hashing cost predictable).
pub fn validate_new_password(password: &str, min_len: usize) -> Result<(), AuthError> {
    let len = password.chars().count();
    if len < min_len {
        return Err(AuthError::BadRequest(format!(
            "password must be at least {min_len} characters"
        )));
    }
    if password.len() > 1024 {
        return Err(AuthError::BadRequest("password is too long".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_and_verify() {
        let h = hash_password("correct horse battery staple").unwrap();
        assert!(h.starts_with("$argon2id$"));
        assert!(verify_password("correct horse battery staple", &h));
        assert!(!verify_password("wrong", &h));
        assert!(!verify_password("x", "not a hash"));
    }

    #[test]
    fn length_rules() {
        assert!(validate_new_password("short", 10).is_err());
        assert!(validate_new_password("long enough!", 10).is_ok());
    }

    #[tokio::test]
    async fn unknown_user_is_false() {
        assert!(!verify_password_async("pw".into(), None).await);
    }
}
