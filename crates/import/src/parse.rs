//! Parsers for each gethomepage YAML file.
//!
//! gethomepage uses "single-key map" lists heavily: a group is written as
//! `- GroupName: [ ... ]` and a service as `- ServiceName: { ... }`. These
//! shapes do not map well onto derived deserializers, so each file is first
//! parsed into a generic [`Value`] and then walked by hand. Entries with an
//! unexpected shape are skipped and reported as [`ImportWarning`]s, while a
//! file whose top level has the wrong shape is an [`ImportError`].

use serde_json::{Map, Value};

use crate::ImportError;
use crate::model::{
    Bookmark, BookmarkGroup, DockerServer, GroupLayout, ImportWarning, InfoWidget, Object, Service,
    ServiceGroup, ServiceWidget, Settings,
};
use crate::subst::{Resolver, substitute};

/// Shared state while parsing one file: its name, the resolver for
/// placeholders, and the warnings found so far.
pub(crate) struct Ctx<'a> {
    pub(crate) file: &'static str,
    pub(crate) resolver: &'a dyn Resolver,
    pub(crate) warnings: Vec<ImportWarning>,
}

impl<'a> Ctx<'a> {
    /// Creates a context for `file`.
    pub(crate) fn new(file: &'static str, resolver: &'a dyn Resolver) -> Self {
        Self {
            file,
            resolver,
            warnings: Vec::new(),
        }
    }

    fn warn(&mut self, path: &str, message: impl Into<String>) {
        self.warnings.push(ImportWarning {
            file: self.file.to_string(),
            path: path.to_string(),
            message: message.into(),
        });
    }

    /// Parses YAML text into a JSON value with placeholders substituted.
    /// Empty documents become `Value::Null`.
    pub(crate) fn load(&mut self, text: &str) -> Result<Value, ImportError> {
        let yaml: serde_yaml_ng::Value =
            serde_yaml_ng::from_str(text).map_err(|source| ImportError::Yaml {
                file: self.file.to_string(),
                source,
            })?;
        let mut value = yaml_to_json(yaml);
        self.substitute_all(&mut value);
        Ok(value)
    }

    fn substitute_all(&mut self, value: &mut Value) {
        match value {
            Value::String(text) if text.contains("{{") => {
                let (out, missing) = substitute(text, self.resolver);
                for name in missing {
                    self.warn(
                        "",
                        format!("placeholder {name} is not set; left as written"),
                    );
                }
                *text = out;
            }
            Value::Array(items) => items.iter_mut().for_each(|item| self.substitute_all(item)),
            Value::Object(map) => map.values_mut().for_each(|item| self.substitute_all(item)),
            _ => {}
        }
    }

    fn shape(&self, message: impl Into<String>) -> ImportError {
        ImportError::Shape {
            file: self.file.to_string(),
            message: message.into(),
        }
    }
}

/// Converts YAML to JSON. Non-string map keys are turned into strings and
/// YAML tags are dropped (keeping the tagged value).
fn yaml_to_json(value: serde_yaml_ng::Value) -> Value {
    use serde_yaml_ng::Value as Y;
    match value {
        Y::Null => Value::Null,
        Y::Bool(b) => Value::Bool(b),
        Y::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::from(i)
            } else if let Some(u) = n.as_u64() {
                Value::from(u)
            } else {
                n.as_f64()
                    .and_then(serde_json::Number::from_f64)
                    .map_or(Value::Null, Value::Number)
            }
        }
        Y::String(s) => Value::String(s),
        Y::Sequence(items) => Value::Array(items.into_iter().map(yaml_to_json).collect()),
        Y::Mapping(map) => {
            let mut out = Map::new();
            for (key, value) in map {
                let key = match key {
                    Y::String(s) => s,
                    other => match yaml_to_json(other) {
                        Value::String(s) => s,
                        Value::Null => "null".to_string(),
                        v => v.to_string(),
                    },
                };
                out.insert(key, yaml_to_json(value));
            }
            Value::Object(out)
        }
        Y::Tagged(tagged) => yaml_to_json(tagged.value),
    }
}

/// Splits a single-key map such as `{ "Plex": {...} }` into its key and
/// value. Returns `None` for any other shape.
fn single_entry(value: Value) -> Option<(String, Value)> {
    match value {
        Value::Object(map) if map.len() == 1 => map.into_iter().next(),
        _ => None,
    }
}

/// Removes `key` from `map` and returns it if it is a string (numbers and
/// booleans are converted to text).
fn take_string(map: &mut Object, key: &str) -> Option<String> {
    match map.remove(key)? {
        Value::String(s) => Some(s),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        other => {
            // Put back values we cannot represent as text.
            map.insert(key.to_string(), other);
            None
        }
    }
}

/// Removes `key` from `map` and returns it as a boolean, accepting `true`,
/// `false` and their string forms.
fn take_bool(map: &mut Object, key: &str) -> Option<bool> {
    match map.remove(key)? {
        Value::Bool(b) => Some(b),
        Value::String(s) if s.eq_ignore_ascii_case("true") => Some(true),
        Value::String(s) if s.eq_ignore_ascii_case("false") => Some(false),
        other => {
            map.insert(key.to_string(), other);
            None
        }
    }
}

/// Removes `key` from `map` and returns it as an integer, accepting numbers
/// and numeric strings.
fn take_i64(map: &mut Object, key: &str) -> Option<i64> {
    match map.remove(key)? {
        Value::Number(n) if n.is_i64() => n.as_i64(),
        Value::String(s) if s.trim().parse::<i64>().is_ok() => s.trim().parse().ok(),
        other => {
            map.insert(key.to_string(), other);
            None
        }
    }
}

fn take_u32(map: &mut Object, key: &str) -> Option<u32> {
    let value = take_i64(map, key)?;
    match u32::try_from(value) {
        Ok(v) => Some(v),
        Err(_) => {
            map.insert(key.to_string(), Value::from(value));
            None
        }
    }
}

/// Parses `services.yaml`.
pub(crate) fn services(ctx: &mut Ctx<'_>, text: &str) -> Result<Vec<ServiceGroup>, ImportError> {
    let root = ctx.load(text)?;
    let items = match root {
        Value::Null => return Ok(Vec::new()),
        Value::Array(items) => items,
        _ => return Err(ctx.shape("expected a list of groups at the top level")),
    };
    let mut groups = Vec::new();
    for (index, item) in items.into_iter().enumerate() {
        match single_entry(item) {
            Some((name, body)) => groups.push(service_group(ctx, name, body)),
            None => ctx.warn(
                &format!("[{index}]"),
                "expected `- Group Name: [...]`; entry skipped",
            ),
        }
    }
    Ok(groups)
}

fn service_group(ctx: &mut Ctx<'_>, name: String, body: Value) -> ServiceGroup {
    let mut group = ServiceGroup {
        name,
        ..ServiceGroup::default()
    };
    let items = match body {
        Value::Array(items) => items,
        Value::Null => Vec::new(),
        _ => {
            ctx.warn(&group.name, "group body is not a list; group left empty");
            return group;
        }
    };
    for (index, item) in items.into_iter().enumerate() {
        let Some((entry_name, entry_body)) = single_entry(item) else {
            ctx.warn(
                &format!("{}[{index}]", group.name),
                "expected `- Service Name: {...}`; entry skipped",
            );
            continue;
        };
        match entry_body {
            // A list under a name is a nested group.
            Value::Array(_) => group
                .groups
                .push(service_group(ctx, entry_name, entry_body)),
            Value::Object(fields) => {
                let path = format!("{}/{}", group.name, entry_name);
                group.services.push(service(ctx, &path, entry_name, fields));
            }
            Value::Null => group.services.push(Service {
                name: entry_name,
                ..Service::default()
            }),
            _ => ctx.warn(
                &format!("{}/{}", group.name, entry_name),
                "service body is not a map; entry skipped",
            ),
        }
    }
    group
}

/// Builds a [`Service`] from its field map. Also used by Docker discovery,
/// which produces the same field names from container labels.
pub(crate) fn service(ctx: &mut Ctx<'_>, path: &str, name: String, mut fields: Object) -> Service {
    let mut widgets = Vec::new();
    if let Some(widget) = fields.remove("widget")
        && let Some(widget) = service_widget(ctx, &format!("{path}/widget"), widget)
    {
        widgets.push(widget);
    }
    match fields.remove("widgets") {
        Some(Value::Array(items)) => {
            for (index, item) in items.into_iter().enumerate() {
                if let Some(widget) = service_widget(ctx, &format!("{path}/widgets[{index}]"), item)
                {
                    widgets.push(widget);
                }
            }
        }
        Some(Value::Null) | None => {}
        Some(_) => ctx.warn(path, "`widgets` is not a list; ignored"),
    }
    Service {
        name,
        href: take_string(&mut fields, "href"),
        icon: take_string(&mut fields, "icon"),
        description: take_string(&mut fields, "description"),
        ping: take_string(&mut fields, "ping"),
        site_monitor: take_string(&mut fields, "siteMonitor"),
        server: take_string(&mut fields, "server"),
        container: take_string(&mut fields, "container"),
        show_stats: take_bool(&mut fields, "showStats"),
        weight: take_i64(&mut fields, "weight"),
        target: take_string(&mut fields, "target"),
        widgets,
        extra: fields,
    }
}

fn service_widget(ctx: &mut Ctx<'_>, path: &str, value: Value) -> Option<ServiceWidget> {
    let Value::Object(mut options) = value else {
        ctx.warn(path, "widget is not a map; ignored");
        return None;
    };
    let Some(kind) = take_string(&mut options, "type") else {
        ctx.warn(path, "widget has no `type`; ignored");
        return None;
    };
    Some(ServiceWidget { kind, options })
}

/// Parses `bookmarks.yaml`.
pub(crate) fn bookmarks(ctx: &mut Ctx<'_>, text: &str) -> Result<Vec<BookmarkGroup>, ImportError> {
    let root = ctx.load(text)?;
    let items = match root {
        Value::Null => return Ok(Vec::new()),
        Value::Array(items) => items,
        _ => return Err(ctx.shape("expected a list of groups at the top level")),
    };
    let mut groups = Vec::new();
    for (index, item) in items.into_iter().enumerate() {
        let Some((name, body)) = single_entry(item) else {
            ctx.warn(
                &format!("[{index}]"),
                "expected `- Group Name: [...]`; entry skipped",
            );
            continue;
        };
        let mut group = BookmarkGroup {
            name,
            bookmarks: Vec::new(),
        };
        let entries = match body {
            Value::Array(entries) => entries,
            Value::Null => Vec::new(),
            _ => {
                ctx.warn(&group.name, "group body is not a list; group left empty");
                groups.push(group);
                continue;
            }
        };
        for (index, entry) in entries.into_iter().enumerate() {
            let path = format!("{}[{index}]", group.name);
            let Some((entry_name, entry_body)) = single_entry(entry) else {
                ctx.warn(&path, "expected `- Bookmark Name: [...]`; entry skipped");
                continue;
            };
            // gethomepage wraps the fields in a one-element list; accept a
            // bare map too.
            let fields = match entry_body {
                Value::Array(mut list) if !list.is_empty() => list.swap_remove(0),
                other => other,
            };
            let Value::Object(mut fields) = fields else {
                ctx.warn(&path, "bookmark body is not a map; entry skipped");
                continue;
            };
            group.bookmarks.push(Bookmark {
                name: entry_name,
                href: take_string(&mut fields, "href"),
                abbr: take_string(&mut fields, "abbr"),
                icon: take_string(&mut fields, "icon"),
                description: take_string(&mut fields, "description"),
                extra: fields,
            });
        }
        groups.push(group);
    }
    Ok(groups)
}

/// Parses `widgets.yaml`.
pub(crate) fn widgets(ctx: &mut Ctx<'_>, text: &str) -> Result<Vec<InfoWidget>, ImportError> {
    let root = ctx.load(text)?;
    let items = match root {
        Value::Null => return Ok(Vec::new()),
        Value::Array(items) => items,
        _ => return Err(ctx.shape("expected a list of widgets at the top level")),
    };
    let mut out = Vec::new();
    for (index, item) in items.into_iter().enumerate() {
        let Some((kind, body)) = single_entry(item) else {
            ctx.warn(
                &format!("[{index}]"),
                "expected `- widgetType: {...}`; entry skipped",
            );
            continue;
        };
        let options = match body {
            Value::Object(map) => map,
            Value::Null => Map::new(),
            _ => {
                ctx.warn(&kind, "widget options are not a map; options dropped");
                Map::new()
            }
        };
        out.push(InfoWidget { kind, options });
    }
    Ok(out)
}

/// Parses `docker.yaml`.
pub(crate) fn docker(ctx: &mut Ctx<'_>, text: &str) -> Result<Vec<DockerServer>, ImportError> {
    let root = ctx.load(text)?;
    let map = match root {
        Value::Null => return Ok(Vec::new()),
        Value::Object(map) => map,
        _ => return Err(ctx.shape("expected a map of server names at the top level")),
    };
    let mut out = Vec::new();
    for (name, body) in map {
        let Value::Object(mut fields) = body else {
            ctx.warn(&name, "server body is not a map; entry skipped");
            continue;
        };
        let port = take_i64(&mut fields, "port");
        let port = match port.map(u16::try_from) {
            Some(Ok(port)) => Some(port),
            Some(Err(_)) => {
                ctx.warn(&name, "port is out of range; ignored");
                None
            }
            None => None,
        };
        let server = DockerServer {
            socket: take_string(&mut fields, "socket"),
            host: take_string(&mut fields, "host"),
            port,
            tls: fields.remove("tls"),
            swarm: take_bool(&mut fields, "swarm").unwrap_or(false),
            extra: fields,
            name,
        };
        if server.socket.is_none() && server.host.is_none() {
            ctx.warn(&server.name, "server has neither `socket` nor `host`");
        }
        out.push(server);
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Parses `settings.yaml`.
pub(crate) fn settings(ctx: &mut Ctx<'_>, text: &str) -> Result<Settings, ImportError> {
    let root = ctx.load(text)?;
    let mut map = match root {
        Value::Null => return Ok(Settings::default()),
        Value::Object(map) => map,
        _ => return Err(ctx.shape("expected a map at the top level")),
    };
    let layout = match map.remove("layout") {
        None | Some(Value::Null) => Vec::new(),
        // `layout:` as a map keeps the file's order (serde_json is built
        // with `preserve_order`).
        Some(Value::Object(groups)) => groups
            .into_iter()
            .map(|(group, body)| group_layout(ctx, group, body))
            .collect(),
        // `layout:` as a list of single-key maps.
        Some(Value::Array(items)) => items
            .into_iter()
            .enumerate()
            .filter_map(|(index, item)| match single_entry(item) {
                Some((group, body)) => Some(group_layout(ctx, group, body)),
                None => {
                    ctx.warn(
                        &format!("layout[{index}]"),
                        "expected `- Group: {...}`; skipped",
                    );
                    None
                }
            })
            .collect(),
        Some(_) => {
            ctx.warn("layout", "layout is neither a map nor a list; ignored");
            Vec::new()
        }
    };
    let background = map.remove("background").filter(|v| !v.is_null());
    Ok(Settings {
        title: take_string(&mut map, "title"),
        description: take_string(&mut map, "description"),
        favicon: take_string(&mut map, "favicon"),
        theme: take_string(&mut map, "theme"),
        color: take_string(&mut map, "color"),
        background,
        card_blur: take_string(&mut map, "cardBlur"),
        header_style: take_string(&mut map, "headerStyle"),
        language: take_string(&mut map, "language"),
        target: take_string(&mut map, "target"),
        layout,
        extra: map,
    })
}

fn group_layout(ctx: &mut Ctx<'_>, group: String, body: Value) -> GroupLayout {
    let mut fields = match body {
        Value::Object(map) => map,
        Value::Null => Map::new(),
        _ => {
            ctx.warn(
                &format!("layout/{group}"),
                "layout entry is not a map; ignored",
            );
            Map::new()
        }
    };
    GroupLayout {
        style: take_string(&mut fields, "style"),
        columns: take_u32(&mut fields, "columns"),
        rows: take_u32(&mut fields, "rows"),
        header: take_bool(&mut fields, "header"),
        icon: take_string(&mut fields, "icon"),
        tab: take_string(&mut fields, "tab"),
        initially_collapsed: take_bool(&mut fields, "initiallyCollapsed"),
        extra: fields,
        group,
    }
}
