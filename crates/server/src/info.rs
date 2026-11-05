//! Header (info) widget data: `GET /api/info/{widgetId}`.
//!
//! | Kind                         | Response                                              |
//! |------------------------------|-------------------------------------------------------|
//! | `datetime` (`clock`)         | `{}`; the browser draws the clock                     |
//! | `resources`                  | `{cpu, memUsed, memTotal, diskUsed, diskTotal}`       |
//! | `openmeteo` (`weather`)      | `{tempC, condition, icon}` from Open-Meteo            |
//! | `docker`                     | `{running, stopped}` across the dashboard's Docker hosts |
//!
//! Options follow gethomepage: `resources` reads `disk` (a path, default
//! `/`; a list uses its first entry); weather reads `latitude` and
//! `longitude`. Sizes are bytes and `cpu` is percent. Results are cached
//! per widget (resources 5 s, Docker 15 s, weather 10 min) so many open
//! dashboards cost one lookup. `resources` describes the machine as seen by
//! the homedit process, which inside a container is the host's CPU and
//! memory and the container's view of the disk path.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::{Path as UrlPath, State};
use homedit_core::{DockerHost, InfoWidget};
use homedit_discovery::{DockerClient, DockerEndpoint};
use serde::Serialize;
use serde_json::{Map, Value, json};
use tokio::sync::Mutex;

use crate::auth::RequireViewer;
use crate::{ApiError, AppState};

/// Open-Meteo forecast endpoint.
pub const OPEN_METEO_URL: &str = "https://api.open-meteo.com/v1/forecast";

/// Cache and HTTP client for info widgets.
pub struct InfoState {
    cache: Mutex<HashMap<i64, (Instant, Value)>>,
    cpu: Mutex<Option<CpuTimes>>,
    http: reqwest::Client,
}

impl Default for InfoState {
    fn default() -> Self {
        Self {
            cache: Mutex::default(),
            cpu: Mutex::default(),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(8))
                .user_agent(format!("homedit/{}", crate::VERSION))
                .build()
                .expect("static reqwest config is valid"),
        }
    }
}

/// Host resource usage.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Resources {
    /// CPU busy percentage since the previous sample.
    pub cpu: f64,
    /// Memory in use (total minus available), bytes.
    pub mem_used: u64,
    /// Total memory, bytes.
    pub mem_total: u64,
    /// Disk space in use, bytes.
    pub disk_used: u64,
    /// Disk size, bytes.
    pub disk_total: u64,
}

/// Aggregate CPU times from `/proc/stat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuTimes {
    /// Idle plus iowait jiffies.
    pub idle: u64,
    /// All jiffies.
    pub total: u64,
}

/// Parses the `cpu` line of `/proc/stat`.
pub fn parse_proc_stat(text: &str) -> Option<CpuTimes> {
    let line = text.lines().find(|l| l.starts_with("cpu "))?;
    let v: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|x| x.parse().ok())
        .collect();
    if v.len() < 4 {
        return None;
    }
    // user nice system idle iowait irq softirq steal (guest is in user).
    let idle = v[3] + v.get(4).copied().unwrap_or(0);
    let total = v.iter().take(8).sum();
    Some(CpuTimes { idle, total })
}

/// Busy percentage between two samples.
pub fn cpu_percent(prev: CpuTimes, now: CpuTimes) -> f64 {
    let total = now.total.saturating_sub(prev.total);
    let idle = now.idle.saturating_sub(prev.idle);
    if total == 0 {
        return 0.0;
    }
    let pct = 100.0 * (total - idle.min(total)) as f64 / total as f64;
    (pct * 10.0).round() / 10.0
}

/// Parses `MemTotal` and `MemAvailable` from `/proc/meminfo` into bytes
/// (total, used).
pub fn parse_meminfo(text: &str) -> Option<(u64, u64)> {
    let field = |name: &str| -> Option<u64> {
        let line = text.lines().find(|l| l.starts_with(name))?;
        let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
        Some(kb * 1024)
    };
    let total = field("MemTotal:")?;
    let avail = field("MemAvailable:").or_else(|| field("MemFree:"))?;
    Some((total, total.saturating_sub(avail)))
}

fn disk_usage(path: &Path) -> Result<(u64, u64), String> {
    let s = rustix::fs::statvfs(path)
        .map_err(|e| format!("cannot read disk {}: {e}", path.display()))?;
    let frag = s.f_frsize;
    let total = s.f_blocks * frag;
    let free = s.f_bfree * frag;
    Ok((total, total.saturating_sub(free)))
}

impl InfoState {
    async fn resources(&self, disk: &Path) -> Result<Resources, String> {
        let read_stat = || {
            std::fs::read_to_string("/proc/stat")
                .ok()
                .and_then(|t| parse_proc_stat(&t))
        };
        let now = read_stat().ok_or("cannot read /proc/stat")?;
        let mut prev = self.cpu.lock().await;
        let cpu = match *prev {
            Some(p) if p.total < now.total => cpu_percent(p, now),
            _ => {
                // First call: take a short second sample.
                tokio::time::sleep(Duration::from_millis(250)).await;
                let later = read_stat().ok_or("cannot read /proc/stat")?;
                let pct = cpu_percent(now, later);
                *prev = Some(later);
                drop(prev);
                return self.finish(pct, disk);
            }
        };
        *prev = Some(now);
        drop(prev);
        self.finish(cpu, disk)
    }

    fn finish(&self, cpu: f64, disk: &Path) -> Result<Resources, String> {
        let (mem_total, mem_used) = std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|t| parse_meminfo(&t))
            .ok_or("cannot read /proc/meminfo")?;
        let (disk_total, disk_used) = disk_usage(disk)?;
        Ok(Resources {
            cpu,
            mem_used,
            mem_total,
            disk_used,
            disk_total,
        })
    }

    async fn weather(&self, opts: &Map<String, Value>) -> Result<Value, String> {
        let coord = |k: &str| -> Option<f64> {
            let v = opts.get(k)?;
            v.as_f64().or_else(|| v.as_str()?.trim().parse().ok())
        };
        let (Some(lat), Some(lon)) = (coord("latitude"), coord("longitude")) else {
            return Err("set latitude and longitude in the widget options".into());
        };
        if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
            return Err("latitude or longitude is out of range".into());
        }
        let resp: Value = self
            .http
            .get(format!(
                "{OPEN_METEO_URL}?latitude={lat}&longitude={lon}&current=temperature_2m,weather_code,is_day"
            ))
            .send()
            .await
            .map_err(|_| "weather service unreachable".to_owned())?
            .error_for_status()
            .map_err(|e| format!("weather service answered {}", e.status().map_or(0, |s| s.as_u16())))?
            .json()
            .await
            .map_err(|_| "weather service sent an unexpected answer".to_owned())?;
        weather_view(&resp).ok_or_else(|| "weather service sent an unexpected answer".into())
    }

    async fn docker(&self, hosts: &[DockerHost]) -> Result<Value, String> {
        if hosts.is_empty() {
            return Err("no Docker host is configured".into());
        }
        let (mut running, mut stopped, mut reached) = (0u64, 0u64, 0usize);
        for h in hosts {
            let ep = match (&h.socket, &h.host) {
                (Some(s), _) => DockerEndpoint::Unix(s.into()),
                (None, Some(host)) => DockerEndpoint::Tcp {
                    host: host.clone(),
                    port: h.port.unwrap_or(2375),
                },
                _ => continue,
            };
            match DockerClient::new(ep).list_containers(true).await {
                Ok(list) => {
                    reached += 1;
                    for c in list {
                        if c.state().is_up() {
                            running += 1;
                        } else {
                            stopped += 1;
                        }
                    }
                }
                Err(e) => tracing::debug!(server = %h.name, error = %e, "docker count failed"),
            }
        }
        if reached == 0 {
            return Err("cannot reach any Docker host".into());
        }
        Ok(json!({ "running": running, "stopped": stopped }))
    }
}

/// Maps a WMO weather code to a short condition and an icon name.
pub fn wmo(code: i64, day: bool) -> (&'static str, &'static str) {
    match code {
        0 => ("Clear", if day { "clear-day" } else { "clear-night" }),
        1 | 2 => (
            "Partly cloudy",
            if day {
                "partly-cloudy-day"
            } else {
                "partly-cloudy-night"
            },
        ),
        3 => ("Cloudy", "cloudy"),
        45 | 48 => ("Fog", "fog"),
        51..=57 => ("Drizzle", "drizzle"),
        61..=67 | 80..=82 => ("Rain", "rain"),
        71..=77 | 85 | 86 => ("Snow", "snow"),
        95..=99 => ("Thunderstorm", "thunderstorm"),
        _ => ("Unknown", "cloudy"),
    }
}

/// Builds `{tempC, condition, icon}` from an Open-Meteo answer.
pub fn weather_view(resp: &Value) -> Option<Value> {
    let cur = resp.get("current")?;
    let temp = cur.get("temperature_2m")?.as_f64()?;
    let code = cur.get("weather_code")?.as_i64()?;
    let day = cur.get("is_day").and_then(Value::as_i64).unwrap_or(1) == 1;
    let (condition, icon) = wmo(code, day);
    Some(json!({ "tempC": (temp * 10.0).round() / 10.0, "condition": condition, "icon": icon }))
}

fn ttl(kind: &str) -> Duration {
    match kind {
        "resources" => Duration::from_secs(5),
        "docker" => Duration::from_secs(15),
        _ => Duration::from_secs(600),
    }
}

fn disk_option(w: &InfoWidget) -> String {
    match w.options.get("disk") {
        Some(Value::String(s)) if !s.is_empty() => s.clone(),
        Some(Value::Array(a)) => a.first().and_then(Value::as_str).unwrap_or("/").to_owned(),
        _ => "/".into(),
    }
}

/// `GET /api/info/{widgetId}`.
pub async fn data(
    RequireViewer(_): RequireViewer,
    State(state): State<AppState>,
    UrlPath(id): UrlPath<i64>,
) -> Result<Json<Value>, ApiError> {
    let dash = state.db.load_dashboard().await?;
    let w = dash
        .info_widgets
        .iter()
        .find(|w| w.id == Some(id))
        .ok_or_else(ApiError::not_found)?;
    let kind = w.kind.as_str();
    if let Some((at, v)) = state.info.cache.lock().await.get(&id)
        && at.elapsed() < ttl(kind)
    {
        return Ok(Json(v.clone()));
    }
    let result = match kind {
        "datetime" | "clock" => Ok(json!({})),
        "resources" => state
            .info
            .resources(Path::new(&disk_option(w)))
            .await
            .map(|r| serde_json::to_value(r).unwrap_or_default()),
        "openmeteo" | "openweathermap" | "weatherapi" | "weather" => {
            state.info.weather(&w.options).await
        }
        "docker" => state.info.docker(&dash.docker_hosts).await,
        _ => return Err(ApiError::not_found()),
    };
    match result {
        Ok(v) => {
            state
                .info
                .cache
                .lock()
                .await
                .insert(id, (Instant::now(), v.clone()));
            Ok(Json(v))
        }
        // An upstream that fails (no coordinates, a Docker host that doesn't answer) is this widget's problem,
        // not homedit's: 200 with the sentence, which the card shows with Copy. Not cached, so the next poll
        // tries again.
        Err(message) => Ok(Json(json!({ "error": message }))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proc_stat_and_cpu() {
        let a = parse_proc_stat("cpu  100 0 100 800 0 0 0 0 0 0\ncpu0 1 2 3 4\n").unwrap();
        assert_eq!(
            a,
            CpuTimes {
                idle: 800,
                total: 1000
            }
        );
        let b = parse_proc_stat("cpu  150 0 150 900 0 0 0 0 0 0\n").unwrap();
        assert_eq!(cpu_percent(a, b), 50.0);
        assert_eq!(cpu_percent(b, b), 0.0);
        assert!(parse_proc_stat("garbage").is_none());
    }

    #[test]
    fn meminfo() {
        let t = "MemTotal:       16000000 kB\nMemFree:  1000 kB\nMemAvailable:   4000000 kB\n";
        assert_eq!(
            parse_meminfo(t),
            Some((16_000_000 * 1024, 12_000_000 * 1024))
        );
    }

    #[test]
    fn weather_mapping() {
        let v =
            weather_view(&json!({"current":{"temperature_2m":12.34,"weather_code":61,"is_day":0}}))
                .unwrap();
        assert_eq!(v, json!({"tempC":12.3,"condition":"Rain","icon":"rain"}));
        assert_eq!(wmo(0, false).1, "clear-night");
        assert!(weather_view(&json!({})).is_none());
    }

    #[tokio::test]
    async fn resources_on_this_machine() {
        let st = InfoState::default();
        let r = st.resources(Path::new("/")).await.unwrap();
        assert!(r.mem_total > 0 && r.disk_total > 0);
        assert!((0.0..=100.0).contains(&r.cpu));
    }
}
