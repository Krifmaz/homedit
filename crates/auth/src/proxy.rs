//! Trusted reverse-proxy header authentication.
//!
//! Forward-auth proxies (Authelia, Authentik, oauth2-proxy, Caddy/Traefik
//! forward_auth, ...) authenticate the user and pass the result in headers
//! such as `Remote-User` and `Remote-Groups`. Those headers are trivially
//! forgeable by anyone who can reach homedit directly, so they are only
//! believed when the TCP peer is a configured trusted proxy.

use axum::http::HeaderMap;

use crate::client_ip::ClientInfo;
use crate::config::ProxyConfig;
use crate::extract::AuthUser;
use crate::role::{Role, UserSource};

/// Reads the proxy headers. Returns `None` when the peer is not trusted or
/// no username header is present.
pub fn authenticate(
    cfg: &ProxyConfig,
    client: &ClientInfo,
    headers: &HeaderMap,
) -> Option<AuthUser> {
    let username = headers.get(cfg.user_header.as_str())?.to_str().ok()?.trim();
    if !client.peer_trusted {
        tracing::debug!(peer = ?client.peer, "ignoring proxy auth header from untrusted peer");
        return None;
    }
    if username.is_empty() || username.len() > 256 {
        return None;
    }
    let groups = headers
        .get_all(cfg.groups_header.as_str())
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split([',', '|']))
        .map(str::trim)
        .filter(|g| !g.is_empty());
    let mut role = cfg.default_role;
    if cfg
        .admin_users
        .iter()
        .any(|u| u.eq_ignore_ascii_case(username))
    {
        role = Role::Admin;
    }
    if let Some(admin_group) = &cfg.admin_group
        && groups.into_iter().any(|g| g == admin_group)
    {
        role = Role::Admin;
    }
    Some(AuthUser {
        id: None,
        username: username.to_owned(),
        role,
        source: UserSource::Proxy,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn client(trusted: bool) -> ClientInfo {
        ClientInfo {
            peer: Some("10.0.0.2".parse().unwrap()),
            peer_trusted: trusted,
            ip: None,
            https: false,
            host: None,
        }
    }

    fn cfg() -> ProxyConfig {
        ProxyConfig {
            admin_group: Some("admins".into()),
            ..ProxyConfig::default()
        }
    }

    #[test]
    fn trusted_peer_with_admin_group() {
        let mut h = HeaderMap::new();
        h.insert("remote-user", HeaderValue::from_static("alice"));
        h.insert("remote-groups", HeaderValue::from_static("users,admins"));
        let u = authenticate(&cfg(), &client(true), &h).unwrap();
        assert_eq!(u.username, "alice");
        assert_eq!(u.role, Role::Admin);
    }

    #[test]
    fn plain_member_is_viewer() {
        let mut h = HeaderMap::new();
        h.insert("remote-user", HeaderValue::from_static("bob"));
        h.insert(
            "remote-groups",
            HeaderValue::from_static("users|admins-not"),
        );
        assert_eq!(
            authenticate(&cfg(), &client(true), &h).unwrap().role,
            Role::Viewer
        );
    }

    #[test]
    fn untrusted_peer_is_ignored() {
        let mut h = HeaderMap::new();
        h.insert("remote-user", HeaderValue::from_static("mallory"));
        h.insert("remote-groups", HeaderValue::from_static("admins"));
        assert!(authenticate(&cfg(), &client(false), &h).is_none());
    }
}
