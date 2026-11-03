//! Status checks: the background checker, the in-memory cache, and live
//! events.
//!
//! [`spawn_checker`] runs forever. Every `statusIntervalSecs` (default 30)
//! it loads the dashboard, works out each service's [`StatusCheck`], runs
//! the probes with bounded concurrency, stores the samples, and publishes
//! them on the [`StatusHub`] broadcast channel, which feeds the SSE stream.
//!
//! Probes are made by the server only. HTTP checks send a plain `GET`
//! without credentials, do not follow redirects, and accept self-signed
//! certificates (common on a LAN; only the status code is read). A service
//! counts as up when it answers with a status below 500. TCP checks open a
//! connection and close it. No raw sockets are used, so no extra
//! privileges are needed.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::StreamExt;
use homedit_core::{StatusCheck, StatusSample};
use homedit_db::now_ms;
use serde::Serialize;
use tokio::sync::broadcast;

use crate::AppState;

/// Samples kept per service for the sparkline.
pub const HISTORY_LEN: usize = 30;
/// Time allowed for one probe.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// Probes running at the same time.
pub const MAX_CONCURRENT_PROBES: usize = 16;
/// How long samples are kept in the database.
pub const RETENTION: Duration = Duration::from_secs(7 * 24 * 3600);
/// Shortest allowed check interval.
pub const MIN_INTERVAL: Duration = Duration::from_secs(5);

/// One point of a service's history, as sent to the browser.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HistoryPoint {
    /// Check time, ms since the epoch.
    pub t: i64,
    /// Whether the service was up.
    pub up: bool,
    /// Latency in ms.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ms: Option<u32>,
}

/// Latest status of one service with its recent history.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceStatus {
    /// Whether the last check succeeded.
    pub up: bool,
    /// HTTP status code of the last check.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<u16>,
    /// Latency of the last check in ms.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u32>,
    /// Error text of the last check, when it failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Time of the last check, ms since the epoch.
    pub checked_at: i64,
    /// Up to [`HISTORY_LEN`] points, oldest first.
    pub history: Vec<HistoryPoint>,
}

/// An event sent to browsers over `/api/events`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "event", rename_all = "camelCase")]
pub enum LiveEvent {
    /// A status check finished.
    #[serde(rename_all = "camelCase")]
    Status {
        /// Service id.
        service_id: i64,
        /// Whether it is up.
        up: bool,
        /// HTTP status code.
        #[serde(skip_serializing_if = "Option::is_none")]
        code: Option<u16>,
        /// Latency in ms.
        #[serde(skip_serializing_if = "Option::is_none")]
        latency_ms: Option<u32>,
        /// Check time.
        checked_at: i64,
    },
    /// The dashboard was saved; clients should reload it.
    Config {
        /// New version.
        version: i64,
    },
    /// Port scan progress (sent to admins only).
    Scan {
        /// Scan job number.
        id: u64,
        /// The scanner's event.
        data: serde_json::Value,
    },
}

impl LiveEvent {
    /// The SSE event name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Status { .. } => "status",
            Self::Config { .. } => "config",
            Self::Scan { .. } => "scan",
        }
    }
}

/// In-memory status cache and live event channel.
pub struct StatusHub {
    history: Mutex<HashMap<i64, VecDeque<StatusSample>>>,
    events: broadcast::Sender<LiveEvent>,
    wake: tokio::sync::Notify,
}

impl Default for StatusHub {
    fn default() -> Self {
        Self {
            history: Mutex::default(),
            events: broadcast::channel(256).0,
            wake: tokio::sync::Notify::new(),
        }
    }
}

impl StatusHub {
    /// Seeds the cache, e.g. from the database at startup.
    pub fn load(&self, samples: Vec<StatusSample>) {
        for s in samples {
            self.record_quiet(s);
        }
    }

    fn record_quiet(&self, s: StatusSample) {
        let mut h = self.history.lock().expect("status lock poisoned");
        let q = h.entry(s.service_id).or_default();
        q.push_back(s);
        while q.len() > HISTORY_LEN {
            q.pop_front();
        }
    }

    /// Adds a sample and publishes it to live listeners.
    pub fn record(&self, s: StatusSample) {
        let ev = LiveEvent::Status {
            service_id: s.service_id,
            up: s.ok,
            code: s.status_code,
            latency_ms: s.latency_ms,
            checked_at: s.checked_at,
        };
        self.record_quiet(s);
        // No listeners is fine.
        let _ = self.events.send(ev);
    }

    /// Publishes any event. A [`LiveEvent::Config`] also wakes the checker
    /// so new or changed cards get a status right away.
    pub fn publish(&self, ev: LiveEvent) {
        if matches!(ev, LiveEvent::Config { .. }) {
            self.wake.notify_one();
        }
        let _ = self.events.send(ev);
    }

    /// Subscribes to live events.
    pub fn subscribe(&self) -> broadcast::Receiver<LiveEvent> {
        self.events.subscribe()
    }

    /// Drops cached history of services not in `keep`.
    pub fn retain(&self, keep: &std::collections::HashSet<i64>) {
        self.history
            .lock()
            .expect("status lock poisoned")
            .retain(|id, _| keep.contains(id));
    }

    /// Latest status and history of every service that has been checked.
    pub fn snapshot(&self) -> HashMap<i64, ServiceStatus> {
        let h = self.history.lock().expect("status lock poisoned");
        h.iter()
            .filter_map(|(id, q)| {
                let last = q.back()?;
                Some((
                    *id,
                    ServiceStatus {
                        up: last.ok,
                        code: last.status_code,
                        latency_ms: last.latency_ms,
                        error: last.error.clone(),
                        checked_at: last.checked_at,
                        history: q
                            .iter()
                            .map(|s| HistoryPoint {
                                t: s.checked_at,
                                up: s.ok,
                                ms: s.latency_ms,
                            })
                            .collect(),
                    },
                ))
            })
            .collect()
    }
}

/// Builds the HTTP client used for status checks.
pub fn probe_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(PROBE_TIMEOUT)
        .connect_timeout(PROBE_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .tls_danger_accept_invalid_certs(true)
        .user_agent(concat!("homedit/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("static reqwest config is valid")
}

/// Runs one probe and returns the sample.
pub async fn probe(client: &reqwest::Client, service_id: i64, check: &StatusCheck) -> StatusSample {
    let start = Instant::now();
    let elapsed = |s: Instant| u32::try_from(s.elapsed().as_millis()).unwrap_or(u32::MAX);
    let (ok, status_code, latency_ms, error) = match check {
        StatusCheck::Http { url } => match client.get(url).send().await {
            Ok(resp) => {
                let code = resp.status().as_u16();
                let ok = code < 500;
                (
                    ok,
                    Some(code),
                    Some(elapsed(start)),
                    (!ok).then(|| format!("HTTP {code}")),
                )
            }
            Err(e) => (false, None, None, Some(describe_reqwest(&e))),
        },
        StatusCheck::Tcp { host, port } => {
            let addr = (host.as_str(), *port);
            match tokio::time::timeout(PROBE_TIMEOUT, tokio::net::TcpStream::connect(addr)).await {
                Ok(Ok(_)) => (true, None, Some(elapsed(start)), None),
                Ok(Err(e)) => (false, None, None, Some(e.kind().to_string())),
                Err(_) => (false, None, None, Some("timed out".into())),
            }
        }
    };
    StatusSample {
        service_id,
        checked_at: now_ms(),
        ok,
        status_code,
        latency_ms,
        error,
    }
}

/// A short, URL-free description of a request error.
fn describe_reqwest(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "timed out".into()
    } else if e.is_connect() {
        "connection failed".into()
    } else {
        "request failed".into()
    }
}

/// Runs one round of checks for every service that asks for one.
pub async fn check_all(state: &AppState, client: &reqwest::Client) -> anyhow::Result<usize> {
    let dash = state.db.load_dashboard().await?;
    let ids: std::collections::HashSet<i64> = dash.services().filter_map(|s| s.id).collect();
    state.status.retain(&ids);
    let checks: Vec<(i64, String, StatusCheck)> = dash
        .services()
        .filter_map(|s| Some((s.id?, s.name.clone(), StatusCheck::for_service(s)?)))
        .collect();
    let n = checks.len();
    let samples: Vec<(String, &'static str, StatusSample)> = futures::stream::iter(checks)
        .map(|(id, name, c)| async move {
            let kind = match c {
                StatusCheck::Http { .. } => "http",
                StatusCheck::Tcp { .. } => "tcp",
            };
            (name, kind, probe(client, id, &c).await)
        })
        .buffer_unordered(MAX_CONCURRENT_PROBES)
        .collect()
        .await;
    for (name, kind, s) in samples {
        if let Err(e) = state.db.insert_sample(&s).await {
            tracing::warn!(error = %e, "cannot store status sample");
        }
        state.metrics.record_status_check(
            &name,
            kind,
            s.ok,
            s.latency_ms.map(|ms| f64::from(ms) / 1000.0),
        );
        state.status.record(s);
    }
    Ok(n)
}

/// Starts the background checker. It runs until the process exits.
pub fn spawn_checker(state: AppState) -> tokio::task::JoinHandle<()> {
    let state = Arc::new(state);
    tokio::spawn(async move {
        let client = probe_client();
        let mut last_prune = Instant::now() - Duration::from_secs(3600);
        loop {
            if let Err(e) = check_all(&state, &client).await {
                tracing::warn!(error = %e, "status round failed");
            }
            if last_prune.elapsed() >= Duration::from_secs(3600) {
                let cutoff = now_ms() - RETENTION.as_millis() as i64;
                match state.db.prune_samples(cutoff).await {
                    Ok(n) if n > 0 => tracing::debug!(removed = n, "pruned status samples"),
                    Ok(_) => {}
                    Err(e) => tracing::warn!(error = %e, "cannot prune status samples"),
                }
                last_prune = Instant::now();
            }
            let secs = state
                .db
                .load_dashboard()
                .await
                .map(|d| d.settings.status_interval_secs)
                .unwrap_or(30);
            let pause = Duration::from_secs(secs.into()).max(MIN_INTERVAL);
            tokio::select! {
                _ = tokio::time::sleep(pause) => {}
                _ = state.status.wake.notified() => {}
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: i64, t: i64, ok: bool) -> StatusSample {
        StatusSample {
            service_id: id,
            checked_at: t,
            ok,
            status_code: None,
            latency_ms: Some(1),
            error: None,
        }
    }

    #[test]
    fn hub_keeps_bounded_history() {
        let hub = StatusHub::default();
        for t in 0..40 {
            hub.record(sample(1, t, t % 3 != 0));
        }
        let snap = hub.snapshot();
        let s = &snap[&1];
        assert_eq!(s.history.len(), HISTORY_LEN);
        assert_eq!(s.history[0].t, 10);
        assert_eq!(s.checked_at, 39);
        assert!(!s.up);
    }

    #[tokio::test]
    async fn hub_publishes() {
        let hub = StatusHub::default();
        let mut rx = hub.subscribe();
        hub.record(sample(5, 1, true));
        match rx.recv().await.unwrap() {
            LiveEvent::Status { service_id, up, .. } => {
                assert_eq!(service_id, 5);
                assert!(up);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn tcp_probe_up_and_down() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let client = probe_client();
        let up = probe(
            &client,
            1,
            &StatusCheck::Tcp {
                host: "127.0.0.1".into(),
                port,
            },
        )
        .await;
        assert!(up.ok);
        drop(listener);
        let down = probe(
            &client,
            1,
            &StatusCheck::Tcp {
                host: "127.0.0.1".into(),
                port,
            },
        )
        .await;
        assert!(!down.ok);
        assert!(down.error.is_some());
    }

    #[tokio::test]
    async fn http_probe_codes() {
        use axum::http::StatusCode;
        let app = axum::Router::new()
            .route("/ok", axum::routing::get(|| async { "hi" }))
            .route(
                "/auth",
                axum::routing::get(|| async { StatusCode::UNAUTHORIZED }),
            )
            .route(
                "/boom",
                axum::routing::get(|| async { StatusCode::BAD_GATEWAY }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = probe_client();
        let run = |path: &str| {
            let check = StatusCheck::Http {
                url: format!("http://{addr}{path}"),
            };
            let client = client.clone();
            async move { probe(&client, 1, &check).await }
        };
        let ok = run("/ok").await;
        assert!(ok.ok);
        assert_eq!(ok.status_code, Some(200));
        assert!(run("/auth").await.ok, "401 means the app is up");
        let boom = run("/boom").await;
        assert!(!boom.ok);
        assert_eq!(boom.error.as_deref(), Some("HTTP 502"));
    }
}
