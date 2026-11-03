//! Server-side request forgery protection for outbound requests.
//!
//! Widgets call third-party APIs from the server, with API keys the browser
//! never sees. That makes the server a proxy an attacker would love to
//! steer at something else (the Docker socket's TCP port, a router admin
//! page, a cloud metadata service). [`SsrfGuard`] holds an allowlist of the
//! hosts the admin actually configured, and [`GuardedClient`] refuses to
//! talk to anything else:
//!
//! * the initial URL must be `http`/`https` and on the allowlist;
//! * every redirect hop is checked the same way (at most 5 hops);
//! * DNS answers are filtered: unspecified, multicast and broadcast
//!   addresses and well-known cloud metadata endpoints are never dialled,
//!   unless an explicit CIDR allow entry covers them;
//! * no HTTP proxy from the environment is used, so these checks cannot be
//!   bypassed through one.
//!
//! Private addresses are allowed when configured, since that is what a
//! homelab dashboard talks to.

use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use ipnet::IpNet;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use url::{Host, Url};

use crate::client_ip::canonical;

/// Why a request was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SsrfError {
    /// The URL could not be parsed.
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
    /// Only `http` and `https` are allowed.
    #[error("scheme `{0}` is not allowed")]
    Scheme(String),
    /// URLs with embedded credentials (`user:pass@host`) are refused.
    #[error("credentials in URLs are not allowed")]
    Credentials,
    /// The host is not one the admin configured.
    #[error("host `{0}` is not a configured service host")]
    HostNotAllowed(String),
    /// The address is never allowed (metadata service, multicast, ...).
    #[error("address {0} is blocked")]
    AddressBlocked(IpAddr),
}

/// One allowlist entry.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum AllowEntry {
    /// A hostname or IP literal, optionally pinned to one port.
    Host {
        /// Lowercased host (IPv6 without brackets).
        host: String,
        /// Port, or `None` for any port.
        port: Option<u16>,
    },
    /// Any address in a CIDR range. Also overrides the built-in blocklist.
    Net(IpNet),
}

impl AllowEntry {
    /// Parses `host`, `host:port`, `[v6]:port`, a URL, or a CIDR.
    pub fn parse(s: &str) -> Result<Self, SsrfError> {
        let s = s.trim();
        if let Ok(net) = s.parse::<IpNet>() {
            return Ok(AllowEntry::Net(net));
        }
        if s.contains("://") {
            let url = Url::parse(s).map_err(|e| SsrfError::InvalidUrl(e.to_string()))?;
            let host = host_string(&url).ok_or_else(|| SsrfError::InvalidUrl(s.into()))?;
            return Ok(AllowEntry::Host { host, port: None });
        }
        let url =
            Url::parse(&format!("http://{s}")).map_err(|e| SsrfError::InvalidUrl(e.to_string()))?;
        let host = host_string(&url).ok_or_else(|| SsrfError::InvalidUrl(s.into()))?;
        Ok(AllowEntry::Host {
            host,
            port: url.port(),
        })
    }
}

fn host_string(url: &Url) -> Option<String> {
    Some(match url.host()? {
        Host::Domain(d) => d.trim_end_matches('.').to_ascii_lowercase(),
        Host::Ipv4(ip) => ip.to_string(),
        Host::Ipv6(ip) => ip.to_string(),
    })
}

/// Addresses that are never dialled unless a [`AllowEntry::Net`] covers
/// them.
pub fn is_always_blocked(ip: IpAddr) -> bool {
    const METADATA_V4: [Ipv4Addr; 2] = [
        Ipv4Addr::new(169, 254, 169, 254),
        Ipv4Addr::new(100, 100, 100, 200),
    ];
    const METADATA_V6: Ipv6Addr = Ipv6Addr::new(0xfd00, 0xec2, 0, 0, 0, 0, 0, 0x254);
    match canonical(ip) {
        IpAddr::V4(v4) => {
            v4.is_unspecified()
                || v4.is_multicast()
                || v4.is_broadcast()
                || METADATA_V4.contains(&v4)
        }
        IpAddr::V6(v6) => v6.is_unspecified() || v6.is_multicast() || v6 == METADATA_V6,
    }
}

/// A shared, updatable allowlist of outbound hosts.
#[derive(Debug, Clone, Default)]
pub struct SsrfGuard {
    entries: Arc<RwLock<BTreeSet<AllowEntry>>>,
}

impl SsrfGuard {
    /// An empty guard: allows nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// A guard allowing the hosts of the given URLs (any port).
    /// Unparsable entries are skipped with a warning.
    pub fn from_urls<'a>(urls: impl IntoIterator<Item = &'a str>) -> Self {
        let g = Self::new();
        g.replace(urls);
        g
    }

    /// Replaces the allowlist, e.g. after the dashboard config changed.
    /// Accepts anything [`AllowEntry::parse`] does.
    pub fn replace<'a>(&self, entries: impl IntoIterator<Item = &'a str>) {
        let set: BTreeSet<AllowEntry> = entries
            .into_iter()
            .filter(|s| !s.trim().is_empty())
            .filter_map(|s| match AllowEntry::parse(s) {
                Ok(e) => Some(e),
                Err(e) => {
                    tracing::warn!(entry = s, error = %e, "ignoring SSRF allowlist entry");
                    None
                }
            })
            .collect();
        *self.entries.write().unwrap_or_else(|e| e.into_inner()) = set;
    }

    /// Adds one entry.
    pub fn allow(&self, entry: AllowEntry) {
        self.entries
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(entry);
    }

    /// Current entries.
    pub fn entries(&self) -> Vec<AllowEntry> {
        self.entries
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect()
    }

    fn net_allows(&self, ip: IpAddr) -> bool {
        let ip = canonical(ip);
        self.entries
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .any(|e| matches!(e, AllowEntry::Net(n) if n.contains(&ip)))
    }

    /// Whether a resolved address may be dialled.
    pub fn check_ip(&self, ip: IpAddr) -> Result<(), SsrfError> {
        if is_always_blocked(ip) && !self.net_allows(ip) {
            Err(SsrfError::AddressBlocked(ip))
        } else {
            Ok(())
        }
    }

    /// Whether a hostname (without port) is on the allowlist for any port.
    fn host_allowed(&self, host: &str, port: Option<u16>) -> bool {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        let host = host.trim_start_matches('[').trim_end_matches(']');
        let ip = host.parse::<IpAddr>().ok();
        self.entries
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .any(|e| match e {
                AllowEntry::Host { host: h, port: p } => {
                    h == host && (p.is_none() || port.is_none() || *p == port)
                }
                AllowEntry::Net(n) => ip.is_some_and(|ip| n.contains(&canonical(ip))),
            })
    }

    /// Checks a URL before a request is made.
    pub fn check_url(&self, url: &Url) -> Result<(), SsrfError> {
        match url.scheme() {
            "http" | "https" => {}
            other => return Err(SsrfError::Scheme(other.into())),
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(SsrfError::Credentials);
        }
        let host = host_string(url).ok_or_else(|| SsrfError::InvalidUrl(url.to_string()))?;
        if let Ok(ip) = host.parse::<IpAddr>() {
            self.check_ip(ip)?;
        }
        if self.host_allowed(&host, url.port_or_known_default()) {
            Ok(())
        } else {
            Err(SsrfError::HostNotAllowed(host))
        }
    }

    /// Parses and checks a URL string.
    pub fn check_str(&self, url: &str) -> Result<Url, SsrfError> {
        let url = Url::parse(url).map_err(|e| SsrfError::InvalidUrl(e.to_string()))?;
        self.check_url(&url)?;
        Ok(url)
    }
}

/// DNS resolver that refuses names off the allowlist and filters blocked
/// addresses out of answers.
#[derive(Debug, Clone)]
struct GuardedResolver {
    guard: SsrfGuard,
}

impl Resolve for GuardedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let guard = self.guard.clone();
        let host = name.as_str().to_owned();
        Box::pin(async move {
            if !guard.host_allowed(&host, None) {
                return Err(Box::new(SsrfError::HostNotAllowed(host))
                    as Box<dyn std::error::Error + Send + Sync>);
            }
            let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await?
                .filter(|a| match guard.check_ip(a.ip()) {
                    Ok(()) => true,
                    Err(e) => {
                        tracing::warn!(host = %host, addr = %a.ip(), "{e}");
                        false
                    }
                })
                .collect();
            if addrs.is_empty() {
                return Err(Box::new(SsrfError::HostNotAllowed(host))
                    as Box<dyn std::error::Error + Send + Sync>);
            }
            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}

/// Options for [`GuardedClient::new`].
#[derive(Debug, Clone)]
pub struct ClientOptions {
    /// Whole-request timeout.
    pub timeout: Duration,
    /// TCP connect timeout.
    pub connect_timeout: Duration,
    /// Accept self-signed or otherwise invalid TLS certificates (common for
    /// Proxmox and other homelab services). Off by default.
    pub accept_invalid_certs: bool,
    /// `User-Agent` header.
    pub user_agent: String,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(10),
            connect_timeout: Duration::from_secs(5),
            accept_invalid_certs: false,
            user_agent: concat!("homedit/", env!("CARGO_PKG_VERSION")).into(),
        }
    }
}

/// Either refused by the guard or failed in transport.
#[derive(Debug, thiserror::Error)]
pub enum GuardedError {
    /// Refused before any network traffic.
    #[error(transparent)]
    Ssrf(#[from] SsrfError),
    /// The HTTP request failed (including redirects refused by the guard).
    #[error(transparent)]
    Http(#[from] reqwest::Error),
}

/// A `reqwest::Client` that only reaches allowlisted hosts.
#[derive(Debug, Clone)]
pub struct GuardedClient {
    client: reqwest::Client,
    guard: SsrfGuard,
}

impl GuardedClient {
    /// Builds a client enforcing `guard`.
    pub fn new(guard: SsrfGuard, opts: ClientOptions) -> Result<Self, reqwest::Error> {
        let redirect_guard = guard.clone();
        let policy = reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= 5 {
                attempt.error("too many redirects")
            } else if let Err(e) = redirect_guard.check_url(attempt.url()) {
                attempt.error(e)
            } else {
                attempt.follow()
            }
        });
        let client = reqwest::Client::builder()
            .redirect(policy)
            .dns_resolver(Arc::new(GuardedResolver {
                guard: guard.clone(),
            }))
            .no_proxy()
            .timeout(opts.timeout)
            .connect_timeout(opts.connect_timeout)
            .danger_accept_invalid_certs(opts.accept_invalid_certs)
            .user_agent(opts.user_agent)
            .build()?;
        Ok(Self { client, guard })
    }

    /// The guard, e.g. to update its allowlist.
    pub fn guard(&self) -> &SsrfGuard {
        &self.guard
    }

    /// Starts a request after checking the URL.
    pub fn request(
        &self,
        method: reqwest::Method,
        url: &str,
    ) -> Result<reqwest::RequestBuilder, SsrfError> {
        let url = self.guard.check_str(url)?;
        Ok(self.client.request(method, url))
    }

    /// `GET url`, checked.
    pub fn get(&self, url: &str) -> Result<reqwest::RequestBuilder, SsrfError> {
        self.request(reqwest::Method::GET, url)
    }

    /// `POST url`, checked.
    pub fn post(&self, url: &str) -> Result<reqwest::RequestBuilder, SsrfError> {
        self.request(reqwest::Method::POST, url)
    }

    /// Sends a built request, re-checking its URL first (in case it was
    /// changed after [`request`](Self::request)).
    pub async fn send(&self, req: reqwest::Request) -> Result<reqwest::Response, GuardedError> {
        self.guard.check_url(req.url())?;
        Ok(self.client.execute(req).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guard() -> SsrfGuard {
        SsrfGuard::from_urls([
            "http://sonarr.lan:8989",
            "192.168.1.10",
            "10.9.0.0/16",
            "plex.lan:32400",
            "[fd00::5]",
        ])
    }

    fn ok(g: &SsrfGuard, u: &str) -> bool {
        g.check_str(u).is_ok()
    }

    #[test]
    fn allowlisted_hosts_pass() {
        let g = guard();
        assert!(ok(&g, "http://sonarr.lan:8989/api/v3/system/status"));
        assert!(ok(&g, "https://SONARR.lan./x"), "case and trailing dot");
        assert!(
            ok(&g, "http://sonarr.lan:7878/"),
            "URL entries allow any port"
        );
        assert!(ok(&g, "http://192.168.1.10:9000/"));
        assert!(ok(&g, "http://10.9.4.4/"));
        assert!(ok(&g, "http://plex.lan:32400/identity"));
        assert!(ok(&g, "http://[fd00::5]:8080/"));
    }

    #[test]
    fn everything_else_fails() {
        let g = guard();
        assert!(!ok(&g, "http://plex.lan:8080/"), "port pinned");
        assert!(!ok(&g, "http://192.168.1.11/"));
        assert!(!ok(&g, "http://evil.example/"));
        assert!(!ok(&g, "file:///etc/passwd"));
        assert!(!ok(&g, "gopher://sonarr.lan/"));
        assert!(!ok(&g, "http://user:pw@sonarr.lan/"));
        assert!(!ok(&g, "http://169.254.169.254/latest/meta-data/"));
        assert!(!ok(&g, "http://[::ffff:192.168.1.11]/"));
    }

    #[test]
    fn metadata_blocked_even_if_listed_as_host() {
        let g = SsrfGuard::from_urls(["169.254.169.254"]);
        assert_eq!(
            g.check_str("http://169.254.169.254/").unwrap_err(),
            SsrfError::AddressBlocked("169.254.169.254".parse().unwrap())
        );
        let g = SsrfGuard::from_urls(["169.254.169.254/32"]);
        assert!(ok(&g, "http://169.254.169.254/"), "explicit CIDR overrides");
    }

    #[test]
    fn replace_updates_live_clones() {
        let g = guard();
        let clone = g.clone();
        g.replace(["new.lan"]);
        assert!(ok(&clone, "http://new.lan/"));
        assert!(!ok(&clone, "http://sonarr.lan:8989/"));
    }
}
