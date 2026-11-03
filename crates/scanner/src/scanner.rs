//! Running a scan from request to report.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinSet;

use crate::catalogue::Catalogue;
use crate::connect::{self, PortState};
use crate::docker::{self, DockerEndpoint};
use crate::error::{Result, ScanError};
use crate::fingerprint::{Fingerprinter, HttpProber};
use crate::policy::{ScanPolicy, canonical};
use crate::ports::PortSet;
use crate::rate::RateLimiter;
use crate::report::{DiscoveredService, HostResult, ScanReport, Source};
use crate::target::{ResolvedHost, Target, resolve_targets};

/// The `tracing` target of the audit records written for every scan.
/// Route it to the scan log.
pub const AUDIT_TARGET: &str = "homedit_scanner::audit";

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
    /// Who asked, recorded in the report and the audit log.
    #[serde(default)]
    pub requested_by: Option<String>,
}

fn yes() -> bool {
    true
}

impl ScanRequest {
    /// A request for `targets` with the default ports and fingerprinting on.
    pub fn new(targets: Vec<Target>) -> Self {
        Self {
            targets,
            ports: PortSet::Common,
            fingerprint: true,
            requested_by: None,
        }
    }
}

/// Live progress, sent while a scan runs (for a progress bar or a
/// server-sent-events stream).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ScanEvent {
    /// Targets are resolved and the port scan begins.
    Started {
        /// Addresses to check.
        addresses: usize,
        /// Ports per address.
        ports: usize,
        /// Total connection attempts planned.
        probes: u64,
    },
    /// Connect-phase progress, sent about once per percent. May be dropped
    /// when the receiver is slow.
    Progress {
        /// Attempts finished.
        done: u64,
        /// Attempts planned.
        total: u64,
        /// Open ports found so far.
        open: usize,
    },
    /// An open port was found (before fingerprinting).
    PortOpen {
        /// Address.
        ip: IpAddr,
        /// Port.
        port: u16,
    },
    /// A service was fingerprinted.
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
    },
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
}

impl Scanner {
    /// A scanner using the built-in app catalogue. Fails if the policy is
    /// invalid (see [`ScanPolicy::validate`]).
    pub fn new(policy: ScanPolicy) -> Result<Self> {
        policy.validate()?;
        Ok(Self {
            policy,
            catalogue: Arc::new(Catalogue::builtin().clone()),
        })
    }

    /// Uses `catalogue` instead of the built-in one.
    pub fn with_catalogue(mut self, catalogue: Catalogue) -> Self {
        self.catalogue = Arc::new(catalogue);
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
        let clock = std::time::Instant::now();
        let hosts = resolve_targets(&req.targets, &self.policy).await?;
        let ports = req.ports.resolve(&self.catalogue);
        if ports.is_empty() {
            return Err(ScanError::Empty("no ports"));
        }
        let total = hosts.len() as u64 * ports.len() as u64;
        if total > self.policy.max_probes {
            return Err(ScanError::TooLarge(format!(
                "{total} probes requested, the limit is {}",
                self.policy.max_probes
            )));
        }

        let targets: Vec<String> = req.targets.iter().map(ToString::to_string).collect();
        tracing::info!(
            target: AUDIT_TARGET,
            requested_by = req.requested_by.as_deref().unwrap_or("-"),
            targets = %targets.join(","),
            ports = ports.len(),
            addresses = hosts.len(),
            probes = total,
            "scan started"
        );
        let emit = Emitter(events);
        emit.send(ScanEvent::Started {
            addresses: hosts.len(),
            ports: ports.len(),
            probes: total,
        })
        .await;

        let limiter = Arc::new(RateLimiter::new(self.policy.connects_per_second));
        let open = self
            .connect_phase(&hosts, &ports, total, &limiter, &emit)
            .await;
        let results = self
            .identify_phase(&hosts, open, req.fingerprint, limiter, &emit)
            .await?;

        let open_count: usize = results.iter().map(|h| h.services.len()).sum();
        let identified = results
            .iter()
            .flat_map(|h| &h.services)
            .filter(|s| s.app.is_some())
            .count();
        let elapsed_ms = clock.elapsed().as_millis() as u64;
        tracing::info!(
            target: AUDIT_TARGET,
            requested_by = req.requested_by.as_deref().unwrap_or("-"),
            targets = %targets.join(","),
            open = open_count,
            identified,
            elapsed_ms,
            "scan finished"
        );
        emit.send(ScanEvent::Finished {
            open: open_count,
            identified,
            elapsed_ms,
        })
        .await;

        Ok(ScanReport {
            requested_by: req.requested_by,
            targets: req.targets,
            ports: req.ports,
            started_at_ms,
            finished_at_ms: now_ms(),
            scanned_addresses: hosts.iter().map(|h| h.ip).collect(),
            probes_sent: total,
            hosts: results,
        })
    }

    /// Tries every `(host, port)` pair and returns the open ones.
    ///
    /// Ports are the outer loop so consecutive connections go to different
    /// hosts, spreading the load instead of hammering one machine.
    async fn connect_phase(
        &self,
        hosts: &[ResolvedHost],
        ports: &[u16],
        total: u64,
        limiter: &RateLimiter,
        emit: &Emitter,
    ) -> Vec<(IpAddr, u16)> {
        let sem = Arc::new(Semaphore::new(self.policy.max_concurrency));
        let timeout = self.policy.connect_timeout();
        let step = (total / 100).max(1);
        let mut set: JoinSet<(IpAddr, u16, PortState)> = JoinSet::new();
        let mut open = Vec::new();
        let mut done = 0u64;

        let mut record =
            |result: std::result::Result<(IpAddr, u16, PortState), tokio::task::JoinError>,
             open: &mut Vec<(IpAddr, u16)>|
             -> Option<ScanEvent> {
                done += 1;
                let mut event = None;
                match result {
                    Ok((ip, port, PortState::Open)) => {
                        open.push((ip, port));
                        event = Some(ScanEvent::PortOpen { ip, port });
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!(error = %e, "probe task failed"),
                }
                if done % step == 0 || done == total {
                    emit.progress(done, total, open.len());
                }
                event
            };

        for &port in ports {
            for host in hosts {
                // The semaphore is never closed, so acquiring cannot fail.
                let Ok(permit) = sem.clone().acquire_owned().await else {
                    break;
                };
                limiter.acquire().await;
                let ip = host.ip;
                set.spawn(async move {
                    let state = connect::probe(SocketAddr::new(ip, port), timeout).await;
                    drop(permit);
                    (ip, port, state)
                });
                while let Some(result) = set.try_join_next() {
                    if let Some(ev) = record(result, &mut open) {
                        emit.send(ev).await;
                    }
                }
            }
        }
        while let Some(result) = set.join_next().await {
            if let Some(ev) = record(result, &mut open) {
                emit.send(ev).await;
            }
        }
        open.sort();
        open
    }

    /// Fingerprints the open ports and groups them by host.
    async fn identify_phase(
        &self,
        hosts: &[ResolvedHost],
        open: Vec<(IpAddr, u16)>,
        fingerprint: bool,
        limiter: Arc<RateLimiter>,
        emit: &Emitter,
    ) -> Result<Vec<HostResult>> {
        let hostname_of = |ip: IpAddr| {
            hosts
                .iter()
                .find(|h| h.ip == ip)
                .and_then(|h| h.hostname.clone())
        };
        let mut found: Vec<(IpAddr, DiscoveredService)> = Vec::with_capacity(open.len());

        if fingerprint && !open.is_empty() {
            let fp = self.fingerprinter(limiter)?;
            let sem = Arc::new(Semaphore::new(self.policy.fingerprint_concurrency));
            let mut set = JoinSet::new();
            for (ip, port) in open {
                let (fp, sem) = (fp.clone(), sem.clone());
                set.spawn(async move {
                    let _permit = sem.acquire_owned().await;
                    (ip, describe(&fp, ip, port, Source::Scan).await)
                });
            }
            while let Some(result) = set.join_next().await {
                match result {
                    Ok((ip, service)) => {
                        emit.send(ScanEvent::Service {
                            ip,
                            hostname: hostname_of(ip),
                            service: Box::new(service.clone()),
                        })
                        .await;
                        found.push((ip, service));
                    }
                    Err(e) => tracing::warn!(error = %e, "fingerprint task failed"),
                }
            }
        } else {
            for (ip, port) in open {
                found.push((ip, bare(port, Source::Scan)));
            }
        }

        let mut results: Vec<HostResult> = Vec::new();
        found.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.port.cmp(&b.1.port)));
        for (ip, service) in found {
            match results.last_mut() {
                Some(h) if h.ip == ip => h.services.push(service),
                _ => results.push(HostResult {
                    ip,
                    hostname: hostname_of(ip),
                    services: vec![service],
                }),
            }
        }
        Ok(results)
    }

    /// Fingerprints a single `ip:port`, e.g. to refresh one result or to
    /// check a port typed in by hand. The address must pass the policy.
    pub async fn fingerprint_service(&self, ip: IpAddr, port: u16) -> Result<DiscoveredService> {
        let ip = canonical(ip);
        self.policy.check_ip(ip)?;
        let fp = self.fingerprinter(Arc::new(RateLimiter::new(self.policy.connects_per_second)))?;
        Ok(describe(&fp, ip, port, Source::Scan).await)
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
            let (fp, sem, port) = (fp.clone(), sem.clone(), svc.port);
            set.spawn(async move {
                let _permit = sem.acquire_owned().await;
                (i, fp.fingerprint(host_ip, port).await)
            });
        }
        while let Some(result) = set.join_next().await {
            let Ok((i, Some((http, app)))) = result else {
                continue;
            };
            let svc = &mut host.services[i];
            svc.http = Some(http);
            // An HTTP match is stronger evidence than the image name.
            if app.is_some() {
                svc.app = app;
            }
        }
        Ok(host)
    }
}

async fn describe(fp: &Fingerprinter, ip: IpAddr, port: u16, source: Source) -> DiscoveredService {
    match fp.fingerprint(ip, port).await {
        Some((http, app)) => DiscoveredService {
            port,
            source,
            http: Some(http),
            app,
            container: None,
            on_dashboard: false,
        },
        None => bare(port, source),
    }
}

fn bare(port: u16, source: Source) -> DiscoveredService {
    DiscoveredService {
        port,
        source,
        http: None,
        app: None,
        container: None,
        on_dashboard: false,
    }
}

/// Wraps the optional event channel. A closed receiver never stops a scan.
struct Emitter(Option<mpsc::Sender<ScanEvent>>);

impl Emitter {
    async fn send(&self, event: ScanEvent) {
        if let Some(tx) = &self.0 {
            let _ = tx.send(event).await;
        }
    }

    fn progress(&self, done: u64, total: u64, open: usize) {
        if let Some(tx) = &self.0 {
            let _ = tx.try_send(ScanEvent::Progress { done, total, open });
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
