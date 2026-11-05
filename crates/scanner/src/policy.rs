//! Safety and performance limits for scans.
//!
//! A [`ScanPolicy`] is set once by the admin (it lives in the app's settings)
//! and applies to every scan. It decides which addresses may be scanned at all,
//! how fast, and how large one scan may be.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use ipnet::IpNet;
use serde::{Deserialize, Serialize};

use crate::error::{Result, ScanError};

/// Admin-controlled limits that every scan obeys.
///
/// The defaults: only private, link-local and loopback addresses, 512
/// connections in flight with a 400 ms connect timeout, no per-second cap,
/// 32 services identified at a time with a 2 s HTTP timeout, and at most
/// 1024 addresses per scan. Only hosts that answer are port-scanned, so the
/// load on a network stays small even with these numbers.
///
/// All durations are stored as milliseconds so the policy round-trips cleanly
/// through JSON, YAML and SQLite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScanPolicy {
    /// Allow addresses outside the private ranges (see [`classify`]).
    ///
    /// Off by default. Even when on, multicast, broadcast and unspecified
    /// addresses are always refused.
    pub allow_public: bool,
    /// Extra ranges the admin allows on top of the private ones, for example
    /// `100.64.0.0/10` for a Tailscale network.
    pub extra_allowed: Vec<IpNet>,
    /// Most TCP connection attempts in flight at the same time.
    pub max_concurrency: usize,
    /// Most new TCP connections started per second, across the whole scan
    /// (HTTP fingerprint requests count too). `0` means unlimited.
    pub connects_per_second: u32,
    /// How long to wait for a TCP handshake before calling a port filtered.
    pub connect_timeout_ms: u64,
    /// Timeout for each HTTP request made while fingerprinting.
    pub http_timeout_ms: u64,
    /// Most services fingerprinted at the same time.
    pub fingerprint_concurrency: usize,
    /// Most addresses one scan may cover after CIDR expansion and DNS.
    pub max_addresses: usize,
    /// Most `(address, port)` pairs one scan may probe.
    pub max_probes: u64,
}

impl Default for ScanPolicy {
    fn default() -> Self {
        Self {
            allow_public: false,
            extra_allowed: Vec::new(),
            max_concurrency: 512,
            connects_per_second: 0,
            connect_timeout_ms: 400,
            http_timeout_ms: 2_000,
            fingerprint_concurrency: 32,
            max_addresses: 1_024,
            max_probes: 10_000_000,
        }
    }
}

impl ScanPolicy {
    /// The TCP connect timeout as a [`Duration`].
    pub fn connect_timeout(&self) -> Duration {
        Duration::from_millis(self.connect_timeout_ms)
    }

    /// The per-request HTTP timeout as a [`Duration`].
    pub fn http_timeout(&self) -> Duration {
        Duration::from_millis(self.http_timeout_ms)
    }

    /// Checks that the numbers make sense (no zero concurrency or timeouts).
    pub fn validate(&self) -> Result<()> {
        if self.max_concurrency == 0 {
            return Err(ScanError::Config(
                "max_concurrency must be at least 1".into(),
            ));
        }
        if self.fingerprint_concurrency == 0 {
            return Err(ScanError::Config(
                "fingerprint_concurrency must be at least 1".into(),
            ));
        }
        if self.connect_timeout_ms == 0 || self.http_timeout_ms == 0 {
            return Err(ScanError::Config("timeouts must be above 0 ms".into()));
        }
        if self.max_addresses == 0 || self.max_probes == 0 {
            return Err(ScanError::Config("size limits must be above 0".into()));
        }
        Ok(())
    }

    /// Decides whether `ip` may be scanned under this policy.
    ///
    /// IPv4-mapped IPv6 addresses (`::ffff:a.b.c.d`) are judged as the IPv4
    /// address they carry, so they cannot be used to slip past the rules.
    pub fn check_ip(&self, ip: IpAddr) -> Result<()> {
        let ip = canonical(ip);
        match classify(ip) {
            AddressClass::Forbidden => Err(ScanError::AddressNotAllowed {
                ip,
                reason: "multicast, broadcast and unspecified addresses are never scanned",
            }),
            AddressClass::Private | AddressClass::Loopback | AddressClass::LinkLocal => Ok(()),
            AddressClass::Public => {
                if self.allow_public || self.extra_allowed.iter().any(|net| net.contains(&ip)) {
                    Ok(())
                } else {
                    Err(ScanError::AddressNotAllowed {
                        ip,
                        reason: "public addresses need allow_public or an extra_allowed range",
                    })
                }
            }
        }
    }
}

/// What kind of network an address belongs to, as far as scanning goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddressClass {
    /// RFC 1918 (`10/8`, `172.16/12`, `192.168/16`) or IPv6 unique local
    /// (`fc00::/7`).
    Private,
    /// `127/8` or `::1`: the server itself, which is often the Docker host.
    Loopback,
    /// `169.254/16` or `fe80::/10`.
    LinkLocal,
    /// Never scanned: multicast, broadcast, unspecified and reserved
    /// "this network" addresses.
    Forbidden,
    /// Anything else. Scanned only when the policy allows it.
    Public,
}

/// Classifies an address. See [`AddressClass`] for the ranges.
pub fn classify(ip: IpAddr) -> AddressClass {
    match canonical(ip) {
        IpAddr::V4(v4) => classify_v4(v4),
        IpAddr::V6(v6) => classify_v6(v6),
    }
}

fn classify_v4(ip: Ipv4Addr) -> AddressClass {
    let o = ip.octets();
    if ip.is_unspecified() || ip.is_broadcast() || ip.is_multicast() || o[0] == 0 || o[0] >= 240 {
        AddressClass::Forbidden
    } else if ip.is_loopback() {
        AddressClass::Loopback
    } else if ip.is_link_local() {
        AddressClass::LinkLocal
    } else if ip.is_private() {
        AddressClass::Private
    } else {
        AddressClass::Public
    }
}

fn classify_v6(ip: Ipv6Addr) -> AddressClass {
    let first = ip.segments()[0];
    if ip.is_unspecified() || ip.is_multicast() {
        AddressClass::Forbidden
    } else if ip.is_loopback() {
        AddressClass::Loopback
    } else if first & 0xffc0 == 0xfe80 {
        AddressClass::LinkLocal
    } else if first & 0xfe00 == 0xfc00 {
        AddressClass::Private
    } else {
        AddressClass::Public
    }
}

/// Turns an IPv4-mapped IPv6 address back into plain IPv4.
pub(crate) fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(v6),
        },
        v4 => v4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn classifies_ranges() {
        for (addr, class) in [
            ("10.0.1.7", AddressClass::Private),
            ("172.16.0.1", AddressClass::Private),
            ("172.31.255.254", AddressClass::Private),
            ("192.168.1.10", AddressClass::Private),
            ("fd12:3456::1", AddressClass::Private),
            ("127.0.0.1", AddressClass::Loopback),
            ("::1", AddressClass::Loopback),
            ("169.254.10.1", AddressClass::LinkLocal),
            ("fe80::1", AddressClass::LinkLocal),
            ("8.8.8.8", AddressClass::Public),
            ("172.32.0.1", AddressClass::Public),
            ("2001:db8::1", AddressClass::Public),
            ("0.0.0.0", AddressClass::Forbidden),
            ("255.255.255.255", AddressClass::Forbidden),
            ("224.0.0.1", AddressClass::Forbidden),
            ("ff02::1", AddressClass::Forbidden),
            ("::", AddressClass::Forbidden),
        ] {
            assert_eq!(classify(ip(addr)), class, "{addr}");
        }
    }

    #[test]
    fn mapped_v6_is_judged_as_v4() {
        assert_eq!(classify(ip("::ffff:8.8.8.8")), AddressClass::Public);
        assert_eq!(classify(ip("::ffff:10.0.0.1")), AddressClass::Private);
        assert!(
            ScanPolicy::default()
                .check_ip(ip("::ffff:8.8.8.8"))
                .is_err()
        );
    }

    #[test]
    fn public_needs_opt_in() {
        let mut policy = ScanPolicy::default();
        assert!(policy.check_ip(ip("10.0.0.1")).is_ok());
        assert!(policy.check_ip(ip("100.100.1.1")).is_err());

        policy.extra_allowed.push("100.64.0.0/10".parse().unwrap());
        assert!(policy.check_ip(ip("100.100.1.1")).is_ok());
        assert!(policy.check_ip(ip("1.1.1.1")).is_err());

        policy.allow_public = true;
        assert!(policy.check_ip(ip("1.1.1.1")).is_ok());
        assert!(
            policy.check_ip(ip("224.0.0.1")).is_err(),
            "multicast never allowed"
        );
    }

    #[test]
    fn validate_rejects_zeroes() {
        let policy = ScanPolicy {
            max_concurrency: 0,
            ..ScanPolicy::default()
        };
        assert!(policy.validate().is_err());
        assert!(ScanPolicy::default().validate().is_ok());
    }

    #[test]
    fn policy_deserializes_with_defaults() {
        let policy: ScanPolicy = serde_json::from_str(r#"{"allow_public":true}"#).unwrap();
        assert!(policy.allow_public);
        assert_eq!(
            policy.max_concurrency,
            ScanPolicy::default().max_concurrency
        );
    }
}
