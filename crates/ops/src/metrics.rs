//! Prometheus metrics for homedit.
//!
//! [`Metrics`] owns a registry with every metric the server exports, plus
//! small methods for the parts of the server that record them (HTTP
//! middleware, status checks, widgets, scans). It is cheap to clone and safe
//! to share between tasks.
//!
//! Label values are kept to a bounded set: HTTP routes use the matched route
//! template (such as `/api/services/{id}`), never the raw path, so a client
//! cannot create unbounded series by requesting random URLs.

use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Instant;

use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::metrics::histogram::{Histogram, exponential_buckets};
use prometheus_client::metrics::info::Info;
use prometheus_client::registry::{Registry, Unit};

/// The content type of the text exposition format served at `/metrics`.
pub const CONTENT_TYPE: &str = "application/openmetrics-text; version=1.0.0; charset=utf-8";

/// Labels for HTTP request counters.
#[derive(Debug, Clone, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct HttpLabels {
    /// HTTP method.
    pub method: String,
    /// Matched route template, or `unmatched`.
    pub route: String,
    /// Response status code.
    pub status: u16,
}

/// Labels for the HTTP latency histogram.
#[derive(Debug, Clone, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct RouteLabels {
    /// HTTP method.
    pub method: String,
    /// Matched route template, or `unmatched`.
    pub route: String,
}

/// Labels for status check counters.
#[derive(Debug, Clone, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct CheckLabels {
    /// `http` or `tcp`.
    pub kind: String,
    /// `up` or `down`.
    pub result: String,
}

/// Labels for per-service gauges.
#[derive(Debug, Clone, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct ServiceLabels {
    /// Service (card) name.
    pub service: String,
}

/// Labels for widget request counters.
#[derive(Debug, Clone, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct WidgetLabels {
    /// Widget type, such as `sonarr`.
    pub widget: String,
    /// `ok`, `error` or `cached`.
    pub outcome: String,
}

/// Labels for scan counters.
#[derive(Debug, Clone, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct OutcomeLabels {
    /// `ok`, `error` or `rejected`.
    pub outcome: String,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, EncodeLabelSet)]
struct BuildLabels {
    version: String,
}

/// The result of a widget fetch, for [`Metrics::record_widget`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidgetOutcome {
    /// Fetched from the upstream API.
    Ok,
    /// The upstream API failed.
    Error,
    /// Served from the widget cache.
    Cached,
}

impl WidgetOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Cached => "cached",
        }
    }
}

struct Inner {
    registry: Registry,
    started: Instant,
    http_requests: Family<HttpLabels, Counter>,
    http_duration: Family<RouteLabels, Histogram>,
    status_checks: Family<CheckLabels, Counter>,
    service_up: Family<ServiceLabels, Gauge>,
    service_latency: Family<ServiceLabels, Gauge<f64, AtomicU64>>,
    widget_requests: Family<WidgetLabels, Counter>,
    scans: Family<OutcomeLabels, Counter>,
    logins: Family<OutcomeLabels, Counter>,
    uptime: Gauge<f64, AtomicU64>,
    resident_memory: Gauge,
}

/// All of homedit's metrics. Clone it freely; clones share the same data.
#[derive(Clone)]
pub struct Metrics(Arc<Inner>);

impl std::fmt::Debug for Metrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Metrics").finish_non_exhaustive()
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new(crate::VERSION)
    }
}

impl Metrics {
    /// Creates the registry with every metric under the `homedit_` prefix.
    /// `version` is exported as `homedit_build_info{version="..."}`.
    pub fn new(version: &str) -> Self {
        let mut registry = Registry::with_prefix("homedit");

        let build_info = Info::new(BuildLabels {
            version: version.to_string(),
        });
        registry.register("build", "homedit build information", build_info);

        let http_requests = Family::<HttpLabels, Counter>::default();
        registry.register(
            "http_requests",
            "HTTP requests handled, by method, route and status",
            http_requests.clone(),
        );
        let http_duration = Family::<RouteLabels, Histogram>::new_with_constructor(
            (|| Histogram::new(exponential_buckets(0.001, 2.5, 12))) as fn() -> Histogram,
        );
        registry.register_with_unit(
            "http_request_duration",
            "HTTP request latency, by method and route",
            Unit::Seconds,
            http_duration.clone(),
        );
        let status_checks = Family::<CheckLabels, Counter>::default();
        registry.register(
            "status_checks",
            "Service status checks run, by kind and result",
            status_checks.clone(),
        );
        let service_up = Family::<ServiceLabels, Gauge>::default();
        registry.register(
            "service_up",
            "1 if the service's last status check succeeded, else 0",
            service_up.clone(),
        );
        let service_latency = Family::<ServiceLabels, Gauge<f64, AtomicU64>>::default();
        registry.register_with_unit(
            "service_response",
            "Response time of the service's last status check",
            Unit::Seconds,
            service_latency.clone(),
        );
        let widget_requests = Family::<WidgetLabels, Counter>::default();
        registry.register(
            "widget_requests",
            "Service widget fetches, by widget type and outcome",
            widget_requests.clone(),
        );
        let scans = Family::<OutcomeLabels, Counter>::default();
        registry.register("scans", "Network scans run, by outcome", scans.clone());
        let logins = Family::<OutcomeLabels, Counter>::default();
        registry.register("logins", "Login attempts, by outcome", logins.clone());
        let uptime = Gauge::<f64, AtomicU64>::default();
        registry.register_with_unit(
            "uptime",
            "Time since the server started",
            Unit::Seconds,
            uptime.clone(),
        );
        let resident_memory = Gauge::default();
        registry.register_with_unit(
            "process_resident_memory",
            "Resident memory of the server process (Linux only)",
            Unit::Bytes,
            resident_memory.clone(),
        );

        Self(Arc::new(Inner {
            registry,
            started: Instant::now(),
            http_requests,
            http_duration,
            status_checks,
            service_up,
            service_latency,
            widget_requests,
            scans,
            logins,
            uptime,
            resident_memory,
        }))
    }

    /// Records one handled HTTP request.
    pub fn record_http(&self, method: &str, route: &str, status: u16, seconds: f64) {
        self.0
            .http_requests
            .get_or_create(&HttpLabels {
                method: method.to_string(),
                route: route.to_string(),
                status,
            })
            .inc();
        self.0
            .http_duration
            .get_or_create(&RouteLabels {
                method: method.to_string(),
                route: route.to_string(),
            })
            .observe(seconds);
    }

    /// Records one status check for `service`. `kind` is `http` or `tcp`;
    /// `latency_seconds` is the response time when the check succeeded.
    pub fn record_status_check(
        &self,
        service: &str,
        kind: &str,
        up: bool,
        latency_seconds: Option<f64>,
    ) {
        self.0
            .status_checks
            .get_or_create(&CheckLabels {
                kind: kind.to_string(),
                result: if up { "up" } else { "down" }.to_string(),
            })
            .inc();
        let labels = ServiceLabels {
            service: service.to_string(),
        };
        self.0.service_up.get_or_create(&labels).set(i64::from(up));
        if let Some(latency) = latency_seconds {
            self.0.service_latency.get_or_create(&labels).set(latency);
        }
    }

    /// Removes the per-service series for a service that was deleted, so it
    /// stops being exported.
    pub fn forget_service(&self, service: &str) {
        let labels = ServiceLabels {
            service: service.to_string(),
        };
        self.0.service_up.remove(&labels);
        self.0.service_latency.remove(&labels);
    }

    /// Records one widget fetch.
    pub fn record_widget(&self, widget: &str, outcome: WidgetOutcome) {
        self.0
            .widget_requests
            .get_or_create(&WidgetLabels {
                widget: widget.to_string(),
                outcome: outcome.as_str().to_string(),
            })
            .inc();
    }

    /// Records one network scan. `outcome` is `ok`, `error` or `rejected`.
    pub fn record_scan(&self, outcome: &str) {
        self.0
            .scans
            .get_or_create(&OutcomeLabels {
                outcome: outcome.to_string(),
            })
            .inc();
    }

    /// Records one login attempt. `outcome` is `ok`, `failed` or
    /// `rate_limited`.
    pub fn record_login(&self, outcome: &str) {
        self.0
            .logins
            .get_or_create(&OutcomeLabels {
                outcome: outcome.to_string(),
            })
            .inc();
    }

    /// Renders every metric in the OpenMetrics text format, updating the
    /// uptime and memory gauges first.
    pub fn render(&self) -> String {
        self.0.uptime.set(self.0.started.elapsed().as_secs_f64());
        if let Some(bytes) = resident_memory_bytes() {
            self.0.resident_memory.set(bytes);
        }
        let mut out = String::new();
        // Writing to a String cannot fail.
        let _ = prometheus_client::encoding::text::encode(&mut out, &self.0.registry);
        out
    }
}

/// Reads the resident set size from `/proc/self/statm`. Returns `None` on
/// systems without procfs.
fn resident_memory_bytes() -> Option<i64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: i64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    // Linux uses 4 KiB pages on amd64 and on arm64 in the default kernels
    // used by Docker hosts; this avoids an unsafe sysconf call.
    Some(pages * 4096)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_recorded_metrics() {
        let metrics = Metrics::new("1.2.3");
        metrics.record_http("GET", "/api/dashboard", 200, 0.004);
        metrics.record_status_check("Plex", "http", true, Some(0.12));
        metrics.record_status_check("Sonarr", "tcp", false, None);
        metrics.record_widget("sonarr", WidgetOutcome::Cached);
        metrics.record_scan("ok");
        metrics.record_login("failed");
        let text = metrics.render();
        for needle in [
            r#"homedit_build_info{version="1.2.3"} 1"#,
            r#"homedit_http_requests_total{method="GET",route="/api/dashboard",status="200"} 1"#,
            "homedit_http_request_duration_seconds_bucket",
            r#"homedit_status_checks_total{kind="http",result="up"} 1"#,
            r#"homedit_service_up{service="Plex"} 1"#,
            r#"homedit_service_up{service="Sonarr"} 0"#,
            r#"homedit_service_response_seconds{service="Plex"} 0.12"#,
            r#"homedit_widget_requests_total{widget="sonarr",outcome="cached"} 1"#,
            r#"homedit_scans_total{outcome="ok"} 1"#,
            r#"homedit_logins_total{outcome="failed"} 1"#,
            "homedit_uptime_seconds",
            "# EOF",
        ] {
            assert!(text.contains(needle), "missing {needle} in:\n{text}");
        }

        metrics.forget_service("Plex");
        assert!(!metrics.render().contains(r#"service="Plex""#));
    }
}
