//! Scan results and the "Add to dashboard" suggestion built from them.

use std::collections::HashSet;
use std::net::IpAddr;

use serde::{Deserialize, Serialize};
use url::Url;

use crate::fingerprint::{AppMatch, HttpInfo, Scheme};
use crate::policy::canonical;
use crate::ports::PortSet;
use crate::target::Target;

/// How a service was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// A TCP connect scan found the port open.
    Scan,
    /// The Docker API lists the port as published.
    Docker,
}

/// The container behind a service found through Docker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerRef {
    /// Short (12 character) container id.
    pub id: String,
    /// Container name without the leading `/`.
    pub name: String,
    /// Image reference, e.g. `lscr.io/linuxserver/sonarr:latest`.
    pub image: String,
}

/// One open port on one host and what is behind it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredService {
    /// The TCP port.
    pub port: u16,
    /// How it was found.
    pub source: Source,
    /// What the web service shows, when the port speaks HTTP(S).
    pub http: Option<HttpInfo>,
    /// The recognised app, if any.
    pub app: Option<AppMatch>,
    /// The container, when found through Docker.
    pub container: Option<ContainerRef>,
    /// The app usually found on this port, when nothing better identified
    /// it (for example "Sonarr" on 8989). Only a hint: [`app`](Self::app)
    /// stays empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guess: Option<String>,
    /// Whether a dashboard card already points here. Filled in by
    /// [`ScanReport::mark_on_dashboard`]; always `false` straight from a scan.
    #[serde(default)]
    pub on_dashboard: bool,
}

impl DiscoveredService {
    /// The URL to open this service at, using `host` (an IP address or
    /// hostname) and the scheme that answered (HTTP when unknown).
    pub fn url(&self, host: &str) -> String {
        let scheme = self.http.as_ref().map_or(Scheme::Http, |h| h.scheme);
        let host = if host.contains(':') && !host.starts_with('[') {
            format!("[{host}]")
        } else {
            host.to_string()
        };
        let default = matches!(
            (scheme, self.port),
            (Scheme::Http, 80) | (Scheme::Https, 443)
        );
        if default {
            format!("{}://{host}/", scheme.as_str())
        } else {
            format!("{}://{host}:{}/", scheme.as_str(), self.port)
        }
    }

    /// A card name for this service: the app name, else the page title, else
    /// the `Server` header, else the container name, else the app usually on
    /// this port, else `Service on port N`.
    pub fn display_name(&self) -> String {
        if let Some(app) = &self.app {
            return app.name.clone();
        }
        let http = self.http.as_ref();
        http.and_then(|h| h.title.clone())
            .or_else(|| http.and_then(|h| h.server.clone()))
            .or_else(|| self.container.as_ref().map(|c| c.name.clone()))
            .or_else(|| self.guess.clone())
            .unwrap_or_else(|| format!("Service on port {}", self.port))
    }

    /// Builds the form values for "Add to dashboard".
    ///
    /// `host` should be the host's name when it has one, else its address.
    pub fn suggestion(&self, host: &str) -> ServiceSuggestion {
        let href = self.url(host);
        let app = self.app.as_ref();
        ServiceSuggestion {
            name: self.display_name(),
            icon: app.map(|a| format!("{}.png", a.icon)),
            widget: app
                .and_then(|a| a.widget.as_ref())
                .map(|w| serde_json::json!({ "type": w, "url": href.trim_end_matches('/') })),
            description: app
                .and_then(|a| a.version.as_ref())
                .map(|v| format!("v{v}")),
            app_id: app.map(|a| a.id.clone()),
            favicon: self
                .http
                .as_ref()
                .and_then(|h| h.favicon.as_ref())
                .and_then(|f| {
                    f.data
                        .as_ref()
                        .map(|d| format!("data:{};base64,{d}", f.mime))
                }),
            href,
        }
    }
}

/// Pre-filled values for a new dashboard card.
///
/// `widget` is gethomepage-shaped JSON (`{"type":"sonarr","url":"..."}`), so
/// it can go straight into a service's widget field; the form then only needs
/// the API key or password.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceSuggestion {
    /// Card name.
    pub name: String,
    /// Card link.
    pub href: String,
    /// Icon in gethomepage notation (`sonarr.png`), when the app is known.
    pub icon: Option<String>,
    /// Short description, currently the version when known.
    pub description: Option<String>,
    /// Widget config without secrets, when the app has a widget.
    pub widget: Option<serde_json::Value>,
    /// The catalogue id of the recognised app.
    pub app_id: Option<String>,
    /// The service's own favicon as a `data:` URL, for unknown apps that the
    /// admin may want to save as a custom icon.
    pub favicon: Option<String>,
}

/// Everything found on one address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostResult {
    /// The address.
    pub ip: IpAddr,
    /// The hostname it was reached by, if it came from DNS.
    pub hostname: Option<String>,
    /// A label for the host: its container name on a Docker network, else
    /// its reverse DNS name. Links keep using the address, since such names
    /// often don't resolve in the browser.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Open ports, sorted by port number.
    pub services: Vec<DiscoveredService>,
}

impl HostResult {
    /// The hostname if known, else the address as text.
    pub fn host_label(&self) -> String {
        self.hostname.clone().unwrap_or_else(|| self.ip.to_string())
    }
}

/// The result of one scan, ready to store and to show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanReport {
    /// Who asked for the scan, as passed in the request (for the scan log).
    pub requested_by: Option<String>,
    /// The targets as requested.
    pub targets: Vec<Target>,
    /// The ports as requested.
    pub ports: PortSet,
    /// Unix time in milliseconds when the scan started.
    pub started_at_ms: u64,
    /// Unix time in milliseconds when the scan finished.
    pub finished_at_ms: u64,
    /// Every address that was checked, open ports or not. Used by
    /// [`diff()`](crate::diff()) to tell "gone" from "not scanned this time".
    pub scanned_addresses: Vec<IpAddr>,
    /// Number of TCP connect attempts made.
    pub probes_sent: u64,
    /// Hosts with at least one open port, sorted by address.
    pub hosts: Vec<HostResult>,
}

impl ScanReport {
    /// Every `(host, service)` pair in the report.
    pub fn services(&self) -> impl Iterator<Item = (&HostResult, &DiscoveredService)> {
        self.hosts
            .iter()
            .flat_map(|h| h.services.iter().map(move |s| (h, s)))
    }

    /// Sets [`DiscoveredService::on_dashboard`] for every service that one of
    /// `existing` (the `href`s and widget URLs already on the dashboard)
    /// points to. URLs are compared by host and port, so a card for
    /// `http://10.0.1.7:8096/web` marks `10.0.1.7:8096`. Hostname matches
    /// count when the scan reached the host by that name.
    pub fn mark_on_dashboard<'a>(&mut self, existing: impl IntoIterator<Item = &'a str>) {
        let known: HashSet<(String, u16)> = existing
            .into_iter()
            .filter_map(|s| Url::parse(s).ok())
            .filter_map(|u| {
                let port = u.port_or_known_default()?;
                let host = match u.host()? {
                    url::Host::Ipv4(ip) => ip.to_string(),
                    url::Host::Ipv6(ip) => canonical(IpAddr::V6(ip)).to_string(),
                    url::Host::Domain(d) => d.to_ascii_lowercase(),
                };
                Some((host, port))
            })
            .collect();
        for host in &mut self.hosts {
            let ip = host.ip.to_string();
            for svc in &mut host.services {
                svc.on_dashboard = known.contains(&(ip.clone(), svc.port))
                    || host
                        .hostname
                        .as_ref()
                        .is_some_and(|n| known.contains(&(n.clone(), svc.port)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fingerprint::Favicon;

    fn svc(port: u16, scheme: Scheme) -> DiscoveredService {
        DiscoveredService {
            port,
            source: Source::Scan,
            http: Some(HttpInfo {
                scheme,
                status: 200,
                title: Some("My App".into()),
                server: None,
                redirect: None,
                favicon: Some(Favicon {
                    url: "http://x/favicon.ico".into(),
                    hash: 1,
                    mime: "image/png".into(),
                    data: Some("AAAA".into()),
                }),
            }),
            app: None,
            container: None,
            guess: None,
            on_dashboard: false,
        }
    }

    #[test]
    fn urls() {
        assert_eq!(
            svc(8096, Scheme::Http).url("10.0.1.7"),
            "http://10.0.1.7:8096/"
        );
        assert_eq!(svc(443, Scheme::Https).url("nas.lan"), "https://nas.lan/");
        assert_eq!(
            svc(8443, Scheme::Https).url("fd00::1"),
            "https://[fd00::1]:8443/"
        );
    }

    #[test]
    fn suggestions() {
        let unknown = svc(4000, Scheme::Http).suggestion("10.0.0.2");
        assert_eq!(unknown.name, "My App");
        assert_eq!(unknown.icon, None);
        assert_eq!(unknown.widget, None);
        assert_eq!(
            unknown.favicon.as_deref(),
            Some("data:image/png;base64,AAAA")
        );

        let mut known = svc(8989, Scheme::Http);
        known.app = Some(AppMatch {
            id: "sonarr".into(),
            name: "Sonarr".into(),
            widget: Some("sonarr".into()),
            icon: "sonarr".into(),
            version: Some("4.0.10".into()),
            confidence: 90,
            evidence: vec![],
        });
        let s = known.suggestion("10.0.1.7");
        assert_eq!(s.name, "Sonarr");
        assert_eq!(s.icon.as_deref(), Some("sonarr.png"));
        assert_eq!(s.description.as_deref(), Some("v4.0.10"));
        assert_eq!(
            s.widget,
            Some(serde_json::json!({"type": "sonarr", "url": "http://10.0.1.7:8989"}))
        );
    }

    #[test]
    fn marks_existing_cards() {
        let mut report = ScanReport {
            requested_by: None,
            targets: vec![],
            ports: PortSet::Common,
            started_at_ms: 0,
            finished_at_ms: 0,
            scanned_addresses: vec![],
            probes_sent: 0,
            hosts: vec![HostResult {
                ip: "10.0.1.7".parse().unwrap(),
                hostname: Some("media.lan".into()),
                name: None,
                services: vec![
                    svc(8096, Scheme::Http),
                    svc(443, Scheme::Https),
                    svc(9000, Scheme::Http),
                ],
            }],
        };
        report.mark_on_dashboard([
            "http://10.0.1.7:8096/web/index.html",
            "https://Media.lan",
            "not a url",
        ]);
        let marks: Vec<bool> = report.hosts[0]
            .services
            .iter()
            .map(|s| s.on_dashboard)
            .collect();
        assert_eq!(marks, vec![true, true, false]);
    }
}
