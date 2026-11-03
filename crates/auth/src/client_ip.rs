//! Working out who is on the other end of a request.
//!
//! `X-Forwarded-For`, `X-Forwarded-Proto` and `X-Forwarded-Host` are only
//! believed when the TCP peer is one of the configured trusted proxies.
//! Otherwise the peer address and the request itself are used.

use std::net::{IpAddr, SocketAddr};

use axum::extract::ConnectInfo;
use axum::http::HeaderMap;
use axum::http::request::Parts;
use ipnet::IpNet;

/// Facts about the connection, computed once per request.
#[derive(Debug, Clone)]
pub struct ClientInfo {
    /// The TCP peer, if the server was started with connect info.
    pub peer: Option<IpAddr>,
    /// Whether `peer` is a trusted proxy.
    pub peer_trusted: bool,
    /// Best guess at the real client address.
    pub ip: Option<IpAddr>,
    /// Whether the client reached us over HTTPS.
    pub https: bool,
    /// The host (and port) the client asked for.
    pub host: Option<String>,
}

/// Whether `ip` is inside any of `nets`. IPv4-mapped IPv6 addresses are
/// compared as IPv4.
pub fn is_trusted(ip: IpAddr, nets: &[IpNet]) -> bool {
    let ip = canonical(ip);
    nets.iter().any(|n| n.contains(&ip))
}

/// Converts `::ffff:a.b.c.d` to `a.b.c.d`.
pub fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(v6)),
        v4 => v4,
    }
}

impl ClientInfo {
    /// Derives client info from request parts.
    pub fn from_parts(parts: &Parts, trusted: &[IpNet]) -> Self {
        let peer = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|c| canonical(c.0.ip()));
        Self::from_headers(
            peer,
            &parts.headers,
            parts.uri.scheme_str() == Some("https"),
            parts.uri.authority().map(|a| a.to_string()),
            trusted,
        )
    }

    /// The pure part of [`from_parts`](Self::from_parts), for tests.
    pub fn from_headers(
        peer: Option<IpAddr>,
        headers: &HeaderMap,
        uri_https: bool,
        uri_authority: Option<String>,
        trusted: &[IpNet],
    ) -> Self {
        let peer_trusted = peer.is_some_and(|p| is_trusted(p, trusted));
        let header = |name: &str| {
            headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::trim)
        };
        let own_host = header("host").map(str::to_owned).or(uri_authority);

        if !peer_trusted {
            return Self {
                peer,
                peer_trusted,
                ip: peer,
                https: uri_https,
                host: own_host,
            };
        }

        // Walk X-Forwarded-For right to left, skipping our own proxies.
        let mut ip = peer;
        if let Some(xff) = header("x-forwarded-for") {
            let hops: Vec<IpAddr> = xff
                .split(',')
                .filter_map(|h| parse_hop(h.trim()))
                .map(canonical)
                .collect();
            ip = hops
                .iter()
                .rev()
                .find(|h| !is_trusted(**h, trusted))
                .or(hops.first())
                .copied()
                .or(peer);
        }
        let https = match header("x-forwarded-proto") {
            Some(p) => p
                .split(',')
                .next()
                .is_some_and(|p| p.trim().eq_ignore_ascii_case("https")),
            None => uri_https,
        };
        let host = header("x-forwarded-host")
            .and_then(|h| h.split(',').next())
            .map(|h| h.trim().to_owned())
            .or(own_host);
        Self {
            peer,
            peer_trusted,
            ip,
            https,
            host,
        }
    }
}

fn parse_hop(s: &str) -> Option<IpAddr> {
    s.parse::<IpAddr>()
        .ok()
        .or_else(|| s.parse::<SocketAddr>().ok().map(|a| a.ip()))
        .or_else(|| s.trim_start_matches('[').split(']').next()?.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, HeaderValue::from_static(v));
        }
        h
    }

    #[test]
    fn untrusted_peer_headers_are_ignored() {
        let h = headers(&[
            ("x-forwarded-for", "1.2.3.4"),
            ("x-forwarded-proto", "https"),
            ("host", "dash.lan"),
        ]);
        let c =
            ClientInfo::from_headers(Some("192.168.1.9".parse().unwrap()), &h, false, None, &[]);
        assert_eq!(c.ip, Some("192.168.1.9".parse().unwrap()));
        assert!(!c.https);
        assert_eq!(c.host.as_deref(), Some("dash.lan"));
    }

    #[test]
    fn trusted_proxy_chain() {
        let trusted: Vec<IpNet> = vec!["172.16.0.0/12".parse().unwrap()];
        let h = headers(&[
            ("x-forwarded-for", "6.6.6.6, 203.0.113.7, 172.18.0.3"),
            ("x-forwarded-proto", "https"),
            ("x-forwarded-host", "dash.example.com"),
        ]);
        let c = ClientInfo::from_headers(
            Some("::ffff:172.18.0.2".parse().unwrap()),
            &h,
            false,
            None,
            &trusted,
        );
        assert!(c.peer_trusted);
        assert_eq!(c.ip, Some("203.0.113.7".parse().unwrap()));
        assert!(c.https);
        assert_eq!(c.host.as_deref(), Some("dash.example.com"));
    }
}
