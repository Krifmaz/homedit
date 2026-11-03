//! A minimal HTTP health probe for `homedit healthcheck`.
//!
//! The container image has no shell or `curl`, so Docker's `HEALTHCHECK`
//! runs the homedit binary itself with a `healthcheck` argument. That
//! command calls [`probe`], which sends `GET /healthz` over plain TCP and
//! succeeds on a `200` answer.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Turns the server's listen address into the address to probe: an
/// unspecified address (`0.0.0.0` or `::`) becomes loopback.
///
/// Returns `None` if `listen` is not a socket address.
pub fn probe_addr_from_listen(listen: &str) -> Option<SocketAddr> {
    let mut addr: SocketAddr = listen.trim().parse().ok()?;
    match addr.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => addr.set_ip(IpAddr::V4(Ipv4Addr::LOCALHOST)),
        IpAddr::V6(ip) if ip.is_unspecified() => addr.set_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)),
        _ => {}
    }
    Some(addr)
}

/// Sends `GET /healthz` to `addr` and checks for a `200` status line.
///
/// Returns an error message when the server cannot be reached, does not
/// answer within `timeout`, or answers with another status.
pub async fn probe(addr: SocketAddr, timeout: Duration) -> Result<(), String> {
    tokio::time::timeout(timeout, probe_inner(addr))
        .await
        .map_err(|_| format!("no answer from {addr} within {timeout:?}"))?
}

async fn probe_inner(addr: SocketAddr) -> Result<(), String> {
    let mut stream = TcpStream::connect(addr)
        .await
        .map_err(|e| format!("cannot connect to {addr}: {e}"))?;
    let request = format!(
        "GET /healthz HTTP/1.1\r\nHost: {addr}\r\nUser-Agent: homedit-healthcheck\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|e| format!("cannot send request: {e}"))?;
    // The status line fits in the first read; never read more than 1 KiB.
    let mut buf = [0u8; 1024];
    let mut len = 0;
    while len < buf.len() {
        let n = stream
            .read(&mut buf[len..])
            .await
            .map_err(|e| format!("cannot read answer: {e}"))?;
        if n == 0 {
            break;
        }
        len += n;
        if buf[..len].contains(&b'\n') {
            break;
        }
    }
    let head = String::from_utf8_lossy(&buf[..len]);
    let status_line = head.lines().next().unwrap_or_default();
    let mut parts = status_line.split_whitespace();
    match (parts.next(), parts.next()) {
        (Some(version), Some("200")) if version.starts_with("HTTP/") => Ok(()),
        _ => Err(format!("unhealthy answer: {status_line:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_unspecified_to_loopback() {
        assert_eq!(
            probe_addr_from_listen("0.0.0.0:3000"),
            Some("127.0.0.1:3000".parse().unwrap())
        );
        assert_eq!(
            probe_addr_from_listen("[::]:80"),
            Some("[::1]:80".parse().unwrap())
        );
        assert_eq!(
            probe_addr_from_listen("10.0.0.5:3000"),
            Some("10.0.0.5:3000".parse().unwrap())
        );
        assert_eq!(probe_addr_from_listen("nope"), None);
    }
}
