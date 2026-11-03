//! Scan targets: CIDR ranges, single addresses and hostnames.

use std::collections::BTreeMap;
use std::fmt;
use std::net::IpAddr;
use std::str::FromStr;

use ipnet::IpNet;
use serde::{Deserialize, Serialize};

use crate::error::{Result, ScanError};
use crate::policy::{ScanPolicy, canonical};

/// One entry in the admin's list of things to scan.
///
/// Parse it from text with [`str::parse`]:
///
/// ```
/// use scanner::Target;
/// let net: Target = "10.0.1.0/24".parse().unwrap();
/// let host: Target = "10.0.1.7".parse().unwrap();
/// let name: Target = "nas.lan".parse().unwrap();
/// assert!(matches!(net, Target::Network(_)));
/// assert!(matches!(host, Target::Address(_)));
/// assert!(matches!(name, Target::Hostname(_)));
/// ```
///
/// Serialized as the same string form, so a list of targets stores as a plain
/// list of strings.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum Target {
    /// A CIDR range such as `192.168.1.0/24` or `fd00::/120`. Host bits are
    /// cleared, so `10.0.0.5/24` means `10.0.0.0/24`.
    Network(IpNet),
    /// A single IPv4 or IPv6 address.
    Address(IpAddr),
    /// A DNS name, resolved with the system resolver when the scan starts.
    Hostname(String),
}

impl FromStr for Target {
    type Err = ScanError;

    fn from_str(input: &str) -> Result<Self> {
        let s = input.trim();
        let invalid = |reason: &str| ScanError::InvalidTarget {
            input: input.to_string(),
            reason: reason.to_string(),
        };
        if s.is_empty() {
            return Err(invalid("empty"));
        }
        if s.contains('/') {
            let net: IpNet = s.parse().map_err(|_| invalid("not a valid CIDR range"))?;
            return Ok(Target::Network(net.trunc()));
        }
        // Allow "[fe80::1]" as people often copy IPv6 addresses from URLs.
        let bare = s
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
            .unwrap_or(s);
        if let Ok(ip) = bare.parse::<IpAddr>() {
            return Ok(Target::Address(canonical(ip)));
        }
        if is_valid_hostname(s) {
            return Ok(Target::Hostname(
                s.trim_end_matches('.').to_ascii_lowercase(),
            ));
        }
        Err(invalid("not an IP address, CIDR range or hostname"))
    }
}

impl TryFrom<String> for Target {
    type Error = ScanError;
    fn try_from(value: String) -> Result<Self> {
        value.parse()
    }
}

impl From<Target> for String {
    fn from(value: Target) -> Self {
        value.to_string()
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Target::Network(net) => write!(f, "{net}"),
            Target::Address(ip) => write!(f, "{ip}"),
            Target::Hostname(name) => f.write_str(name),
        }
    }
}

/// Checks a hostname against RFC 1123 rules: letters, digits and hyphens in
/// dot-separated labels of 1–63 characters, 253 characters in total, no label
/// starting or ending with a hyphen. Rejecting anything else up front keeps
/// odd input away from the resolver.
fn is_valid_hostname(s: &str) -> bool {
    let s = s.strip_suffix('.').unwrap_or(s);
    if s.is_empty() || s.len() > 253 {
        return false;
    }
    // A name made only of digits and dots is a mistyped IPv4 address.
    if s.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return false;
    }
    s.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    })
}

/// An address that will be scanned, with the hostname it came from if any.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedHost {
    /// The address to connect to.
    pub ip: IpAddr,
    /// The hostname the admin typed, when the address came from DNS.
    pub hostname: Option<String>,
}

impl Target {
    /// How many addresses this target covers without expanding it. Hostnames
    /// count as one. Saturates at `u128::MAX` for huge IPv6 ranges.
    pub fn address_count(&self) -> u128 {
        match self {
            Target::Network(net) => {
                let host_bits = u32::from(net.max_prefix_len() - net.prefix_len());
                let total = 1u128.checked_shl(host_bits).unwrap_or(u128::MAX);
                // Skip network and broadcast addresses for IPv4 ranges larger
                // than /31, like `IpNet::hosts` does.
                if matches!(net, IpNet::V4(_)) && host_bits >= 2 {
                    total - 2
                } else {
                    total
                }
            }
            Target::Address(_) | Target::Hostname(_) => 1,
        }
    }
}

/// Expands and resolves `targets` into a sorted, de-duplicated host list and
/// checks every address against `policy`.
///
/// Fails without scanning anything if any address is refused, if the total
/// exceeds [`ScanPolicy::max_addresses`], or if a hostname does not resolve.
/// Hostnames are resolved with the system resolver through
/// [`tokio::net::lookup_host`]; no shell or external program is involved.
pub async fn resolve_targets(targets: &[Target], policy: &ScanPolicy) -> Result<Vec<ResolvedHost>> {
    if targets.is_empty() {
        return Err(ScanError::Empty("no targets"));
    }
    let limit = policy.max_addresses as u128;
    let declared: u128 = targets
        .iter()
        .map(Target::address_count)
        .fold(0, u128::saturating_add);
    if declared > limit {
        return Err(ScanError::TooLarge(format!(
            "{declared} addresses requested, the limit is {limit}"
        )));
    }

    let mut hosts: BTreeMap<IpAddr, Option<String>> = BTreeMap::new();
    for target in targets {
        match target {
            Target::Network(net) => {
                for ip in net.hosts() {
                    hosts.entry(ip).or_insert(None);
                }
            }
            Target::Address(ip) => {
                hosts.entry(*ip).or_insert(None);
            }
            Target::Hostname(name) => {
                let found = tokio::net::lookup_host((name.as_str(), 0))
                    .await
                    .map_err(|e| ScanError::Resolve {
                        host: name.clone(),
                        reason: e.to_string(),
                    })?;
                let mut any = false;
                for addr in found {
                    any = true;
                    let slot = hosts.entry(canonical(addr.ip())).or_insert(None);
                    if slot.is_none() {
                        *slot = Some(name.clone());
                    }
                }
                if !any {
                    return Err(ScanError::Resolve {
                        host: name.clone(),
                        reason: "no addresses".into(),
                    });
                }
            }
        }
    }

    if hosts.len() > policy.max_addresses {
        return Err(ScanError::TooLarge(format!(
            "{} addresses after DNS, the limit is {}",
            hosts.len(),
            policy.max_addresses
        )));
    }
    for ip in hosts.keys() {
        policy.check_ip(*ip)?;
    }
    Ok(hosts
        .into_iter()
        .map(|(ip, hostname)| ResolvedHost { ip, hostname })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_each_kind() {
        assert_eq!(
            "10.0.0.5/24".parse::<Target>().unwrap(),
            Target::Network("10.0.0.0/24".parse().unwrap())
        );
        assert_eq!(
            " 10.0.1.7 ".parse::<Target>().unwrap(),
            Target::Address("10.0.1.7".parse().unwrap())
        );
        assert_eq!(
            "[fd00::1]".parse::<Target>().unwrap(),
            Target::Address("fd00::1".parse().unwrap())
        );
        assert_eq!(
            "NAS.lan.".parse::<Target>().unwrap(),
            Target::Hostname("nas.lan".into())
        );
    }

    #[test]
    fn rejects_bad_input() {
        for bad in [
            "",
            "10.0.0.0/33",
            "300.1.1.1",
            "host;rm -rf /",
            "$(whoami)",
            "-bad.lan",
            "a..b",
            "http://nas.lan",
            "nas.lan:8080",
        ] {
            assert!(bad.parse::<Target>().is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn counts_addresses() {
        let t = |s: &str| s.parse::<Target>().unwrap().address_count();
        assert_eq!(t("10.0.0.0/24"), 254);
        assert_eq!(t("10.0.0.0/31"), 2);
        assert_eq!(t("10.0.0.0/32"), 1);
        assert_eq!(t("fd00::/120"), 256);
        assert_eq!(t("fd00::/0"), u128::MAX);
    }

    #[test]
    fn serde_uses_strings() {
        let list: Vec<Target> = serde_json::from_str(r#"["10.0.0.0/30","nas.lan"]"#).unwrap();
        assert_eq!(
            serde_json::to_string(&list).unwrap(),
            r#"["10.0.0.0/30","nas.lan"]"#
        );
    }

    #[tokio::test]
    async fn resolve_enforces_limits_and_policy() {
        let policy = ScanPolicy {
            max_addresses: 10,
            ..ScanPolicy::default()
        };
        let big = vec!["10.0.0.0/24".parse().unwrap()];
        assert!(matches!(
            resolve_targets(&big, &policy).await,
            Err(ScanError::TooLarge(_))
        ));

        let public = vec!["8.8.8.8".parse().unwrap()];
        assert!(matches!(
            resolve_targets(&public, &policy).await,
            Err(ScanError::AddressNotAllowed { .. })
        ));

        let small = vec![
            "10.0.0.0/30".parse().unwrap(),
            "10.0.0.2".parse().unwrap(),
            "localhost".parse().unwrap(),
        ];
        let hosts = resolve_targets(&small, &policy).await.unwrap();
        let ips: Vec<String> = hosts.iter().map(|h| h.ip.to_string()).collect();
        assert!(ips.contains(&"10.0.0.1".to_string()));
        assert!(ips.contains(&"10.0.0.2".to_string()));
        assert_eq!(ips.iter().filter(|ip| *ip == "10.0.0.2").count(), 1);
        assert!(
            hosts
                .iter()
                .any(|h| h.hostname.as_deref() == Some("localhost"))
        );
    }
}
