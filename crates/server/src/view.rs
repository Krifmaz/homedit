//! The JSON shapes the frontend reads and writes, and their mapping to and
//! from the stored [`Dashboard`].
//!
//! Two documents share these types:
//!
//! * the **dashboard view** (`GET /api/dashboard`, inlined into the page):
//!   icons are resolved URLs, hidden groups are dropped for viewers, and
//!   secret widget fields are masked;
//! * the **config document** (`GET`/`PUT /api/config`): icons stay as names,
//!   all groups are included, and secret fields are sent as
//!   `{"$secret":"set"}`. On `PUT`, that placeholder keeps the stored value,
//!   a string replaces it, and `null` clears it.
//!
//! Anything the view does not model (unknown gethomepage keys, layout
//! options without a form field) is taken from the stored document when a
//! config is saved, so editing in the UI never drops it.

use homedit_core::icons::{self, IconKind};
use homedit_core::secrets::{MASK, is_secret_key};
use homedit_core::{
    Bookmark, BookmarkGroup, Dashboard, Group, GroupLayout, InfoWidget, Service, Settings,
    StatusCheck, Theme,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// Whole document as the frontend sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocView {
    /// Page settings.
    pub settings: SettingsView,
    /// Service groups.
    pub groups: Vec<GroupView>,
    /// Bookmark groups.
    #[serde(default)]
    pub bookmarks: Vec<BookmarkGroupView>,
    /// Header widgets.
    #[serde(default)]
    pub info_widgets: Vec<InfoWidgetView>,
    /// Version the document was read at. Ignored on input; the `If-Match`
    /// header carries the expected version.
    #[serde(default)]
    pub version: i64,
}

/// Page settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    /// Page title.
    pub title: String,
    /// `light`, `dark` or `system`.
    pub theme: String,
    /// Accent palette name.
    #[serde(default)]
    pub accent: Option<String>,
    /// Background image.
    #[serde(default)]
    pub background: Option<BackgroundView>,
    /// Extra CSS.
    #[serde(default)]
    pub custom_css: Option<String>,
    /// Search bar provider.
    #[serde(default)]
    pub search: Option<SearchView>,
}

/// Background image settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundView {
    /// Image URL.
    pub url: String,
    /// Blur (CSS length or a Tailwind-style size such as `sm`).
    #[serde(default)]
    pub blur: Option<String>,
    /// Opacity in percent.
    #[serde(default)]
    pub opacity: Option<u16>,
}

/// Search bar settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchView {
    /// `google`, `duckduckgo`, `bing`, `brave`, `startpage` or `custom`.
    pub provider: String,
    /// URL template for `custom`; `{q}` is replaced by the query.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// A group of cards.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupView {
    /// Database id, `null` for a new group.
    #[serde(default)]
    pub id: Option<i64>,
    /// Display name.
    pub name: String,
    /// Group icon (URL in the dashboard view, name in the config doc).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Hidden from viewers.
    #[serde(default)]
    pub hidden: bool,
    /// Layout.
    #[serde(default)]
    pub layout: LayoutView,
    /// Cards.
    #[serde(default)]
    pub items: Vec<ServiceView>,
    /// Nested groups.
    #[serde(default)]
    pub groups: Vec<GroupView>,
}

/// Group layout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutView {
    /// Number of columns.
    #[serde(default)]
    pub columns: Option<u8>,
    /// `grid`, `row` or `column`.
    #[serde(default = "default_style")]
    pub style: String,
}

fn default_style() -> String {
    "column".into()
}

impl Default for LayoutView {
    fn default() -> Self {
        Self {
            columns: None,
            style: default_style(),
        }
    }
}

/// A service card.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceView {
    /// Database id, `null` for a new card.
    #[serde(default)]
    pub id: Option<i64>,
    /// Display name.
    pub name: String,
    /// Icon (URL in the dashboard view, name in the config doc).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// How to draw the icon: `image`, or `mask` for a tinted glyph.
    /// Output only.
    #[serde(default, skip_serializing_if = "Option::is_none", skip_deserializing)]
    pub icon_kind: Option<IconKind>,
    /// Link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
    /// Short text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Status check.
    #[serde(default)]
    pub check: Option<CheckView>,
    /// Service widget.
    #[serde(default)]
    pub widget: Option<WidgetView>,
}

/// A status check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckView {
    /// `http`, `tcp` or `ping` (`ping` is a TCP connect; no raw sockets).
    pub kind: String,
    /// URL for `http`; `host` or `host:port` for `tcp`/`ping`.
    #[serde(default)]
    pub target: Option<String>,
}

/// A service widget: its type and its fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WidgetView {
    /// Widget type (`sonarr`, `plex`, ...).
    #[serde(rename = "type")]
    pub kind: String,
    /// Widget fields except `type`.
    #[serde(default)]
    pub fields: Map<String, Value>,
}

/// A bookmark group.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkGroupView {
    /// Database id.
    #[serde(default)]
    pub id: Option<i64>,
    /// Display name.
    pub name: String,
    /// Bookmarks.
    #[serde(default)]
    pub items: Vec<BookmarkView>,
}

/// A bookmark.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkView {
    /// Database id.
    #[serde(default)]
    pub id: Option<i64>,
    /// Display name.
    pub name: String,
    /// Abbreviation shown without an icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abbr: Option<String>,
    /// Icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Link.
    pub href: String,
}

/// A header widget.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InfoWidgetView {
    /// Database id.
    #[serde(default)]
    pub id: Option<i64>,
    /// `clock`, `weather`, `resources`, `docker`, or another kind.
    #[serde(rename = "type")]
    pub kind: String,
    /// Kind-specific options.
    #[serde(default)]
    pub options: Map<String, Value>,
}

/// Which document is being produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Dashboard view for rendering; `show_hidden` keeps hidden groups.
    Dashboard {
        /// Include groups hidden from viewers.
        show_hidden: bool,
    },
    /// Editable config document.
    Config,
}

/// The placeholder for a stored secret in the config document.
pub fn secret_placeholder() -> Value {
    json!({ "$secret": "set" })
}

fn is_secret_placeholder(v: &Value) -> bool {
    v.get("$secret").and_then(Value::as_str) == Some("set")
}

/// Builds the frontend document from the stored dashboard.
pub fn to_view(d: &Dashboard, version: i64, mode: Mode) -> DocView {
    let show_hidden = match mode {
        Mode::Dashboard { show_hidden } => show_hidden,
        Mode::Config => true,
    };
    DocView {
        settings: settings_view(&d.settings, &d.info_widgets),
        groups: d
            .groups
            .iter()
            .filter_map(|g| group_view(g, mode, show_hidden))
            .collect(),
        bookmarks: d
            .bookmark_groups
            .iter()
            .map(|bg| BookmarkGroupView {
                id: bg.id,
                name: bg.name.clone(),
                items: bg
                    .bookmarks
                    .iter()
                    .map(|b| BookmarkView {
                        id: b.id,
                        name: b.name.clone(),
                        abbr: b.abbr.clone(),
                        icon: icon_out(b.icon.as_deref(), mode).0,
                        href: b.href.clone(),
                    })
                    .collect(),
            })
            .collect(),
        info_widgets: d
            .info_widgets
            .iter()
            .filter(|w| w.kind != "search")
            .map(|w| InfoWidgetView {
                id: w.id,
                kind: info_kind_out(&w.kind).to_owned(),
                options: w.options.clone(),
            })
            .collect(),
        version,
    }
}

fn icon_out(name: Option<&str>, mode: Mode) -> (Option<String>, Option<IconKind>) {
    match (name, mode) {
        (None, _) => (None, None),
        (Some(n), Mode::Config) => (Some(n.to_owned()), None),
        (Some(n), Mode::Dashboard { .. }) => match icons::resolve(n) {
            Some(r) => (Some(r.url), Some(r.kind)),
            None => (None, None),
        },
    }
}

fn settings_view(s: &Settings, widgets: &[InfoWidget]) -> SettingsView {
    // gethomepage configures search as an info widget; fall back to it.
    let search_widget = widgets.iter().find(|w| w.kind == "search");
    let provider = s.search_provider.clone().or_else(|| {
        search_widget.and_then(|w| {
            w.options
                .get("provider")
                .and_then(|p| p.as_str().or_else(|| p.get(0)?.as_str()))
                .map(str::to_owned)
        })
    });
    let url = s
        .search_url
        .clone()
        .or_else(|| search_widget.and_then(|w| w.options.get("url")?.as_str().map(str::to_owned)));
    SettingsView {
        title: s.title.clone(),
        theme: match s.theme {
            Theme::Light => "light",
            Theme::Dark => "dark",
            Theme::Auto => "system",
        }
        .into(),
        accent: s.color.clone(),
        background: s.background.as_ref().map(|b| BackgroundView {
            url: b.image.clone(),
            blur: b.blur.clone(),
            opacity: b.opacity,
        }),
        custom_css: s.custom_css.clone(),
        search: provider.map(|provider| SearchView { provider, url }),
    }
}

fn group_view(g: &Group, mode: Mode, show_hidden: bool) -> Option<GroupView> {
    if g.hidden_from_viewers && !show_hidden {
        return None;
    }
    Some(GroupView {
        id: g.id,
        name: g.name.clone(),
        icon: icon_out(g.layout.icon.as_deref(), mode).0,
        hidden: g.hidden_from_viewers,
        layout: LayoutView {
            columns: g.layout.columns,
            style: g.layout.style.clone().unwrap_or_else(default_style),
        },
        items: g.services.iter().map(|s| service_view(s, mode)).collect(),
        groups: g
            .groups
            .iter()
            .filter_map(|c| group_view(c, mode, show_hidden))
            .collect(),
    })
}

fn service_view(s: &Service, mode: Mode) -> ServiceView {
    let (icon, icon_kind) = icon_out(s.icon.as_deref(), mode);
    let check = match StatusCheck::for_service(s) {
        Some(StatusCheck::Http { url }) => Some(CheckView {
            kind: "http".into(),
            target: Some(url),
        }),
        Some(StatusCheck::Tcp { .. }) => Some(CheckView {
            kind: "tcp".into(),
            target: s.ping.clone(),
        }),
        None => None,
    };
    let widget = s.widget.as_ref().and_then(Value::as_object).map(|w| {
        let mut fields = w.clone();
        let kind = fields
            .remove("type")
            .and_then(|t| t.as_str().map(str::to_owned))
            .unwrap_or_default();
        for (k, v) in fields.iter_mut() {
            if is_secret_key(k) && !v.is_null() && v.as_str() != Some("") {
                *v = match mode {
                    Mode::Config => secret_placeholder(),
                    Mode::Dashboard { .. } => Value::String(MASK.into()),
                };
            }
        }
        WidgetView { kind, fields }
    });
    ServiceView {
        id: s.id,
        name: s.name.clone(),
        icon,
        icon_kind,
        href: s.href.clone(),
        description: s.description.clone(),
        check,
        widget,
    }
}

fn info_kind_out(kind: &str) -> &str {
    match kind {
        "datetime" => "clock",
        "openmeteo" | "openweathermap" | "weatherapi" => "weather",
        other => other,
    }
}

fn info_kind_in(kind: &str) -> &str {
    match kind {
        "clock" => "datetime",
        "weather" => "openmeteo",
        other => other,
    }
}

/// Error from [`from_view`].
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ViewError {
    /// A field failed validation.
    #[error("{0}")]
    Invalid(String),
}

/// Builds the dashboard to store from an edited config document.
///
/// `stored` is the current dashboard; it supplies secrets kept with the
/// placeholder and every field the view does not model.
pub fn from_view(doc: &DocView, stored: &Dashboard) -> Result<Dashboard, ViewError> {
    let old_services: std::collections::HashMap<i64, &Service> = stored
        .services()
        .filter_map(|s| s.id.map(|id| (id, s)))
        .collect();
    let mut old_groups = std::collections::HashMap::new();
    fn index<'a>(gs: &'a [Group], m: &mut std::collections::HashMap<i64, &'a Group>) {
        for g in gs {
            if let Some(id) = g.id {
                m.insert(id, g);
            }
            index(&g.groups, m);
        }
    }
    index(&stored.groups, &mut old_groups);

    let mut groups = Vec::with_capacity(doc.groups.len());
    for g in &doc.groups {
        groups.push(group_in(g, &old_groups, &old_services)?);
    }

    let settings = settings_in(&doc.settings, &stored.settings)?;
    let old_bg: std::collections::HashMap<i64, &BookmarkGroup> = stored
        .bookmark_groups
        .iter()
        .filter_map(|b| b.id.map(|id| (id, b)))
        .collect();
    let mut bookmark_groups = Vec::new();
    for bg in &doc.bookmarks {
        let old = bg.id.and_then(|id| old_bg.get(&id));
        let old_b: std::collections::HashMap<i64, &Bookmark> = old
            .map(|o| {
                o.bookmarks
                    .iter()
                    .filter_map(|b| b.id.map(|id| (id, b)))
                    .collect()
            })
            .unwrap_or_default();
        let mut bookmarks = Vec::new();
        for b in &bg.items {
            require(&b.name, "bookmark name")?;
            require(&b.href, "bookmark link")?;
            let prev = b.id.and_then(|id| old_b.get(&id));
            bookmarks.push(Bookmark {
                id: b.id,
                name: b.name.trim().to_owned(),
                href: b.href.trim().to_owned(),
                abbr: b.abbr.clone(),
                icon: b.icon.clone(),
                description: prev.and_then(|p| p.description.clone()),
            });
        }
        require(&bg.name, "bookmark group name")?;
        bookmark_groups.push(BookmarkGroup {
            id: bg.id,
            name: bg.name.trim().to_owned(),
            layout: old.map(|o| o.layout.clone()).unwrap_or_default(),
            bookmarks,
        });
    }

    let mut info_widgets: Vec<InfoWidget> = doc
        .info_widgets
        .iter()
        .map(|w| InfoWidget {
            id: w.id,
            kind: info_kind_in(&w.kind).to_owned(),
            options: w.options.clone(),
        })
        .collect();
    // The search widget is edited through settings.search; keep the stored
    // one only when settings do not override it.
    if settings.search_provider.is_none() {
        info_widgets.extend(
            stored
                .info_widgets
                .iter()
                .filter(|w| w.kind == "search")
                .cloned(),
        );
    }

    Ok(Dashboard {
        settings,
        groups,
        bookmark_groups,
        info_widgets,
        docker_hosts: stored.docker_hosts.clone(),
    })
}

fn require(value: &str, what: &str) -> Result<(), ViewError> {
    if value.trim().is_empty() {
        Err(ViewError::Invalid(format!("{what} must not be empty")))
    } else {
        Ok(())
    }
}

fn settings_in(v: &SettingsView, old: &Settings) -> Result<Settings, ViewError> {
    let theme = match v.theme.as_str() {
        "light" => Theme::Light,
        "dark" => Theme::Dark,
        "system" | "auto" => Theme::Auto,
        other => return Err(ViewError::Invalid(format!("unknown theme {other:?}"))),
    };
    let mut s = old.clone();
    s.title = v.title.trim().to_owned();
    s.theme = theme;
    s.color = v.accent.clone();
    s.custom_css = v.custom_css.clone();
    s.background = v.background.as_ref().map(|b| {
        let mut bg = old.background.clone().unwrap_or_default();
        bg.image = b.url.clone();
        bg.blur = b.blur.clone();
        bg.opacity = b.opacity;
        bg
    });
    s.search_provider = v.search.as_ref().map(|x| x.provider.clone());
    s.search_url = v.search.as_ref().and_then(|x| x.url.clone());
    Ok(s)
}

fn group_in(
    g: &GroupView,
    old_groups: &std::collections::HashMap<i64, &Group>,
    old_services: &std::collections::HashMap<i64, &Service>,
) -> Result<Group, ViewError> {
    require(&g.name, "group name")?;
    if !matches!(g.layout.style.as_str(), "grid" | "row" | "column") {
        return Err(ViewError::Invalid(format!(
            "unknown layout style {:?}",
            g.layout.style
        )));
    }
    let old = g.id.and_then(|id| old_groups.get(&id));
    let layout = GroupLayout {
        style: Some(g.layout.style.clone()),
        columns: g.layout.columns,
        icon: g.icon.clone(),
        ..old.map(|o| o.layout.clone()).unwrap_or_default()
    };
    let mut services = Vec::with_capacity(g.items.len());
    for s in &g.items {
        services.push(service_in(
            s,
            s.id.and_then(|id| old_services.get(&id).copied()),
        )?);
    }
    let mut groups = Vec::with_capacity(g.groups.len());
    for c in &g.groups {
        groups.push(group_in(c, old_groups, old_services)?);
    }
    Ok(Group {
        id: g.id,
        name: g.name.trim().to_owned(),
        layout,
        hidden_from_viewers: g.hidden,
        services,
        groups,
    })
}

fn service_in(v: &ServiceView, old: Option<&Service>) -> Result<Service, ViewError> {
    require(&v.name, "service name")?;
    let mut s = old.cloned().unwrap_or_default();
    s.id = v.id;
    s.name = v.name.trim().to_owned();
    s.icon = v.icon.clone().filter(|i| !i.trim().is_empty());
    s.href = v.href.clone().filter(|h| !h.trim().is_empty());
    s.description = v.description.clone().filter(|d| !d.trim().is_empty());
    s.site_monitor = None;
    s.ping = None;
    if let Some(c) = &v.check {
        let target = c.target.clone().filter(|t| !t.trim().is_empty());
        match c.kind.as_str() {
            "http" => {
                let t = target.or_else(|| s.href.clone()).ok_or_else(|| {
                    ViewError::Invalid(format!("{}: HTTP check needs a URL", s.name))
                })?;
                s.site_monitor = Some(t);
            }
            "tcp" | "ping" => {
                let t = target.ok_or_else(|| {
                    ViewError::Invalid(format!("{}: TCP check needs a host", s.name))
                })?;
                s.ping = Some(t);
            }
            other => {
                return Err(ViewError::Invalid(format!("unknown check kind {other:?}")));
            }
        }
        if StatusCheck::for_service(&s).is_none() {
            return Err(ViewError::Invalid(format!(
                "{}: check target is not valid",
                s.name
            )));
        }
    }
    s.widget = match &v.widget {
        None => None,
        Some(w) => {
            require(&w.kind, "widget type")?;
            let old_fields = old
                .and_then(|o| o.widget.as_ref())
                .and_then(Value::as_object);
            let mut out = Map::new();
            out.insert("type".into(), Value::String(w.kind.clone()));
            for (k, val) in &w.fields {
                if k == "type" {
                    continue;
                }
                if is_secret_placeholder(val) || val.as_str() == Some(MASK) {
                    // Keep the stored secret, if there is one.
                    if let Some(prev) = old_fields.and_then(|f| f.get(k)) {
                        out.insert(k.clone(), prev.clone());
                    }
                } else if !val.is_null() {
                    out.insert(k.clone(), val.clone());
                }
            }
            Some(Value::Object(out))
        }
    };
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored() -> Dashboard {
        let mut svc = Service::new("Sonarr", Some("http://sonarr:8989"));
        svc.id = Some(7);
        svc.icon = Some("sonarr.png".into());
        svc.site_monitor = Some("http://sonarr:8989".into());
        svc.widget = Some(json!({"type":"sonarr","url":"http://sonarr:8989","key":"SECRET"}));
        svc.extra.insert("weight".into(), json!(3));
        let hidden = Group {
            id: Some(2),
            name: "Admin".into(),
            hidden_from_viewers: true,
            ..Group::default()
        };
        Dashboard {
            groups: vec![
                Group {
                    id: Some(1),
                    name: "Media".into(),
                    services: vec![svc],
                    ..Group::default()
                },
                hidden,
            ],
            ..Dashboard::default()
        }
    }

    #[test]
    fn dashboard_view_masks_and_resolves() {
        let v = to_view(&stored(), 4, Mode::Dashboard { show_hidden: false });
        assert_eq!(v.version, 4);
        assert_eq!(v.groups.len(), 1, "hidden group dropped");
        let s = &v.groups[0].items[0];
        assert!(s.icon.as_deref().unwrap().starts_with("https://"));
        assert_eq!(s.icon_kind, Some(IconKind::Image));
        let w = s.widget.as_ref().unwrap();
        assert_eq!(w.kind, "sonarr");
        assert_eq!(w.fields["key"], json!(MASK));
        assert_eq!(s.check.as_ref().unwrap().kind, "http");
        let text = serde_json::to_string(&v).unwrap();
        assert!(!text.contains("SECRET"));
    }

    #[test]
    fn config_round_trip_keeps_secret_and_extra() {
        let st = stored();
        let doc = to_view(&st, 1, Mode::Config);
        assert_eq!(doc.groups.len(), 2);
        assert_eq!(doc.groups[0].items[0].icon.as_deref(), Some("sonarr.png"));
        assert_eq!(
            doc.groups[0].items[0].widget.as_ref().unwrap().fields["key"],
            secret_placeholder()
        );
        let back = from_view(&doc, &st).unwrap();
        let svc = &back.groups[0].services[0];
        assert_eq!(svc.widget.as_ref().unwrap()["key"], json!("SECRET"));
        assert_eq!(svc.extra["weight"], json!(3));
        assert_eq!(
            back.groups,
            st.groups
                .iter()
                .map(|g| {
                    let mut g = g.clone();
                    g.layout.style = Some("column".into());
                    g
                })
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn replace_and_clear_secret() {
        let st = stored();
        let mut doc = to_view(&st, 1, Mode::Config);
        let fields = &mut doc.groups[0].items[0].widget.as_mut().unwrap().fields;
        fields.insert("key".into(), json!("NEW"));
        let back = from_view(&doc, &st).unwrap();
        assert_eq!(
            back.groups[0].services[0].widget.as_ref().unwrap()["key"],
            json!("NEW")
        );

        let fields = &mut doc.groups[0].items[0].widget.as_mut().unwrap().fields;
        fields.insert("key".into(), Value::Null);
        let back = from_view(&doc, &st).unwrap();
        assert!(
            back.groups[0].services[0]
                .widget
                .as_ref()
                .unwrap()
                .get("key")
                .is_none()
        );
    }

    #[test]
    fn placeholder_on_new_service_is_dropped() {
        let st = stored();
        let mut doc = to_view(&st, 1, Mode::Config);
        let mut new = doc.groups[0].items[0].clone();
        new.id = None;
        doc.groups[1].items.push(new);
        let back = from_view(&doc, &st).unwrap();
        let w = back.groups[1].services[0].widget.as_ref().unwrap();
        assert!(
            w.get("key").is_none(),
            "secret must not be copied to a new card"
        );
    }

    #[test]
    fn validation() {
        let st = stored();
        let mut doc = to_view(&st, 1, Mode::Config);
        doc.groups[0].items[0].check = Some(CheckView {
            kind: "tcp".into(),
            target: None,
        });
        assert!(from_view(&doc, &st).is_err());
        let mut doc = to_view(&st, 1, Mode::Config);
        doc.settings.theme = "neon".into();
        assert!(from_view(&doc, &st).is_err());
        let mut doc = to_view(&st, 1, Mode::Config);
        doc.groups[0].name = " ".into();
        assert!(from_view(&doc, &st).is_err());
    }

    #[test]
    fn search_widget_maps_to_settings() {
        let mut d = stored();
        d.info_widgets.push(InfoWidget {
            id: Some(1),
            kind: "search".into(),
            options: json!({"provider":"duckduckgo"})
                .as_object()
                .unwrap()
                .clone(),
        });
        let v = to_view(&d, 1, Mode::Dashboard { show_hidden: true });
        assert_eq!(v.settings.search.unwrap().provider, "duckduckgo");
        assert!(v.info_widgets.is_empty());
    }
}
