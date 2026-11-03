//! Small internal helpers shared by several modules.

use std::time::{SystemTime, UNIX_EPOCH};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use sha2::{Digest, Sha256};

/// Current time as whole seconds since the Unix epoch.
pub(crate) fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Returns `n` bytes from the operating system's CSPRNG, base64url encoded.
pub(crate) fn random_token(n: usize) -> String {
    let mut buf = vec![0u8; n];
    rand::rngs::OsRng.fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

/// SHA-256 of a token. Session tokens are only ever stored hashed, so a
/// leaked database does not hand out live sessions.
pub(crate) fn sha256(data: &str) -> Vec<u8> {
    Sha256::digest(data.as_bytes()).to_vec()
}

/// Constant-time string comparison.
pub(crate) fn ct_eq(a: &str, b: &str) -> bool {
    use subtle::ConstantTimeEq;
    a.len() == b.len() && bool::from(a.as_bytes().ct_eq(b.as_bytes()))
}

/// Reads one cookie from the request's `Cookie` headers.
pub(crate) fn get_cookie(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(axum::http::header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(cookie::Cookie::split_parse)
        .filter_map(Result::ok)
        .find(|c| c.name() == name)
        .map(|c| c.value().to_owned())
}

/// Builds a `Set-Cookie` header value. `max_age = None` makes a session
/// cookie; `Some(0)` deletes the cookie.
pub(crate) fn set_cookie(
    name: &str,
    value: &str,
    max_age: Option<i64>,
    secure: bool,
    same_site: cookie::SameSite,
) -> axum::http::HeaderValue {
    let mut c = cookie::Cookie::build((name.to_owned(), value.to_owned()))
        .path("/")
        .http_only(true)
        .secure(secure)
        .same_site(same_site);
    if let Some(secs) = max_age {
        c = c.max_age(cookie::time::Duration::seconds(secs));
    }
    axum::http::HeaderValue::from_str(&c.build().to_string())
        .unwrap_or_else(|_| axum::http::HeaderValue::from_static(""))
}
