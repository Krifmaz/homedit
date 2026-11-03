//! Service widget endpoints.
//!
//! * `GET /api/widgets/catalog` – every widget type with its form schema.
//! * `GET /api/widgets/{serviceId}/data` – live stats for one card. The
//!   request to the upstream app is made by the server, with the stored
//!   (decrypted) credentials, through the widget crate's SSRF-guarded
//!   client and per-card cache.
//! * `POST /api/widgets/test` – the editor's "test connection" button.
//!   Secret placeholders are filled from the stored card when `serviceId`
//!   is given.

use axum::Json;
use axum::extract::{Path, State};
use homedit_auth::secrets::is_placeholder;
use homedit_core::secrets::MASK;
use homedit_core::{Dashboard, Service};
use homedit_ops::WidgetOutcome;
use homedit_widgets::{FieldType, StatValue, WidgetConfig, WidgetSchema};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::auth::{RequireAdmin, RequireViewer};
use crate::{ApiError, AppState, secrets};

/// One form field, in the shape the frontend expects.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FieldView {
    /// Key in the widget's fields.
    pub key: String,
    /// Label.
    pub label: String,
    /// `string`, `url`, `secret`, `number`, `bool` or `select`.
    pub kind: &'static str,
    /// Whether a value is required.
    pub required: bool,
    /// Help text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    /// Default value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    /// Choices for `select`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
}

/// One widget type, in the shape the frontend expects.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CatalogEntry {
    /// Type key.
    #[serde(rename = "type")]
    pub kind: String,
    /// Display name.
    pub name: String,
    /// Picker category.
    pub category: &'static str,
    /// One-line description.
    pub description: String,
    /// Form fields.
    pub fields: Vec<FieldView>,
}

/// Picker category of a widget type.
pub fn category(kind: &str) -> &'static str {
    match kind {
        "plex" | "jellyfin" | "emby" | "immich" | "audiobookshelf" | "jellystat" => "Media",
        "sonarr" | "radarr" | "lidarr" | "prowlarr" | "overseerr" | "jellyseerr" | "seerr" => {
            "Media management"
        }
        "qbittorrent" | "sabnzbd" | "slskd" => "Downloads",
        "proxmox" | "portainer" | "docker" => "Infrastructure",
        "pihole" | "adguard" => "Network",
        "uptimekuma" => "Monitoring",
        "homeassistant" => "Home automation",
        _ => "Other",
    }
}

/// Converts the widget crate's schema to the frontend's catalogue entry.
pub fn catalog_entry(s: &WidgetSchema) -> CatalogEntry {
    CatalogEntry {
        category: category(&s.kind),
        kind: s.kind.clone(),
        name: s.display_name.clone(),
        description: s.description.clone(),
        fields: s
            .fields
            .iter()
            .map(|f| {
                let (kind, options) = match &f.field_type {
                    FieldType::Url => ("url", None),
                    FieldType::Secret => ("secret", None),
                    FieldType::Number { .. } => ("number", None),
                    FieldType::Bool => ("bool", None),
                    FieldType::Select { options } => (
                        "select",
                        Some(options.iter().map(|o| o.value.clone()).collect()),
                    ),
                    _ => ("string", None),
                };
                FieldView {
                    key: f.key.clone(),
                    label: f.label.clone(),
                    kind,
                    required: f.required,
                    help: (!f.help.is_empty()).then(|| f.help.clone()),
                    default: f.default.clone(),
                    options,
                }
            })
            .collect(),
    }
}

/// `GET /api/widgets/catalog`.
pub async fn catalog(
    RequireViewer(_): RequireViewer,
    State(state): State<AppState>,
) -> Json<Vec<CatalogEntry>> {
    let mut list: Vec<CatalogEntry> = state.widgets.schemas().iter().map(catalog_entry).collect();
    list.sort_by(|a, b| a.name.cmp(&b.name));
    Json(list)
}

/// One stat as the frontend shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StatView {
    /// Label.
    pub label: String,
    /// Value, formatted for display unless it is a plain number.
    pub value: Value,
    /// Unit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

/// Response of `GET /api/widgets/{id}/data`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DataView {
    /// Stats in display order.
    pub fields: Vec<StatView>,
    /// Detail rows (now playing, queue), when the widget has them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<homedit_widgets::ListItem>,
    /// Error message when the upstream call failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Formats a number with up to one decimal and a scaled binary unit.
fn scaled(v: f64, units: &[&str]) -> (Value, Option<String>) {
    let mut v = v;
    let mut i = 0;
    while v.abs() >= 1024.0 && i + 1 < units.len() {
        v /= 1024.0;
        i += 1;
    }
    (round1(v), Some(units[i].to_owned()))
}

fn round1(v: f64) -> Value {
    let r = (v * 10.0).round() / 10.0;
    serde_json::Number::from_f64(r).map_or(Value::Null, Value::Number)
}

/// Turns a stat value into a display value and unit.
pub fn stat_view(label: &str, v: &StatValue) -> StatView {
    let (value, unit) = match v {
        StatValue::Number(n) => (round1(*n), None),
        StatValue::Text(t) => (Value::String(t.clone()), None),
        StatValue::Bytes(b) => scaled(*b, &["B", "KiB", "MiB", "GiB", "TiB", "PiB"]),
        StatValue::Rate(r) => scaled(*r, &["B/s", "KiB/s", "MiB/s", "GiB/s"]),
        StatValue::Percent(p) => (round1(*p), Some("%".into())),
        StatValue::Duration(s) => (Value::String(human_duration(*s)), None),
        StatValue::Millis(ms) => (round1(*ms), Some("ms".into())),
        #[allow(unreachable_patterns)]
        _ => (Value::Null, None),
    };
    StatView {
        label: label.to_owned(),
        value,
        unit,
    }
}

/// `90061` → `1d 1h`, `3725` → `1h 2m`, `42` → `42s`.
pub fn human_duration(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    let (d, h, m) = (s / 86_400, (s % 86_400) / 3600, (s % 3600) / 60);
    if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else if m > 0 {
        format!("{m}m {}s", s % 60)
    } else {
        format!("{s}s")
    }
}

fn find_service(d: &Dashboard, id: i64) -> Option<(&Service, bool)> {
    fn walk(groups: &[homedit_core::Group], id: i64, hidden: bool) -> Option<(&Service, bool)> {
        for g in groups {
            let h = hidden || g.hidden_from_viewers;
            if let Some(s) = g.services.iter().find(|s| s.id == Some(id)) {
                return Some((s, h));
            }
            if let Some(r) = walk(&g.groups, id, h) {
                return Some(r);
            }
        }
        None
    }
    walk(&d.groups, id, false)
}

/// `GET /api/widgets/{serviceId}/data`.
pub async fn data(
    RequireViewer(user): RequireViewer,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<DataView>, ApiError> {
    let dash = state.db.load_dashboard().await?;
    let (svc, hidden) = find_service(&dash, id).ok_or_else(ApiError::not_found)?;
    if !user.can_view_group(hidden) {
        return Err(ApiError::not_found());
    }
    let widget = svc.widget.as_ref().ok_or_else(ApiError::not_found)?;
    let kind = widget
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    let plain = secrets::open_widget(widget, state.auth.secrets()).map_err(|e| {
        tracing::warn!(service = id, error = %e, "cannot decrypt widget secrets");
        ApiError::internal("cannot decrypt widget secrets")
    })?;
    match state
        .widgets
        .fetch_service(&state.widget_ctx, id, &plain)
        .await
    {
        Ok(d) => {
            state.metrics.record_widget(&kind, WidgetOutcome::Ok);
            Ok(Json(DataView {
                fields: d
                    .stats
                    .iter()
                    .map(|s| stat_view(&s.label, &s.value))
                    .collect(),
                items: d.items.clone(),
                error: None,
            }))
        }
        Err(e) => {
            state.metrics.record_widget(&kind, WidgetOutcome::Error);
            Ok(Json(DataView {
                fields: Vec::new(),
                items: Vec::new(),
                error: Some(e.to_string()),
            }))
        }
    }
}

/// Body of `POST /api/widgets/test`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestRequest {
    /// Widget type.
    #[serde(rename = "type")]
    pub kind: String,
    /// Field values; secrets may be placeholders.
    #[serde(default)]
    pub fields: Map<String, Value>,
    /// Saved card whose stored secrets fill placeholders.
    #[serde(default)]
    pub service_id: Option<i64>,
}

/// Response of `POST /api/widgets/test`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestResponse {
    /// Whether it worked.
    pub ok: bool,
    /// Message for the admin.
    pub message: String,
    /// Round trip in ms.
    pub latency_ms: u64,
}

/// `POST /api/widgets/test`.
pub async fn test(
    RequireAdmin(_): RequireAdmin,
    State(state): State<AppState>,
    Json(req): Json<TestRequest>,
) -> Result<Json<TestResponse>, ApiError> {
    let stored = match req.service_id {
        Some(id) => {
            let dash = state.db.load_dashboard().await?;
            find_service(&dash, id)
                .and_then(|(s, _)| s.widget.clone())
                .map(|w| secrets::open_widget(&w, state.auth.secrets()))
                .transpose()
                .map_err(ApiError::internal)?
        }
        None => None,
    };
    let mut w = Map::new();
    w.insert("type".into(), Value::String(req.kind.clone()));
    for (k, v) in req.fields {
        if is_placeholder(&v) || v.as_str() == Some(MASK) {
            if let Some(prev) = stored.as_ref().and_then(|s| s.get(&k)) {
                w.insert(k, prev.clone());
            }
        } else if !v.is_null() {
            w.insert(k, v);
        }
    }
    let (kind, cfg) = WidgetConfig::from_homepage(&Value::Object(w))
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    let r = state
        .widgets
        .test_connection(&state.widget_ctx, &kind, cfg)
        .await;
    Ok(Json(TestResponse {
        ok: r.ok,
        message: r.message,
        latency_ms: r.latency_ms,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_formatting() {
        let b = stat_view("Size", &StatValue::Bytes(3.0 * 1024.0 * 1024.0 * 1024.0));
        assert_eq!(b.value, serde_json::json!(3.0));
        assert_eq!(b.unit.as_deref(), Some("GiB"));
        let p = stat_view("CPU", &StatValue::Percent(12.345));
        assert_eq!(p.value, serde_json::json!(12.3));
        assert_eq!(human_duration(90_061.0), "1d 1h");
        assert_eq!(human_duration(3725.0), "1h 2m");
        assert_eq!(human_duration(42.0), "42s");
    }

    #[test]
    fn catalog_has_all_builtins() {
        let reg = homedit_widgets::WidgetRegistry::builtin();
        let cat: Vec<CatalogEntry> = reg.schemas().iter().map(catalog_entry).collect();
        let sonarr = cat.iter().find(|c| c.kind == "sonarr").expect("sonarr");
        assert_eq!(sonarr.category, "Media management");
        assert_eq!(sonarr.fields[0].key, "url");
        assert!(sonarr.fields.iter().any(|f| f.kind == "secret"));
    }
}
