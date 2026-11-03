//! Plain data types produced by the importer.
//!
//! These types mirror the structure of a gethomepage configuration closely,
//! but use Rust naming and explicit optional fields. Anything the importer
//! does not model explicitly is preserved in an `extra` map, so nothing from
//! the source configuration is silently lost.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A JSON object, used for free-form options and unknown fields.
pub type Object = Map<String, Value>;

/// Everything read from a gethomepage configuration folder.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ImportedConfig {
    /// Global settings from `settings.yaml`.
    pub settings: Settings,
    /// Service groups from `services.yaml`, in file order.
    pub services: Vec<ServiceGroup>,
    /// Bookmark groups from `bookmarks.yaml`, in file order.
    pub bookmarks: Vec<BookmarkGroup>,
    /// Information widgets (top bar) from `widgets.yaml`, in file order.
    pub widgets: Vec<InfoWidget>,
    /// Docker servers from `docker.yaml`, sorted by name.
    pub docker: Vec<DockerServer>,
    /// Non-fatal problems found while importing (skipped entries, unknown
    /// shapes, unresolved variables).
    pub warnings: Vec<ImportWarning>,
}

impl ImportedConfig {
    /// Iterates over every service in every group, including nested groups,
    /// together with the path of group names that contains it.
    pub fn all_services(&self) -> Vec<(Vec<&str>, &Service)> {
        fn walk<'a>(
            group: &'a ServiceGroup,
            path: &mut Vec<&'a str>,
            out: &mut Vec<(Vec<&'a str>, &'a Service)>,
        ) {
            path.push(&group.name);
            for service in &group.services {
                out.push((path.clone(), service));
            }
            for child in &group.groups {
                walk(child, path, out);
            }
            path.pop();
        }
        let mut out = Vec::new();
        for group in &self.services {
            walk(group, &mut Vec::new(), &mut out);
        }
        out
    }
}

/// A non-fatal problem found during import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportWarning {
    /// The source file name, such as `services.yaml`.
    pub file: String,
    /// A path inside the file, such as `Media/Plex/widget`.
    pub path: String,
    /// What went wrong, in plain words.
    pub message: String,
}

/// A named group of services. Groups may contain nested sub-groups.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ServiceGroup {
    /// The group's display name.
    pub name: String,
    /// Services directly in this group, in file order.
    pub services: Vec<Service>,
    /// Nested sub-groups, in file order.
    pub groups: Vec<ServiceGroup>,
}

/// One service card.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Service {
    /// The card's display name.
    pub name: String,
    /// The link opened when the card is clicked.
    pub href: Option<String>,
    /// An icon reference: a dashboard-icons name (`plex.png`), a selfh.st
    /// name (`sh-plex`), a Material Design name (`mdi-...`), or a URL.
    pub icon: Option<String>,
    /// A short description shown under the name.
    pub description: Option<String>,
    /// A host or URL to check with ICMP/TCP ping (`ping:` in gethomepage).
    pub ping: Option<String>,
    /// A URL to check with an HTTP request (`siteMonitor:` in gethomepage).
    pub site_monitor: Option<String>,
    /// The name of a Docker server from `docker.yaml`.
    pub server: Option<String>,
    /// The container name on that Docker server.
    pub container: Option<String>,
    /// Whether to show container stats by default.
    pub show_stats: Option<bool>,
    /// Sort weight; lower sorts first.
    pub weight: Option<i64>,
    /// Link target, such as `_blank` or `_self`.
    pub target: Option<String>,
    /// Service widgets attached to the card (`widget:` and `widgets:`).
    pub widgets: Vec<ServiceWidget>,
    /// Fields the importer does not model explicitly, kept verbatim.
    pub extra: Object,
}

/// A service widget, such as a Sonarr queue or Plex stream count.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ServiceWidget {
    /// The widget type, such as `sonarr` or `customapi`.
    pub kind: String,
    /// All other widget options (`url`, `key`, `fields`, ...), verbatim.
    pub options: Object,
}

impl ServiceWidget {
    /// The widget's `url` option, if it is a string.
    pub fn url(&self) -> Option<&str> {
        self.options.get("url").and_then(Value::as_str)
    }

    /// Names of options that hold secrets (API keys, passwords, tokens), in
    /// sorted order. Callers should store these encrypted and never send
    /// them back to the browser.
    pub fn secret_fields(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self
            .options
            .keys()
            .map(String::as_str)
            .filter(|name| is_secret_field(name))
            .collect();
        names.sort_unstable();
        names
    }
}

/// Returns true when an option name looks like it holds a secret.
///
/// Matches names such as `key`, `apiKey`, `api_key`, `password`, `token`,
/// `accessToken`, `secret` and `clientSecret`, case-insensitively. Names
/// that point to a file instead of holding the secret (such as `keyFile`)
/// do not match.
pub fn is_secret_field(name: &str) -> bool {
    let lower = name.to_ascii_lowercase().replace(['_', '-'], "");
    if lower.ends_with("file") || lower.ends_with("path") {
        return false;
    }
    const EXACT: &[&str] = &["key", "pass", "passwd", "pin", "auth", "cookie"];
    const SUFFIXES: &[&str] = &["key", "password", "token", "secret"];
    EXACT.contains(&lower.as_str()) || SUFFIXES.iter().any(|suffix| lower.ends_with(suffix))
}

/// A named group of bookmarks.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BookmarkGroup {
    /// The group's display name.
    pub name: String,
    /// Bookmarks in this group, in file order.
    pub bookmarks: Vec<Bookmark>,
}

/// One bookmark link.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Bookmark {
    /// The bookmark's display name.
    pub name: String,
    /// The link target.
    pub href: Option<String>,
    /// A short abbreviation shown when there is no icon (`abbr:`).
    pub abbr: Option<String>,
    /// An icon reference, as for [`Service::icon`].
    pub icon: Option<String>,
    /// A short description.
    pub description: Option<String>,
    /// Fields the importer does not model explicitly, kept verbatim.
    pub extra: Object,
}

/// An information widget from `widgets.yaml`, such as `resources`,
/// `datetime`, `openmeteo` or `search`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct InfoWidget {
    /// The widget type.
    pub kind: String,
    /// The widget's options, verbatim.
    pub options: Object,
}

/// A Docker server from `docker.yaml`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DockerServer {
    /// The server's name, referenced by [`Service::server`].
    pub name: String,
    /// Path to a Unix socket, such as `/var/run/docker.sock`.
    pub socket: Option<String>,
    /// Hostname or IP of a Docker API (or socket proxy) over TCP.
    pub host: Option<String>,
    /// TCP port of the Docker API.
    pub port: Option<u16>,
    /// TLS options (`caFile`, `certFile`, `keyFile`), verbatim.
    pub tls: Option<Value>,
    /// Whether the server is a Swarm manager.
    pub swarm: bool,
    /// Fields the importer does not model explicitly, kept verbatim.
    pub extra: Object,
}

/// Global settings from `settings.yaml`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// Page title.
    pub title: Option<String>,
    /// Page description (meta tag).
    pub description: Option<String>,
    /// Favicon URL.
    pub favicon: Option<String>,
    /// `light` or `dark`.
    pub theme: Option<String>,
    /// Accent palette name, such as `slate` or `zinc`.
    pub color: Option<String>,
    /// Background image: either a URL string or an object with `image`,
    /// `blur`, `saturate`, `brightness` and `opacity`.
    pub background: Option<Value>,
    /// Blur applied to cards, such as `sm` or `md`.
    pub card_blur: Option<String>,
    /// Header style, such as `boxed` or `underlined`.
    pub header_style: Option<String>,
    /// UI language code.
    pub language: Option<String>,
    /// Default link target.
    pub target: Option<String>,
    /// Per-group layout, in the order the groups should be shown.
    pub layout: Vec<GroupLayout>,
    /// Fields the importer does not model explicitly, kept verbatim.
    pub extra: Object,
}

/// Layout options for one group, from the `layout:` setting.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GroupLayout {
    /// The group's name (service or bookmark group).
    pub group: String,
    /// `row` or `column`.
    pub style: Option<String>,
    /// Number of columns when the style is `row`.
    pub columns: Option<u32>,
    /// Number of rows, when set.
    pub rows: Option<u32>,
    /// Whether to show the group header.
    pub header: Option<bool>,
    /// Icon shown next to the group header.
    pub icon: Option<String>,
    /// Tab the group belongs to.
    pub tab: Option<String>,
    /// Whether the group starts collapsed.
    pub initially_collapsed: Option<bool>,
    /// Fields the importer does not model explicitly, kept verbatim.
    pub extra: Object,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_secret_fields() {
        for name in [
            "key",
            "apiKey",
            "api_key",
            "password",
            "token",
            "accessToken",
            "clientSecret",
            "pass",
        ] {
            assert!(is_secret_field(name), "{name} should be secret");
        }
        for name in ["url", "username", "keyFile", "fields", "monkeys", "type"] {
            assert!(!is_secret_field(name), "{name} should not be secret");
        }
    }
}
