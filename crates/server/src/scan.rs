//! Port scanner endpoints (admin only).
//!
//! * `POST /api/scan` with a `ScanRequest` body (`targets`, `ports`,
//!   `fingerprint`) starts a scan. Only one runs at a time; a second request
//!   gets `409`. Progress goes to admins over `/api/events` as `scan` events.
//! * `GET /api/scan` returns the running or last job: its progress events,
//!   and when finished the report with every service marked `onDashboard`
//!   or not, plus a ready-made card suggestion for each.
//! * `GET /api/scan/history` lists stored reports, newest first.
//!
//! Each finished scan is compared with the previous stored one. The
//! changes (`diff`: services that appeared, disappeared or changed) go into
//! the job and, when there are any, out as a `changes` scan event with one
//! readable line per change. Scheduled rescans live in [`crate::schedule`].
//!
//! Policy comes from the environment: `HOMEDIT_SCAN_ALLOW_PUBLIC=true`
//! allows non-private targets, `HOMEDIT_SCAN_CONCURRENCY` and
//! `HOMEDIT_SCAN_RATE` (connects per second) tune the load. Scans are plain
//! TCP connects with no extra privileges, and every scan is logged (target
//! `scanner::audit` and the `scan_reports` table).

use std::sync::Mutex;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use scanner::{ScanDiff, ScanEvent, ScanPolicy, ScanReport, ScanRequest, Scanner};
use serde::Serialize;
use serde_json::Value;
use sqlx::Row;

use crate::auth::RequireAdmin;
use crate::status::LiveEvent;
use crate::{ApiError, AppState};

/// Progress events kept for `GET /api/scan`.
pub const MAX_EVENTS: usize = 500;

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
}

fn env_parse<T: std::str::FromStr>(name: &str) -> anyhow::Result<Option<T>> {
    match std::env::var(name) {
        Ok(v) if !v.trim().is_empty() => v
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
        let mut p = ScanPolicy::default();
        if let Some(v) = env_parse::<bool>("HOMEDIT_SCAN_ALLOW_PUBLIC")? {
            p.allow_public = v;
        }
        if let Some(v) = env_parse("HOMEDIT_SCAN_CONCURRENCY")? {
            p.max_concurrency = v;
        }
        if let Some(v) = env_parse("HOMEDIT_SCAN_RATE")? {
            p.connects_per_second = v;
        }
        Ok(Self {
            scanner: Scanner::new(p)?,
            job: Mutex::new(Job::default()),
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
    Json(mut req): Json<ScanRequest>,
) -> Result<(StatusCode, Json<Started>), ApiError> {
    req.requested_by = Some(admin.author());
    let id = begin(&state, req)?;
    Ok((StatusCode::ACCEPTED, Json(Started { id })))
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
