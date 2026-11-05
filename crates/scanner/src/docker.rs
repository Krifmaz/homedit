//! Published ports from the Docker API, so apps on a known Docker host show
//! up instantly without scanning.
//!
//! Only `GET /containers/json` is called, so a read-only socket mount (or a
//! socket proxy that only allows `CONTAINERS=1`) is enough. The request is
//! written by hand over the Unix socket or TCP connection; no `docker`
//! binary or shell is involved.

use std::collections::{BTreeMap, HashSet};
use std::net::IpAddr;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::catalogue::Catalogue;
use crate::error::{Result, ScanError};
use crate::fingerprint::AppMatch;
use crate::report::{ContainerRef, DiscoveredService, HostResult, Source};

/// Largest Docker API response read.
const MAX_RESPONSE: usize = 16 * 1024 * 1024;

/// How long the whole Docker request may take.
const DOCKER_TIMEOUT: Duration = Duration::from_secs(10);

/// Confidence given to a match made from the image name alone.
const IMAGE_CONFIDENCE: u32 = 60;

/// Where the Docker API lives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum DockerEndpoint {
    /// A Unix socket, e.g. `/var/run/docker.sock`.
    Unix(PathBuf),
    /// Plain TCP, e.g. a socket proxy at `socket-proxy:2375`. TLS
    /// endpoints (port 2376 with client certificates) are not supported.
    Tcp {
        /// Hostname or address.
        host: String,
        /// Port.
        port: u16,
    },
}

impl FromStr for DockerEndpoint {
    type Err = ScanError;

    /// Accepts `unix:///path`, a bare absolute path, `tcp://host:port` or
    /// `http://host:port`.
    fn from_str(s: &str) -> Result<Self> {
        let s = s.trim();
        if let Some(path) = s.strip_prefix("unix://") {
            return Ok(DockerEndpoint::Unix(PathBuf::from(path)));
        }
        if s.starts_with('/') {
            return Ok(DockerEndpoint::Unix(PathBuf::from(s)));
        }
        let rest = s
            .strip_prefix("tcp://")
            .or_else(|| s.strip_prefix("http://"))
            .ok_or_else(|| ScanError::Docker(format!("unsupported endpoint {s:?}")))?;
        let rest = rest.trim_end_matches('/');
        let (host, port) = rest
            .rsplit_once(':')
            .ok_or_else(|| ScanError::Docker(format!("endpoint {s:?} needs a port")))?;
        let port: u16 = port
            .parse()
            .map_err(|_| ScanError::Docker(format!("bad port in {s:?}")))?;
        let host = host.trim_start_matches('[').trim_end_matches(']');
        if host.is_empty() {
            return Err(ScanError::Docker(format!("endpoint {s:?} needs a host")));
        }
        Ok(DockerEndpoint::Tcp {
            host: host.to_string(),
            port,
        })
    }
}

impl TryFrom<String> for DockerEndpoint {
    type Error = ScanError;
    fn try_from(value: String) -> Result<Self> {
        value.parse()
    }
}

impl From<DockerEndpoint> for String {
    fn from(value: DockerEndpoint) -> Self {
        match value {
            DockerEndpoint::Unix(path) => format!("unix://{}", path.display()),
            DockerEndpoint::Tcp { host, port } if host.contains(':') => {
                format!("tcp://[{host}]:{port}")
            }
            DockerEndpoint::Tcp { host, port } => format!("tcp://{host}:{port}"),
        }
    }
}

/// One port a running container publishes on its host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedPort {
    /// The container.
    pub container: ContainerRef,
    /// Container state, e.g. `running`.
    pub state: String,
    /// The host address the port is bound to; `None` for all addresses
    /// (`0.0.0.0` or `::`).
    pub host_ip: Option<IpAddr>,
    /// The port on the host.
    pub host_port: u16,
    /// The port inside the container.
    pub container_port: u16,
    /// The app the image ships, if the catalogue knows it.
    pub app: Option<AppMatch>,
    /// The container's `homepage.*` labels, for gethomepage-style
    /// auto-discovery.
    pub labels: BTreeMap<String, String>,
}

impl PublishedPort {
    /// Turns this port into a [`DiscoveredService`] (source
    /// [`Source::Docker`]). There is no HTTP info until it is fingerprinted.
    pub fn to_service(&self) -> DiscoveredService {
        DiscoveredService {
            port: self.host_port,
            source: Source::Docker,
            http: None,
            app: self.app.clone(),
            container: Some(self.container.clone()),
            guess: None,
            on_dashboard: false,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ApiContainer {
    id: String,
    #[serde(default)]
    names: Vec<String>,
    #[serde(default)]
    image: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    labels: Option<BTreeMap<String, String>>,
    #[serde(default)]
    ports: Vec<ApiPort>,
    #[serde(default)]
    network_settings: Option<ApiNetworkSettings>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ApiNetworkSettings {
    #[serde(default)]
    networks: Option<BTreeMap<String, ApiEndpoint>>,
}

#[derive(Deserialize)]
struct ApiEndpoint {
    #[serde(rename = "IPAddress", default)]
    ip_address: String,
}

/// A running container's address on one of its Docker networks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerAddress {
    /// The container.
    pub container: ContainerRef,
    /// Its address on the network.
    pub ip: IpAddr,
    /// The Docker network's name.
    pub network: String,
    /// The app the image ships, if the catalogue knows it.
    pub app: Option<AppMatch>,
}

/// Everything a scan uses from one Docker host, read in one request.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockerInventory {
    /// Ports published on the host, as [`published_ports`] lists them.
    pub published: Vec<PublishedPort>,
    /// Container addresses on Docker networks, so hosts found there are
    /// named by their container.
    pub addresses: Vec<ContainerAddress>,
}

#[derive(Deserialize)]
struct ApiPort {
    #[serde(rename = "IP")]
    ip: Option<String>,
    #[serde(rename = "PrivatePort")]
    private_port: u16,
    #[serde(rename = "PublicPort")]
    public_port: Option<u16>,
    #[serde(rename = "Type")]
    kind: String,
}

/// Lists the TCP ports published by running containers.
///
/// Ports bound to both `0.0.0.0` and `::` are reported once. Containers
/// without published ports are left out (they are not reachable on the
/// host). Results are sorted by host port.
pub async fn published_ports(
    endpoint: &DockerEndpoint,
    catalogue: &Catalogue,
) -> Result<Vec<PublishedPort>> {
    Ok(inventory(endpoint, catalogue).await?.published)
}

/// Lists the published ports and the network addresses of running
/// containers, with one `GET /containers/json`.
pub async fn inventory(
    endpoint: &DockerEndpoint,
    catalogue: &Catalogue,
) -> Result<DockerInventory> {
    let body = tokio::time::timeout(DOCKER_TIMEOUT, get(endpoint, "/containers/json"))
        .await
        .map_err(|_| ScanError::Docker("timed out".into()))??;
    let containers: Vec<ApiContainer> =
        serde_json::from_slice(&body).map_err(|e| ScanError::Docker(format!("bad JSON: {e}")))?;
    Ok(parse_containers(containers, catalogue))
}

fn parse_containers(containers: Vec<ApiContainer>, catalogue: &Catalogue) -> DockerInventory {
    let mut out = Vec::new();
    let mut addresses = Vec::new();
    for c in containers {
        let container = ContainerRef {
            id: c.id.chars().take(12).collect(),
            name: c
                .names
                .first()
                .map(|n| n.trim_start_matches('/').to_string())
                .unwrap_or_default(),
            image: c.image.clone(),
        };
        let app = catalogue.by_image(&c.image).map(|def| {
            AppMatch::from_def(
                def,
                IMAGE_CONFIDENCE,
                None,
                vec![format!("docker image {}", c.image)],
            )
        });
        let labels: BTreeMap<String, String> = c
            .labels
            .unwrap_or_default()
            .into_iter()
            .filter(|(k, _)| k.starts_with("homepage."))
            .collect();
        let networks = c
            .network_settings
            .and_then(|n| n.networks)
            .unwrap_or_default();
        for (network, ep) in networks {
            if let Ok(ip) = ep.ip_address.parse::<IpAddr>() {
                addresses.push(ContainerAddress {
                    container: container.clone(),
                    ip,
                    network,
                    app: app.clone(),
                });
            }
        }
        let mut seen = HashSet::new();
        for p in &c.ports {
            let Some(public) = p.public_port else {
                continue;
            };
            if p.kind != "tcp" {
                continue;
            }
            let host_ip =
                p.ip.as_deref()
                    .and_then(|s| s.parse::<IpAddr>().ok())
                    .filter(|ip| !ip.is_unspecified());
            if !seen.insert((host_ip, public)) {
                continue;
            }
            out.push(PublishedPort {
                container: container.clone(),
                state: c.state.clone(),
                host_ip,
                host_port: public,
                container_port: p.private_port,
                app: app.clone(),
                labels: labels.clone(),
            });
        }
    }
    out.sort_by(|a, b| {
        a.host_port
            .cmp(&b.host_port)
            .then(a.host_ip.cmp(&b.host_ip))
    });
    addresses.sort_by_key(|a| a.ip);
    DockerInventory {
        published: out,
        addresses,
    }
}

/// Groups published ports into a [`HostResult`] for the Docker host at
/// `host_ip`. Ports bound to a specific other address are left out.
pub fn host_result(
    host_ip: IpAddr,
    hostname: Option<String>,
    ports: &[PublishedPort],
) -> HostResult {
    let mut services: Vec<DiscoveredService> = ports
        .iter()
        .filter(|p| p.host_ip.is_none_or(|ip| ip == host_ip))
        .map(PublishedPort::to_service)
        .collect();
    services.sort_by_key(|s| s.port);
    services.dedup_by_key(|s| s.port);
    HostResult {
        ip: host_ip,
        hostname,
        name: None,
        services,
    }
}

async fn get(endpoint: &DockerEndpoint, path: &str) -> Result<Vec<u8>> {
    match endpoint {
        DockerEndpoint::Unix(socket) => {
            #[cfg(unix)]
            {
                let stream = tokio::net::UnixStream::connect(socket)
                    .await
                    .map_err(|e| ScanError::Docker(format!("connect {}: {e}", socket.display())))?;
                request(stream, path).await
            }
            #[cfg(not(unix))]
            {
                let _ = socket;
                Err(ScanError::Docker(
                    "unix sockets are not supported on this platform".into(),
                ))
            }
        }
        DockerEndpoint::Tcp { host, port } => {
            let stream = tokio::net::TcpStream::connect((host.as_str(), *port))
                .await
                .map_err(|e| ScanError::Docker(format!("connect {host}:{port}: {e}")))?;
            request(stream, path).await
        }
    }
}

async fn request<S: AsyncRead + AsyncWrite + Unpin>(mut stream: S, path: &str) -> Result<Vec<u8>> {
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: docker\r\nUser-Agent: homedit-scanner\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(req.as_bytes()).await?;
    let mut raw = Vec::new();
    let mut buf = [0u8; 16 * 1024];
    loop {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        raw.extend_from_slice(&buf[..n]);
        if raw.len() > MAX_RESPONSE {
            return Err(ScanError::Docker("response too large".into()));
        }
    }
    parse_response(&raw)
}

/// Parses an HTTP/1.x response and returns the body of a 2xx answer,
/// decoding `Transfer-Encoding: chunked`.
fn parse_response(raw: &[u8]) -> Result<Vec<u8>> {
    let bad = |m: &str| ScanError::Docker(format!("bad HTTP response: {m}"));
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| bad("no header end"))?;
    let head = String::from_utf8_lossy(&raw[..split]);
    let body = &raw[split + 4..];
    let mut lines = head.split("\r\n");
    let status: u16 = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| bad("no status"))?;
    let mut chunked = false;
    let mut length: Option<usize> = None;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
            if k == "transfer-encoding" && v.to_ascii_lowercase().contains("chunked") {
                chunked = true;
            } else if k == "content-length" {
                length = v.parse().ok();
            }
        }
    }
    let body = if chunked {
        decode_chunked(body).ok_or_else(|| bad("broken chunked body"))?
    } else if let Some(len) = length {
        body.get(..len).ok_or_else(|| bad("short body"))?.to_vec()
    } else {
        body.to_vec()
    };
    if !(200..300).contains(&status) {
        let msg = String::from_utf8_lossy(&body[..body.len().min(200)]).into_owned();
        return Err(ScanError::Docker(format!("API returned {status}: {msg}")));
    }
    Ok(body)
}

fn decode_chunked(mut data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let line_end = data.windows(2).position(|w| w == b"\r\n")?;
        let size_line = std::str::from_utf8(&data[..line_end]).ok()?;
        let size_hex = size_line.split(';').next()?.trim();
        let size = usize::from_str_radix(size_hex, 16).ok()?;
        data = &data[line_end + 2..];
        if size == 0 {
            return Some(out);
        }
        out.extend_from_slice(data.get(..size)?);
        data = data.get(size + 2..)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_endpoints() {
        assert_eq!(
            "unix:///var/run/docker.sock"
                .parse::<DockerEndpoint>()
                .unwrap(),
            DockerEndpoint::Unix("/var/run/docker.sock".into())
        );
        assert_eq!(
            "/run/docker.sock".parse::<DockerEndpoint>().unwrap(),
            DockerEndpoint::Unix("/run/docker.sock".into())
        );
        assert_eq!(
            "tcp://socket-proxy:2375".parse::<DockerEndpoint>().unwrap(),
            DockerEndpoint::Tcp {
                host: "socket-proxy".into(),
                port: 2375
            }
        );
        assert!("ssh://host".parse::<DockerEndpoint>().is_err());
        assert!("tcp://host".parse::<DockerEndpoint>().is_err());
        let s: String = DockerEndpoint::Tcp {
            host: "fd00::2".into(),
            port: 2375,
        }
        .into();
        assert_eq!(s, "tcp://[fd00::2]:2375");
    }

    #[test]
    fn http_parsing() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\n[{}]\r\n0\r\n\r\n";
        assert_eq!(parse_response(raw).unwrap(), b"[{}]");
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n[]extra";
        assert_eq!(parse_response(raw).unwrap(), b"[]");
        let raw = b"HTTP/1.1 403 Forbidden\r\nContent-Length: 6\r\n\r\ndenied";
        assert!(parse_response(raw).is_err());
        assert!(parse_response(b"garbage").is_err());
    }
}
