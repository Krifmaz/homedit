//! A minimal, read-only Docker Engine API client.
//!
//! The client only ever sends `GET` requests, so it cannot start, stop or
//! change containers even when it is given a writable socket. It speaks
//! HTTP/1.1 over a Unix socket (`/var/run/docker.sock`) or plain TCP (a
//! Docker socket proxy such as `tecnativa/docker-socket-proxy`).

use std::path::PathBuf;
use std::time::Duration;

use http::{Request, StatusCode};
use http_body_util::{BodyExt, Empty, Limited};
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::DiscoveryError;
use crate::model::Container;

/// The largest response body the client accepts (8 MiB). A host with a few
/// hundred containers returns well under 1 MiB from `/containers/json`.
pub const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

/// Where the Docker API lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DockerEndpoint {
    /// A Unix socket path, such as `/var/run/docker.sock`.
    Unix(PathBuf),
    /// A TCP host and port, such as a socket proxy at `dockerproxy:2375`.
    /// TLS is not supported; put a TLS-terminating proxy in front instead.
    Tcp {
        /// Hostname or IP address.
        host: String,
        /// TCP port.
        port: u16,
    },
}

impl DockerEndpoint {
    /// The default local socket, `/var/run/docker.sock`.
    pub fn default_socket() -> Self {
        Self::Unix(PathBuf::from("/var/run/docker.sock"))
    }

    /// Builds an endpoint from a gethomepage `docker.yaml` entry. A socket
    /// wins over a host. Returns `None` when neither is set.
    pub fn from_server(server: &homedit_import::DockerServer) -> Option<Self> {
        if let Some(socket) = &server.socket {
            return Some(Self::Unix(PathBuf::from(socket)));
        }
        server.host.as_ref().map(|host| Self::Tcp {
            host: host.clone(),
            port: server.port.unwrap_or(2375),
        })
    }
}

impl std::fmt::Display for DockerEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unix(path) => write!(f, "unix://{}", path.display()),
            Self::Tcp { host, port } => write!(f, "tcp://{host}:{port}"),
        }
    }
}

/// A read-only Docker API client. Cheap to clone.
#[derive(Debug, Clone)]
pub struct DockerClient {
    endpoint: DockerEndpoint,
    timeout: Duration,
}

impl DockerClient {
    /// Creates a client for `endpoint` with a 5 second request timeout.
    pub fn new(endpoint: DockerEndpoint) -> Self {
        Self {
            endpoint,
            timeout: Duration::from_secs(5),
        }
    }

    /// Sets the timeout for a whole request (connect, send and read).
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// The endpoint this client talks to.
    pub fn endpoint(&self) -> &DockerEndpoint {
        &self.endpoint
    }

    /// Checks that the API answers (`GET /_ping`).
    pub async fn ping(&self) -> Result<(), DiscoveryError> {
        self.get_bytes("/_ping").await.map(|_| ())
    }

    /// Lists containers (`GET /containers/json`). With `all`, stopped
    /// containers are included too.
    pub async fn list_containers(&self, all: bool) -> Result<Vec<Container>, DiscoveryError> {
        let path = if all {
            "/containers/json?all=true"
        } else {
            "/containers/json"
        };
        self.get_json(path).await
    }

    /// Sends `GET path` and decodes the JSON response.
    pub async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T, DiscoveryError> {
        let body = self.get_bytes(path).await?;
        serde_json::from_slice(&body).map_err(DiscoveryError::Decode)
    }

    /// Sends `GET path` and returns the response body. `path` must start
    /// with `/` and is sent as-is (callers build it from constants, never
    /// from user input).
    pub async fn get_bytes(&self, path: &str) -> Result<Bytes, DiscoveryError> {
        if !path.starts_with('/') {
            return Err(DiscoveryError::InvalidPath(path.to_string()));
        }
        tokio::time::timeout(self.timeout, self.request(path))
            .await
            .map_err(|_| DiscoveryError::Timeout(self.timeout))?
    }

    async fn request(&self, path: &str) -> Result<Bytes, DiscoveryError> {
        match &self.endpoint {
            DockerEndpoint::Unix(socket) => {
                let stream = tokio::net::UnixStream::connect(socket)
                    .await
                    .map_err(|e| DiscoveryError::Connect(self.endpoint.to_string(), e))?;
                send_get(stream, "localhost", path).await
            }
            DockerEndpoint::Tcp { host, port } => {
                let stream = tokio::net::TcpStream::connect((host.as_str(), *port))
                    .await
                    .map_err(|e| DiscoveryError::Connect(self.endpoint.to_string(), e))?;
                send_get(stream, host, path).await
            }
        }
    }
}

/// Sends one HTTP/1.1 `GET` over `stream` and reads the body, up to
/// [`MAX_BODY_BYTES`].
async fn send_get<S>(stream: S, host: &str, path: &str) -> Result<Bytes, DiscoveryError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .map_err(DiscoveryError::Http)?;
    tokio::spawn(async move {
        if let Err(err) = conn.await {
            tracing::debug!(error = %err, "docker connection closed with error");
        }
    });
    let request = Request::get(path)
        .header(http::header::HOST, host)
        .header(http::header::ACCEPT, "application/json")
        .body(Empty::<Bytes>::new())
        .map_err(|e| DiscoveryError::InvalidPath(e.to_string()))?;
    let response = sender
        .send_request(request)
        .await
        .map_err(DiscoveryError::Http)?;
    let status = response.status();
    let body = Limited::new(response.into_body(), MAX_BODY_BYTES)
        .collect()
        .await
        .map_err(|e| DiscoveryError::Body(e.to_string()))?
        .to_bytes();
    if status != StatusCode::OK {
        let message = String::from_utf8_lossy(&body).chars().take(200).collect();
        return Err(DiscoveryError::Status(status.as_u16(), message));
    }
    Ok(body)
}
