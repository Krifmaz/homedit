//! Results and timing on a simulated /24.
//!
//! A fake [`Network`] stands in for a home LAN: a router, a NAS, a Docker
//! host with a reverse DNS name, a desktop with every port closed, a
//! firewalled server that only answers ARP, a firewalled box known only to
//! DNS, and 248 empty addresses that time out. The clock is paused, so the
//! test measures the scan's own timing (connect timeouts, concurrency)
//! without waiting for it.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use scanner::connect::PortState;
use scanner::net::BoxFuture;
use scanner::{
    Network, PortList, PortSet, ScanEvent, ScanPhase, ScanPolicy, ScanReport, ScanRequest, Scanner,
};
use tokio::time::Instant;

/// Round trip to a host that is up.
const RTT: Duration = Duration::from_millis(2);

#[derive(Debug, Clone, Default)]
struct Host {
    open: HashSet<u16>,
    /// Drops packets to closed ports instead of refusing them.
    firewalled: bool,
    /// Shows up in the ARP table.
    arp: bool,
    /// Its reverse DNS name.
    ptr: Option<&'static str>,
}

#[derive(Debug, Default)]
struct FakeLan {
    hosts: HashMap<IpAddr, Host>,
    probes: AtomicU64,
}

impl FakeLan {
    fn host(mut self, last: u8, h: Host) -> Self {
        self.hosts.insert(ip(last), h);
        self
    }
}

fn ip(last: u8) -> IpAddr {
    format!("10.0.1.{last}").parse().unwrap()
}

fn open(ports: &[u16]) -> HashSet<u16> {
    ports.iter().copied().collect()
}

impl Network for FakeLan {
    fn probe(&self, addr: SocketAddr, timeout: Duration) -> BoxFuture<PortState> {
        self.probes.fetch_add(1, Ordering::Relaxed);
        let host = self.hosts.get(&addr.ip()).cloned();
        Box::pin(async move {
            match host {
                Some(h) if h.open.contains(&addr.port()) => {
                    tokio::time::sleep(RTT).await;
                    PortState::Open
                }
                Some(h) if !h.firewalled => {
                    tokio::time::sleep(RTT).await;
                    PortState::Closed
                }
                // Firewalled ports and empty addresses never answer.
                _ => {
                    tokio::time::sleep(timeout).await;
                    PortState::Filtered
                }
            }
        })
    }

    fn reverse_lookup(&self, ip: IpAddr, _timeout: Duration) -> BoxFuture<Option<String>> {
        let name = self.hosts.get(&ip).and_then(|h| h.ptr).map(str::to_string);
        Box::pin(async move {
            tokio::time::sleep(Duration::from_millis(5)).await;
            name
        })
    }

    fn neighbours(&self) -> BoxFuture<Vec<IpAddr>> {
        let list = self
            .hosts
            .iter()
            .filter(|(_, h)| h.arp)
            .map(|(ip, _)| *ip)
            .collect();
        Box::pin(async move { list })
    }
}

fn lan() -> FakeLan {
    FakeLan::default()
        // Router.
        .host(
            1,
            Host {
                open: open(&[53, 80, 443]),
                arp: true,
                ..Host::default()
            },
        )
        // This server: skipped when it is in a range.
        .host(
            5,
            Host {
                open: open(&[3000]),
                arp: true,
                ..Host::default()
            },
        )
        // NAS with Jellyfin.
        .host(
            10,
            Host {
                open: open(&[5000, 8096]),
                arp: true,
                ..Host::default()
            },
        )
        // Firewalled server: no discovery port answers, only ARP finds it.
        .host(
            20,
            Host {
                open: open(&[8989]),
                firewalled: true,
                arp: true,
                ..Host::default()
            },
        )
        // Desktop: up, nothing listening.
        .host(
            30,
            Host {
                arp: true,
                ..Host::default()
            },
        )
        // Docker host, named in DNS.
        .host(
            40,
            Host {
                open: open(&[3001, 9000]),
                arp: true,
                ptr: Some("docker.lan."),
                ..Host::default()
            },
        )
        // Behind a router hop: no ARP, firewalled, known only to DNS.
        .host(
            50,
            Host {
                open: open(&[7878]),
                firewalled: true,
                ptr: Some("media.lan"),
                ..Host::default()
            },
        )
}

/// What the fake LAN really has on `ports`, without this server.
fn truth(ports: &[u16]) -> BTreeSet<(IpAddr, u16)> {
    let lan = lan();
    let mut out = BTreeSet::new();
    for (ip, h) in &lan.hosts {
        if *ip == self_ip() {
            continue;
        }
        for p in &h.open {
            if ports.contains(p) {
                out.insert((*ip, *p));
            }
        }
    }
    out
}

fn self_ip() -> IpAddr {
    ip(5)
}

fn scanner(net: Arc<FakeLan>) -> Scanner {
    Scanner::new(ScanPolicy::default())
        .unwrap()
        .with_network(net)
        .with_own_addresses([self_ip()])
}

fn request(targets: &[&str], ports: PortSet, discover: bool) -> ScanRequest {
    let mut req = ScanRequest::new(targets.iter().map(|t| t.parse().unwrap()).collect());
    req.ports = ports;
    req.fingerprint = false;
    req.discover = discover;
    req
}

fn found(report: &ScanReport) -> BTreeSet<(IpAddr, u16)> {
    report.services().map(|(h, s)| (h.ip, s.port)).collect()
}

fn list(text: &str) -> PortSet {
    PortSet::List {
        ports: text.parse::<PortList>().unwrap(),
    }
}

#[tokio::test(start_paused = true)]
async fn live_hosts_first_matches_a_full_scan_and_is_much_faster() {
    let text = "22,53,80,443,3000-3100,5000,7878,8096,8989,9000";
    let ports: Vec<u16> = list(text).resolve(scanner::Catalogue::builtin());

    let net = Arc::new(lan());
    let start = Instant::now();
    let full = scanner(net.clone())
        .scan(request(&["10.0.1.0/24"], list(text), false), None)
        .await
        .unwrap();
    let full_time = start.elapsed();
    let full_probes = net.probes.load(Ordering::Relaxed);

    let net = Arc::new(lan());
    let start = Instant::now();
    let fast = scanner(net.clone())
        .scan(request(&["10.0.1.0/24"], list(text), true), None)
        .await
        .unwrap();
    let fast_time = start.elapsed();
    let fast_probes = net.probes.load(Ordering::Relaxed);

    assert_eq!(
        found(&full),
        truth(&ports),
        "the full scan finds everything"
    );
    assert_eq!(found(&fast), found(&full), "same results either way");
    assert_eq!(fast.probes_sent, fast_probes);
    assert!(
        fast_time * 4 < full_time,
        "live hosts first: {fast_time:?}, full scan: {full_time:?}"
    );
    eprintln!(
        "live hosts first: {fast_time:?}, {fast_probes} probes; full scan: {full_time:?}, {full_probes} probes"
    );
    assert!(
        fast_probes * 5 < full_probes,
        "{fast_probes} probes against {full_probes}"
    );
    // The server itself is left out of the range.
    assert!(!found(&fast).iter().any(|(ip, _)| *ip == self_ip()));
    assert_eq!(fast.scanned_addresses.len(), 253);
}

#[tokio::test(start_paused = true)]
async fn default_ports_on_a_full_24_finish_in_under_30_seconds() {
    let net = Arc::new(lan());
    let (tx, mut rx) = tokio::sync::mpsc::channel(100_000);
    let start = Instant::now();
    let report = scanner(net.clone())
        .scan(request(&["10.0.1.0/24"], PortSet::Common, true), Some(tx))
        .await
        .unwrap();
    let took = start.elapsed();
    eprintln!(
        "default ports on a /24: {took:?}, {} probes",
        report.probes_sent
    );
    assert!(took < Duration::from_secs(30), "took {took:?}");

    let ports = PortSet::Common.resolve(scanner::Catalogue::builtin());
    assert_eq!(found(&report), truth(&ports));

    let mut events = Vec::new();
    while let Ok(e) = rx.try_recv() {
        events.push(e);
    }
    // Every live host was announced, the DNS name included.
    let up: HashMap<IpAddr, Option<String>> = events
        .iter()
        .filter_map(|e| match e {
            ScanEvent::HostUp { ip, name } => Some((*ip, name.clone())),
            _ => None,
        })
        .collect();
    let mut ups: Vec<u8> = up
        .keys()
        .map(|ip| match ip {
            IpAddr::V4(v4) => v4.octets()[3],
            IpAddr::V6(_) => 0,
        })
        .collect();
    ups.sort();
    assert_eq!(ups, vec![1, 10, 20, 30, 40, 50]);
    assert_eq!(up[&ip(40)].as_deref(), Some("docker.lan"));
    let host40 = report.hosts.iter().find(|h| h.ip == ip(40)).unwrap();
    assert_eq!(host40.name.as_deref(), Some("docker.lan"));
    // Links keep the address.
    assert_eq!(host40.hostname, None);

    // Progress counts hosts and apps for both phases.
    let last_discover = events.iter().rev().find_map(|e| match e {
        ScanEvent::Progress {
            phase: ScanPhase::Discover,
            hosts,
            done,
            total,
            ..
        } => Some((*hosts, *done, *total)),
        _ => None,
    });
    let (hosts, done, total) = last_discover.expect("discovery progress");
    assert_eq!((done, total), (253, 253));
    assert!(hosts >= 4, "{hosts}");
    assert!(events.iter().any(|e| matches!(
        e,
        ScanEvent::Progress {
            phase: ScanPhase::Ports,
            hosts: 6,
            ..
        }
    )));
    assert!(matches!(
        events.last(),
        Some(ScanEvent::Finished { hosts: 6, .. })
    ));

    // Unidentified ports are named after the app usually on them.
    let sonarr = report
        .services()
        .find(|(h, s)| h.ip == ip(20) && s.port == 8989)
        .unwrap()
        .1;
    assert_eq!(sonarr.guess.as_deref(), Some("Sonarr"));
    assert_eq!(sonarr.display_name(), "Sonarr");
}

#[tokio::test(start_paused = true)]
async fn typed_addresses_are_always_scanned() {
    let net = Arc::new(lan());
    // Typing the server's own address scans it; an empty address is
    // scanned too (and finds nothing), without a live-host check.
    let report = scanner(net.clone())
        .scan(
            request(&["10.0.1.5", "10.0.1.99"], list("3000,8096"), true),
            None,
        )
        .await
        .unwrap();
    assert_eq!(found(&report), BTreeSet::from([(ip(5), 3000)]));
    assert_eq!(report.probes_sent, 4);
}
