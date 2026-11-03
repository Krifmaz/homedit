//! Masking of secret values before data leaves the server.
//!
//! Widget configs carry API keys and passwords. Until a value is stored in the
//! encrypted secret store, the server must never send it to a browser in
//! full. [`redact`] walks a JSON value and replaces every secret-looking
//! field with [`MASK`].

use serde_json::Value;

/// Placeholder shown instead of a secret.
pub const MASK: &str = "••••";

/// Field names (compared case-insensitively, ignoring `_` and `-`) whose
/// values are secrets.
const SECRET_KEYS: &[&str] = &[
    "key",
    "apikey",
    "password",
    "pass",
    "token",
    "accesstoken",
    "secret",
    "clientsecret",
    "auth",
    "authorization",
    "cookie",
];

/// Whether a field name holds a secret.
pub fn is_secret_key(key: &str) -> bool {
    let norm: String = key
        .chars()
        .filter(|c| !matches!(c, '_' | '-'))
        .flat_map(char::to_lowercase)
        .collect();
    SECRET_KEYS.contains(&norm.as_str())
}

/// Replaces every non-empty secret value in `value`, at any depth, with
/// [`MASK`].
pub fn redact(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                if is_secret_key(k) && !v.is_null() && v.as_str() != Some("") {
                    *v = Value::String(MASK.into());
                } else {
                    redact(v);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(redact),
        _ => {}
    }
}

/// Puts secrets back after an edit: wherever `incoming` still holds
/// [`MASK`], the value from `stored` is used instead. This lets a browser
/// send back a form it received with masked fields without wiping secrets.
pub fn restore_masked(incoming: &mut Value, stored: &Value) {
    match (incoming, stored) {
        (Value::Object(new), Value::Object(old)) => {
            for (k, v) in new.iter_mut() {
                if v.as_str() == Some(MASK) {
                    if let Some(orig) = old.get(k) {
                        *v = orig.clone();
                    }
                } else if let Some(orig) = old.get(k) {
                    restore_masked(v, orig);
                }
            }
        }
        (Value::Array(new), Value::Array(old)) => {
            for (n, o) in new.iter_mut().zip(old) {
                restore_masked(n, o);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_nested() {
        let mut v = json!({"type":"sonarr","url":"http://s","key":"abc",
            "inner":[{"api_key":"x","password":""}],"username":"me"});
        redact(&mut v);
        assert_eq!(
            v,
            json!({"type":"sonarr","url":"http://s","key":MASK,
            "inner":[{"api_key":MASK,"password":""}],"username":"me"})
        );
    }

    #[test]
    fn restores() {
        let stored = json!({"key":"abc","url":"http://a"});
        let mut incoming = json!({"key":MASK,"url":"http://b"});
        restore_masked(&mut incoming, &stored);
        assert_eq!(incoming, json!({"key":"abc","url":"http://b"}));
    }
}
