//! Which TCP ports a scan checks.

use std::collections::BTreeSet;
use std::fmt;
use std::ops::RangeInclusive;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::catalogue::Catalogue;
use crate::error::{Result, ScanError};

/// Ports that are always part of [`PortSet::Common`], on top of the
/// `3000..=9999` block and the catalogue's own ports.
const COMMON_EXTRA: &[u16] = &[80, 81, 443, 8443, 10000, 32400];

/// The block of ports most self-hosted apps listen on.
const COMMON_BLOCK: RangeInclusive<u16> = 3000..=9999;

/// The ports a scan checks on every address.
///
/// Serialized with a `kind` tag, e.g. `{"kind":"common"}` or
/// `{"kind":"list","ports":"80,443,8000-8100"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PortSet {
    /// The default: 80, 443, 3000–9999 and every port from the built-in app
    /// catalogue (for example 32400 for Plex and 13378 for Audiobookshelf).
    #[default]
    Common,
    /// Only the ports from the app catalogue. Much faster than
    /// [`PortSet::Common`], handy for scheduled rescans.
    Catalogue,
    /// Every port from 1 to 65535. Opt-in, because it is slow.
    Full,
    /// An explicit list.
    List {
        /// The ports, written as text such as `"80,443,8000-8100"`.
        ports: PortList,
    },
}

impl PortSet {
    /// Returns the sorted, de-duplicated ports for this set.
    pub fn resolve(&self, catalogue: &Catalogue) -> Vec<u16> {
        let mut set: BTreeSet<u16> = BTreeSet::new();
        match self {
            PortSet::Common => {
                set.extend(COMMON_BLOCK);
                set.extend(COMMON_EXTRA);
                set.extend(catalogue.ports());
            }
            PortSet::Catalogue => set.extend(catalogue.ports()),
            PortSet::Full => set.extend(1..=u16::MAX),
            PortSet::List { ports } => {
                for range in &ports.0 {
                    set.extend(range.clone());
                }
            }
        }
        set.into_iter().collect()
    }

    /// Whether `port` is in this set, without building the whole list.
    pub fn contains(&self, port: u16, catalogue: &Catalogue) -> bool {
        match self {
            PortSet::Common => {
                COMMON_BLOCK.contains(&port)
                    || COMMON_EXTRA.contains(&port)
                    || catalogue.ports().any(|p| p == port)
            }
            PortSet::Catalogue => catalogue.ports().any(|p| p == port),
            PortSet::Full => port != 0,
            PortSet::List { ports } => ports.0.iter().any(|r| r.contains(&port)),
        }
    }
}

/// A list of ports and ranges such as `80,443,8000-8100`.
///
/// Port 0 is rejected, ranges must go up (`9000-8000` is an error), and
/// whitespace around items is ignored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PortList(Vec<RangeInclusive<u16>>);

impl PortList {
    /// The ranges in the order they were written.
    pub fn ranges(&self) -> &[RangeInclusive<u16>] {
        &self.0
    }
}

impl FromStr for PortList {
    type Err = ScanError;

    fn from_str(input: &str) -> Result<Self> {
        let invalid = |reason: String| ScanError::InvalidPorts {
            input: input.to_string(),
            reason,
        };
        let parse_port = |s: &str| -> Result<u16> {
            match s.trim().parse::<u16>() {
                Ok(0) => Err(invalid("port 0 is not valid".into())),
                Ok(p) => Ok(p),
                Err(_) => Err(invalid(format!("{:?} is not a port number", s.trim()))),
            }
        };
        let mut ranges = Vec::new();
        for item in input.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let range = match item.split_once('-') {
                Some((lo, hi)) => {
                    let (lo, hi) = (parse_port(lo)?, parse_port(hi)?);
                    if lo > hi {
                        return Err(invalid(format!("range {item} goes backwards")));
                    }
                    lo..=hi
                }
                None => {
                    let p = parse_port(item)?;
                    p..=p
                }
            };
            ranges.push(range);
        }
        if ranges.is_empty() {
            return Err(invalid("no ports given".into()));
        }
        Ok(PortList(ranges))
    }
}

impl TryFrom<String> for PortList {
    type Error = ScanError;
    fn try_from(value: String) -> Result<Self> {
        value.parse()
    }
}

impl From<PortList> for String {
    fn from(value: PortList) -> Self {
        value.to_string()
    }
}

impl fmt::Display for PortList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, r) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(",")?;
            }
            if r.start() == r.end() {
                write!(f, "{}", r.start())?;
            } else {
                write!(f, "{}-{}", r.start(), r.end())?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::Catalogue;

    #[test]
    fn parses_lists() {
        let list: PortList = " 80, 443 ,8000-8002".parse().unwrap();
        assert_eq!(list.to_string(), "80,443,8000-8002");
        let set = PortSet::List { ports: list };
        assert_eq!(
            set.resolve(Catalogue::builtin()),
            vec![80, 443, 8000, 8001, 8002]
        );
        for bad in ["", "0", "70000", "9000-8000", "http", "80;443"] {
            assert!(bad.parse::<PortList>().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn common_includes_catalogue_ports() {
        let cat = Catalogue::builtin();
        let ports = PortSet::Common.resolve(cat);
        for p in [80, 443, 3000, 8096, 8989, 9999, 32400, 13378] {
            assert!(ports.contains(&p), "{p} missing");
            assert!(PortSet::Common.contains(p, cat));
        }
        assert!(!ports.contains(&22));
        assert!(!PortSet::Common.contains(22, cat));
        assert_eq!(PortSet::Full.resolve(cat).len(), 65535);
    }

    #[test]
    fn serde_round_trip() {
        let set: PortSet =
            serde_json::from_str(r#"{"kind":"list","ports":"80,8000-8010"}"#).unwrap();
        assert_eq!(
            serde_json::to_string(&set).unwrap(),
            r#"{"kind":"list","ports":"80,8000-8010"}"#
        );
        let set: PortSet = serde_json::from_str(r#"{"kind":"full"}"#).unwrap();
        assert_eq!(set, PortSet::Full);
    }
}
