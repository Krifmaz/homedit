//! Status checks: what to probe for a service, and what a probe returns.

use serde::{Deserialize, Serialize};

use crate::Service;

/// Default port for a `ping` target without an explicit port.
pub const DEFAULT_PING_PORT: u16 = 80;

/// The probe to run for one service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum StatusCheck {
    /// HTTP(S) GET; up when the response status is below 500.
    Http {
        /// URL to request.
        url: String,
    },
    /// TCP connect; up when the connection opens.
    Tcp {
        /// Host name or IP address.
        host: String,
        /// Port number.
        port: u16,
    },
}

impl StatusCheck {
    /// Works out which probe a service asks for.
    ///
    /// `siteMonitor` wins over `ping`. A `ping` value may be a bare host, a
    /// `host:port` pair, `[v6]:port`, or a URL (its host and port are used).
    /// Returns `None` when the service asks for no check or the value cannot
    /// be parsed.
    pub fn for_service(service: &Service) -> Option<Self> {
        if let Some(url) = service.site_monitor.as_deref().map(str::trim)
            && url::Url::parse(url).is_ok_and(|u| matches!(u.scheme(), "http" | "https"))
        {
            return Some(Self::Http {
                url: url.to_owned(),
            });
        }
        let ping = service.ping.as_deref()?.trim();
        parse_host_port(ping).map(|(host, port)| Self::Tcp { host, port })
    }
}

/// Parses `host`, `host:port`, `[v6]`, `[v6]:port`, or a URL.
fn parse_host_port(value: &str) -> Option<(String, u16)> {
    if value.is_empty() {
        return None;
    }
    if value.contains("://") {
        let u = url::Url::parse(value).ok()?;
        let host = u.host_str()?.trim_matches(['[', ']']).to_owned();
        return Some((host, u.port_or_known_default().unwrap_or(DEFAULT_PING_PORT)));
    }
    if let Some(rest) = value.strip_prefix('[') {
        let (host, after) = rest.split_once(']')?;
        let port = match after.strip_prefix(':') {
            Some(p) => p.parse().ok()?,
            None if after.is_empty() => DEFAULT_PING_PORT,
            None => return None,
        };
        return Some((host.to_owned(), port));
    }
    match value.rsplit_once(':') {
        // More than one colon and no brackets: a bare IPv6 address.
        Some((h, _)) if h.contains(':') => Some((value.to_owned(), DEFAULT_PING_PORT)),
        Some((h, p)) => Some((h.to_owned(), p.parse().ok()?)),
        None => Some((value.to_owned(), DEFAULT_PING_PORT)),
    }
}

/// The result of one probe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusSample {
    /// Service the sample belongs to.
    pub service_id: i64,
    /// When the probe finished, in milliseconds since the Unix epoch.
    pub checked_at: i64,
    /// Whether the service counted as up.
    pub ok: bool,
    /// HTTP status code, for HTTP checks that got a response.
    pub status_code: Option<u16>,
    /// Round-trip time in milliseconds, when the probe got an answer.
    pub latency_ms: Option<u32>,
    /// Short error text when the probe failed.
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svc(ping: Option<&str>, monitor: Option<&str>) -> Service {
        Service {
            ping: ping.map(Into::into),
            site_monitor: monitor.map(Into::into),
            ..Service::default()
        }
    }

    #[test]
    fn site_monitor_wins() {
        let c = StatusCheck::for_service(&svc(Some("nas"), Some("http://nas:8080")));
        assert_eq!(
            c,
            Some(StatusCheck::Http {
                url: "http://nas:8080".into()
            })
        );
    }

    #[test]
    fn bad_site_monitor_falls_back_to_ping() {
        let c = StatusCheck::for_service(&svc(Some("nas:22"), Some("ftp://nas")));
        assert_eq!(
            c,
            Some(StatusCheck::Tcp {
                host: "nas".into(),
                port: 22
            })
        );
    }

    #[test]
    fn ping_forms() {
        let cases = [
            ("10.0.0.2", ("10.0.0.2", 80)),
            ("10.0.0.2:8096", ("10.0.0.2", 8096)),
            ("[fd00::1]:443", ("fd00::1", 443)),
            ("[fd00::1]", ("fd00::1", 80)),
            ("fd00::1", ("fd00::1", 80)),
            ("https://jelly.lan", ("jelly.lan", 443)),
            ("http://jelly.lan:8096/web", ("jelly.lan", 8096)),
        ];
        for (input, (h, p)) in cases {
            assert_eq!(parse_host_port(input), Some((h.to_owned(), p)), "{input}");
        }
        assert_eq!(parse_host_port("host:notaport"), None);
        assert_eq!(parse_host_port(""), None);
    }

    #[test]
    fn no_check() {
        assert_eq!(StatusCheck::for_service(&svc(None, None)), None);
    }
}
