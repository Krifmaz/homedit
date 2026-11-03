//! Cross-site request forgery protection.
//!
//! Two independent checks run on every state-changing request (anything
//! other than `GET`, `HEAD`, `OPTIONS` and `TRACE`):
//!
//! 1. **Origin.** If the browser sent `Origin`, it must match the host the
//!    request was made to (or an entry in
//!    [`AuthConfig::allowed_origins`](crate::AuthConfig::allowed_origins)).
//!    Without `Origin`, a `Sec-Fetch-Site: cross-site` header is rejected.
//! 2. **Token.** The `X-CSRF-Token` header must equal the caller's CSRF
//!    token: the one bound to their session, or, for callers without a
//!    session (auth disabled, proxy auth, anonymous), the value of the
//!    `HttpOnly`, `SameSite=Strict` CSRF cookie. The frontend gets the token
//!    from `GET /api/me`.

use axum::http::{HeaderMap, Method};

use crate::client_ip::ClientInfo;
use crate::error::AuthError;
use crate::util::ct_eq;

/// Header the frontend puts the token in.
pub const CSRF_HEADER: &str = "x-csrf-token";
/// Cookie holding the token for callers without a session.
pub const CSRF_COOKIE: &str = "homedit_csrf";

/// Methods that must not change state and so skip CSRF checks.
pub fn is_safe_method(m: &Method) -> bool {
    matches!(
        *m,
        Method::GET | Method::HEAD | Method::OPTIONS | Method::TRACE
    )
}

/// Check 1: `Origin` / `Sec-Fetch-Site`.
pub fn check_origin(
    headers: &HeaderMap,
    client: &ClientInfo,
    allowed: &[String],
) -> Result<(), AuthError> {
    let get = |n: &str| headers.get(n).and_then(|v| v.to_str().ok());
    match get("origin") {
        Some("null") => Err(AuthError::Csrf("opaque origin")),
        Some(origin) => {
            let origin = origin.trim_end_matches('/');
            if allowed
                .iter()
                .any(|a| a.trim_end_matches('/').eq_ignore_ascii_case(origin))
            {
                return Ok(());
            }
            let origin_host = origin.split_once("://").map(|(_, h)| h).unwrap_or(origin);
            match &client.host {
                Some(host) if host_matches(origin_host, host) => Ok(()),
                _ => Err(AuthError::Csrf("origin does not match host")),
            }
        }
        None => match get("sec-fetch-site") {
            Some("cross-site") => Err(AuthError::Csrf("cross-site request")),
            _ => Ok(()),
        },
    }
}

/// Compares `host[:port]` strings, treating a missing port as the default
/// and ignoring case.
fn host_matches(a: &str, b: &str) -> bool {
    fn strip_default(h: &str) -> &str {
        h.strip_suffix(":80")
            .or_else(|| h.strip_suffix(":443"))
            .unwrap_or(h)
    }
    strip_default(a).eq_ignore_ascii_case(strip_default(b))
}

/// Check 2: the `X-CSRF-Token` header against the expected token.
pub fn check_token(headers: &HeaderMap, expected: &str) -> Result<(), AuthError> {
    let sent = headers
        .get(CSRF_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !expected.is_empty() && ct_eq(sent, expected) {
        Ok(())
    } else {
        Err(AuthError::Csrf("missing or wrong X-CSRF-Token"))
    }
}

/// Whether a cookie value looks like a token we issued (43 base64url chars).
pub(crate) fn plausible_token(v: &str) -> bool {
    v.len() == 43
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn client(host: &str) -> ClientInfo {
        ClientInfo {
            peer: None,
            peer_trusted: false,
            ip: None,
            https: false,
            host: Some(host.into()),
        }
    }

    fn h(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut m = HeaderMap::new();
        for (k, v) in pairs {
            m.insert(*k, HeaderValue::from_static(v));
        }
        m
    }

    #[test]
    fn same_origin_passes() {
        assert!(
            check_origin(
                &h(&[("origin", "http://dash.lan:3000")]),
                &client("dash.lan:3000"),
                &[]
            )
            .is_ok()
        );
        assert!(
            check_origin(
                &h(&[("origin", "https://dash.example.com")]),
                &client("dash.example.com:443"),
                &[]
            )
            .is_ok()
        );
    }

    #[test]
    fn cross_origin_fails() {
        assert!(
            check_origin(
                &h(&[("origin", "https://evil.example")]),
                &client("dash.lan"),
                &[]
            )
            .is_err()
        );
        assert!(check_origin(&h(&[("origin", "null")]), &client("dash.lan"), &[]).is_err());
        assert!(
            check_origin(
                &h(&[("sec-fetch-site", "cross-site")]),
                &client("dash.lan"),
                &[]
            )
            .is_err()
        );
    }

    #[test]
    fn allowed_origin_list() {
        let allowed = vec!["https://other.example/".to_string()];
        assert!(
            check_origin(
                &h(&[("origin", "https://other.example")]),
                &client("dash.lan"),
                &allowed
            )
            .is_ok()
        );
    }

    #[test]
    fn non_browser_without_origin_passes_origin_check() {
        assert!(check_origin(&HeaderMap::new(), &client("dash.lan"), &[]).is_ok());
    }

    #[test]
    fn token_check() {
        assert!(check_token(&h(&[("x-csrf-token", "abc")]), "abc").is_ok());
        assert!(check_token(&h(&[("x-csrf-token", "abd")]), "abc").is_err());
        assert!(check_token(&HeaderMap::new(), "abc").is_err());
        assert!(check_token(&HeaderMap::new(), "").is_err());
    }
}
