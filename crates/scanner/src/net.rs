//! What the scanner needs from the network, and what this machine's networks are.
//!
//! The scanner never touches sockets directly: it goes through a [`Network`],
//! which the real [`SystemNetwork`] implements with plain TCP connects, the
//! system resolver and the kernel's neighbour (ARP) table. Tests swap in a
//! fake network to check results and timing on a /24 without real hosts.
//!
//! [`local_networks`] finds the private networks this machine is on (each
//! interface's /24) and its own addresses, which make the default scan
//! targets.

use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::pin::Pin;
use std::time::Duration;

use ipnet::{IpNet, Ipv4Net};
use serde::{Deserialize, Serialize};

use crate::connect::{self, PortState};

/// A boxed, sendable future, as returned by [`Network`] methods.
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// The network operations a scan uses.
pub trait Network: Send + Sync + std::fmt::Debug + 'static {
    /// Tries one TCP handshake to `addr`, giving up after `timeout`.
    fn probe(&self, addr: SocketAddr, timeout: Duration) -> BoxFuture<PortState>;

    /// The name in the reverse DNS (PTR) record of `ip`, if any answers
    /// within `timeout`.
    fn reverse_lookup(&self, ip: IpAddr, timeout: Duration) -> BoxFuture<Option<String>>;

    /// Addresses the kernel has recently resolved to a hardware address
    /// (the ARP / neighbour table): hosts that answered on the local link.
    fn neighbours(&self) -> BoxFuture<Vec<IpAddr>>;
}

/// The real network: TCP connects, the system resolver and `/proc/net/arp`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemNetwork;

impl Network for SystemNetwork {
    fn probe(&self, addr: SocketAddr, timeout: Duration) -> BoxFuture<PortState> {
        Box::pin(connect::probe(addr, timeout))
    }

    fn reverse_lookup(&self, ip: IpAddr, timeout: Duration) -> BoxFuture<Option<String>> {
        Box::pin(async move {
            // getnameinfo blocks; a slow lookup's thread is left to finish on its own.
            let lookup = tokio::task::spawn_blocking(move || dns_lookup::lookup_addr(&ip).ok());
            match tokio::time::timeout(timeout, lookup).await {
                Ok(Ok(Some(name))) => clean_name(&name, ip),
                _ => None,
            }
        })
    }

    fn neighbours(&self) -> BoxFuture<Vec<IpAddr>> {
        Box::pin(async {
            tokio::fs::read_to_string("/proc/net/arp")
                .await
                .map(|t| parse_arp(&t))
                .unwrap_or_default()
        })
    }
}

/// A PTR answer as a label: no trailing dot, and nothing that is just the
/// address written back.
fn clean_name(name: &str, ip: IpAddr) -> Option<String> {
    let name = name.trim().trim_end_matches('.');
    if name.is_empty() || name.parse::<IpAddr>().is_ok() || name == ip.to_string() {
        return None;
    }
    Some(name.to_ascii_lowercase())
}

/// Turns a reverse DNS name into the label shown for a host. Docker's
/// embedded DNS answers `jellyfin.mystack_default`; network names contain
/// `_`, which LAN hostnames never do, so those are cut to the container name.
pub fn short_name(name: &str) -> String {
    let name = name.trim_end_matches('.');
    match name.split_once('.') {
        Some((first, rest)) if !first.is_empty() && rest.contains('_') => first.to_string(),
        _ => name.to_string(),
    }
}

/// Parses `/proc/net/arp`: the addresses whose entry is complete (flag
/// `0x2`) with a real hardware address.
pub fn parse_arp(text: &str) -> Vec<IpAddr> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let cols: Vec<&str> = line.split_whitespace().collect();
            let (ip, flags, mac) = (cols.first()?, cols.get(2)?, cols.get(3)?);
            let flags = u32::from_str_radix(flags.trim_start_matches("0x"), 16).ok()?;
            if flags & 0x2 == 0 || *mac == "00:00:00:00:00:00" {
                return None;
            }
            ip.parse().ok()
        })
        .collect()
}

/// One network this machine is on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalNetwork {
    /// The /24 around the interface's address (or the interface's own
    /// prefix when it is smaller than a /24).
    pub net: IpNet,
    /// The interface name, e.g. `eth0` or `docker0`.
    pub interface: String,
    /// This machine's address on it.
    pub address: IpAddr,
    /// Whether it looks like a Docker network (a `docker*` or `br-*`
    /// bridge). Inside a container every network is a Docker one, which the
    /// app can tell from the Docker API.
    pub docker: bool,
}

/// The private IPv4 networks of this machine, at most a /24 each, and every
/// address it has (to leave out of scans).
pub fn local_networks() -> (Vec<LocalNetwork>, Vec<IpAddr>) {
    let Ok(ifaces) = if_addrs::get_if_addrs() else {
        return (Vec::new(), Vec::new());
    };
    let list = ifaces.into_iter().filter_map(|i| match i.addr {
        if_addrs::IfAddr::V4(v4) => Some((i.name, v4.ip, v4.prefixlen)),
        if_addrs::IfAddr::V6(_) => None,
    });
    let mut own: Vec<IpAddr> = Vec::new();
    let mut nets = Vec::new();
    for (name, ip, prefix) in list {
        own.push(IpAddr::V4(ip));
        if let Some(n) = local_network(&name, ip, prefix)
            && !nets.iter().any(|x: &LocalNetwork| x.net == n.net)
        {
            nets.push(n);
        }
    }
    // Real LAN interfaces first, so the default target lists them on top.
    nets.sort_by_key(|n| n.docker);
    (nets, own)
}

/// The network to scan for one interface address, if it is a private one.
pub fn local_network(interface: &str, ip: Ipv4Addr, prefix: u8) -> Option<LocalNetwork> {
    if !ip.is_private() || ip.is_loopback() {
        return None;
    }
    let net = Ipv4Net::new(ip, prefix.clamp(24, 30)).ok()?.trunc();
    Some(LocalNetwork {
        net: IpNet::V4(net),
        interface: interface.to_string(),
        address: IpAddr::V4(ip),
        docker: is_docker_interface(interface),
    })
}

fn is_docker_interface(name: &str) -> bool {
    name.starts_with("docker") || name.starts_with("br-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arp_table() {
        let t = "IP address       HW type     Flags       HW address            Mask     Device\n\
                 10.0.1.1         0x1         0x2         aa:bb:cc:dd:ee:01     *        eth0\n\
                 10.0.1.9         0x1         0x0         00:00:00:00:00:00     *        eth0\n\
                 10.0.1.7         0x1         0x6         aa:bb:cc:dd:ee:07     *        eth0\n";
        assert_eq!(
            parse_arp(t),
            vec![
                "10.0.1.1".parse::<IpAddr>().unwrap(),
                "10.0.1.7".parse().unwrap()
            ]
        );
    }

    #[test]
    fn names() {
        assert_eq!(short_name("jellyfin.media_default."), "jellyfin");
        assert_eq!(short_name("nas.lan"), "nas.lan");
        let ip: IpAddr = "10.0.0.5".parse().unwrap();
        assert_eq!(clean_name("NAS.lan.", ip).as_deref(), Some("nas.lan"));
        assert_eq!(clean_name("10.0.0.5", ip), None);
    }

    #[test]
    fn networks_from_interfaces() {
        let n = local_network("eth0", "10.0.1.23".parse().unwrap(), 16).unwrap();
        assert_eq!(n.net.to_string(), "10.0.1.0/24");
        assert!(!n.docker);
        let d = local_network("br-1a2b", "172.18.0.1".parse().unwrap(), 16).unwrap();
        assert!(d.docker);
        let small = local_network("wg0", "10.9.0.5".parse().unwrap(), 28).unwrap();
        assert_eq!(small.net.to_string(), "10.9.0.0/28");
        assert!(local_network("eth1", "8.8.8.8".parse().unwrap(), 24).is_none());
    }
}
