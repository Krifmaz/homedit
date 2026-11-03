//! The TCP connect probe.
//!
//! This is a plain `connect()` through the operating system, the same thing
//! any client does. It needs no raw sockets and no extra privileges, and it
//! never sends data: the connection is closed as soon as it is established.

use std::io::ErrorKind;
use std::net::SocketAddr;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;

/// What a connect attempt found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortState {
    /// The handshake completed: something is listening.
    Open,
    /// The host answered with a reset: nothing is listening.
    Closed,
    /// No answer before the timeout, or the network said the host is
    /// unreachable. Usually a firewall or a host that is down.
    Filtered,
}

/// Tries one TCP handshake to `addr`, giving up after `timeout`.
pub async fn probe(addr: SocketAddr, timeout: Duration) -> PortState {
    match tokio::time::timeout(timeout, TcpStream::connect(addr)).await {
        Ok(Ok(stream)) => {
            drop(stream);
            PortState::Open
        }
        Ok(Err(e)) if e.kind() == ErrorKind::ConnectionRefused => PortState::Closed,
        Ok(Err(_)) | Err(_) => PortState::Filtered,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn open_and_closed() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        assert_eq!(probe(addr, Duration::from_secs(1)).await, PortState::Open);
        drop(listener);
        assert_eq!(probe(addr, Duration::from_secs(1)).await, PortState::Closed);
    }
}
