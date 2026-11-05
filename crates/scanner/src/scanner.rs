//! Running a scan from request to report.
//!
//! A scan runs in three overlapping steps:
//!
//! 1. **Docker**: when the app gave the scanner a Docker endpoint, the ports
//!    containers publish are listed first. They appear at once, named after
//!    their container and image, and the LAN scan skips them.
//! 2. **Live hosts**: every address from a network range gets a quick check
//!    (a handful of common ports, where even a refused connection proves the
//!    host is up), a reverse DNS lookup and a look at the ARP table. Only
//!    hosts that answer are port-scanned, so a /24 with a dozen machines
//!    costs a dozen port scans, not 254. Addresses and hostnames typed one by
//!    one are always scanned.
//! 3. **Ports**: every port of every live host, with each open port
//!    identified (page title, `Server` header, catalogue checks, then the
//!    app usually on that port) as soon as it is found.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinSet;

use crate::catalogue::Catalogue;
use crate::connect::PortState;
use crate::docker::{self, DockerEndpoint};
use crate::error::{Result, ScanError};
use crate::fingerprint::{Fingerprinter, HttpProber};
use crate::net::{Network, SystemNetwork, short_name};
use crate::policy::{ScanPolicy, canonical};
use crate::ports::PortSet;
use crate::rate::RateLimiter;
use crate::report::{ContainerRef, DiscoveredService, HostResult, ScanReport, Source};
use crate::target::{ResolvedHost, Target, resolve_targets};

/// The `tracing` target of the audit records written for every scan.
/// Route it to the scan log.
pub const AUDIT_TARGET: &str = "homedit_scanner::audit";

/// Ports tried to tell whether a host is up. An open port or a refused
/// connection both prove it is; the ones that answer are not tried again.
pub const DISCOVERY_PORTS: &[u16] = &[
    80, 443, 22, 445, 8080, 53, 139, 5000, 3000, 8443, 8006, 9000, 8123, 32400, 548, 62078,
];

/// How long a reverse DNS lookup may take while finding live hosts.
const REVERSE_DNS_TIMEOUT: Duration = Duration::from_millis(800);

/// Reverse DNS lookups in flight at once.
const REVERSE_DNS_CONCURRENCY: usize = 256;

/// What to scan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanRequest {
    /// Networks, addresses and hostnames.
    pub targets: Vec<Target>,
    /// Ports to check on every address. Defaults to [`PortSet::Common`].
    #[serde(default)]
    pub ports: PortSet,
    /// Identify what answers on open ports (on by default). Off gives a
    /// bare list of open ports, much faster.
    #[serde(default = "yes")]
    pub fingerprint: bool,
    /// Find live hosts first and port-scan only those (on by default). Off
    /// port-scans every address of every range, which is much slower but
    /// also finds hosts that answer on none of the [`DISCOVERY_PORTS`],
    /// have no reverse DNS name and are not in the ARP table.
    #[serde(default = "yes")]
    pub discover: bool,
    /// List the ports Docker containers publish before scanning (on by
    /// default; needs a Docker endpoint, see [`Scanner::with_docker`]).
    #[serde(default = "yes")]
    pub docker: bool,
    /// The address the Docker host's published ports are reached at. When
    /// empty, the scanner's default (see [`Scanner::with_docker`]) is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docker_host: Option<IpAddr>,
    /// Who asked, recorded in the report and the audit log.
    #[serde(default)]
    pub requested_by: Option<String>,
}

fn yes() -> bool {
    true
}

impl ScanRequest {
    /// A request for `targets` with the default ports, fingerprinting,
    /// live-host discovery and Docker listing on.
    pub fn new(targets: Vec<Target>) -> Self {
        Self {
            targets,
            ports: PortSet::Common,
            fingerprint: true,
            discover: true,
            docker: true,
            docker_host: None,
            requested_by: None,
        }
    }
}

/// Which step a scan is in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanPhase {
    /// Finding live hosts.
    Discover,
    /// Checking ports on live hosts.
    #[default]
    Ports,
}

/// Live progress, sent while a scan runs (for a progress bar or a
/// server-sent-events stream).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ScanEvent {
    /// Targets are resolved and the scan begins.
    Started {
        /// Addresses to check.
        addresses: usize,
        /// Ports per address.
        ports: usize,
        /// Most connection attempts the scan could make (every port of
        /// every address); finding live hosts first usually cuts it a lot.
        probes: u64,
    },
    /// Progress, sent about once per percent of each phase. May be dropped
    /// when the receiver is slow.
    Progress {
        /// Steps finished in this phase: addresses checked while finding
        /// hosts, connection attempts while checking ports.
        done: u64,
        /// Steps planned in this phase.
        total: u64,
        /// Open ports found so far.
        open: usize,
        /// Live hosts found so far.
        #[serde(default)]
        hosts: usize,
        /// Apps found so far: open ports that were recognised or answer
        /// over HTTP.
        #[serde(default)]
        apps: usize,
        /// The phase this progress is about.
        #[serde(default)]
        phase: ScanPhase,
    },
    /// A host answered.
    HostUp {
        /// Address.
        ip: IpAddr,
        /// Its container or reverse DNS name, when known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    /// An open port was found (before fingerprinting).
    PortOpen {
        /// Address.
        ip: IpAddr,
        /// Port.
        port: u16,
    },
    /// A service was found or identified. Docker services are sent once
    /// when listed and again once fingerprinted; the later one wins.
    Service {
        /// Address.
        ip: IpAddr,
        /// Hostname, if reached by name.
        hostname: Option<String>,
        /// What was found.
        service: Box<DiscoveredService>,
    },
    /// The scan is done; the report follows as the return value.
    Finished {
        /// Open ports found.
        open: usize,
        /// Of those, how many were identified as a known app.
        identified: usize,
        /// Wall time in milliseconds.
        elapsed_ms: u64,
        /// Live hosts found.
        #[serde(default)]
        hosts: usize,
    },
}

/// A Docker host whose published ports are listed before scanning.
#[derive(Debug, Clone)]
struct DockerSource {
    endpoint: DockerEndpoint,
    host_ip: Option<IpAddr>,
}

/// Scans networks and identifies the apps it finds.
///
/// One `Scanner` can run many scans, one after another or at the same time;
/// each scan gets its own rate limiter. To cancel a scan, drop its future:
/// every in-flight probe is aborted.
#[derive(Debug, Clone)]
pub struct Scanner {
    policy: ScanPolicy,
    catalogue: Arc<Catalogue>,
    net: Arc<dyn Network>,
    own: Arc<HashSet<IpAddr>>,
    docker: Option<DockerSource>,
}

impl Scanner {
    /// A scanner using the built-in app catalogue and the real network.
    /// Fails if the policy is invalid (see [`ScanPolicy::validate`]).
    pub fn new(policy: ScanPolicy) -> Result<Self> {
        policy.validate()?;
        Ok(Self {
            policy,
            catalogue: Arc::new(Catalogue::builtin().clone()),
            net: Arc::new(SystemNetwork),
            own: Arc::new(HashSet::new()),
            docker: None,
        })
    }

    /// Uses `catalogue` instead of the built-in one.
    pub fn with_catalogue(mut self, catalogue: Catalogue) -> Self {
        self.catalogue = Arc::new(catalogue);
        self
    }

    /// Uses `net` instead of the real network (for tests).
    pub fn with_network(mut self, net: Arc<dyn Network>) -> Self {
        self.net = net;
        self
    }

    /// This machine's own addresses: left out when a network range covers
    /// them (typing one in still scans it).
    pub fn with_own_addresses(mut self, own: impl IntoIterator<Item = IpAddr>) -> Self {
        self.own = Arc::new(own.into_iter().map(canonical).collect());
        self
    }

    /// Lists the ports published on the Docker host at `endpoint` at the
    /// start of every scan. They are shown at `host_ip` unless a request
    /// names its own [`ScanRequest::docker_host`]; with neither, only the
    /// container names on Docker networks are used.
    pub fn with_docker(mut self, endpoint: DockerEndpoint, host_ip: Option<IpAddr>) -> Self {
        self.docker = Some(DockerSource {
            endpoint,
            host_ip: host_ip.map(canonical),
        });
        self
    }

    /// The policy this scanner enforces.
    pub fn policy(&self) -> &ScanPolicy {
        &self.policy
    }

    /// The app catalogue in use.
    pub fn catalogue(&self) -> &Catalogue {
        &self.catalogue
    }

    fn fingerprinter(&self, limiter: Arc<RateLimiter>) -> Result<Fingerprinter> {
        let http = HttpProber::new(self.policy.http_timeout())
            .map_err(|e| ScanError::Config(format!("HTTP client: {e}")))?;
        Ok(Fingerprinter::new(http, self.catalogue.clone(), limiter))
    }

    /// The app usually found on `port`, as a hint for services nothing
    /// else identified.
    fn port_guess(&self, port: u16) -> Option<String> {
        self.catalogue
            .apps()
            .iter()
            .find(|a| a.ports.contains(&port))
            .map(|a| a.name.to_string())
    }

    /// Runs a scan.
    ///
    /// Every target is resolved and checked against the policy *before* any
    /// packet is sent; if one is refused, nothing is scanned. Progress goes
    /// to `events` when given. An audit record is written to
    /// [`AUDIT_TARGET`] when the scan starts and when it ends.
    pub async fn scan(
        &self,
        req: ScanRequest,
        events: Option<mpsc::Sender<ScanEvent>>,
    ) -> Result<ScanReport> {
        let started_at_ms = now_ms();
        let clock = tokio::time::Instant::now();
        let resolved = resolve_targets(&req.targets, &self.policy).await?;
        let typed: HashSet<IpAddr> = req
            .targets
            .iter()
            .filter_map(|t| match t {
                Target::Address(ip) => Some(canonical(*ip)),
                _ => None,
            })
            .collect();
        // Addresses typed one by one (or by name) are always scanned; the
        // rest came from ranges and are checked for life first.
        let explicit = |h: &ResolvedHost| h.hostname.is_some() || typed.contains(&h.ip);
        let hosts: Vec<ResolvedHost> = resolved
            .into_iter()
            .filter(|h| explicit(h) || !self.own.contains(&h.ip))
            .collect();
        let ports = req.ports.resolve(&self.catalogue);
        if ports.is_empty() {
            return Err(ScanError::Empty("no ports"));
        }
        let most = hosts.len() as u64 * ports.len() as u64;
        if !req.discover && most > self.policy.max_probes {
            return Err(too_many(most, self.policy.max_probes));
        }

        let targets: Vec<String> = req.targets.iter().map(ToString::to_string).collect();
        tracing::info!(
            target: AUDIT_TARGET,
            requested_by = req.requested_by.as_deref().unwrap_or("-"),
            targets = %targets.join(","),
            ports = ports.len(),
            addresses = hosts.len(),
            "scan started"
        );
        let emit = Emitter(events);
        emit.send(ScanEvent::Started {
            addresses: hosts.len(),
            ports: ports.len(),
            probes: most,
        })
        .await;

        let limiter = Arc::new(RateLimiter::new(self.policy.connects_per_second));
        let fp = if req.fingerprint {
            Some(self.fingerprinter(limiter.clone())?)
        } else {
            None
        };
        let probes = Arc::new(AtomicU64::new(0));
        let run = Run {
            scanner: self,
            emit: emit.clone(),
            limiter,
            sem: Arc::new(Semaphore::new(self.policy.max_concurrency)),
            probes: probes.clone(),
            found: Arc::new(Found::default()),
            identify: Identify {
                fp,
                sem: Arc::new(Semaphore::new(self.policy.fingerprint_concurrency)),
                tasks: JoinSet::new(),
            },
        };
        let (report_hosts, live_count) = run
            .go(&req, &hosts, &ports, explicit_set(&hosts, &typed))
            .await?;

        let open_count: usize = report_hosts.iter().map(|h| h.services.len()).sum();
        let identified = report_hosts
            .iter()
            .flat_map(|h| &h.services)
            .filter(|s| s.app.is_some())
            .count();
        let elapsed_ms = clock.elapsed().as_millis() as u64;
        tracing::info!(
            target: AUDIT_TARGET,
            requested_by = req.requested_by.as_deref().unwrap_or("-"),
            targets = %targets.join(","),
            hosts = live_count,
            open = open_count,
            identified,
            elapsed_ms,
            "scan finished"
        );
        emit.send(ScanEvent::Finished {
            open: open_count,
            identified,
            elapsed_ms,
            hosts: live_count,
        })
        .await;

        Ok(ScanReport {
            requested_by: req.requested_by,
            targets: req.targets,
            ports: req.ports,
            started_at_ms,
            finished_at_ms: now_ms(),
            scanned_addresses: hosts.iter().map(|h| h.ip).collect(),
            probes_sent: probes.load(Ordering::Relaxed),
            hosts: report_hosts,
        })
    }

    /// Fingerprints a single `ip:port`, e.g. to refresh one result or to
    /// check a port typed in by hand. The address must pass the policy.
    pub async fn fingerprint_service(&self, ip: IpAddr, port: u16) -> Result<DiscoveredService> {
        let ip = canonical(ip);
        self.policy.check_ip(ip)?;
        let fp = self.fingerprinter(Arc::new(RateLimiter::new(self.policy.connects_per_second)))?;
        let mut svc = bare(port, Source::Scan);
        enrich(&fp, ip, &mut svc).await;
        if svc.app.is_none() {
            svc.guess = self.port_guess(port);
        }
        Ok(svc)
    }

    /// Lists the ports published by containers on the Docker host at
    /// `host_ip`, reached through `endpoint`, without scanning.
    ///
    /// Apps are recognised from the image name right away. With `fingerprint`
    /// on, each port is also fingerprinted over HTTP (which confirms the app
    /// and fills in title, favicon and version); `host_ip` must then pass the
    /// policy.
    pub async fn docker_services(
        &self,
        endpoint: &DockerEndpoint,
        host_ip: IpAddr,
        hostname: Option<String>,
        fingerprint: bool,
    ) -> Result<HostResult> {
        let host_ip = canonical(host_ip);
        let published = docker::published_ports(endpoint, &self.catalogue).await?;
        let mut host = docker::host_result(host_ip, hostname, &published);
        if !fingerprint || host.services.is_empty() {
            return Ok(host);
        }
        self.policy.check_ip(host_ip)?;
        let fp = self.fingerprinter(Arc::new(RateLimiter::new(self.policy.connects_per_second)))?;
        let sem = Arc::new(Semaphore::new(self.policy.fingerprint_concurrency));
        let mut set = JoinSet::new();
        for (i, svc) in host.services.iter().enumerate() {
            let (fp, sem, mut svc) = (fp.clone(), sem.clone(), svc.clone());
            set.spawn(async move {
                let _permit = sem.acquire_owned().await;
                enrich(&fp, host_ip, &mut svc).await;
                (i, svc)
            });
        }
        while let Some(result) = set.join_next().await {
            if let Ok((i, svc)) = result {
                host.services[i] = svc;
            }
        }
        Ok(host)
    }
}

fn too_many(n: u64, limit: u64) -> ScanError {
    ScanError::TooLarge(format!("{n} probes requested, the limit is {limit}"))
}

fn explicit_set(hosts: &[ResolvedHost], typed: &HashSet<IpAddr>) -> HashSet<IpAddr> {
    hosts
        .iter()
        .filter(|h| h.hostname.is_some() || typed.contains(&h.ip))
        .map(|h| h.ip)
        .collect()
}

/// What a running scan has found so far, shared with its tasks.
#[derive(Default)]
struct Found {
    /// Live hosts and their names.
    hosts: Mutex<BTreeMap<IpAddr, Option<String>>>,
    /// Open ports seen (from Docker, discovery or the port scan).
    open: Mutex<HashSet<(IpAddr, u16)>>,
    /// Services counted as apps.
    apps: Mutex<HashSet<(IpAddr, u16)>>,
    /// Open ports for progress.
    open_count: AtomicUsize,
}

impl Found {
    fn hosts(&self) -> usize {
        lock(&self.hosts).len()
    }

    fn apps(&self) -> usize {
        lock(&self.apps).len()
    }

    /// Marks `ip` live; true the first time.
    fn host_up(&self, ip: IpAddr, name: Option<String>) -> bool {
        let mut h = lock(&self.hosts);
        match h.get_mut(&ip) {
            Some(slot) => {
                if slot.is_none() && name.is_some() {
                    *slot = name;
                }
                false
            }
            None => {
                h.insert(ip, name);
                true
            }
        }
    }

    /// Records an open port; true the first time.
    fn port_open(&self, ip: IpAddr, port: u16) -> bool {
        let new = lock(&self.open).insert((ip, port));
        if new {
            self.open_count.fetch_add(1, Ordering::Relaxed);
        }
        new
    }

    fn note_service(&self, ip: IpAddr, svc: &DiscoveredService) {
        if svc.app.is_some() || svc.http.is_some() {
            lock(&self.apps).insert((ip, svc.port));
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Identifies open ports as they are found, a few at a time.
struct Identify {
    fp: Option<Fingerprinter>,
    sem: Arc<Semaphore>,
    tasks: JoinSet<(IpAddr, DiscoveredService)>,
}

/// One scan in progress.
struct Run<'a> {
    scanner: &'a Scanner,
    emit: Emitter,
    limiter: Arc<RateLimiter>,
    sem: Arc<Semaphore>,
    probes: Arc<AtomicU64>,
    found: Arc<Found>,
    identify: Identify,
}

impl Run<'_> {
    async fn go(
        mut self,
        req: &ScanRequest,
        hosts: &[ResolvedHost],
        ports: &[u16],
        explicit: HashSet<IpAddr>,
    ) -> Result<(Vec<HostResult>, usize)> {
        let hostnames: Arc<HashMap<IpAddr, String>> = Arc::new(
            hosts
                .iter()
                .filter_map(|h| Some((h.ip, h.hostname.clone()?)))
                .collect(),
        );
        let port_set: HashSet<u16> = ports.iter().copied().collect();
        let mut services: BTreeMap<(IpAddr, u16), DiscoveredService> = BTreeMap::new();
        // Container addresses on Docker networks: exact names, and the
        // container behind every port found there.
        let mut containers: HashMap<IpAddr, (ContainerRef, Option<crate::AppMatch>)> =
            HashMap::new();
        // Ports already known (from Docker or the live-host check).
        let mut skip: HashSet<(IpAddr, u16)> = HashSet::new();

        // 1. Docker: published ports first, at once.
        if req.docker
            && let Some(src) = &self.scanner.docker
        {
            match docker::inventory(&src.endpoint, &self.scanner.catalogue).await {
                Ok(inv) => {
                    for a in inv.addresses {
                        let ip = canonical(a.ip);
                        containers.entry(ip).or_insert((a.container, a.app));
                    }
                    if let Some(h) = req.docker_host.map(canonical).or(src.host_ip) {
                        let listed = docker::host_result(h, None, &inv.published);
                        if !listed.services.is_empty() && self.found.host_up(h, None) {
                            self.emit
                                .send(ScanEvent::HostUp { ip: h, name: None })
                                .await;
                        }
                        let can_fingerprint = self.scanner.policy.check_ip(h).is_ok();
                        for svc in listed.services {
                            skip.insert((h, svc.port));
                            self.found.port_open(h, svc.port);
                            self.found.note_service(h, &svc);
                            self.emit
                                .send(ScanEvent::Service {
                                    ip: h,
                                    hostname: hostnames.get(&h).cloned(),
                                    service: Box::new(svc.clone()),
                                })
                                .await;
                            if can_fingerprint && self.identify.fp.is_some() {
                                self.spawn_identify(h, svc, &hostnames);
                            } else {
                                services.insert((h, svc.port), svc);
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "cannot list Docker containers; scanning without them")
                }
            }
        }

        // 2. Live hosts.
        for h in hosts {
            let known = containers.get(&h.ip).map(|c| c.0.name.clone());
            if (explicit.contains(&h.ip) || known.is_some() || !req.discover)
                && self.found.host_up(h.ip, known.clone())
            {
                self.emit
                    .send(ScanEvent::HostUp {
                        ip: h.ip,
                        name: known,
                    })
                    .await;
            }
        }
        let unknown: Vec<IpAddr> = hosts
            .iter()
            .map(|h| h.ip)
            .filter(|ip| !lock(&self.found.hosts).contains_key(ip))
            .collect();
        if req.discover && !unknown.is_empty() {
            let checked = self.discover(&unknown).await;
            for (ip, port, state) in checked {
                if !port_set.contains(&port) {
                    continue;
                }
                skip.insert((ip, port));
                if state == PortState::Open {
                    self.open(ip, port, &containers, &hostnames).await;
                }
            }
        }

        // 3. Ports of live hosts, identified as they open.
        let live: Vec<IpAddr> = lock(&self.found.hosts).keys().copied().collect();
        let in_scope: HashSet<IpAddr> = hosts.iter().map(|h| h.ip).collect();
        // Ports outer, hosts inner: consecutive connections go to different
        // machines instead of hammering one.
        let work: Vec<(IpAddr, u16)> = ports
            .iter()
            .flat_map(|&p| live.iter().map(move |&ip| (ip, p)))
            .filter(|&(ip, p)| in_scope.contains(&ip) && !skip.contains(&(ip, p)))
            .collect();
        let total = work.len() as u64;
        if total > self.scanner.policy.max_probes {
            return Err(too_many(total, self.scanner.policy.max_probes));
        }
        let step = (total / 100).max(1);
        let timeout = self.scanner.policy.connect_timeout();
        let mut set: JoinSet<(IpAddr, u16, PortState)> = JoinSet::new();
        let mut done = 0u64;
        self.progress(0, total, ScanPhase::Ports);
        for (ip, port) in work {
            // The semaphore is never closed, so acquiring cannot fail.
            let Ok(permit) = self.sem.clone().acquire_owned().await else {
                break;
            };
            self.limiter.acquire().await;
            let (net, probes) = (self.scanner.net.clone(), self.probes.clone());
            set.spawn(async move {
                probes.fetch_add(1, Ordering::Relaxed);
                let state = net.probe(SocketAddr::new(ip, port), timeout).await;
                drop(permit);
                (ip, port, state)
            });
            while let Some(r) = set.try_join_next() {
                done += 1;
                if let Ok((ip, port, PortState::Open)) = r {
                    self.open(ip, port, &containers, &hostnames).await;
                }
                if done.is_multiple_of(step) {
                    self.progress(done, total, ScanPhase::Ports);
                }
            }
            while let Some(Ok((ip, svc))) = self.identify.tasks.try_join_next() {
                services.insert((ip, svc.port), svc);
            }
        }
        while let Some(r) = set.join_next().await {
            done += 1;
            if let Ok((ip, port, PortState::Open)) = r {
                self.open(ip, port, &containers, &hostnames).await;
            }
            if done.is_multiple_of(step) || done == total {
                self.progress(done, total, ScanPhase::Ports);
            }
        }
        while let Some(r) = self.identify.tasks.join_next().await {
            match r {
                Ok((ip, svc)) => {
                    services.insert((ip, svc.port), svc);
                }
                Err(e) => tracing::warn!(error = %e, "fingerprint task failed"),
            }
        }

        let live_count = self.found.hosts();
        let names = lock(&self.found.hosts).clone();
        let mut results: Vec<HostResult> = Vec::new();
        for ((ip, _), svc) in services {
            match results.last_mut() {
                Some(h) if h.ip == ip => h.services.push(svc),
                _ => results.push(HostResult {
                    ip,
                    hostname: hostnames.get(&ip).cloned(),
                    name: names.get(&ip).cloned().flatten(),
                    services: vec![svc],
                }),
            }
        }
        Ok((results, live_count))
    }

    /// Finds which of `candidates` are up: a quick connect to a few common
    /// ports, reverse DNS and the ARP table, all at once. Returns every
    /// connect attempt that finished, so the port scan does not repeat it.
    async fn discover(&self, candidates: &[IpAddr]) -> Vec<(IpAddr, u16, PortState)> {
        let total = candidates.len() as u64;
        let step = (total / 100).max(1);
        let timeout = self.scanner.policy.connect_timeout();
        self.progress(0, total, ScanPhase::Discover);

        let mut dns = JoinSet::new();
        let dns_sem = Arc::new(Semaphore::new(REVERSE_DNS_CONCURRENCY));
        for &ip in candidates {
            let (net, sem) = (self.scanner.net.clone(), dns_sem.clone());
            dns.spawn(async move {
                let _permit = sem.acquire_owned().await;
                (ip, net.reverse_lookup(ip, REVERSE_DNS_TIMEOUT).await)
            });
        }
        let mut tcp = JoinSet::new();
        for &ip in candidates {
            let (net, sem, limiter, probes) = (
                self.scanner.net.clone(),
                self.sem.clone(),
                self.limiter.clone(),
                self.probes.clone(),
            );
            tcp.spawn(async move {
                let mut set = JoinSet::new();
                for &port in DISCOVERY_PORTS {
                    let (net, sem, limiter, probes) =
                        (net.clone(), sem.clone(), limiter.clone(), probes.clone());
                    set.spawn(async move {
                        let _permit = sem.acquire_owned().await;
                        limiter.acquire().await;
                        probes.fetch_add(1, Ordering::Relaxed);
                        (port, net.probe(SocketAddr::new(ip, port), timeout).await)
                    });
                }
                // The first answer proves the host is up; the rest are
                // dropped (aborted) and left to the port scan.
                let mut out = Vec::new();
                while let Some(r) = set.join_next().await {
                    let Ok((port, state)) = r else { continue };
                    out.push((port, state));
                    if state != PortState::Filtered {
                        break;
                    }
                }
                (ip, out)
            });
        }

        let mut checked = Vec::new();
        let mut done = 0u64;
        let mut names: HashMap<IpAddr, String> = HashMap::new();
        loop {
            tokio::select! {
                Some(r) = tcp.join_next() => {
                    let Ok((ip, out)) = r else { continue };
                    done += 1;
                    if out.iter().any(|(_, s)| *s != PortState::Filtered) {
                        self.up(ip, names.get(&ip).cloned()).await;
                    }
                    checked.extend(out.into_iter().map(|(p, s)| (ip, p, s)));
                    if done.is_multiple_of(step) || done == total {
                        self.progress(done, total, ScanPhase::Discover);
                    }
                }
                Some(r) = dns.join_next() => {
                    if let Ok((ip, Some(name))) = r {
                        let name = short_name(&name);
                        names.insert(ip, name.clone());
                        self.up(ip, Some(name)).await;
                    }
                }
                else => break,
            }
        }
        // The connects above made the kernel resolve every address on the
        // local link; hosts that answered ARP are up even if they drop TCP.
        let wanted: HashSet<IpAddr> = candidates.iter().copied().collect();
        for ip in self.scanner.net.neighbours().await {
            let ip = canonical(ip);
            if wanted.contains(&ip) {
                self.up(ip, None).await;
            }
        }
        checked
    }

    async fn up(&self, ip: IpAddr, name: Option<String>) {
        if self.found.host_up(ip, name.clone()) {
            self.emit.send(ScanEvent::HostUp { ip, name }).await;
        }
    }

    fn progress(&self, done: u64, total: u64, phase: ScanPhase) {
        self.emit.try_send(ScanEvent::Progress {
            done,
            total,
            open: self.found.open_count.load(Ordering::Relaxed),
            hosts: self.found.hosts(),
            apps: self.found.apps(),
            phase,
        });
    }

    /// Handles an open port: reports it and starts identifying it.
    async fn open(
        &mut self,
        ip: IpAddr,
        port: u16,
        containers: &HashMap<IpAddr, (ContainerRef, Option<crate::AppMatch>)>,
        hostnames: &Arc<HashMap<IpAddr, String>>,
    ) {
        if !self.found.port_open(ip, port) {
            return;
        }
        self.emit.send(ScanEvent::PortOpen { ip, port }).await;
        let mut svc = bare(port, Source::Scan);
        if let Some((c, app)) = containers.get(&ip) {
            svc.container = Some(c.clone());
            svc.app = app.clone();
        }
        self.spawn_identify(ip, svc, hostnames);
    }

    /// Identifies `svc` in the background (or just names it, with
    /// fingerprinting off) and reports it when done.
    fn spawn_identify(
        &mut self,
        ip: IpAddr,
        mut svc: DiscoveredService,
        hostnames: &Arc<HashMap<IpAddr, String>>,
    ) {
        let fp = self.identify.fp.clone();
        let sem = self.identify.sem.clone();
        let (emit, found) = (self.emit.clone(), self.found.clone());
        let guess = self.scanner.port_guess(svc.port);
        let hostname = hostnames.get(&ip).cloned();
        self.identify.tasks.spawn(async move {
            if let Some(fp) = fp {
                let _permit = sem.acquire_owned().await;
                enrich(&fp, ip, &mut svc).await;
            }
            if svc.app.is_none() {
                svc.guess = guess;
            }
            found.note_service(ip, &svc);
            emit.send(ScanEvent::Service {
                ip,
                hostname,
                service: Box::new(svc.clone()),
            })
            .await;
            (ip, svc)
        });
    }
}

/// Fills in `svc` from an HTTP fingerprint of `ip:svc.port`. An HTTP match
/// is stronger evidence than a Docker image name, so it replaces one.
async fn enrich(fp: &Fingerprinter, ip: IpAddr, svc: &mut DiscoveredService) {
    if let Some((http, app)) = fp.fingerprint(ip, svc.port).await {
        svc.http = Some(http);
        if app.is_some() {
            svc.app = app;
        }
    }
}

fn bare(port: u16, source: Source) -> DiscoveredService {
    DiscoveredService {
        port,
        source,
        http: None,
        app: None,
        container: None,
        guess: None,
        on_dashboard: false,
    }
}

/// Wraps the optional event channel. A closed receiver never stops a scan.
#[derive(Clone)]
struct Emitter(Option<mpsc::Sender<ScanEvent>>);

impl Emitter {
    async fn send(&self, event: ScanEvent) {
        if let Some(tx) = &self.0 {
            let _ = tx.send(event).await;
        }
    }

    fn try_send(&self, event: ScanEvent) {
        if let Some(tx) = &self.0 {
            let _ = tx.try_send(event);
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
