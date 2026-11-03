//! Conversion from the importer's types into [`homedit_core::Dashboard`].
//!
//! The importer first reads gethomepage files into the types in
//! [`crate::model`], which follow the files closely. This module maps those
//! onto homedit's own data model so the server can save the result with one
//! call. Fields homedit has no column for are kept in each struct's `extra`
//! map under their gethomepage key, so an export can write them back.

use homedit_core::model as core;
use serde_json::{Map, Value};

use crate::model::{
    Bookmark, BookmarkGroup, DockerServer, GroupLayout, ImportWarning, ImportedConfig, InfoWidget,
    Service, ServiceGroup, Settings,
};

/// The result of importing a gethomepage folder: a dashboard ready to save,
/// plus any warnings to show the admin.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportReport {
    /// The imported dashboard. Ids are all `None` (not saved yet).
    pub dashboard: core::Dashboard,
    /// Non-fatal problems found during import and conversion.
    pub warnings: Vec<ImportWarning>,
}

impl ImportedConfig {
    /// Converts this config into a [`core::Dashboard`].
    ///
    /// Group layouts from `settings.yaml` are attached to the service and
    /// bookmark groups with the same name. A `search` info widget also sets
    /// the dashboard's search provider.
    pub fn into_report(self) -> ImportReport {
        let mut warnings = self.warnings;
        let layouts = &self.settings.layout;

        let groups = self
            .services
            .into_iter()
            .map(|g| group(g, layouts))
            .collect();
        let bookmark_groups = self
            .bookmarks
            .into_iter()
            .map(|g| bookmark_group(g, layouts, &mut warnings))
            .collect();
        let mut settings = settings(self.settings, &mut warnings);
        if let Some(search) = self.widgets.iter().find(|w| w.kind == "search") {
            if let Some(provider) = search.options.get("provider").and_then(Value::as_str) {
                settings.search_provider = Some(provider.to_string());
            }
            if let Some(url) = search.options.get("url").and_then(Value::as_str) {
                // gethomepage appends the query to `url`; homedit uses `{q}`.
                let template = if url.contains("{q}") {
                    url.to_string()
                } else {
                    format!("{url}{{q}}")
                };
                settings.search_url = Some(template);
            }
        }
        let info_widgets = self.widgets.into_iter().map(info_widget).collect();
        let docker_hosts = self.docker.into_iter().map(docker_host).collect();

        ImportReport {
            dashboard: core::Dashboard {
                settings,
                groups,
                bookmark_groups,
                info_widgets,
                docker_hosts,
            },
            warnings,
        }
    }
}

fn find_layout<'a>(layouts: &'a [GroupLayout], name: &str) -> Option<&'a GroupLayout> {
    layouts.iter().find(|l| l.group == name)
}

fn layout(source: Option<&GroupLayout>) -> core::GroupLayout {
    let Some(source) = source else {
        return core::GroupLayout::default();
    };
    let mut extra = source.extra.clone();
    if let Some(rows) = source.rows {
        extra.insert("rows".into(), Value::from(rows));
    }
    let columns = source.columns.map(|c| u8::try_from(c).unwrap_or(u8::MAX));
    core::GroupLayout {
        style: source.style.clone(),
        columns,
        header: source.header,
        icon: source.icon.clone(),
        initially_collapsed: source.initially_collapsed,
        tab: source.tab.clone(),
        extra,
    }
}

fn group(source: ServiceGroup, layouts: &[GroupLayout]) -> core::Group {
    core::Group {
        id: None,
        layout: layout(find_layout(layouts, &source.name)),
        hidden_from_viewers: false,
        services: source.services.into_iter().map(to_core_service).collect(),
        groups: source
            .groups
            .into_iter()
            .map(|g| group(g, layouts))
            .collect(),
        name: source.name,
    }
}

/// Turns a widget into gethomepage-shaped JSON: `{"type": ..., ...options}`.
fn widget_json(widget: crate::model::ServiceWidget) -> Value {
    let mut map = Map::with_capacity(widget.options.len() + 1);
    map.insert("type".into(), Value::String(widget.kind));
    map.extend(widget.options);
    Value::Object(map)
}

/// Converts one imported service into a [`core::Service`].
///
/// The first widget becomes [`core::Service::widget`]; any further widgets
/// are kept in `extra["widgets"]`. `showStats`, `weight` and `target` are
/// kept in `extra` under their gethomepage keys.
pub fn to_core_service(source: Service) -> core::Service {
    let mut extra = source.extra;
    if let Some(show) = source.show_stats {
        extra.insert("showStats".into(), Value::Bool(show));
    }
    if let Some(weight) = source.weight {
        extra.insert("weight".into(), Value::from(weight));
    }
    if let Some(target) = source.target {
        extra.insert("target".into(), Value::String(target));
    }
    let mut widgets = source.widgets.into_iter().map(widget_json);
    let widget = widgets.next();
    let rest: Vec<Value> = widgets.collect();
    if !rest.is_empty() {
        extra.insert("widgets".into(), Value::Array(rest));
    }
    core::Service {
        id: None,
        name: source.name,
        href: source.href,
        description: source.description,
        icon: source.icon,
        ping: source.ping,
        site_monitor: source.site_monitor,
        server: source.server,
        container: source.container,
        widget,
        extra,
    }
}

fn bookmark_group(
    source: BookmarkGroup,
    layouts: &[GroupLayout],
    warnings: &mut Vec<ImportWarning>,
) -> core::BookmarkGroup {
    let layout = layout(find_layout(layouts, &source.name));
    let group_name = source.name;
    let bookmarks = source
        .bookmarks
        .into_iter()
        .map(|b| bookmark(&group_name, b, warnings))
        .collect();
    core::BookmarkGroup {
        id: None,
        name: group_name,
        layout,
        bookmarks,
    }
}

fn bookmark(group: &str, source: Bookmark, warnings: &mut Vec<ImportWarning>) -> core::Bookmark {
    if !source.extra.is_empty() {
        let keys: Vec<&str> = source.extra.keys().map(String::as_str).collect();
        warnings.push(ImportWarning {
            file: "bookmarks.yaml".into(),
            path: format!("{group}/{}", source.name),
            message: format!(
                "fields not supported on bookmarks were dropped: {}",
                keys.join(", ")
            ),
        });
    }
    if source.href.is_none() {
        warnings.push(ImportWarning {
            file: "bookmarks.yaml".into(),
            path: format!("{group}/{}", source.name),
            message: "bookmark has no href".into(),
        });
    }
    core::Bookmark {
        id: None,
        name: source.name,
        href: source.href.unwrap_or_default(),
        abbr: source.abbr,
        icon: source.icon,
        description: source.description,
    }
}

fn info_widget(source: InfoWidget) -> core::InfoWidget {
    core::InfoWidget {
        id: None,
        kind: source.kind,
        options: source.options,
    }
}

fn docker_host(source: DockerServer) -> core::DockerHost {
    let mut extra = source.extra;
    if let Some(tls) = source.tls {
        extra.insert("tls".into(), tls);
    }
    if source.swarm {
        extra.insert("swarm".into(), Value::Bool(true));
    }
    core::DockerHost {
        name: source.name,
        socket: source.socket,
        host: source.host,
        port: source.port,
        extra,
    }
}

fn percent(value: Option<&Value>) -> Option<u16> {
    match value? {
        Value::Number(n) => n.as_u64().and_then(|v| u16::try_from(v).ok()),
        Value::String(s) => s.trim().trim_end_matches('%').parse().ok(),
        _ => None,
    }
}

fn background(value: Value, warnings: &mut Vec<ImportWarning>) -> Option<core::Background> {
    match value {
        Value::String(image) => Some(core::Background {
            image,
            ..core::Background::default()
        }),
        Value::Object(map) => {
            let Some(image) = map.get("image").and_then(Value::as_str) else {
                warnings.push(ImportWarning {
                    file: "settings.yaml".into(),
                    path: "background".into(),
                    message: "background has no image; ignored".into(),
                });
                return None;
            };
            let blur = match map.get("blur") {
                Some(Value::String(s)) => Some(s.clone()),
                Some(Value::Number(n)) => Some(n.to_string()),
                _ => None,
            };
            Some(core::Background {
                image: image.to_string(),
                blur,
                saturate: percent(map.get("saturate")),
                brightness: percent(map.get("brightness")),
                opacity: percent(map.get("opacity")),
            })
        }
        _ => {
            warnings.push(ImportWarning {
                file: "settings.yaml".into(),
                path: "background".into(),
                message: "background is neither a URL nor a map; ignored".into(),
            });
            None
        }
    }
}

fn settings(source: Settings, warnings: &mut Vec<ImportWarning>) -> core::Settings {
    let defaults = core::Settings::default();
    let mut extra = source.extra;
    if let Some(blur) = source.card_blur {
        extra.insert("cardBlur".into(), Value::String(blur));
    }
    if let Some(style) = source.header_style {
        extra.insert("headerStyle".into(), Value::String(style));
    }
    let theme = match source.theme.as_deref() {
        Some("light") => core::Theme::Light,
        Some("dark") => core::Theme::Dark,
        None => core::Theme::Auto,
        Some(other) => {
            warnings.push(ImportWarning {
                file: "settings.yaml".into(),
                path: "theme".into(),
                message: format!("unknown theme `{other}`; using auto"),
            });
            core::Theme::Auto
        }
    };
    core::Settings {
        title: source.title.unwrap_or(defaults.title),
        description: source.description,
        favicon: source.favicon,
        theme,
        color: source.color,
        background: source.background.and_then(|b| background(b, warnings)),
        language: source.language,
        target: source.target,
        extra,
        ..defaults
    }
}
