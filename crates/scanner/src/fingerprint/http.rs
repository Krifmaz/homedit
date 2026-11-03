//! A small HTTP client tuned for fingerprinting.

use std::borrow::Cow;
use std::time::Duration;

use reqwest::redirect;
use url::Url;

/// Most body bytes read from any response. Pages are cut here; titles and
/// API answers are always near the top.
pub const MAX_BODY: usize = 256 * 1024;

/// Most redirects followed, and only within the same host.
const MAX_REDIRECTS: usize = 3;

/// A response, reduced to what the matcher needs.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    /// The status code.
    pub status: u16,
    /// Headers with lowercase names, in the order received.
    pub headers: Vec<(String, String)>,
    /// The body, cut at [`MAX_BODY`] bytes.
    pub body: Vec<u8>,
    /// The URL that answered, after same-host redirects.
    pub url: Url,
}

impl HttpResponse {
    /// The first header with this (lowercase) name.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// The body as text, replacing invalid UTF-8.
    pub fn text(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.body)
    }

    /// The body parsed as JSON, if it is JSON.
    pub fn json(&self) -> Option<serde_json::Value> {
        serde_json::from_slice(&self.body).ok()
    }

    /// Whether the status is 2xx.
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// The HTTP client used to look at open ports.
///
/// * Never uses a proxy from the environment: scans must reach the LAN
///   directly, and must not leak internal addresses to a proxy.
/// * Accepts self-signed and expired certificates, which are normal in a
///   homelab. Nothing secret is ever sent, so this is safe here.
/// * Follows at most three redirects, and only to the same host, so a
///   service cannot bounce the scanner to an outside address.
/// * Sends no cookies or credentials and keeps no idle connections.
#[derive(Debug, Clone)]
pub struct HttpProber {
    client: reqwest::Client,
}

impl HttpProber {
    /// Builds a prober with the given per-request timeout.
    pub fn new(timeout: Duration) -> Result<Self, reqwest::Error> {
        let policy = redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                return attempt.stop();
            }
            let same_host = attempt
                .previous()
                .first()
                .is_some_and(|first| first.host() == attempt.url().host());
            if same_host {
                attempt.follow()
            } else {
                attempt.stop()
            }
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(timeout)
            .connect_timeout(timeout)
            .redirect(policy)
            .tls_danger_accept_invalid_certs(true)
            .pool_max_idle_per_host(0)
            .user_agent(concat!("homedit-scanner/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self { client })
    }

    /// `GET`s `url` and reads at most [`MAX_BODY`] bytes of the body.
    pub async fn get(&self, url: Url) -> Result<HttpResponse, reqwest::Error> {
        let mut resp = self.client.get(url).send().await?;
        let status = resp.status().as_u16();
        let url = resp.url().clone();
        let headers = resp
            .headers()
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str().to_ascii_lowercase(),
                    String::from_utf8_lossy(v.as_bytes()).into_owned(),
                )
            })
            .collect();
        let mut body = Vec::new();
        while let Some(chunk) = resp.chunk().await? {
            let room = MAX_BODY - body.len();
            if chunk.len() >= room {
                body.extend_from_slice(&chunk[..room]);
                break;
            }
            body.extend_from_slice(&chunk);
        }
        Ok(HttpResponse {
            status,
            headers,
            body,
            url,
        })
    }
}
