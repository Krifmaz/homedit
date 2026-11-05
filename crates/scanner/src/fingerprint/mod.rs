//! Working out which app answers on an open port.
//!
//! For each open port the [`Fingerprinter`]:
//!
//! 1. `GET`s `/` over HTTP (or HTTPS first on TLS-style ports), falling back
//!    to the other scheme. If neither speaks HTTP the port is reported as a
//!    plain TCP service.
//! 2. Reads the page title, `Server` header, redirect target and favicon.
//! 3. Scores every catalogue app on those clues ([`Signal`]s) and its usual
//!    ports.
//! 4. Runs the confirming [`Probe`]s of the best few candidates (known API
//!    paths such as `/System/Info/Public` or `/identity`), which can also
//!    reveal the version.
//! 5. Picks the highest score above [`MATCH_THRESHOLD`].
//!
//! Each request waits on the scan's [`RateLimiter`], so fingerprinting obeys
//! the same per-second budget as the port scan.

mod hash;
mod html;
mod http;

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use url::Url;

pub use hash::{favicon_hash, murmur3_32};
pub use html::{favicon_href, title as page_title};
pub use http::{HttpProber, HttpResponse, MAX_BODY};

use crate::catalogue::{AppDef, Catalogue, Check, Probe, Signal, VersionSource};
use crate::rate::RateLimiter;

/// Lowest score that counts as a match. A title or a strong API probe alone
/// clears it; a default port alone never does.
pub const MATCH_THRESHOLD: u32 = 50;

/// Score added when the port is one of the app's default ports.
const PORT_BONUS: u32 = 10;

/// Most candidate apps whose probes are run for one port.
const MAX_CANDIDATES: usize = 6;

/// Largest favicon that is hashed.
const MAX_FAVICON: usize = 100 * 1024;

/// Largest favicon whose bytes are kept in the result for display.
const MAX_FAVICON_INLINE: usize = 32 * 1024;

/// Ports where HTTPS is tried before plain HTTP.
const TLS_FIRST: &[u16] = &[443, 2376, 5001, 8006, 8007, 8443, 8920, 9443, 10443];

/// HTTP or HTTPS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scheme {
    /// Plain HTTP.
    Http,
    /// HTTP over TLS.
    Https,
}

impl Scheme {
    /// `"http"` or `"https"`.
    pub fn as_str(self) -> &'static str {
        match self {
            Scheme::Http => "http",
            Scheme::Https => "https",
        }
    }

    fn other(self) -> Self {
        match self {
            Scheme::Http => Scheme::Https,
            Scheme::Https => Scheme::Http,
        }
    }
}

/// A favicon found on a web service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Favicon {
    /// Where it was fetched from.
    pub url: String,
    /// Shodan-style hash, see [`favicon_hash`].
    pub hash: i32,
    /// Content type reported by the server, or sniffed from the bytes.
    pub mime: String,
    /// The image as base64, when small enough to show in the UI
    /// (32 KiB or less). Use it as `data:{mime};base64,{data}`.
    pub data: Option<String>,
}

/// The app a service was identified as.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppMatch {
    /// Catalogue id, e.g. `jellyfin`.
    pub id: String,
    /// Display name, e.g. `Jellyfin`.
    pub name: String,
    /// Dashboard widget type, when the app has one.
    pub widget: Option<String>,
    /// Icon name in the dashboard-icons set.
    pub icon: String,
    /// Version, when an unauthenticated endpoint reveals it.
    pub version: Option<String>,
    /// 0–100, how sure the match is.
    pub confidence: u8,
    /// Human-readable list of what matched, for the scan log and the UI.
    pub evidence: Vec<String>,
}

impl AppMatch {
    /// Builds a match for `app` with the given score and evidence.
    pub fn from_def(
        app: &AppDef,
        score: u32,
        version: Option<String>,
        evidence: Vec<String>,
    ) -> Self {
        Self {
            id: app.id.to_string(),
            name: app.name.to_string(),
            widget: app.widget.map(str::to_string),
            icon: app.icon.to_string(),
            version,
            confidence: score.min(100) as u8,
            evidence,
        }
    }
}

/// What was learned about a port that speaks HTTP.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpInfo {
    /// Which scheme answered.
    pub scheme: Scheme,
    /// Status code of `/`.
    pub status: u16,
    /// Page title of `/`, cleaned up into a name: no "Login |", "Sign in ·",
    /// "Dashboard" and the like ([`clean_title`](crate::naming::clean_title)).
    /// `None` when the title was empty or only such words. Matching uses the
    /// raw title.
    pub title: Option<String>,
    /// `Server` header of `/`.
    pub server: Option<String>,
    /// Where `/` redirected to, when it pointed elsewhere.
    pub redirect: Option<String>,
    /// The favicon, if one was found.
    pub favicon: Option<Favicon>,
}

/// Identifies apps on open ports. Cheap to clone.
#[derive(Debug, Clone)]
pub struct Fingerprinter {
    http: HttpProber,
    catalogue: Arc<Catalogue>,
    limiter: Arc<RateLimiter>,
}

impl Fingerprinter {
    /// Builds a fingerprinter around an HTTP prober, a catalogue and the
    /// scan's rate limiter.
    pub fn new(http: HttpProber, catalogue: Arc<Catalogue>, limiter: Arc<RateLimiter>) -> Self {
        Self {
            http,
            catalogue,
            limiter,
        }
    }

    async fn get(&self, url: Url) -> Option<HttpResponse> {
        self.limiter.acquire().await;
        match self.http.get(url.clone()).await {
            Ok(resp) => Some(resp),
            Err(e) => {
                tracing::trace!(%url, error = %e, "fingerprint request failed");
                None
            }
        }
    }

    /// Looks at `ip:port`. Returns `None` when it does not speak HTTP(S);
    /// otherwise what the web service shows and, if recognised, the app.
    pub async fn fingerprint(&self, ip: IpAddr, port: u16) -> Option<(HttpInfo, Option<AppMatch>)> {
        let first = if TLS_FIRST.contains(&port) {
            Scheme::Https
        } else {
            Scheme::Http
        };
        let (scheme, root) = match self.get(base_url(first, ip, port)).await {
            Some(resp) if !wants_tls(first, &resp) => (first, resp),
            Some(resp) => match self.get(base_url(first.other(), ip, port)).await {
                Some(tls) => (first.other(), tls),
                None => (first, resp),
            },
            None => {
                let other = first.other();
                (other, self.get(base_url(other, ip, port)).await?)
            }
        };

        let body = root.text();
        let title = html::title(&body);
        let server = root.header("server").map(str::to_string);
        let redirect = (root.status / 100 == 3)
            .then(|| root.header("location").map(str::to_string))
            .flatten()
            .or_else(|| {
                let start = base_url(scheme, ip, port);
                (root.url != start).then(|| root.url.to_string())
            });
        let favicon = self.favicon(&root, &body).await;

        let app = self
            .identify(scheme, ip, port, &root, title.as_deref(), favicon.as_ref())
            .await;

        let info = HttpInfo {
            scheme,
            status: root.status,
            title: title.as_deref().and_then(crate::naming::clean_title),
            server,
            redirect,
            favicon,
        };
        Some((info, app))
    }

    async fn favicon(&self, root: &HttpResponse, body: &str) -> Option<Favicon> {
        let href = html::favicon_href(body).unwrap_or_else(|| "/favicon.ico".to_string());
        let url = root.url.join(&href).ok()?;
        // Only same-host icons: never let a page send the scanner elsewhere.
        if url.host() != root.url.host() || !matches!(url.scheme(), "http" | "https") {
            return None;
        }
        let resp = self.get(url).await?;
        if !resp.is_success() || resp.body.is_empty() || resp.body.len() > MAX_FAVICON {
            return None;
        }
        let mime = resp
            .header("content-type")
            .map(|v| v.split(';').next().unwrap_or(v).trim().to_ascii_lowercase())
            .filter(|m| m.starts_with("image/"))
            .or_else(|| sniff_image(&resp.body).map(str::to_string))?;
        Some(Favicon {
            url: resp.url.to_string(),
            hash: favicon_hash(&resp.body),
            mime,
            data: (resp.body.len() <= MAX_FAVICON_INLINE).then(|| STANDARD.encode(&resp.body)),
        })
    }

    async fn identify(
        &self,
        scheme: Scheme,
        ip: IpAddr,
        port: u16,
        root: &HttpResponse,
        title: Option<&str>,
        favicon: Option<&Favicon>,
    ) -> Option<AppMatch> {
        let body_lower = root.text().to_ascii_lowercase();
        let mut scored: Vec<Candidate<'_>> = self
            .catalogue
            .apps()
            .iter()
            .enumerate()
            .filter_map(|(order, app)| {
                let mut c = Candidate {
                    app,
                    order,
                    score: 0,
                    port_match: app.ports.contains(&port),
                    evidence: Vec::new(),
                    version: None,
                };
                for signal in app.signals {
                    if let Some(ev) = signal_matches(signal, root, &body_lower, title, favicon) {
                        c.score += signal.weight();
                        c.evidence.push(ev);
                    }
                }
                if c.port_match {
                    c.score += PORT_BONUS;
                    c.evidence.push(format!("default port {port}"));
                }
                (c.score > 0).then_some(c)
            })
            .collect();
        scored.sort_by(Candidate::rank);
        scored.truncate(MAX_CANDIDATES);

        // Several apps share probe paths (the *arr apps, the Seerr family),
        // so each path is fetched at most once.
        let mut cache: HashMap<&'static str, Option<HttpResponse>> = HashMap::new();
        for cand in &mut scored {
            for probe in cand.app.probes {
                if !cache.contains_key(probe.path) {
                    let resp = match base_url(scheme, ip, port).join(probe.path) {
                        Ok(url) => self.get(url).await,
                        Err(_) => None,
                    };
                    cache.insert(probe.path, resp);
                }
                let Some(Some(resp)) = cache.get(probe.path) else {
                    continue;
                };
                if probe_matches(probe, resp) {
                    cand.score += probe.weight;
                    cand.evidence
                        .push(format!("GET {} -> {}", probe.path, resp.status));
                    if cand.version.is_none() {
                        cand.version = probe.version.and_then(|v| read_version(v, resp));
                    }
                }
            }
        }
        scored.sort_by(Candidate::rank);
        let best = scored.into_iter().next()?;
        (best.score >= MATCH_THRESHOLD)
            .then(|| AppMatch::from_def(best.app, best.score, best.version, best.evidence))
    }
}

struct Candidate<'a> {
    app: &'a AppDef,
    order: usize,
    score: u32,
    port_match: bool,
    evidence: Vec<String>,
    version: Option<String>,
}

impl Candidate<'_> {
    /// Highest score first, then default-port matches, then catalogue order.
    fn rank(a: &Self, b: &Self) -> std::cmp::Ordering {
        b.score
            .cmp(&a.score)
            .then(b.port_match.cmp(&a.port_match))
            .then(a.order.cmp(&b.order))
    }
}

/// The URL of `/` for a scheme, address and port.
pub fn base_url(scheme: Scheme, ip: IpAddr, port: u16) -> Url {
    let host = match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => format!("[{v6}]"),
    };
    // Built only from an address and a number, so it always parses.
    Url::parse(&format!("{}://{host}:{port}/", scheme.as_str()))
        .unwrap_or_else(|_| unreachable!("URL from address and port"))
}

/// Whether a plain-HTTP answer says the port actually wants TLS.
fn wants_tls(scheme: Scheme, resp: &HttpResponse) -> bool {
    scheme == Scheme::Http
        && resp.status == 400
        && resp.text().to_ascii_lowercase().contains("https")
}

fn contains_ci(haystack_lower: &str, needle: &str) -> bool {
    haystack_lower.contains(&needle.to_ascii_lowercase())
}

fn signal_matches(
    signal: &Signal,
    root: &HttpResponse,
    body_lower: &str,
    title: Option<&str>,
    favicon: Option<&Favicon>,
) -> Option<String> {
    let hit = match *signal {
        Signal::TitleIs(t) => title.is_some_and(|x| x.trim().eq_ignore_ascii_case(t)),
        Signal::TitleContains(t) => title.is_some_and(|x| contains_ci(&x.to_ascii_lowercase(), t)),
        Signal::BodyContains(t) => contains_ci(body_lower, t),
        Signal::Header(name, value) => root
            .header(name)
            .is_some_and(|v| value.is_empty() || contains_ci(&v.to_ascii_lowercase(), value)),
        Signal::FaviconHash(h) => favicon.is_some_and(|f| f.hash == h),
    };
    hit.then(|| match *signal {
        Signal::TitleIs(t) | Signal::TitleContains(t) => format!("title {t:?}"),
        Signal::BodyContains(t) => format!("page mentions {t:?}"),
        Signal::Header(name, "") => format!("header {name}"),
        Signal::Header(name, value) => format!("header {name}: {value}"),
        Signal::FaviconHash(h) => format!("favicon hash {h}"),
    })
}

/// Whether every check of `probe` holds for `resp`.
pub fn probe_matches(probe: &Probe, resp: &HttpResponse) -> bool {
    let mut json: Option<Option<serde_json::Value>> = None;
    let mut json = || json.get_or_insert_with(|| resp.json()).clone();
    probe.checks.iter().all(|check| match *check {
        Check::Status(code) => resp.status == code,
        Check::Success => resp.is_success(),
        Check::JsonHas(ptr) => json().is_some_and(|v| v.pointer(ptr).is_some_and(|x| !x.is_null())),
        Check::JsonEquals(ptr, want) => json().is_some_and(|v| {
            v.pointer(ptr)
                .and_then(|x| x.as_str())
                .is_some_and(|s| s.eq_ignore_ascii_case(want))
        }),
        Check::BodyContains(t) => contains_ci(&resp.text().to_ascii_lowercase(), t),
        Check::HeaderPresent(name) => resp.header(name).is_some(),
        Check::BodyIsVersion => looks_like_version(&resp.text()).is_some(),
    })
}

/// Reads a version from a probe response.
pub fn read_version(source: VersionSource, resp: &HttpResponse) -> Option<String> {
    let scalar = |v: &serde_json::Value| match v {
        serde_json::Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    };
    let version = match source {
        VersionSource::Json(ptr) => resp.json().and_then(|v| v.pointer(ptr).and_then(scalar)),
        VersionSource::JsonParts(a, b, c) => resp.json().and_then(|v| {
            let parts: Option<Vec<String>> = [a, b, c]
                .iter()
                .map(|p| v.pointer(p).and_then(scalar))
                .collect();
            parts.map(|p| p.join("."))
        }),
        VersionSource::XmlAttr(name) => xml_attr(&resp.text(), name),
        VersionSource::Body => looks_like_version(&resp.text()),
        VersionSource::Header(name) => resp.header(name).map(|v| v.trim().to_string()),
    }?;
    let version = version.strip_prefix('v').unwrap_or(&version).to_string();
    (!version.is_empty() && version.len() <= 64).then_some(version)
}

/// Returns the trimmed text without a leading `v` or quotes if it looks like
/// a version number (`4.6.2`, `v1.2.3-beta`), else `None`.
fn looks_like_version(text: &str) -> Option<String> {
    let t = text.trim().trim_matches('"');
    let t = t.strip_prefix('v').unwrap_or(t);
    let ok = t.len() <= 40
        && t.starts_with(|c: char| c.is_ascii_digit())
        && t.contains('.')
        && t.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | '_'));
    ok.then(|| t.to_string())
}

/// Finds `name="value"` (or single quotes) preceded by whitespace, skipping
/// the `<?xml version="1.0"?>` declaration.
fn xml_attr(xml: &str, name: &str) -> Option<String> {
    let xml = match xml.trim_start().strip_prefix("<?") {
        Some(rest) => rest.split_once("?>").map_or(rest, |(_, after)| after),
        None => xml,
    };
    let mut pos = 0;
    while let Some(i) = xml[pos..].find(name) {
        let start = pos + i;
        let after = start + name.len();
        pos = after;
        let before_ok = xml[..start].ends_with(|c: char| c.is_whitespace());
        let rest = &xml[after..];
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        let Some(quote) = rest.chars().next().filter(|c| *c == '"' || *c == '\'') else {
            continue;
        };
        if !before_ok {
            continue;
        }
        let value = &rest[1..];
        let end = value.find(quote)?;
        return Some(value[..end].to_string());
    }
    None
}

/// Guesses an image type from its first bytes.
fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG") {
        Some("image/png")
    } else if bytes.starts_with(&[0, 0, 1, 0]) {
        Some("image/x-icon")
    } else if bytes.starts_with(b"GIF8") {
        Some("image/gif")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if String::from_utf8_lossy(&bytes[..bytes.len().min(256)])
        .to_ascii_lowercase()
        .contains("<svg")
    {
        Some("image/svg+xml")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resp(status: u16, body: &str) -> HttpResponse {
        HttpResponse {
            status,
            headers: vec![("x-version".into(), "2.13.5".into())],
            body: body.as_bytes().to_vec(),
            url: Url::parse("http://10.0.0.1:1/").unwrap(),
        }
    }

    #[test]
    fn versions() {
        let r = resp(
            200,
            r#"{"Version":"10.10.3","major":1,"minor":120,"patch":2}"#,
        );
        assert_eq!(
            read_version(VersionSource::Json("/Version"), &r).as_deref(),
            Some("10.10.3")
        );
        assert_eq!(
            read_version(VersionSource::JsonParts("/major", "/minor", "/patch"), &r).as_deref(),
            Some("1.120.2")
        );
        assert_eq!(
            read_version(VersionSource::Header("x-version"), &r).as_deref(),
            Some("2.13.5")
        );
        let xml = resp(
            200,
            r#"<MediaContainer size="0" claimed="1" machineIdentifier="abc" version="1.41.2.9200-c6bbc1b53"/>"#,
        );
        assert_eq!(
            read_version(VersionSource::XmlAttr("version"), &xml).as_deref(),
            Some("1.41.2.9200-c6bbc1b53")
        );
        assert_eq!(
            read_version(VersionSource::Body, &resp(200, "v4.6.7\n")).as_deref(),
            Some("4.6.7")
        );
        assert_eq!(
            read_version(VersionSource::Body, &resp(200, "Forbidden")),
            None
        );
    }

    #[test]
    fn xml_attr_needs_whole_name() {
        assert_eq!(
            xml_attr(r#"<a apiversion="2" version="1.0"/>"#, "version").as_deref(),
            Some("1.0")
        );
        assert_eq!(xml_attr(r#"<a apiversion="2"/>"#, "version"), None);
        assert_eq!(
            xml_attr("<?xml version=\"1.0\"?>\n<M version=\"1.41\"/>", "version").as_deref(),
            Some("1.41")
        );
    }

    #[test]
    fn sniffs_images() {
        assert_eq!(sniff_image(b"\x89PNG\r\n"), Some("image/png"));
        assert_eq!(sniff_image(&[0, 0, 1, 0, 1]), Some("image/x-icon"));
        assert_eq!(sniff_image(b"<?xml?><svg>"), Some("image/svg+xml"));
        assert_eq!(sniff_image(b"<html>"), None);
    }

    #[test]
    fn base_urls() {
        assert_eq!(
            base_url(Scheme::Https, "fd00::1".parse().unwrap(), 8443).as_str(),
            "https://[fd00::1]:8443/"
        );
    }
}
