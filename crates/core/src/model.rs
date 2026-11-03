//! The dashboard document.
//!
//! The field names follow gethomepage's YAML keys (camelCase on the wire) so
//! that an imported `services.yaml` maps onto these types with few surprises.
//! Every struct keeps unknown keys in an `extra` map, so nothing from an
//! imported config is silently lost when it is saved again.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Database row id. `None` means "not saved yet".
pub type Id = Option<i64>;

/// The complete dashboard: everything needed to render the page.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Dashboard {
    /// Page-wide settings (title, theme, background, ...).
    pub settings: Settings,
    /// Top-level service groups, in display order.
    pub groups: Vec<Group>,
    /// Bookmark groups, in display order.
    pub bookmark_groups: Vec<BookmarkGroup>,
    /// Info widgets shown in the header (clock, weather, resources, search).
    pub info_widgets: Vec<InfoWidget>,
    /// Docker hosts that services may reference by name.
    pub docker_hosts: Vec<DockerHost>,
}

impl Dashboard {
    /// Iterates over every service in every group, including nested groups,
    /// depth first in display order.
    pub fn services(&self) -> impl Iterator<Item = &Service> {
        let mut out = Vec::new();
        fn walk<'a>(groups: &'a [Group], out: &mut Vec<&'a Service>) {
            for g in groups {
                out.extend(g.services.iter());
                walk(&g.groups, out);
            }
        }
        walk(&self.groups, &mut out);
        out.into_iter()
    }
}

/// Light, dark, or follow the browser.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    /// Always light.
    Light,
    /// Always dark.
    Dark,
    /// Follow `prefers-color-scheme`.
    #[default]
    Auto,
}

/// Page-wide settings, modelled on gethomepage's `settings.yaml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Page title shown in the header and the browser tab.
    pub title: String,
    /// Optional subtitle / meta description.
    pub description: Option<String>,
    /// Favicon URL or icon name.
    pub favicon: Option<String>,
    /// Colour scheme.
    pub theme: Theme,
    /// Accent palette name (e.g. `slate`, `blue`).
    pub color: Option<String>,
    /// Background image settings.
    pub background: Option<Background>,
    /// Extra CSS appended to the page.
    pub custom_css: Option<String>,
    /// UI language (BCP 47 tag).
    pub language: Option<String>,
    /// Web search provider used by the search bar (`google`, `duckduckgo`,
    /// `bing`, `brave`, `startpage`, or `custom`).
    pub search_provider: Option<String>,
    /// URL template for a custom search provider; `{q}` is replaced by the
    /// URL-encoded query.
    pub search_url: Option<String>,
    /// Seconds between status checks. Defaults to 30.
    pub status_interval_secs: u32,
    /// Open links in a new tab (`_blank`) or the same one (`_self`).
    pub target: Option<String>,
    /// Unknown keys, kept so they round-trip.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            title: "Homedit".into(),
            description: None,
            favicon: None,
            theme: Theme::Auto,
            color: None,
            background: None,
            custom_css: None,
            language: None,
            search_provider: None,
            search_url: None,
            status_interval_secs: 30,
            target: None,
            extra: Map::new(),
        }
    }
}

/// Background image and effects.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Background {
    /// Image URL (or a path under `/icons/custom/`).
    pub image: String,
    /// CSS blur amount, e.g. `sm`, `md`, or a pixel value.
    pub blur: Option<String>,
    /// Saturation percentage.
    pub saturate: Option<u16>,
    /// Brightness percentage.
    pub brightness: Option<u16>,
    /// Opacity percentage.
    pub opacity: Option<u16>,
}

/// How a group lays out its cards.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GroupLayout {
    /// `row` (cards side by side in a grid) or `column` (stacked).
    pub style: Option<String>,
    /// Number of columns when `style` is `row`.
    pub columns: Option<u8>,
    /// Show the group header. Defaults to true.
    pub header: Option<bool>,
    /// Icon shown next to the group name.
    pub icon: Option<String>,
    /// Start collapsed.
    pub initially_collapsed: Option<bool>,
    /// Tab name, for dashboards split into tabs.
    pub tab: Option<String>,
    /// Unknown keys, kept so they round-trip.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A named group of service cards. Groups can nest.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Group {
    /// Database id.
    pub id: Id,
    /// Display name. Unique among siblings.
    pub name: String,
    /// Layout options.
    pub layout: GroupLayout,
    /// Hide this group from users with the viewer role.
    pub hidden_from_viewers: bool,
    /// Cards in display order.
    pub services: Vec<Service>,
    /// Nested groups in display order.
    pub groups: Vec<Group>,
}

/// One service card.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Service {
    /// Database id. Stable across saves; status history is keyed on it.
    pub id: Id,
    /// Display name.
    pub name: String,
    /// Link opened when the card is clicked.
    pub href: Option<String>,
    /// Short text under the name.
    pub description: Option<String>,
    /// Icon name (`sonarr`, `sonarr.png`, `sh-sonarr`, `mdi-server`,
    /// `si-github`), absolute URL, or custom upload path.
    pub icon: Option<String>,
    /// Host (optionally `host:port`) to check with a TCP connect.
    pub ping: Option<String>,
    /// URL to check with an HTTP request.
    pub site_monitor: Option<String>,
    /// Docker host name (from [`Dashboard::docker_hosts`]).
    pub server: Option<String>,
    /// Container name on that Docker host.
    pub container: Option<String>,
    /// Service widget configuration (`type` plus widget-specific fields).
    /// Kept as raw JSON; the widgets crate validates it.
    pub widget: Option<Value>,
    /// Unknown keys, kept so they round-trip.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Service {
    /// Creates a service with just a name and link.
    pub fn new(name: impl Into<String>, href: Option<&str>) -> Self {
        Self {
            name: name.into(),
            href: href.map(str::to_owned),
            ..Self::default()
        }
    }
}

/// A named list of bookmarks.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BookmarkGroup {
    /// Database id.
    pub id: Id,
    /// Display name.
    pub name: String,
    /// Layout options.
    pub layout: GroupLayout,
    /// Bookmarks in display order.
    pub bookmarks: Vec<Bookmark>,
}

/// One bookmark link.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Bookmark {
    /// Database id.
    pub id: Id,
    /// Display name.
    pub name: String,
    /// Link target.
    pub href: String,
    /// Two-letter abbreviation shown when there is no icon.
    pub abbr: Option<String>,
    /// Icon name or URL (same rules as [`Service::icon`]).
    pub icon: Option<String>,
    /// Short text under the name.
    pub description: Option<String>,
}

/// A header widget such as the clock, weather, host resources or search.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct InfoWidget {
    /// Database id.
    pub id: Id,
    /// Widget kind (`datetime`, `openmeteo`, `resources`, `search`, ...).
    pub kind: String,
    /// Kind-specific options.
    pub options: Map<String, Value>,
}

/// A Docker daemon that services can reference by name.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DockerHost {
    /// Name referenced by [`Service::server`].
    pub name: String,
    /// Unix socket path, e.g. `/var/run/docker.sock`.
    pub socket: Option<String>,
    /// TCP host for a remote daemon or socket proxy.
    pub host: Option<String>,
    /// TCP port for a remote daemon or socket proxy.
    pub port: Option<u16>,
    /// Unknown keys (TLS settings and so on), kept so they round-trip.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}
