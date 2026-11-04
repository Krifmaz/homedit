//! Outbound HTTP for widgets, with timeouts and a server-side request
//! forgery guard.
//!
//! Widgets never build their own HTTP clients. They go through
//! [`WidgetContext`], which:
//!
//! - applies connect and total timeouts,
//! - refuses to follow redirects (so a service cannot bounce a request,
//!   with its API key, to another host),
//! - only allows requests to the host and port of the widget's configured
//!   `url`, plus hosts the admin explicitly allowed,
//! - caps response bodies so a misbehaving service cannot exhaust memory,
//! - keeps credentials out of error messages.

use std::sync::Arc;
use std::time::Duration;

use reqwest::{Method, RequestBuilder, Response, redirect};
use serde::de::DeserializeOwned;
use url::Url;

use crate::config::WidgetConfig;
use crate::error::{WidgetError, status_error};

/// Default total timeout of one widget request.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Default connect timeout.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Largest response body a widget will read (8 MiB).
pub const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

/// Name of the boolean config field that disables TLS certificate
/// verification for one widget (for services with self-signed
/// certificates, common for Proxmox).
pub const INSECURE_TLS_FIELD: &str = "insecure_tls";

/// Options for building a [`WidgetContext`].
#[derive(Debug, Clone)]
pub struct ContextOptions {
    /// Total timeout per request.
    pub timeout: Duration,
    /// Connect timeout per request.
    pub connect_timeout: Duration,
    /// Hosts (as `host` or `host:port`) every widget may reach in addition
    /// to the host of its own `url`.
    pub extra_allowed_hosts: Vec<String>,
    /// User agent sent with every request.
    pub user_agent: String,
}

impl Default for ContextOptions {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            extra_allowed_hosts: Vec::new(),
            user_agent: format!("homedit/{}", crate::VERSION),
        }
    }
}

#[derive(Debug)]
struct Inner {
    strict: reqwest::Client,
    insecure: reqwest::Client,
    extra_allowed_hosts: Vec<String>,
}

/// Shared, cheaply clonable HTTP context handed to every widget call.
#[derive(Debug, Clone)]
pub struct WidgetContext {
    inner: Arc<Inner>,
}

impl WidgetContext {
    /// Builds a context with the given options.
    pub fn new(opts: ContextOptions) -> Result<Self, WidgetError> {
        let build = |insecure: bool| {
            reqwest::Client::builder()
                .timeout(opts.timeout)
                .connect_timeout(opts.connect_timeout)
                .redirect(redirect::Policy::none())
                .user_agent(opts.user_agent.clone())
                .danger_accept_invalid_certs(insecure)
                .build()
                .map_err(|e| WidgetError::Http(e.to_string()))
        };
        Ok(Self {
            inner: Arc::new(Inner {
                strict: build(false)?,
                insecure: build(true)?,
                extra_allowed_hosts: opts
                    .extra_allowed_hosts
                    .iter()
                    .map(|h| h.to_ascii_lowercase())
                    .collect(),
            }),
        })
    }

    /// Builds a context with default options.
    pub fn with_defaults() -> Result<Self, WidgetError> {
        Self::new(ContextOptions::default())
    }

    /// Checks that `url` may be requested for a widget configured with `cfg`.
    ///
    /// Allowed are the exact host and port of the configured `url`, and the
    /// extra hosts from [`ContextOptions::extra_allowed_hosts`].
    pub fn check_allowed(&self, cfg: &WidgetConfig, url: &Url) -> Result<(), WidgetError> {
        let base = cfg.base_url()?;
        let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
        if !matches!(url.scheme(), "http" | "https") {
            return Err(WidgetError::HostNotAllowed(host));
        }
        let same_origin = base.scheme() == url.scheme()
            && base.host_str().map(str::to_ascii_lowercase).as_deref() == Some(host.as_str())
            && base.port_or_known_default() == url.port_or_known_default();
        if same_origin {
            return Ok(());
        }
        let with_port = url
            .port_or_known_default()
            .map(|p| format!("{host}:{p}"))
            .unwrap_or_else(|| host.clone());
        if self
            .inner
            .extra_allowed_hosts
            .iter()
            .any(|h| *h == host || *h == with_port)
        {
            return Ok(());
        }
        Err(WidgetError::HostNotAllowed(host))
    }

    /// Starts a request to `path_and_query` under the widget's base URL.
    pub fn request(
        &self,
        cfg: &WidgetConfig,
        method: Method,
        path_and_query: &str,
    ) -> Result<RequestBuilder, WidgetError> {
        let url = cfg.endpoint(path_and_query)?;
        self.request_url(cfg, method, url)
    }

    /// Starts a request to an absolute URL, after the host check.
    pub fn request_url(
        &self,
        cfg: &WidgetConfig,
        method: Method,
        url: Url,
    ) -> Result<RequestBuilder, WidgetError> {
        self.check_allowed(cfg, &url)?;
        let client = if cfg.bool(INSECURE_TLS_FIELD).unwrap_or(false) {
            &self.inner.insecure
        } else {
            &self.inner.strict
        };
        Ok(client.request(method, url))
    }

    /// Sends a request and turns non-success statuses into errors.
    pub async fn send(&self, req: RequestBuilder) -> Result<Response, WidgetError> {
        let resp = req.send().await?;
        let status = resp.status();
        if status.is_success() {
            Ok(resp)
        } else {
            Err(status_error(status.as_u16()))
        }
    }

    /// Sends a request and reads the body, up to [`MAX_BODY_BYTES`].
    pub async fn send_bytes(&self, req: RequestBuilder) -> Result<Vec<u8>, WidgetError> {
        let mut resp = self.send(req).await?;
        if resp
            .content_length()
            .is_some_and(|len| len > MAX_BODY_BYTES as u64)
        {
            return Err(WidgetError::Parse("response too large".into()));
        }
        let mut body = Vec::new();
        while let Some(chunk) = resp.chunk().await? {
            if body.len() + chunk.len() > MAX_BODY_BYTES {
                return Err(WidgetError::Parse("response too large".into()));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }

    /// Sends a request and decodes a JSON body.
    pub async fn send_json<T: DeserializeOwned>(
        &self,
        req: RequestBuilder,
    ) -> Result<T, WidgetError> {
        let body = self
            .send_bytes(req.header("Accept", "application/json"))
            .await?;
        Ok(serde_json::from_slice(&body)?)
    }

    /// `GET`s a path under the base URL with extra headers and decodes JSON.
    pub async fn get_json<T: DeserializeOwned>(
        &self,
        cfg: &WidgetConfig,
        path_and_query: &str,
        headers: &[(&str, &str)],
    ) -> Result<T, WidgetError> {
        let mut req = self.request(cfg, Method::GET, path_and_query)?;
        for (name, value) in headers {
            req = req.header(*name, *value);
        }
        self.send_json(req).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(extra: &[&str]) -> WidgetContext {
        WidgetContext::new(ContextOptions {
            extra_allowed_hosts: extra.iter().map(|s| (*s).into()).collect(),
            ..Default::default()
        })
        .unwrap()
    }

    #[test]
    fn allows_only_configured_origin() {
        let cfg = WidgetConfig::default().with("url", "http://sonarr.lan:8989/base");
        let c = ctx(&[]);
        let ok = Url::parse("http://SONARR.lan:8989/api/v3/queue").unwrap();
        assert!(c.check_allowed(&cfg, &ok).is_ok());
        for bad in [
            "http://sonarr.lan:8080/",
            "http://169.254.169.254/latest/meta-data",
            "https://sonarr.lan:8989/",
            "file:///etc/passwd",
        ] {
            let url = Url::parse(bad).unwrap();
            assert!(
                c.check_allowed(&cfg, &url).is_err(),
                "{bad} should be denied"
            );
        }
    }

    #[test]
    fn extra_hosts_are_allowed() {
        let cfg = WidgetConfig::default().with("url", "http://a.lan");
        let c = ctx(&["b.lan", "c.lan:9000"]);
        assert!(
            c.check_allowed(&cfg, &Url::parse("http://b.lan:1234/").unwrap())
                .is_ok()
        );
        assert!(
            c.check_allowed(&cfg, &Url::parse("http://c.lan:9000/").unwrap())
                .is_ok()
        );
        assert!(
            c.check_allowed(&cfg, &Url::parse("http://c.lan:9001/").unwrap())
                .is_err()
        );
    }
}
