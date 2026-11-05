//! Port scanner endpoints (admin only).
//!
//! * `POST /api/scan` with a `ScanRequest` body (`targets`, `ports`,
//!   `fingerprint`) starts a scan. Only one runs at a time; a second request
//!   gets `409`. Progress goes to admins over `/api/events` as `scan` events.
//! * `GET /api/scan` returns the running or last job: its progress events,
//!   and when finished the report with every service marked `onDashboard`
//!   or not, plus a ready-made card suggestion for each.
//! * `GET /api/scan/history` lists stored reports, newest first.
//! * `GET /api/scan/networks` lists the server's private networks (each
//!   interface's /24, Docker ones marked), the default scan targets.
//!
//! Scans list the Docker host's published ports first when a Docker address
//! is set (`HOMEDIT_DOCKER_HOST`), shown at the server's LAN address or, in a
//! container, at the address the admin opened homedit at. Ranges skip the
//! server's own addresses, and only hosts that answer are port-scanned.
//!
//! Each finished scan is compared with the previous stored one. The
//! changes (`diff`: services that appeared, disappeared or changed) go into
//! the job and, when there are any, out as a `changes` scan event with one
//! readable line per change. Scheduled rescans live in [`crate::schedule`].
//!
//! Policy comes from the environment: `HOMEDIT_SCAN_ALLOW_PUBLIC=true`
//! allows non-private targets, `HOMEDIT_SCAN_CONCURRENCY` (512) and
//! `HOMEDIT_SCAN_RATE` (connects per second, 0 = no limit, the default)
//! tune the load. Scans are plain
//! TCP connects with no extra privileges, and every scan is logged (target
//! `scanner::audit` and the `scan_reports` table).

use std::net::IpAddr;
use std::sync::Mutex;

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use scanner::{
    DockerEndpoint, LocalNetwork, ScanDiff, ScanEvent, ScanPolicy, ScanReport, ScanRequest,
    Scanner, classify,
};
use serde::Serialize;
use serde_json::Value;
use sqlx::Row;

use crate::auth::RequireAdmin;
use crate::status::LiveEvent;
use crate::{ApiError, AppState};

/// Progress events kept for `GET /api/scan`.
pub const MAX_EVENTS: usize = 2_000;

/// State of the current or last scan.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    /// Increasing job number.
    pub id: u64,
    /// Whether the scan is still running.
    pub running: bool,
    /// Progress events, oldest first (at most [`MAX_EVENTS`]).
    pub events: Vec<ScanEvent>,
    /// The finished report.
    pub report: Option<ScanReport>,
    /// Card suggestions for each found service, keyed `ip:port`.
    pub suggestions: serde_json::Map<String, Value>,
    /// Error text when the scan failed.
    pub error: Option<String>,
    /// Changes since the previous stored scan (absent for the first one).
    pub diff: Option<ScanDiff>,
}

/// The scanner and its job slot.
pub struct ScanJobs {
    scanner: Scanner,
    job: Mutex<Job>,
    networks: Vec<LocalNetwork>,
    docker: Option<DockerEndpoint>,
}

fn parse_var<T: std::str::FromStr>(
    get: &impl Fn(&str) -> Option<String>,
    name: &str,
) -> anyhow::Result<Option<T>> {
    match get(name) {
        Some(v) if !v.trim().is_empty() => v
            .trim()
            .parse()
            .map(Some)
            .map_err(|_| anyhow::anyhow!("{name} has an invalid value")),
        _ => Ok(None),
    }
}

impl ScanJobs {
    /// Builds the scanner from the environment.
    pub fn from_env() -> anyhow::Result<Self> {
        Self::from_lookup(|k| std::env::var(k).ok())
    }

    /// Builds the scanner from `HOMEDIT_SCAN_*` values read through `get` (the server settings).
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let mut p = ScanPolicy::default();
        if let Some(v) = parse_var::<bool>(&get, "HOMEDIT_SCAN_ALLOW_PUBLIC")? {
            p.allow_public = v;
        }
        if let Some(v) = parse_var(&get, "HOMEDIT_SCAN_CONCURRENCY")? {
            p.max_concurrency = v;
        }
        if let Some(v) = parse_var(&get, "HOMEDIT_SCAN_RATE")? {
            p.connects_per_second = v;
        }
        let (networks, own) = scanner::local_networks();
        let mut scanner = Scanner::new(p)?.with_own_addresses(own);
        let docker = get("HOMEDIT_DOCKER_HOST")
            .filter(|v| !v.trim().is_empty())
            .and_then(|v| v.parse::<DockerEndpoint>().ok());
        if let Some(ep) = &docker {
            // Published ports are reached at the server's LAN address when it
            // has one (host networking or no container); in a container the
            // request says where (see `docker_host`).
            let lan = networks.iter().find(|n| !n.docker).map(|n| n.address);
            scanner = scanner.with_docker(ep.clone(), lan);
        }
        Ok(Self {
            scanner,
            job: Mutex::new(Job::default()),
            networks,
            docker,
        })
    }

    fn snapshot(&self) -> Job {
        self.job.lock().expect("scan lock poisoned").clone()
    }
}

/// `GET /api/scan`.
pub async fn current(RequireAdmin(_): RequireAdmin, State(state): State<AppState>) -> Json<Job> {
    Json(state.scans.snapshot())
}

/// Response of `POST /api/scan`.
#[derive(Debug, Serialize)]
pub struct Started {
    /// Job number.
    pub id: u64,
}

/// `POST /api/scan`.
pub async fn start(
    RequireAdmin(admin): RequireAdmin,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut req): Json<ScanRequest>,
) -> Result<(StatusCode, Json<Started>), ApiError> {
    req.requested_by = Some(admin.author());
    if req.docker_host.is_none() {
        req.docker_host = docker_host(&headers).await;
    }
    let id = begin(&state, req)?;
    Ok((StatusCode::ACCEPTED, Json(Started { id })))
}

/// The address the admin reached homedit at, when it is a private one: where
/// a Docker host's published ports are reached from the browser. `None`
/// behind a public name or when the server knows its LAN address itself.
pub async fn docker_host(headers: &HeaderMap) -> Option<IpAddr> {
    let host = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get(header::HOST))?
        .to_str()
        .ok()?
        .split(',')
        .next()?
        .trim();
    let name = match host.rsplit_once(':') {
        // `[fd00::1]:3000`, `10.0.1.5:3000`, `nas.lan:3000`; a bare IPv6
        // address has more than one colon and no brackets.
        Some((h, p)) if p.bytes().all(|b| b.is_ascii_digit()) && !h.ends_with(':') => h,
        _ => host,
    };
    let name = name.trim_start_matches('[').trim_end_matches(']');
    let ip = match name.parse::<IpAddr>() {
        Ok(ip) => ip,
        Err(_) => {
            let found = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                tokio::net::lookup_host((name, 0)),
            )
            .await
            .ok()?
            .ok()?;
            found.map(|a| a.ip()).find(IpAddr::is_ipv4)?
        }
    };
    matches!(
        classify(ip),
        scanner::AddressClass::Private | scanner::AddressClass::LinkLocal
    )
    .then_some(ip)
}

/// One network of the server, as `GET /api/scan/networks` lists it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Network {
    /// The range, e.g. `10.0.1.0/24`.
    pub cidr: String,
    /// The interface, e.g. `eth0`.
    pub interface: String,
    /// The server's own address on it (skipped by scans).
    pub address: String,
    /// Whether it is a Docker network.
    pub docker: bool,
}

/// `GET /api/scan/networks`: the server's private networks, LAN first.
pub async fn networks(
    RequireAdmin(_): RequireAdmin,
    State(state): State<AppState>,
) -> Json<Vec<Network>> {
    let jobs = &state.scans;
    // Inside a container every interface is on a Docker network, which
    // only the Docker API can tell.
    let containers: Vec<IpAddr> = match &jobs.docker {
        Some(ep) => scanner::docker::inventory(ep, jobs.scanner.catalogue())
            .await
            .map(|i| i.addresses.into_iter().map(|a| a.ip).collect())
            .unwrap_or_default(),
        None => Vec::new(),
    };
    let mut out: Vec<Network> = jobs
        .networks
        .iter()
        .map(|n| Network {
            cidr: n.net.to_string(),
            interface: n.interface.clone(),
            address: n.address.to_string(),
            docker: n.docker || containers.iter().any(|ip| n.net.contains(ip)),
        })
        .collect();
    out.sort_by_key(|n| n.docker);
    Json(out)
}

/// Whether a scan is running now.
pub fn is_running(state: &AppState) -> bool {
    state.scans.job.lock().expect("scan lock poisoned").running
}

/// Starts `req` in the background and returns its job number, or `409` if
/// a scan is already running. Used by `POST /api/scan` and the scheduler.
pub fn begin(state: &AppState, req: ScanRequest) -> Result<u64, ApiError> {
    if req.targets.is_empty() {
        return Err(ApiError::bad_request("add at least one target"));
    }
    let id = {
        let mut job = state.scans.job.lock().expect("scan lock poisoned");
        if job.running {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "a scan is already running",
            ));
        }
        let id = job.id + 1;
        *job = Job {
            id,
            running: true,
            ..Job::default()
        };
        id
    };
    let st = state.clone();
    tokio::spawn(async move { run(st, id, req).await });
    Ok(id)
}

async fn run(state: AppState, id: u64, req: ScanRequest) {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<ScanEvent>(256);
    let forward = {
        let state = state.clone();
        tokio::spawn(async move {
            while let Some(ev) = rx.recv().await {
                if let Ok(v) = serde_json::to_value(&ev) {
                    state.status.publish(LiveEvent::Scan { id, data: v });
                }
                let mut job = state.scans.job.lock().expect("scan lock poisoned");
                if job.events.len() >= MAX_EVENTS {
                    job.events.remove(0);
                }
                job.events.push(ev);
            }
        })
    };
    let result = state.scans.scanner.scan(req, Some(tx)).await;
    let _ = forward.await;
    let mut out = Job {
        id,
        running: false,
        ..state.scans.snapshot()
    };
    match result {
        Ok(mut report) => {
            if let Ok(dash) = state.db.load_dashboard().await {
                let urls: Vec<String> = dash
                    .services()
                    .flat_map(|s| {
                        let w = s
                            .widget
                            .as_ref()
                            .and_then(|w| w.get("url"))
                            .and_then(Value::as_str)
                            .map(str::to_owned);
                        s.href.clone().into_iter().chain(w)
                    })
                    .collect();
                report.mark_on_dashboard(urls.iter().map(String::as_str));
            }
            for (host, svc) in report.services() {
                let label = host.host_label();
                if let Ok(v) = serde_json::to_value(svc.suggestion(&label)) {
                    out.suggestions
                        .insert(format!("{}:{}", host.ip, svc.port), v);
                }
            }
            state.metrics.record_scan("ok");
            if let Some(prev) = latest(&state).await {
                let d = scanner::diff(&prev, &report, scanner::Catalogue::builtin());
                if !d.is_empty() {
                    let messages = d.messages();
                    for m in &messages {
                        tracing::info!(target: scanner::AUDIT_TARGET, "scan change: {m}");
                    }
                    state.status.publish(LiveEvent::Scan {
                        id,
                        data: serde_json::json!({ "event": "changes", "messages": messages }),
                    });
                }
                out.diff = Some(d);
            }
            if let Err(e) = store(&state, &report).await {
                tracing::warn!(error = %e, "cannot store scan report");
            }
            out.report = Some(report);
        }
        Err(e) => {
            state.metrics.record_scan("error");
            out.error = Some(e.to_string());
        }
    }
    *state.scans.job.lock().expect("scan lock poisoned") = out;
    state.status.publish(LiveEvent::Scan {
        id,
        data: serde_json::json!({ "event": "finished" }),
    });
}

/// The newest stored report.
pub async fn latest(state: &AppState) -> Option<ScanReport> {
    let text: String =
        sqlx::query_scalar("SELECT report FROM scan_reports ORDER BY id DESC LIMIT 1")
            .fetch_optional(state.db.pool())
            .await
            .ok()??;
    serde_json::from_str(&text).ok()
}

async fn store(state: &AppState, report: &ScanReport) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO scan_reports (started_at, finished_at, requested_by, report) VALUES (?, ?, ?, ?)",
    )
    .bind(report.started_at_ms as i64)
    .bind(report.finished_at_ms as i64)
    .bind(&report.requested_by)
    .bind(serde_json::to_string(report)?)
    .execute(state.db.pool())
    .await?;
    Ok(())
}

/// One stored scan.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredScan {
    /// Row id.
    pub id: i64,
    /// Start time, ms since the epoch.
    pub started_at: i64,
    /// End time.
    pub finished_at: i64,
    /// Who ran it.
    pub requested_by: Option<String>,
    /// Number of services found.
    pub services: usize,
}

/// `GET /api/scan/history`.
pub async fn history(
    RequireAdmin(_): RequireAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<StoredScan>>, ApiError> {
    let rows = sqlx::query(
        "SELECT id, started_at, finished_at, requested_by, report FROM scan_reports
         ORDER BY id DESC LIMIT 50",
    )
    .fetch_all(state.db.pool())
    .await
    .map_err(ApiError::internal)?;
    Ok(Json(
        rows.into_iter()
            .map(|r| {
                let report: Option<ScanReport> = serde_json::from_str(r.get::<&str, _>(4)).ok();
                StoredScan {
                    id: r.get(0),
                    started_at: r.get(1),
                    finished_at: r.get(2),
                    requested_by: r.get(3),
                    services: report.map_or(0, |r| r.services().count()),
                }
            })
            .collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(v: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::HOST, v.parse().unwrap());
        h
    }

    #[tokio::test]
    async fn docker_host_from_the_address_used() {
        let ip = |s: &str| Some(s.parse::<IpAddr>().unwrap());
        assert_eq!(docker_host(&host("10.0.1.5:3000")).await, ip("10.0.1.5"));
        assert_eq!(docker_host(&host("192.168.1.2")).await, ip("192.168.1.2"));
        assert_eq!(docker_host(&host("[fd00::5]:3000")).await, ip("fd00::5"));
        // Public and loopback addresses are no use to a LAN browser.
        assert_eq!(docker_host(&host("8.8.8.8:3000")).await, None);
        assert_eq!(docker_host(&host("127.0.0.1:3000")).await, None);
        let mut h = host("127.0.0.1:3000");
        h.insert("x-forwarded-host", "10.0.1.9:443".parse().unwrap());
        assert_eq!(docker_host(&h).await, ip("10.0.1.9"));
    }
}
