//! Encrypted secrets (API keys, passwords) at rest, masked in responses.
//!
//! * [`SecretBox`] encrypts with AES-256-GCM. The 32-byte key comes from
//!   `HOMEDIT_SECRET_KEY` (base64) or `HOMEDIT_SECRET_KEY_FILE`; see
//!   [`SecretBox::from_env`] and [`SecretBox::from_env_or_file`].
//! * Ciphertexts are strings: `enc:v1:` followed by base64 of
//!   `nonce (12 bytes) || ciphertext || tag`. An optional context string
//!   (e.g. `service:12:key`) is bound as associated data, so a ciphertext
//!   cannot be moved to another field.
//! * [`Secret`] holds a decrypted value. Its `Debug`, `Display` and
//!   `Serialize` never show the value: they print `••••` or, when
//!   serialized, the `{"$secret":"set"}` placeholder from the frontend
//!   contract. The memory is zeroed on drop.
//! * [`SecretInput`] parses what the browser sends back: the placeholder
//!   (or `••••`) means "keep", `null` means "clear", a string means "set".
//! * [`encrypt_json_secrets`] / [`decrypt_json_secrets`] /
//!   [`mask_json_secrets`] apply all this to gethomepage-style widget JSON,
//!   using [`homedit_core::secrets::is_secret_key`] to find secret fields.
//! * [`SecretStore`] keeps named secrets in the `auth_secrets` table.

use std::fmt;
use std::path::Path;
use std::sync::Arc;

use aes_gcm::aead::{Aead, KeyInit, OsRng, Payload};
use aes_gcm::{AeadCore, Aes256Gcm, Key, Nonce};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use sqlx::SqlitePool;
use zeroize::{Zeroize, Zeroizing};

pub use homedit_core::secrets::MASK;

/// Prefix of every ciphertext string.
pub const CIPHERTEXT_PREFIX: &str = "enc:v1:";
/// Environment variable holding the base64 key.
pub const KEY_ENV: &str = "HOMEDIT_SECRET_KEY";
/// Environment variable naming a file that holds the base64 key (Docker
/// secrets).
pub const KEY_FILE_ENV: &str = "HOMEDIT_SECRET_KEY_FILE";
/// Key placeholder key in JSON: `{"$secret": "set"}`.
pub const PLACEHOLDER_KEY: &str = "$secret";

/// Errors from key loading and decryption.
#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    /// No key configured.
    #[error(
        "no secret key: set {KEY_ENV} (generate one with `openssl rand -base64 32`) or {KEY_FILE_ENV}"
    )]
    MissingKey,
    /// The key is not 32 bytes of base64.
    #[error("secret key must be 32 bytes, base64 encoded")]
    BadKey,
    /// The ciphertext is malformed, was made with another key, or was
    /// tampered with.
    #[error("cannot decrypt secret (wrong key or corrupted value)")]
    Decrypt,
    /// Reading or writing the key file failed.
    #[error("key file: {0}")]
    Io(#[from] std::io::Error),
    /// Database error in [`SecretStore`].
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// AES-256-GCM encryption with one key. Cheap to clone.
#[derive(Clone)]
pub struct SecretBox {
    cipher: Arc<Aes256Gcm>,
}

impl fmt::Debug for SecretBox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretBox(..)")
    }
}

impl SecretBox {
    /// Uses a raw 32-byte key.
    pub fn from_key(key: &[u8; 32]) -> Self {
        Self {
            cipher: Arc::new(Aes256Gcm::new(&Key::<Aes256Gcm>::from(*key))),
        }
    }

    /// Uses a base64 (standard or URL-safe, padding optional) 32-byte key.
    pub fn from_base64(key: &str) -> Result<Self, SecretError> {
        let key = key.trim();
        let mut bytes = Zeroizing::new(
            STANDARD
                .decode(key)
                .or_else(|_| {
                    base64::engine::general_purpose::URL_SAFE_NO_PAD
                        .decode(key.trim_end_matches('='))
                })
                .map_err(|_| SecretError::BadKey)?,
        );
        let arr: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| SecretError::BadKey)?;
        bytes.zeroize();
        let mut arr = Zeroizing::new(arr);
        let out = Self::from_key(&arr);
        arr.zeroize();
        Ok(out)
    }

    /// Reads the key from `HOMEDIT_SECRET_KEY`, or from the file named by
    /// `HOMEDIT_SECRET_KEY_FILE`.
    pub fn from_env() -> Result<Self, SecretError> {
        if let Ok(k) = std::env::var(KEY_ENV) {
            if !k.trim().is_empty() {
                return Self::from_base64(&k);
            }
        }
        if let Ok(path) = std::env::var(KEY_FILE_ENV) {
            let k = Zeroizing::new(std::fs::read_to_string(path)?);
            return Self::from_base64(&k);
        }
        Err(SecretError::MissingKey)
    }

    /// Like [`from_env`](Self::from_env), but when no key is configured,
    /// loads `fallback` or creates it (mode 0600) with a fresh random key and
    /// logs a warning. Back the file up: losing it makes stored secrets
    /// unreadable.
    pub fn from_env_or_file(fallback: &Path) -> Result<Self, SecretError> {
        match Self::from_env() {
            Err(SecretError::MissingKey) => {}
            other => return other,
        }
        if fallback.exists() {
            let k = Zeroizing::new(std::fs::read_to_string(fallback)?);
            return Self::from_base64(&k);
        }
        let key = Zeroizing::new(Self::generate_key());
        if let Some(dir) = fallback.parent() {
            std::fs::create_dir_all(dir)?;
        }
        write_private(fallback, key.as_bytes())?;
        tracing::warn!(
            path = %fallback.display(),
            "{KEY_ENV} is not set; generated a secret key file. Back it up, or set {KEY_ENV} instead."
        );
        Self::from_base64(&key)
    }

    /// A fresh random key, base64 encoded, suitable for `HOMEDIT_SECRET_KEY`.
    pub fn generate_key() -> String {
        let key = Aes256Gcm::generate_key(OsRng);
        STANDARD.encode(key)
    }

    /// Encrypts `plaintext`, binding it to `context` (use `""` for none).
    pub fn encrypt(&self, plaintext: &str, context: &str) -> String {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ct = self
            .cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: plaintext.as_bytes(),
                    aad: context.as_bytes(),
                },
            )
            .expect("AES-GCM encryption cannot fail for in-memory buffers");
        let mut out = Vec::with_capacity(nonce.len() + ct.len());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        format!("{CIPHERTEXT_PREFIX}{}", STANDARD.encode(out))
    }

    /// Decrypts a value produced by [`encrypt`](Self::encrypt) with the same
    /// `context`.
    pub fn decrypt(&self, ciphertext: &str, context: &str) -> Result<Secret, SecretError> {
        let b64 = ciphertext
            .strip_prefix(CIPHERTEXT_PREFIX)
            .ok_or(SecretError::Decrypt)?;
        let raw = STANDARD.decode(b64).map_err(|_| SecretError::Decrypt)?;
        if raw.len() < 12 + 16 {
            return Err(SecretError::Decrypt);
        }
        let (nonce, ct) = raw.split_at(12);
        let nonce: [u8; 12] = nonce.try_into().map_err(|_| SecretError::Decrypt)?;
        let pt = self
            .cipher
            .decrypt(
                &Nonce::from(nonce),
                Payload {
                    msg: ct,
                    aad: context.as_bytes(),
                },
            )
            .map_err(|_| SecretError::Decrypt)?;
        let s = String::from_utf8(pt).map_err(|e| {
            let mut b = e.into_bytes();
            b.zeroize();
            SecretError::Decrypt
        })?;
        Ok(Secret::new(s))
    }
}

/// Whether a string is one of our ciphertexts.
pub fn is_ciphertext(s: &str) -> bool {
    s.starts_with(CIPHERTEXT_PREFIX)
}

#[cfg(unix)]
fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(data)
}

#[cfg(not(unix))]
fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, data)
}

/// A decrypted secret. Never printed or serialized in clear; zeroed on drop.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(Zeroizing<String>);

impl Secret {
    /// Wraps a plaintext value.
    pub fn new(value: impl Into<String>) -> Self {
        Self(Zeroizing::new(value.into()))
    }

    /// The plaintext. Use only where the value is actually needed (building
    /// an outbound request, an explicit export).
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Whether the value is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The JSON placeholder sent to browsers instead of the value.
    pub fn placeholder() -> Value {
        serde_json::json!({ PLACEHOLDER_KEY: "set" })
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(MASK)
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(MASK)
    }
}

impl Serialize for Secret {
    /// Serializes as `{"$secret":"set"}`, never the value.
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        Self::placeholder().serialize(s)
    }
}

/// `serialize_with` helper for the rare places that must write the
/// plaintext, such as a YAML export the admin explicitly asked to include
/// secrets in.
pub fn serialize_exposed<S: Serializer>(secret: &Secret, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(secret.expose())
}

/// What the browser sent for a secret field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretInput {
    /// Placeholder or mask sent back unchanged: keep the stored value.
    Keep,
    /// `null` or `""`: remove the value.
    Clear,
    /// A new value.
    Set(Secret),
}

impl SecretInput {
    /// Interprets a JSON value from a form.
    pub fn from_json(v: &Value) -> Self {
        match v {
            Value::Null => SecretInput::Clear,
            Value::String(s) if s.is_empty() => SecretInput::Clear,
            Value::String(s) if s == MASK => SecretInput::Keep,
            Value::String(s) => SecretInput::Set(Secret::new(s.clone())),
            v if is_placeholder(v) => SecretInput::Keep,
            other => SecretInput::Set(Secret::new(other.to_string())),
        }
    }
}

impl<'de> Deserialize<'de> for SecretInput {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Option::<Value>::deserialize(d)?.unwrap_or(Value::Null);
        Ok(Self::from_json(&v))
    }
}

/// Whether `v` is the `{"$secret": ...}` placeholder.
pub fn is_placeholder(v: &Value) -> bool {
    matches!(v, Value::Object(m) if m.len() == 1 && m.contains_key(PLACEHOLDER_KEY))
}

fn walk_secrets(
    v: &mut Value,
    path: &mut Vec<String>,
    f: &mut dyn FnMut(&str, &mut Value) -> Result<(), SecretError>,
) -> Result<(), SecretError> {
    match v {
        Value::Object(map) => {
            for (k, child) in map.iter_mut() {
                path.push(k.clone());
                if homedit_core::secrets::is_secret_key(k)
                    && !is_placeholder(child)
                    && !child.is_object()
                    && !child.is_array()
                {
                    f(&path.join("."), child)?;
                } else {
                    walk_secrets(child, path, f)?;
                }
                path.pop();
            }
        }
        Value::Array(items) => {
            for (i, child) in items.iter_mut().enumerate() {
                path.push(i.to_string());
                walk_secrets(child, path, f)?;
                path.pop();
            }
        }
        _ => {}
    }
    Ok(())
}

fn ctx(scope: &str, field: &str) -> String {
    format!("{scope}:{field}")
}

/// Encrypts every plaintext secret field of `v` in place. `scope`
/// identifies the owner (e.g. `service:12`) and is bound into each
/// ciphertext together with the field path. Already-encrypted values,
/// placeholders, `null` and empty strings are left alone.
pub fn encrypt_json_secrets(sb: &SecretBox, scope: &str, v: &mut Value) {
    let _ = walk_secrets(v, &mut Vec::new(), &mut |path, field| {
        if let Value::String(s) = field {
            if !s.is_empty() && !is_ciphertext(s) && s != MASK {
                let mut plain = std::mem::take(s);
                *s = sb.encrypt(&plain, &ctx(scope, path));
                plain.zeroize();
            }
        } else if !field.is_null() {
            // Numbers/bools in secret fields are stringified and encrypted.
            *field = Value::String(sb.encrypt(&field.to_string(), &ctx(scope, path)));
        }
        Ok(())
    });
}

/// Decrypts every ciphertext in `v` in place (server-side use only, e.g.
/// right before a widget request). Fails if any value cannot be decrypted.
pub fn decrypt_json_secrets(sb: &SecretBox, scope: &str, v: &mut Value) -> Result<(), SecretError> {
    walk_secrets(v, &mut Vec::new(), &mut |path, field| {
        if let Value::String(s) = field {
            if is_ciphertext(s) {
                let secret = sb.decrypt(s, &ctx(scope, path))?;
                *s = secret.expose().to_owned();
            }
        }
        Ok(())
    })
}

/// Replaces every non-empty secret field of `v` with the
/// `{"$secret":"set"}` placeholder, for responses to the browser.
pub fn mask_json_secrets(v: &mut Value) {
    let _ = walk_secrets(v, &mut Vec::new(), &mut |_, field| {
        if !(field.is_null() || field.as_str() == Some("")) {
            *field = Secret::placeholder();
        }
        Ok(())
    });
}

/// Merges an edited JSON document from the browser with the stored one:
/// wherever `incoming` still has the placeholder (or `••••`), the stored
/// (encrypted) value is kept; new plaintext values are then encrypted.
pub fn merge_json_secrets(sb: &SecretBox, scope: &str, incoming: &mut Value, stored: &Value) {
    fn restore(new: &mut Value, old: Option<&Value>) {
        match new {
            Value::Object(map) => {
                for (k, v) in map.iter_mut() {
                    let o = old.and_then(|o| o.get(k));
                    if is_placeholder(v) || v.as_str() == Some(MASK) {
                        *v = o.cloned().unwrap_or(Value::Null);
                    } else {
                        restore(v, o);
                    }
                }
            }
            Value::Array(items) => {
                for (i, v) in items.iter_mut().enumerate() {
                    restore(v, old.and_then(|o| o.get(i)));
                }
            }
            _ => {}
        }
    }
    restore(incoming, Some(stored));
    encrypt_json_secrets(sb, scope, incoming);
}

/// Named secrets in the `auth_secrets` table, encrypted with the name as
/// associated data.
#[derive(Debug, Clone)]
pub struct SecretStore {
    pool: SqlitePool,
    sb: SecretBox,
}

impl SecretStore {
    /// Wraps a pool and key.
    pub fn new(pool: SqlitePool, sb: SecretBox) -> Self {
        Self { pool, sb }
    }

    /// Stores or replaces a secret.
    pub async fn put(&self, name: &str, value: &Secret) -> Result<(), SecretError> {
        let ct = self.sb.encrypt(value.expose(), name);
        sqlx::query(
            "INSERT INTO auth_secrets (name, ciphertext, updated_at) VALUES (?, ?, ?)
             ON CONFLICT (name) DO UPDATE SET ciphertext = excluded.ciphertext, updated_at = excluded.updated_at",
        )
        .bind(name)
        .bind(ct)
        .bind(crate::util::now_unix())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Reads and decrypts a secret.
    pub async fn get(&self, name: &str) -> Result<Option<Secret>, SecretError> {
        let ct: Option<String> =
            sqlx::query_scalar("SELECT ciphertext FROM auth_secrets WHERE name = ?")
                .bind(name)
                .fetch_optional(&self.pool)
                .await?;
        ct.map(|c| self.sb.decrypt(&c, name)).transpose()
    }

    /// Whether a secret exists, without decrypting it.
    pub async fn exists(&self, name: &str) -> Result<bool, SecretError> {
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM auth_secrets WHERE name = ?")
            .bind(name)
            .fetch_one(&self.pool)
            .await?;
        Ok(n > 0)
    }

    /// Applies a form value: keep, clear, or set.
    pub async fn apply(&self, name: &str, input: &SecretInput) -> Result<(), SecretError> {
        match input {
            SecretInput::Keep => Ok(()),
            SecretInput::Clear => self.delete(name).await,
            SecretInput::Set(s) => self.put(name, s).await,
        }
    }

    /// Deletes a secret.
    pub async fn delete(&self, name: &str) -> Result<(), SecretError> {
        sqlx::query("DELETE FROM auth_secrets WHERE name = ?")
            .bind(name)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Names of all stored secrets.
    pub async fn names(&self) -> Result<Vec<String>, SecretError> {
        Ok(
            sqlx::query_scalar("SELECT name FROM auth_secrets ORDER BY name")
                .fetch_all(&self.pool)
                .await?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sb() -> SecretBox {
        SecretBox::from_key(&[7u8; 32])
    }

    #[test]
    fn round_trip_with_context() {
        let ct = sb().encrypt("hunter2", "service:1:key");
        assert!(ct.starts_with(CIPHERTEXT_PREFIX));
        assert!(!ct.contains("hunter2"));
        assert_eq!(
            sb().decrypt(&ct, "service:1:key").unwrap().expose(),
            "hunter2"
        );
        assert!(
            sb().decrypt(&ct, "service:2:key").is_err(),
            "context is bound"
        );
        assert!(
            SecretBox::from_key(&[8u8; 32])
                .decrypt(&ct, "service:1:key")
                .is_err(),
            "key is checked"
        );
    }

    #[test]
    fn nonces_differ() {
        assert_ne!(sb().encrypt("a", ""), sb().encrypt("a", ""));
    }

    #[test]
    fn tamper_detected() {
        let mut ct = sb().encrypt("value", "");
        let last = ct.pop().unwrap();
        ct.push(if last == 'A' { 'B' } else { 'A' });
        assert!(sb().decrypt(&ct, "").is_err());
    }

    #[test]
    fn key_parsing() {
        let k = SecretBox::generate_key();
        assert!(SecretBox::from_base64(&k).is_ok());
        assert!(matches!(
            SecretBox::from_base64("dG9vIHNob3J0"),
            Err(SecretError::BadKey)
        ));
    }

    #[test]
    fn secret_never_prints() {
        let s = Secret::new("p@ss");
        assert_eq!(format!("{s:?} {s}"), "•••• ••••");
        assert_eq!(serde_json::to_value(&s).unwrap(), json!({"$secret": "set"}));
    }

    #[test]
    fn secret_input_parsing() {
        let p: SecretInput = serde_json::from_value(json!({"$secret": "set"})).unwrap();
        assert_eq!(p, SecretInput::Keep);
        assert_eq!(SecretInput::from_json(&json!("••••")), SecretInput::Keep);
        assert_eq!(SecretInput::from_json(&json!(null)), SecretInput::Clear);
        assert_eq!(
            SecretInput::from_json(&json!("new")),
            SecretInput::Set(Secret::new("new"))
        );
    }

    #[test]
    fn json_encrypt_mask_merge_decrypt() {
        let sb = sb();
        let mut w = json!({"type": "sonarr", "url": "http://sonarr:8989", "key": "abc123", "fields": ["queue"]});
        encrypt_json_secrets(&sb, "service:4", &mut w);
        let stored = w.clone();
        assert!(is_ciphertext(w["key"].as_str().unwrap()));
        assert_eq!(w["url"], "http://sonarr:8989");

        let mut shown = stored.clone();
        mask_json_secrets(&mut shown);
        assert_eq!(shown["key"], json!({"$secret": "set"}));

        // Browser sends the placeholder back with another field changed.
        let mut incoming = shown.clone();
        incoming["url"] = json!("http://sonarr:9999");
        merge_json_secrets(&sb, "service:4", &mut incoming, &stored);
        assert_eq!(incoming["key"], stored["key"]);

        // Browser sets a new key.
        let mut incoming2 = shown;
        incoming2["key"] = json!("newkey");
        merge_json_secrets(&sb, "service:4", &mut incoming2, &stored);
        decrypt_json_secrets(&sb, "service:4", &mut incoming2).unwrap();
        assert_eq!(incoming2["key"], "newkey");

        // A ciphertext copied to another service does not decrypt.
        let mut moved = stored;
        assert!(decrypt_json_secrets(&sb, "service:5", &mut moved).is_err());
    }

    #[test]
    fn key_file_is_created_private() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secret.key");
        // Only meaningful when the env var is not set in the test environment.
        if std::env::var(KEY_ENV).is_err() && std::env::var(KEY_FILE_ENV).is_err() {
            let a = SecretBox::from_env_or_file(&path).unwrap();
            let b = SecretBox::from_env_or_file(&path).unwrap();
            let ct = a.encrypt("x", "");
            assert_eq!(b.decrypt(&ct, "").unwrap().expose(), "x");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
        }
    }
}
