//! Scheduled rescans (admin only).
//!
//! * `GET /api/scan/schedule` returns the schedule (disabled by default).
//! * `PUT /api/scan/schedule` with `{enabled, intervalHours, request}`
//!   replaces it. `request` is the same body `POST /api/scan` takes; the
//!   catalogue ports (`{"kind":"catalogue"}`) keep rescans quick.
//!
//! A background task checks once a minute and starts the scan when it is
//! due and no other scan is running. Scheduled scans are recorded as
//! requested by `schedule` and go through the same policy, rate limit and
//! audit log as manual ones. Each finished scan is compared with the
//! previous one (see [`crate::scan`]).

use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use scanner::ScanRequest;
use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::auth::RequireAdmin;
use crate::{ApiError, AppState, scan};

/// Shortest allowed interval.
pub const MIN_HOURS: u32 = 1;
/// Longest allowed interval (30 days).
pub const MAX_HOURS: u32 = 720;
/// How often the background task looks at the schedule.
pub const TICK: Duration = Duration::from_secs(60);
/// `requested_by` of scheduled scans.
pub const AUTHOR: &str = "schedule";

/// The rescan schedule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Schedule {
    /// Whether rescans run.
    pub enabled: bool,
    /// Hours between the start of one rescan and the next.
    pub interval_hours: u32,
    /// What to scan.
    pub request: ScanRequest,
    /// When the last scheduled scan started, ms since the epoch.
    #[serde(default, skip_deserializing)]
    pub last_run_at: Option<i64>,
    /// When the next one is due, ms since the epoch (when enabled).
    #[serde(default, skip_deserializing)]
    pub next_run_at: Option<i64>,
}

impl Default for Schedule {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_hours: 24,
            request: ScanRequest {
                ports: scanner::PortSet::Catalogue,
                ..ScanRequest::new(Vec::new())
            },
            last_run_at: None,
            next_run_at: None,
        }
    }
}

impl Schedule {
    /// When the next run is due, or `None` when disabled. A schedule that
    /// never ran is due right away.
    pub fn next_due(&self) -> Option<i64> {
        if !self.enabled {
            return None;
        }
        Some(match self.last_run_at {
            Some(t) => t + i64::from(self.interval_hours) * 3_600_000,
            None => 0,
        })
    }

    /// Whether a run is due at `now_ms`.
    pub fn is_due(&self, now_ms: i64) -> bool {
        self.next_due().is_some_and(|t| now_ms >= t)
    }

    fn validate(&self) -> Result<(), ApiError> {
        if !(MIN_HOURS..=MAX_HOURS).contains(&self.interval_hours) {
            return Err(ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("intervalHours must be between {MIN_HOURS} and {MAX_HOURS}"),
            ));
        }
        if self.enabled && self.request.targets.is_empty() {
            return Err(ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "add at least one target before turning rescans on",
            ));
        }
        Ok(())
    }

    fn with_next(mut self) -> Self {
        self.next_run_at = self
            .next_due()
            .map(|t| t.max(self.last_run_at.unwrap_or(0)));
        self
    }
}

/// Loads the schedule, or the default when none is saved.
pub async fn load(state: &AppState) -> anyhow::Result<Schedule> {
    let row = sqlx::query(
        "SELECT enabled, interval_hours, request, last_run_at FROM scan_schedule WHERE id = 1",
    )
    .fetch_optional(state.db.pool())
    .await?;
    let Some(r) = row else {
        return Ok(Schedule::default());
    };
    Ok(Schedule {
        enabled: r.get::<i64, _>(0) != 0,
        interval_hours: u32::try_from(r.get::<i64, _>(1)).unwrap_or(24),
        request: serde_json::from_str(r.get::<&str, _>(2))?,
        last_run_at: r.get(3),
        next_run_at: None,
    })
}

async fn save(state: &AppState, s: &Schedule, by: &str) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO scan_schedule (id, enabled, interval_hours, request, last_run_at, updated_by)
         VALUES (1, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET enabled = excluded.enabled,
           interval_hours = excluded.interval_hours, request = excluded.request,
           last_run_at = excluded.last_run_at, updated_by = excluded.updated_by",
    )
    .bind(s.enabled)
    .bind(i64::from(s.interval_hours))
    .bind(serde_json::to_string(&s.request)?)
    .bind(s.last_run_at)
    .bind(by)
    .execute(state.db.pool())
    .await?;
    Ok(())
}

/// `GET /api/scan/schedule`.
pub async fn get(
    RequireAdmin(_): RequireAdmin,
    State(state): State<AppState>,
) -> Result<Json<Schedule>, ApiError> {
    Ok(Json(
        load(&state).await.map_err(ApiError::internal)?.with_next(),
    ))
}

/// `PUT /api/scan/schedule`.
pub async fn put(
    RequireAdmin(admin): RequireAdmin,
    State(state): State<AppState>,
    Json(mut body): Json<Schedule>,
) -> Result<Json<Schedule>, ApiError> {
    body.validate()?;
    let old = load(&state).await.map_err(ApiError::internal)?;
    body.last_run_at = old.last_run_at;
    body.request.requested_by = None;
    save(&state, &body, &admin.author())
        .await
        .map_err(ApiError::internal)?;
    tracing::info!(
        target: scanner::AUDIT_TARGET,
        by = %admin.author(),
        enabled = body.enabled,
        hours = body.interval_hours,
        "scan schedule updated"
    );
    Ok(Json(body.with_next()))
}

/// Starts the scheduled scan if it is due and nothing else is scanning.
/// Returns the job number of a started scan.
pub async fn tick(state: &AppState) -> anyhow::Result<Option<u64>> {
    let mut s = load(state).await?;
    if !s.is_due(homedit_db::now_ms()) || scan::is_running(state) {
        return Ok(None);
    }
    s.last_run_at = Some(homedit_db::now_ms());
    save(state, &s, AUTHOR).await?;
    let mut req = s.request.clone();
    req.requested_by = Some(AUTHOR.into());
    match scan::begin(state, req) {
        Ok(id) => Ok(Some(id)),
        Err(_) => Ok(None),
    }
}

/// Runs [`tick`] every [`TICK`] for the life of the server.
pub fn spawn(state: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut every = tokio::time::interval(TICK);
        every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            every.tick().await;
            match tick(&state).await {
                Ok(Some(id)) => tracing::info!(id, "scheduled rescan started"),
                Ok(None) => {}
                Err(e) => tracing::warn!(error = %e, "scheduled rescan failed"),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn due_rules() {
        let mut s = Schedule::default();
        assert!(!s.is_due(i64::MAX), "disabled never runs");
        s.enabled = true;
        assert!(s.is_due(0), "never ran: due now");
        s.last_run_at = Some(1_000);
        s.interval_hours = 2;
        assert!(!s.is_due(1_000 + 7_199_999));
        assert!(s.is_due(1_000 + 7_200_000));
        assert_eq!(s.clone().with_next().next_run_at, Some(1_000 + 7_200_000));
    }

    #[test]
    fn validation() {
        let mut s = Schedule::default();
        assert!(s.validate().is_ok(), "disabled without targets is fine");
        s.enabled = true;
        assert!(s.validate().is_err(), "enabled needs targets");
        s.request.targets = vec!["10.0.0.0/24".parse().unwrap()];
        assert!(s.validate().is_ok());
        s.interval_hours = 0;
        assert!(s.validate().is_err());
    }
}
