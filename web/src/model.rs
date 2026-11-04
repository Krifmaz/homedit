//! The dashboard document as the web app works with it, and its translation from and to the server's JSON.
//!
//! The server stores ids as integers (`null` for a new item), versions as integers, timestamps as Unix
//! milliseconds, and keeps gethomepage-style settings (an accent palette name, background blur as a CSS size,
//! opacity in percent, search URL templates with `{q}`). The app uses string ids it can mint locally, string
//! versions, a CSS accent colour, a blur in pixels, an opacity from 0 to 1 and `%s` templates.
//!
//! Every function here is pure, so the mapping is tested without a server or a browser.

use serde_json::{Map, Value, json};
use std::cell::Cell;
use std::collections::BTreeMap;

/// A role a visitor has. Only admins may edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Sees the dashboard.
    Viewer,
    /// Can edit.
    Admin,
}

impl Role {
    /// The role named `s` (`viewer`, `admin`).
    pub fn parse(s: &str) -> Option<Role> {
        match s {
            "viewer" => Some(Role::Viewer),
            "admin" => Some(Role::Admin),
            _ => None,
        }
    }

    /// The role's name on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Viewer => "viewer",
            Role::Admin => "admin",
        }
    }
}

/// Sign-in methods the server offers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AuthMethods {
    /// User name and password.
    pub local: bool,
    /// The single sign-on button's label, `None` when it is off.
    pub oidc: Option<String>,
    /// A trusted reverse proxy signs people in.
    pub proxy: bool,
    /// Where Sign out sends the browser for the proxy.
    pub proxy_logout_url: Option<String>,
}

/// `GET /api/me`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Me {
    /// The signed-in user's name, `None` when anonymous or sign-in is off.
    pub user: Option<String>,
    /// Where the account comes from (`local`, `oidc`, `proxy`).
    pub source: Option<String>,
    /// The effective role; `None` with `auth_enabled` means the visitor must sign in first.
    pub role: Option<Role>,
    /// Whether sign-in is on.
    pub auth_enabled: bool,
    /// The token sent back as `X-CSRF-Token` on changes.
    pub csrf_token: String,
    /// No accounts yet: the first-run setup is offered.
    pub setup_required: bool,
    /// Sign-in methods.
    pub methods: AuthMethods,
}

impl Me {
    /// Reads `GET /api/me`.
    pub fn from_wire(v: &Value) -> Me {
        let user = v.get("user").filter(|u| u.is_object());
        let m = v.get("methods").cloned().unwrap_or(Value::Null);
        Me {
            user: user.and_then(|u| str_of(u, "name")),
            source: user.and_then(|u| str_of(u, "source")),
            role: v.get("role").and_then(Value::as_str).and_then(Role::parse),
            auth_enabled: v
                .get("authEnabled")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            csrf_token: str_of(v, "csrfToken").unwrap_or_default(),
            setup_required: v
                .get("setupRequired")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            methods: AuthMethods {
                local: m.get("local").and_then(Value::as_bool).unwrap_or(true),
                oidc: str_of(&m, "oidc"),
                proxy: m.get("proxy").and_then(Value::as_bool).unwrap_or(false),
                proxy_logout_url: str_of(&m, "proxyLogoutUrl"),
            },
        }
    }

    /// Whether this visitor may edit.
    pub fn is_admin(&self) -> bool {
        self.role == Some(Role::Admin)
    }
}

/// Light, dark, or follow the system.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ThemeMode {
    /// Light.
    Light,
    /// Dark.
    Dark,
    /// Follows `prefers-color-scheme`.
    #[default]
    System,
}

impl ThemeMode {
    /// The mode's name on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
            ThemeMode::System => "system",
        }
    }
}

/// The full-page background image.
#[derive(Clone, Debug, PartialEq)]
pub struct Background {
    /// Image URL.
    pub url: String,
    /// Blur in pixels.
    pub blur: f64,
    /// Opacity from 0 to 1.
    pub opacity: f64,
    /// Saturation in percent (100 unchanged).
    pub saturate: f64,
    /// Brightness in percent (100 unchanged).
    pub brightness: f64,
}

impl Background {
    /// A background showing `url` unchanged.
    pub fn new(url: &str) -> Background {
        Background {
            url: url.into(),
            blur: 0.0,
            opacity: 1.0,
            saturate: 100.0,
            brightness: 100.0,
        }
    }
}

/// Where the search bar sends a query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Search {
    /// Google.
    Google,
    /// DuckDuckGo (the default).
    DuckDuckGo,
    /// Bing.
    Bing,
    /// A URL where `%s` is the query.
    Custom(String),
}

impl Search {
    /// The provider's name on the wire.
    pub fn provider(&self) -> &'static str {
        match self {
            Search::Google => "google",
            Search::DuckDuckGo => "duckduckgo",
            Search::Bing => "bing",
            Search::Custom(_) => "custom",
        }
    }

    /// The query URL template, `%s` for the query.
    pub fn template(&self) -> String {
        match self {
            Search::Google => "https://www.google.com/search?q=%s".into(),
            Search::DuckDuckGo => "https://duckduckgo.com/?q=%s".into(),
            Search::Bing => "https://www.bing.com/search?q=%s".into(),
            Search::Custom(u) => u.clone(),
        }
    }

    /// The URL that searches for `q`.
    pub fn url_for(&self, q: &str) -> String {
        let t = self.template();
        let t = if t.contains("%s") {
            t
        } else {
            Search::DuckDuckGo.template()
        };
        t.replace("%s", &js_encode(q))
    }
}

/// `encodeURIComponent` in plain Rust.
pub fn js_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Dashboard-wide settings.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// The page title.
    pub title: String,
    /// Light, dark or system.
    pub theme: ThemeMode,
    /// The accent colour (`#rrggbb`), empty for the theme's own.
    pub accent: String,
    /// Background image.
    pub background: Option<Background>,
    /// Frosted-glass blur behind cards in pixels, 0 for solid cards.
    pub card_blur: f64,
    /// Extra CSS.
    pub custom_css: String,
    /// The search bar.
    pub search: Search,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            title: String::new(),
            theme: ThemeMode::System,
            accent: String::new(),
            background: None,
            card_blur: 0.0,
            custom_css: String::new(),
            search: Search::DuckDuckGo,
        }
    }
}

/// How a group lays out its cards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutStyle {
    /// Fills columns.
    Grid,
    /// One scrolling row.
    Row,
    /// Stacked.
    Column,
}

impl LayoutStyle {
    /// The style's name on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            LayoutStyle::Grid => "grid",
            LayoutStyle::Row => "row",
            LayoutStyle::Column => "column",
        }
    }

    /// The style named `s`, grid otherwise.
    pub fn parse(s: &str) -> LayoutStyle {
        match s {
            "row" => LayoutStyle::Row,
            "column" => LayoutStyle::Column,
            _ => LayoutStyle::Grid,
        }
    }
}

/// A group's layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    /// Columns on wide screens (1 to 8).
    pub columns: u32,
    /// Grid, row or column.
    pub style: LayoutStyle,
}

impl Default for Layout {
    fn default() -> Self {
        Layout {
            columns: 3,
            style: LayoutStyle::Grid,
        }
    }
}

/// A status check.
#[derive(Clone, Debug, PartialEq)]
pub struct Check {
    /// `http`, `tcp` or `ping`.
    pub kind: String,
    /// The target, when not the card's link.
    pub target: Option<String>,
    /// The expected HTTP status.
    pub expect_status: Option<u16>,
}

/// A service widget on a card.
#[derive(Clone, Debug, PartialEq)]
pub struct WidgetConfig {
    /// The widget type from the catalogue (`sonarr`).
    pub kind: String,
    /// Field values by key; a stored secret is `{"$secret":"set"}`.
    pub fields: Map<String, Value>,
}

/// Whether a value is the placeholder the server sends for a stored secret.
pub fn is_secret_placeholder(v: &Value) -> bool {
    v.get("$secret").and_then(Value::as_str) == Some("set")
}

/// A service card.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Service {
    /// Server id, or one minted here for a new card.
    pub id: String,
    /// Name.
    pub name: String,
    /// Icon (a resolved URL in `/api/dashboard`, a name in `/api/config`).
    pub icon: Option<String>,
    /// Whether the icon is a monochrome mask.
    pub icon_mask: bool,
    /// Link.
    pub href: Option<String>,
    /// One line under the name.
    pub description: Option<String>,
    /// Status check.
    pub check: Option<Check>,
    /// Service widget.
    pub widget: Option<WidgetConfig>,
}

/// A group of cards.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Group {
    /// Id.
    pub id: String,
    /// Heading.
    pub name: String,
    /// Heading icon.
    pub icon: Option<String>,
    /// Hidden from viewers.
    pub hidden: bool,
    /// Layout.
    pub layout: Layout,
    /// Cards.
    pub items: Vec<Service>,
    /// Sub-groups.
    pub groups: Vec<Group>,
}

/// A bookmark.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Bookmark {
    /// Id.
    pub id: String,
    /// Name.
    pub name: String,
    /// Up to two letters shown without an icon.
    pub abbr: Option<String>,
    /// Icon.
    pub icon: Option<String>,
    /// Whether the icon is a mask.
    pub icon_mask: bool,
    /// Link.
    pub href: String,
}

/// A list of bookmarks.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct BookmarkGroup {
    /// Id.
    pub id: String,
    /// Heading.
    pub name: String,
    /// Bookmarks.
    pub items: Vec<Bookmark>,
}

/// The info widget types.
pub const INFO_TYPES: [&str; 4] = ["clock", "weather", "resources", "docker"];

/// An info widget in the header strip.
#[derive(Clone, Debug, PartialEq)]
pub struct InfoWidget {
    /// Id.
    pub id: String,
    /// `clock`, `weather`, `resources` or `docker`.
    pub kind: String,
    /// Type-specific options.
    pub options: Map<String, Value>,
}

/// The whole document (`/api/dashboard`, `/api/config`, a history version).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Doc {
    /// Settings.
    pub settings: Settings,
    /// Groups.
    pub groups: Vec<Group>,
    /// Bookmark lists.
    pub bookmarks: Vec<BookmarkGroup>,
    /// Info widgets.
    pub info_widgets: Vec<InfoWidget>,
    /// The version, sent as `If-Match` on save.
    pub version: String,
}

/// Tailwind 500 shades for the accent names gethomepage uses.
pub const PALETTE: [(&str, &str); 23] = [
    ("slate", "#64748b"),
    ("gray", "#6b7280"),
    ("zinc", "#71717a"),
    ("neutral", "#737373"),
    ("stone", "#78716c"),
    ("red", "#ef4444"),
    ("orange", "#f97316"),
    ("amber", "#f59e0b"),
    ("yellow", "#eab308"),
    ("lime", "#84cc16"),
    ("green", "#22c55e"),
    ("emerald", "#10b981"),
    ("teal", "#14b8a6"),
    ("cyan", "#06b6d4"),
    ("sky", "#0ea5e9"),
    ("blue", "#3b82f6"),
    ("indigo", "#6366f1"),
    ("violet", "#8b5cf6"),
    ("purple", "#a855f7"),
    ("fuchsia", "#d946ef"),
    ("pink", "#ec4899"),
    ("rose", "#f43f5e"),
    ("white", "#ffffff"),
];

/// The accent the React app stored when none was picked; read as "the theme's own".
pub const OLD_DEFAULT_ACCENT: &str = "#3b6fd8";

fn str_of(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_owned)
}

fn num_of(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(Value::as_f64).filter(|n| n.is_finite())
}

fn arr_of<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn obj_of(v: &Value, k: &str) -> Map<String, Value> {
    v.get(k)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

/// Parses a stored blur (`"sm"`, `"8px"`, `"8"`, `8`) into pixels.
pub fn blur_to_px(v: &Value) -> f64 {
    match v {
        Value::Number(n) => n.as_f64().unwrap_or(0.0).max(0.0),
        Value::String(s) => {
            let s = s.trim().to_ascii_lowercase();
            let named = match s.as_str() {
                "none" => Some(0.0),
                "sm" => Some(4.0),
                "" => Some(8.0),
                "md" => Some(12.0),
                "lg" => Some(16.0),
                "xl" => Some(24.0),
                "2xl" => Some(40.0),
                "3xl" => Some(64.0),
                _ => None,
            };
            named.unwrap_or_else(|| {
                let digits: String = s
                    .chars()
                    .take_while(|c| c.is_ascii_digit() || *c == '.')
                    .collect();
                digits.parse::<f64>().map(|n| n.max(0.0)).unwrap_or(0.0)
            })
        }
        _ => 0.0,
    }
}

fn percent_or_100(v: Option<f64>) -> f64 {
    v.map(|n| n.round().clamp(0.0, 200.0)).unwrap_or(100.0)
}

fn wire_percent(n: f64) -> Value {
    let v = n.clamp(0.0, 200.0).round();
    if v == 100.0 {
        Value::Null
    } else {
        json!(v as i64)
    }
}

/// A server id (an integer) as a number; anything else was minted here and is `null`.
pub fn to_wire_id(id: &str) -> Value {
    if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) {
        id.parse::<i64>().map(Value::from).unwrap_or(Value::Null)
    } else {
        Value::Null
    }
}

thread_local! {
    static MINTED: Cell<u64> = const { Cell::new(0) };
}

/// A fresh id with a readable prefix (`s-k3x9a2`), never a number, so the server sees it as new.
pub fn new_id(prefix: &str) -> String {
    let n = MINTED.with(|c| {
        c.set(c.get() + 1);
        c.get()
    });
    let seed = random_u32() as u64 ^ (n.wrapping_mul(0x9E37_79B9));
    let mut s = String::new();
    let mut x = seed;
    for _ in 0..8 {
        let d = (x % 36) as u8;
        s.push(if d < 10 {
            (b'0' + d) as char
        } else {
            (b'a' + d - 10) as char
        });
        x /= 36;
        if x == 0 {
            x = seed.rotate_left(17) ^ n;
        }
    }
    format!("{prefix}-{s}")
}

#[cfg(target_arch = "wasm32")]
fn random_u32() -> u32 {
    (js_sys::Math::random() * u32::MAX as f64) as u32
}

#[cfg(not(target_arch = "wasm32"))]
fn random_u32() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(7)
}

fn id_from_wire(v: Option<&Value>, prefix: &str, next: &mut u64) -> String {
    match v {
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::String(s)) if !s.is_empty() => s.clone(),
        _ => {
            *next += 1;
            format!("{prefix}-new{next}")
        }
    }
}

impl Settings {
    /// Reads the server's settings.
    pub fn from_wire(v: &Value) -> Settings {
        let raw = str_of(v, "accent").unwrap_or_default();
        let raw = raw.trim();
        let accent = PALETTE
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(raw))
            .map(|(_, c)| c.to_string())
            .unwrap_or_else(|| raw.to_string());
        let accent = if accent.eq_ignore_ascii_case(OLD_DEFAULT_ACCENT) {
            String::new()
        } else {
            accent
        };
        let background = v.get("background").filter(|b| b.is_object()).and_then(|b| {
            let url = str_of(b, "url").filter(|u| !u.is_empty())?;
            Some(Background {
                url,
                blur: blur_to_px(b.get("blur").unwrap_or(&Value::Null)),
                opacity: num_of(b, "opacity")
                    .map(|o| (o / 100.0).clamp(0.0, 1.0))
                    .unwrap_or(1.0),
                saturate: percent_or_100(num_of(b, "saturate")),
                brightness: percent_or_100(num_of(b, "brightness")),
            })
        });
        let search = match v.get("search").filter(|s| s.is_object()) {
            Some(s) => match str_of(s, "provider").as_deref() {
                Some("google") => Search::Google,
                Some("duckduckgo") => Search::DuckDuckGo,
                Some("bing") => Search::Bing,
                _ => match str_of(s, "url").filter(|u| !u.is_empty()) {
                    Some(u) => Search::Custom(u.replace("{q}", "%s")),
                    None => Search::DuckDuckGo,
                },
            },
            None => Search::DuckDuckGo,
        };
        Settings {
            title: str_of(v, "title").unwrap_or_default(),
            theme: match v.get("theme").and_then(Value::as_str) {
                Some("light") => ThemeMode::Light,
                Some("dark") => ThemeMode::Dark,
                _ => ThemeMode::System,
            },
            accent,
            background,
            card_blur: blur_to_px(v.get("cardBlur").unwrap_or(&Value::Null)),
            custom_css: str_of(v, "customCss").unwrap_or_default(),
            search,
        }
    }

    /// Writes the settings in the server's shape.
    pub fn to_wire(&self) -> Value {
        let px = |n: f64| {
            if n > 0.0 {
                json!(format!("{}px", n.round() as i64))
            } else {
                Value::Null
            }
        };
        json!({
            "title": self.title,
            "theme": self.theme.as_str(),
            "accent": if self.accent.is_empty() { Value::Null } else { json!(self.accent) },
            "background": match &self.background {
                Some(b) if !b.url.is_empty() => json!({
                    "url": b.url,
                    "blur": px(b.blur),
                    "opacity": (b.opacity.clamp(0.0, 1.0) * 100.0).round() as i64,
                    "saturate": wire_percent(b.saturate),
                    "brightness": wire_percent(b.brightness),
                }),
                _ => Value::Null,
            },
            "cardBlur": px(self.card_blur),
            "customCss": if self.custom_css.is_empty() { Value::Null } else { json!(self.custom_css) },
            "search": match &self.search {
                Search::Custom(u) => json!({ "provider": "custom", "url": u.replace("%s", "{q}") }),
                s => json!({ "provider": s.provider() }),
            },
        })
    }
}

fn service_from_wire(o: &Value, next: &mut u64) -> Service {
    let check = o.get("check").filter(|c| c.is_object()).and_then(|c| {
        let kind = str_of(c, "kind")?;
        matches!(kind.as_str(), "http" | "tcp" | "ping").then(|| Check {
            kind,
            target: str_of(c, "target"),
            expect_status: num_of(c, "expectStatus").map(|n| n as u16),
        })
    });
    let widget = o
        .get("widget")
        .filter(|w| w.is_object())
        .map(|w| WidgetConfig {
            kind: str_of(w, "type").unwrap_or_default(),
            fields: obj_of(w, "fields"),
        });
    Service {
        id: id_from_wire(o.get("id"), "s", next),
        name: str_of(o, "name").unwrap_or_default(),
        icon: str_of(o, "icon"),
        icon_mask: o.get("iconKind").and_then(Value::as_str) == Some("mask"),
        href: str_of(o, "href"),
        description: str_of(o, "description"),
        check,
        widget,
    }
}

fn group_from_wire(o: &Value, next: &mut u64) -> Group {
    let layout = o.get("layout").cloned().unwrap_or(Value::Null);
    let style = LayoutStyle::parse(layout.get("style").and_then(Value::as_str).unwrap_or(""));
    let id = id_from_wire(o.get("id"), "g", next);
    Group {
        id,
        name: str_of(o, "name").unwrap_or_default(),
        icon: str_of(o, "icon"),
        hidden: o.get("hidden").and_then(Value::as_bool).unwrap_or(false),
        layout: Layout {
            columns: num_of(&layout, "columns")
                .map(|n| n as u32)
                .unwrap_or(if style == LayoutStyle::Row { 4 } else { 3 }),
            style,
        },
        items: arr_of(o, "items")
            .iter()
            .map(|s| service_from_wire(s, next))
            .collect(),
        groups: arr_of(o, "groups")
            .iter()
            .map(|g| group_from_wire(g, next))
            .collect(),
    }
}

fn opt(s: &Option<String>) -> Value {
    match s {
        Some(s) if !s.is_empty() => json!(s),
        _ => Value::Null,
    }
}

fn group_to_wire(g: &Group) -> Value {
    json!({
        "id": to_wire_id(&g.id),
        "name": g.name,
        "icon": opt(&g.icon),
        "hidden": g.hidden,
        "layout": { "columns": g.layout.columns, "style": g.layout.style.as_str() },
        "items": g.items.iter().map(|s| json!({
            "id": to_wire_id(&s.id),
            "name": s.name,
            "icon": opt(&s.icon),
            "href": opt(&s.href),
            "description": opt(&s.description),
            "check": s.check.as_ref().map(|c| json!({
                "kind": c.kind,
                "target": opt(&c.target),
                "expectStatus": c.expect_status,
            })),
            "widget": s.widget.as_ref().map(|w| json!({ "type": w.kind, "fields": w.fields })),
        })).collect::<Vec<_>>(),
        "groups": g.groups.iter().map(group_to_wire).collect::<Vec<_>>(),
    })
}

impl Doc {
    /// Reads a whole document.
    pub fn from_wire(d: &Value) -> Doc {
        let mut n = 0u64;
        let groups = arr_of(d, "groups")
            .iter()
            .map(|g| group_from_wire(g, &mut n))
            .collect();
        let bookmarks = arr_of(d, "bookmarks")
            .iter()
            .map(|o| BookmarkGroup {
                id: id_from_wire(o.get("id"), "b", &mut n),
                name: str_of(o, "name").unwrap_or_default(),
                items: arr_of(o, "items")
                    .iter()
                    .map(|b| Bookmark {
                        id: id_from_wire(b.get("id"), "bm", &mut n),
                        name: str_of(b, "name").unwrap_or_default(),
                        abbr: str_of(b, "abbr"),
                        icon: str_of(b, "icon"),
                        icon_mask: b.get("iconKind").and_then(Value::as_str) == Some("mask"),
                        href: str_of(b, "href").unwrap_or_default(),
                    })
                    .collect(),
            })
            .collect();
        let info_widgets = arr_of(d, "infoWidgets")
            .iter()
            .filter_map(|o| {
                let kind = str_of(o, "type").filter(|t| INFO_TYPES.contains(&t.as_str()))?;
                Some(InfoWidget {
                    id: id_from_wire(o.get("id"), "iw", &mut n),
                    kind,
                    options: obj_of(o, "options"),
                })
            })
            .collect();
        Doc {
            settings: Settings::from_wire(d.get("settings").unwrap_or(&Value::Null)),
            groups,
            bookmarks,
            info_widgets,
            version: match d.get("version") {
                None | Some(Value::Null) => String::new(),
                Some(Value::String(s)) => s.clone(),
                Some(v) => v.to_string(),
            },
        }
    }

    /// Writes the document for `PUT /api/config`.
    pub fn to_wire(&self) -> Value {
        json!({
            "settings": self.settings.to_wire(),
            "groups": self.groups.iter().map(group_to_wire).collect::<Vec<_>>(),
            "bookmarks": self.bookmarks.iter().map(|b| json!({
                "id": to_wire_id(&b.id),
                "name": b.name,
                "items": b.items.iter().map(|x| json!({
                    "id": to_wire_id(&x.id),
                    "name": x.name,
                    "abbr": opt(&x.abbr),
                    "icon": opt(&x.icon),
                    "href": x.href,
                })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "infoWidgets": self.info_widgets.iter().map(|w| json!({
                "id": to_wire_id(&w.id),
                "type": w.kind,
                "options": w.options,
            })).collect::<Vec<_>>(),
            "version": self.version.parse::<i64>().unwrap_or(0),
        })
    }
}

/// One status sample.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    /// Unix ms.
    pub t: f64,
    /// Whether the check passed.
    pub up: bool,
    /// Response time in ms.
    pub ms: Option<f64>,
}

/// A service's status.
#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    /// Up.
    pub up: bool,
    /// HTTP status.
    pub code: Option<u16>,
    /// Last response time in ms.
    pub latency_ms: Option<f64>,
    /// When it was checked (Unix ms).
    pub checked_at: f64,
    /// Recent samples, oldest first.
    pub history: Vec<Sample>,
}

/// Statuses by service id.
pub type StatusMap = BTreeMap<String, Status>;

/// Samples kept per service.
pub const HISTORY_LEN: usize = 60;

/// Reads `GET /api/status`.
pub fn status_from_wire(v: &Value) -> StatusMap {
    let mut out = StatusMap::new();
    let Some(o) = v.as_object() else { return out };
    for (k, raw) in o {
        if !raw.is_object() {
            continue;
        }
        out.insert(
            k.clone(),
            Status {
                up: raw.get("up").and_then(Value::as_bool).unwrap_or(false),
                code: num_of(raw, "code").map(|n| n as u16),
                latency_ms: num_of(raw, "latencyMs"),
                checked_at: num_of(raw, "checkedAt").unwrap_or(0.0),
                history: arr_of(raw, "history")
                    .iter()
                    .filter(|h| h.is_object())
                    .map(|h| Sample {
                        t: num_of(h, "t").unwrap_or(0.0),
                        up: h.get("up").and_then(Value::as_bool).unwrap_or(false),
                        ms: num_of(h, "ms"),
                    })
                    .collect(),
            },
        );
    }
    out
}

/// Applies one `status` event (`{serviceId, up, code, latencyMs, checkedAt}`) to `map`; false when it is not one.
pub fn apply_status_event(map: &mut StatusMap, v: &Value, now: f64) -> bool {
    let id = match v.get("serviceId") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => return false,
    };
    let checked_at = num_of(v, "checkedAt").unwrap_or(now);
    let up = v.get("up").and_then(Value::as_bool).unwrap_or(false);
    let latency_ms = num_of(v, "latencyMs");
    let mut history = map.remove(&id).map(|s| s.history).unwrap_or_default();
    history.push(Sample {
        t: checked_at,
        up,
        ms: latency_ms,
    });
    if history.len() > HISTORY_LEN {
        history.drain(..history.len() - HISTORY_LEN);
    }
    map.insert(
        id,
        Status {
            up,
            code: num_of(v, "code").map(|n| n as u16),
            latency_ms,
            checked_at,
            history,
        },
    );
    true
}

/// An entry of `GET /api/history`.
#[derive(Clone, Debug, PartialEq)]
pub struct HistoryEntry {
    /// Version.
    pub version: String,
    /// When it was saved, Unix ms.
    pub created_at: f64,
    /// Who saved it.
    pub author: String,
    /// What changed.
    pub summary: String,
}

/// Reads `GET /api/history`; times may be Unix ms or ISO text (read as 0 here and shown as is).
pub fn history_from_wire(v: &Value) -> Vec<HistoryEntry> {
    v.as_array()
        .map(|a| {
            a.iter()
                .map(|o| HistoryEntry {
                    version: match o.get("version") {
                        Some(Value::String(s)) => s.clone(),
                        Some(Value::Null) | None => String::new(),
                        Some(x) => x.to_string(),
                    },
                    created_at: num_of(o, "createdAt").unwrap_or(0.0),
                    author: str_of(o, "author").unwrap_or_else(|| "unknown".into()),
                    summary: str_of(o, "summary").unwrap_or_else(|| "Saved".into()),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// What the server put in the page for the first paint (`<script id="homedit-data">`): the dashboard and the
/// statuses.
pub fn inline_from_wire(raw: &Value) -> Option<(Doc, StatusMap)> {
    if let Some(d) = raw.get("dashboard") {
        return Some((
            Doc::from_wire(d),
            status_from_wire(raw.get("status").unwrap_or(&Value::Null)),
        ));
    }
    raw.get("groups")
        .map(|_| (Doc::from_wire(raw), StatusMap::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server_doc() -> Value {
        json!({
            "settings": {
                "title": "Lab", "theme": "dark", "accent": "emerald",
                "background": { "url": "/bg.jpg", "blur": "sm", "opacity": 50, "saturate": 120 },
                "cardBlur": "md", "customCss": null,
                "search": { "provider": "custom", "url": "https://s.test/?q={q}" }
            },
            "groups": [{
                "id": 7, "name": "Media", "hidden": false, "layout": { "columns": null, "style": "column" },
                "items": [{
                    "id": 12, "name": "Sonarr", "icon": "sonarr.png", "iconKind": "image",
                    "href": "http://sonarr:8989", "check": { "kind": "http", "target": null },
                    "widget": { "type": "sonarr", "fields": { "url": "http://sonarr:8989", "key": { "$secret": "set" } } }
                }],
                "groups": [{ "id": 8, "name": "Sub", "hidden": true, "layout": { "columns": 2, "style": "row" }, "items": [], "groups": [] }]
            }],
            "bookmarks": [{ "id": 3, "name": "Dev", "items": [{ "id": 4, "name": "GitHub", "abbr": "GH", "href": "https://github.com" }] }],
            "infoWidgets": [{ "id": 1, "type": "clock", "options": {} }, { "id": 2, "type": "unknown-kind", "options": {} }],
            "version": 5
        })
    }

    #[test]
    fn reads_the_server_document() {
        let d = Doc::from_wire(&server_doc());
        assert_eq!(d.version, "5");
        assert_eq!(d.groups[0].id, "7");
        assert_eq!(
            d.groups[0].layout,
            Layout {
                columns: 3,
                style: LayoutStyle::Column
            }
        );
        assert!(d.groups[0].groups[0].hidden);
        let s = &d.groups[0].items[0];
        assert_eq!(s.id, "12");
        assert_eq!(
            s.check,
            Some(Check {
                kind: "http".into(),
                target: None,
                expect_status: None
            })
        );
        assert!(is_secret_placeholder(
            &s.widget.as_ref().unwrap().fields["key"]
        ));
        assert_eq!(d.bookmarks[0].items[0].id, "4");
        assert_eq!(
            d.info_widgets
                .iter()
                .map(|w| w.kind.as_str())
                .collect::<Vec<_>>(),
            ["clock"]
        );
        assert_eq!(
            d.settings,
            Settings {
                title: "Lab".into(),
                theme: ThemeMode::Dark,
                accent: "#10b981".into(),
                background: Some(Background {
                    url: "/bg.jpg".into(),
                    blur: 4.0,
                    opacity: 0.5,
                    saturate: 120.0,
                    brightness: 100.0
                }),
                card_blur: 12.0,
                custom_css: String::new(),
                search: Search::Custom("https://s.test/?q=%s".into()),
            }
        );
    }

    #[test]
    fn round_trips_server_ids_and_sends_null_for_new_items() {
        let mut d = Doc::from_wire(&server_doc());
        d.groups[0].items.push(Service {
            id: new_id("s"),
            name: "New".into(),
            ..Service::default()
        });
        let w = d.to_wire();
        assert_eq!(w["version"], 5);
        assert_eq!(w["groups"][0]["id"], 7);
        assert_eq!(w["groups"][0]["items"][0]["id"], 12);
        assert_eq!(w["groups"][0]["items"][1]["id"], Value::Null);
        let back = Doc::from_wire(&w);
        assert_eq!(back.groups[0].groups[0].name, "Sub");
        assert_eq!(back.bookmarks, d.bookmarks);
    }

    #[test]
    fn writes_settings_in_the_servers_shape() {
        let s = Settings::from_wire(&server_doc()["settings"]);
        assert_eq!(
            s.to_wire(),
            json!({
                "title": "Lab", "theme": "dark", "accent": "#10b981",
                "background": { "url": "/bg.jpg", "blur": "4px", "opacity": 50, "saturate": 120, "brightness": null },
                "cardBlur": "12px", "customCss": null,
                "search": { "provider": "custom", "url": "https://s.test/?q={q}" }
            })
        );
        // The React app's stored default reads as the theme's own accent.
        assert_eq!(
            Settings::from_wire(&json!({ "accent": "#3b6fd8" })).accent,
            ""
        );
        assert_eq!(Settings::from_wire(&Value::Null).search, Search::DuckDuckGo);
    }

    #[test]
    fn parses_blur_and_ids() {
        assert_eq!(blur_to_px(&json!("md")), 12.0);
        assert_eq!(blur_to_px(&json!("10px")), 10.0);
        assert_eq!(blur_to_px(&Value::Null), 0.0);
        assert_eq!(to_wire_id("42"), json!(42));
        assert_eq!(to_wire_id("s-1"), Value::Null);
        assert_eq!(to_wire_id(""), Value::Null);
        let a = new_id("s");
        assert!(a.starts_with("s-") && a.len() == 10 && to_wire_id(&a).is_null());
        assert_ne!(a, new_id("s"));
    }

    #[test]
    fn reads_history_status_and_events() {
        let h = history_from_wire(
            &json!([{ "version": 3, "createdAt": 0, "author": null, "summary": null }]),
        );
        assert_eq!(
            h,
            [HistoryEntry {
                version: "3".into(),
                created_at: 0.0,
                author: "unknown".into(),
                summary: "Saved".into()
            }]
        );
        let mut st = status_from_wire(
            &json!({ "12": { "up": true, "latencyMs": 20, "checkedAt": 5, "history": [{ "t": 1, "up": false }] } }),
        );
        assert_eq!(
            st["12"].history,
            [Sample {
                t: 1.0,
                up: false,
                ms: None
            }]
        );
        assert!(apply_status_event(
            &mut st,
            &json!({ "serviceId": 12, "up": false, "checkedAt": 9 }),
            0.0
        ));
        assert_eq!(st["12"].history.len(), 2);
        assert!(!st["12"].up);
        assert!(!apply_status_event(&mut st, &json!({}), 0.0));
    }

    #[test]
    fn reads_me_and_the_inline_data() {
        let me = Me::from_wire(&json!({
            "user": { "name": "ann", "source": "local" }, "role": "admin", "authEnabled": true, "csrfToken": "t",
            "methods": { "local": true, "oidc": "SSO", "proxy": false }
        }));
        assert!(me.is_admin());
        assert_eq!(
            (me.user.as_deref(), me.methods.oidc.as_deref()),
            (Some("ann"), Some("SSO"))
        );
        assert_eq!(
            Me::from_wire(&json!({ "user": null, "role": null })).role,
            None
        );
        let (d, s) = inline_from_wire(&json!({ "dashboard": server_doc(), "status": {} })).unwrap();
        assert_eq!((d.groups.len(), s.len()), (1, 0));
        assert!(inline_from_wire(&json!({})).is_none());
    }

    #[test]
    fn search_urls() {
        assert_eq!(
            Search::DuckDuckGo.url_for("a b&c"),
            "https://duckduckgo.com/?q=a%20b%26c"
        );
        assert_eq!(
            Search::Custom("https://x/?q=%s&l=1".into()).url_for("é"),
            "https://x/?q=%C3%A9&l=1"
        );
        assert_eq!(
            Search::Custom("https://x/".into()).url_for("a"),
            "https://duckduckgo.com/?q=a"
        );
    }
}
