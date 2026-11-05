//! Comparing two scans, for scheduled rescans.

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::net::IpAddr;

use serde::{Deserialize, Serialize};

use crate::catalogue::Catalogue;
use crate::report::{DiscoveredService, ScanReport};

/// One service in a [`ScanDiff`], with the host it lives on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceChange {
    /// The host's address.
    pub ip: IpAddr,
    /// The host's name, if it was reached by name.
    pub hostname: Option<String>,
    /// The service as seen in the scan that has it.
    pub service: DiscoveredService,
}

impl ServiceChange {
    fn label(&self) -> String {
        let host = self.hostname.clone().unwrap_or_else(|| self.ip.to_string());
        let host = if host.contains(':') {
            format!("[{host}]")
        } else {
            host
        };
        format!("{}:{}", host, self.service.port)
    }
}

/// What changed between two scans.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanDiff {
    /// Services in the new scan that the old one did not have.
    pub appeared: Vec<ServiceChange>,
    /// Services the old scan had that stopped answering. Only ports the new
    /// scan actually checked count, so a smaller rescan never reports
    /// services outside its scope as gone.
    pub disappeared: Vec<ServiceChange>,
    /// Services on the same address and port that are now identified as a
    /// different app (the new state is recorded).
    pub changed: Vec<ServiceChange>,
}

impl ScanDiff {
    /// Whether nothing changed.
    pub fn is_empty(&self) -> bool {
        self.appeared.is_empty() && self.disappeared.is_empty() && self.changed.is_empty()
    }

    /// One readable line per change, e.g. `Jellyfin found on 10.0.1.7:8096`
    /// or `Sonarr on 10.0.1.7:8989 stopped answering`. Suitable for
    /// notifications.
    pub fn messages(&self) -> Vec<String> {
        let mut out = Vec::new();
        for c in &self.appeared {
            out.push(format!(
                "{} found on {}",
                c.service.display_name(),
                c.label()
            ));
        }
        for c in &self.disappeared {
            out.push(format!(
                "{} on {} stopped answering",
                c.service.display_name(),
                c.label()
            ));
        }
        for c in &self.changed {
            out.push(format!("{} is now {}", c.label(), c.service.display_name()));
        }
        out
    }
}

impl fmt::Display for ScanDiff {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.messages().join("\n"))
    }
}

/// Compares `previous` and `current`.
///
/// A service is identified by address and port. It counts as disappeared
/// only if `current` scanned that address and that port (checked against the
/// current scan's port set and `catalogue`, which [`PortSet::Common`]
/// depends on).
///
/// [`PortSet::Common`]: crate::PortSet::Common
pub fn diff(previous: &ScanReport, current: &ScanReport, catalogue: &Catalogue) -> ScanDiff {
    let index = |r: &ScanReport| -> BTreeMap<(IpAddr, u16), ServiceChange> {
        r.services()
            .map(|(h, s)| {
                (
                    (h.ip, s.port),
                    ServiceChange {
                        ip: h.ip,
                        hostname: h.hostname.clone(),
                        service: s.clone(),
                    },
                )
            })
            .collect()
    };
    let old = index(previous);
    let new = index(current);
    let scanned: HashSet<IpAddr> = current.scanned_addresses.iter().copied().collect();

    let mut out = ScanDiff::default();
    for (key, change) in &new {
        match old.get(key) {
            None => out.appeared.push(change.clone()),
            Some(before) => {
                let id = |c: &ServiceChange| c.service.app.as_ref().map(|a| a.id.clone());
                // Only report a change when both scans recognised an app;
                // a match that comes and goes is noise, not news.
                if id(before).is_some() && id(change).is_some() && id(before) != id(change) {
                    out.changed.push(change.clone());
                }
            }
        }
    }
    for (key, change) in &old {
        let (ip, port) = *key;
        if !new.contains_key(key)
            && scanned.contains(&ip)
            && current.ports.contains(port, catalogue)
        {
            out.disappeared.push(change.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fingerprint::AppMatch;
    use crate::ports::{PortList, PortSet};
    use crate::report::{HostResult, Source};

    fn app(id: &str, name: &str) -> AppMatch {
        AppMatch {
            id: id.into(),
            name: name.into(),
            widget: Some(id.into()),
            icon: id.into(),
            version: None,
            confidence: 90,
            evidence: vec![],
        }
    }

    fn svc(port: u16, a: Option<AppMatch>) -> DiscoveredService {
        DiscoveredService {
            port,
            source: Source::Scan,
            http: None,
            app: a,
            container: None,
            guess: None,
            on_dashboard: false,
        }
    }

    fn report(
        ports: PortSet,
        addrs: &[&str],
        hosts: Vec<(&str, Vec<DiscoveredService>)>,
    ) -> ScanReport {
        ScanReport {
            requested_by: None,
            targets: vec![],
            ports,
            started_at_ms: 0,
            finished_at_ms: 0,
            scanned_addresses: addrs.iter().map(|a| a.parse().unwrap()).collect(),
            probes_sent: 0,
            hosts: hosts
                .into_iter()
                .map(|(ip, services)| HostResult {
                    ip: ip.parse().unwrap(),
                    hostname: None,
                    name: None,
                    services,
                })
                .collect(),
        }
    }

    #[test]
    fn finds_new_gone_and_changed() {
        let cat = Catalogue::builtin();
        let before = report(
            PortSet::Common,
            &["10.0.1.7", "10.0.1.8"],
            vec![
                (
                    "10.0.1.7",
                    vec![
                        svc(8989, Some(app("sonarr", "Sonarr"))),
                        svc(5055, Some(app("overseerr", "Overseerr"))),
                    ],
                ),
                ("10.0.1.8", vec![svc(8080, None)]),
            ],
        );
        let after = report(
            PortSet::Common,
            &["10.0.1.7"],
            vec![(
                "10.0.1.7",
                vec![
                    svc(8096, Some(app("jellyfin", "Jellyfin"))),
                    svc(5055, Some(app("seerr", "Seerr"))),
                ],
            )],
        );
        let d = diff(&before, &after, cat);
        assert_eq!(
            d.messages(),
            vec![
                "Jellyfin found on 10.0.1.7:8096".to_string(),
                "Sonarr on 10.0.1.7:8989 stopped answering".to_string(),
                "10.0.1.7:5055 is now Seerr".to_string(),
            ]
        );
        // 10.0.1.8 was not scanned this time, so its service is not "gone".
        assert!(d.disappeared.iter().all(|c| c.ip.to_string() != "10.0.1.8"));
    }

    #[test]
    fn out_of_scope_ports_are_not_gone() {
        let cat = Catalogue::builtin();
        let before = report(
            PortSet::Full,
            &["10.0.0.2"],
            vec![("10.0.0.2", vec![svc(22, None)])],
        );
        let ports: PortList = "80,443".parse().unwrap();
        let after = report(PortSet::List { ports }, &["10.0.0.2"], vec![]);
        assert!(diff(&before, &after, cat).is_empty());
    }
}
