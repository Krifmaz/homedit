//! A tiny mock network: fake app servers on loopback addresses.

#![allow(dead_code)]

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// One canned HTTP answer.
#[derive(Clone)]
pub struct Route {
    pub path: &'static str,
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    pub body: Vec<u8>,
}

impl Route {
    pub fn new(path: &'static str, status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            path,
            status,
            headers: Vec::new(),
            body: body.into(),
        }
    }

    pub fn html(path: &'static str, title: &str) -> Self {
        Self::new(
            path,
            200,
            format!("<!doctype html><html><head><title>{title}</title></head><body></body></html>"),
        )
        .header("content-type", "text/html")
    }

    pub fn json(path: &'static str, json: &str) -> Self {
        Self::new(path, 200, json).header("content-type", "application/json")
    }

    pub fn header(mut self, name: &'static str, value: &str) -> Self {
        self.headers.push((name, value.to_string()));
        self
    }
}

/// A running mock server. Dropping it stops the server.
pub struct Mock {
    pub addr: SocketAddr,
    task: JoinHandle<()>,
}

impl Mock {
    pub fn port(&self) -> u16 {
        self.addr.port()
    }
}

impl Drop for Mock {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn bind(ip: IpAddr) -> TcpListener {
    TcpListener::bind(SocketAddr::new(ip, 0))
        .await
        .expect("bind mock")
}

pub fn localhost() -> IpAddr {
    "127.0.0.1".parse().unwrap()
}

/// Serves `routes` over plain HTTP on `ip` and a free port.
pub async fn http(ip: IpAddr, routes: Vec<Route>) -> Mock {
    let listener = bind(ip).await;
    let addr = listener.local_addr().unwrap();
    let routes = Arc::new(routes);
    let task = tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let routes = routes.clone();
            tokio::spawn(async move { serve(stream, &routes).await });
        }
    });
    Mock { addr, task }
}

/// Serves `routes` over HTTPS with a fresh self-signed certificate.
pub async fn https(ip: IpAddr, routes: Vec<Route>) -> Mock {
    let cert =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string(), ip.to_string()]).unwrap();
    let der_cert = cert.cert.der().clone();
    let der_key = rustls::pki_types::PrivateKeyDer::Pkcs8(cert.signing_key.serialize_der().into());
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(vec![der_cert], der_key)
    .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));

    let listener = bind(ip).await;
    let addr = listener.local_addr().unwrap();
    let routes = Arc::new(routes);
    let task = tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let (routes, acceptor) = (routes.clone(), acceptor.clone());
            tokio::spawn(async move {
                if let Ok(tls) = acceptor.accept(stream).await {
                    serve(tls, &routes).await;
                }
            });
        }
    });
    Mock { addr, task }
}

/// A non-HTTP service that greets with a banner, like SSH.
pub async fn banner(ip: IpAddr, text: &'static str) -> Mock {
    let listener = bind(ip).await;
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let _ = stream.write_all(text.as_bytes()).await;
                let mut buf = [0u8; 512];
                let _ = stream.read(&mut buf).await;
            });
        }
    });
    Mock { addr, task }
}

/// A port number that nothing listens on.
pub async fn closed_port(ip: IpAddr) -> u16 {
    let l = bind(ip).await;
    l.local_addr().unwrap().port()
}

async fn serve<S: AsyncRead + AsyncWrite + Unpin>(mut stream: S, routes: &[Route]) {
    let mut req = Vec::new();
    let mut buf = [0u8; 2048];
    while !req.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => req.extend_from_slice(&buf[..n]),
        }
        if req.len() > 16 * 1024 {
            return;
        }
    }
    let text = String::from_utf8_lossy(&req);
    let path = text.split_whitespace().nth(1).unwrap_or("/").to_string();
    let not_found = Route::new("", 404, "not found");
    let route = routes.iter().find(|r| r.path == path).unwrap_or(&not_found);
    let mut head = format!(
        "HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n",
        route.status,
        route.body.len()
    );
    for (k, v) in &route.headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(&route.body).await;
    let _ = stream.shutdown().await;
}

/// A 1x1 transparent PNG.
pub const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae,
    0x42, 0x60, 0x82,
];

// Recorded-style responses, trimmed to the fields the scanner reads.

pub fn jellyfin(ip: IpAddr) -> impl std::future::Future<Output = Mock> {
    http(
        ip,
        vec![
            Route::html("/", "Jellyfin"),
            Route::json(
                "/System/Info/Public",
                r#"{"LocalAddress":"http://10.0.1.7:8096","ServerName":"media","Version":"10.10.3","ProductName":"Jellyfin Server","OperatingSystem":"","Id":"a1b2c3","StartupWizardCompleted":true}"#,
            ),
        ],
    )
}

pub fn plex(ip: IpAddr) -> impl std::future::Future<Output = Mock> {
    http(
        ip,
        vec![
            Route::new("/", 401, "<html><head><title>Unauthorized</title></head><body><h1>401 Unauthorized</h1></body></html>")
                .header("x-plex-protocol", "1.0")
                .header("content-type", "text/html"),
            Route::new(
                "/identity",
                200,
                r#"<?xml version="1.0" encoding="UTF-8"?>
<MediaContainer size="0" apiVersion="1.1.1" claimed="1" machineIdentifier="0123456789abcdef" version="1.41.3.9314-a0bfb8370">
</MediaContainer>"#,
            )
            .header("content-type", "text/xml"),
        ],
    )
}

pub fn sonarr(ip: IpAddr) -> impl std::future::Future<Output = Mock> {
    http(
        ip,
        vec![
            Route::html("/", "Sonarr"),
            Route::new("/api/v3/system/status", 401, ""),
        ],
    )
}

pub fn qbittorrent(ip: IpAddr) -> impl std::future::Future<Output = Mock> {
    http(
        ip,
        vec![
            Route::html("/", "qBittorrent WebUI"),
            Route::new("/api/v2/app/version", 200, "v5.0.2"),
        ],
    )
}

pub fn unknown_app(ip: IpAddr) -> impl std::future::Future<Output = Mock> {
    http(
        ip,
        vec![
            Route::new(
                "/",
                200,
                r#"<html><head><title>Family Photos</title><link rel="icon" href="/static/icon.png"></head></html>"#,
            )
            .header("server", "nginx/1.27.0"),
            Route::new("/static/icon.png", 200, PNG).header("content-type", "image/png"),
        ],
    )
}
