//! The built-in catalogue of self-hosted apps the scanner can recognise.
//!
//! Each [`AppDef`] says which ports an app usually listens on, which Docker
//! images ship it, what its web page looks like ([`Signal`]s) and which API
//! paths confirm it ([`Probe`]s). The matching itself lives in
//! [`crate::fingerprint`].
//!
//! `id` and `widget` use the lowercase names that gethomepage uses for its
//! widget types (`sonarr`, `qbittorrent`, `pihole`, ...), so a match can be
//! turned straight into a dashboard widget.
//!
//! All fingerprints here were written from each project's public API docs
//! and default web pages. Only unauthenticated endpoints are probed, and only
//! with `GET`.

use std::sync::OnceLock;

/// A clue found on the root page (`/`) or in its response headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// The `<title>` equals this text, ignoring case and surrounding spaces.
    TitleIs(&'static str),
    /// The `<title>` contains this text, ignoring case.
    TitleContains(&'static str),
    /// The body contains this text, ignoring case.
    BodyContains(&'static str),
    /// The named header (lowercase) contains this text, ignoring case.
    /// An empty value only checks that the header is present.
    Header(&'static str, &'static str),
    /// The favicon's Shodan-style hash (see
    /// [`favicon_hash`](crate::fingerprint::favicon_hash)) equals this value.
    FaviconHash(i32),
}

impl Signal {
    /// How much this signal adds to an app's score when it is present.
    pub(crate) fn weight(&self) -> u32 {
        match self {
            Signal::TitleIs(_) => 70,
            Signal::FaviconHash(_) => 70,
            Signal::TitleContains(_) => 60,
            Signal::Header(_, _) => 50,
            Signal::BodyContains(_) => 30,
        }
    }
}

/// A condition on the response to a [`Probe`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    /// The status code equals this value.
    Status(u16),
    /// The status code is 2xx.
    Success,
    /// The body is JSON and has a non-null value at this JSON pointer
    /// (RFC 6901, e.g. `/version`).
    JsonHas(&'static str),
    /// The body is JSON and the string at this pointer equals the value,
    /// ignoring case.
    JsonEquals(&'static str, &'static str),
    /// The body contains this text, ignoring case.
    BodyContains(&'static str),
    /// The named header (lowercase) is present.
    HeaderPresent(&'static str),
    /// The trimmed body looks like a bare version number such as `v4.6.2`.
    BodyIsVersion,
}

/// Where to read an app's version from a probe response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionSource {
    /// A string or number at this JSON pointer.
    Json(&'static str),
    /// Three numbers at these JSON pointers, joined as `major.minor.patch`.
    JsonParts(&'static str, &'static str, &'static str),
    /// An XML attribute of that name on the first element that has it.
    XmlAttr(&'static str),
    /// The whole trimmed body, minus a leading `v`.
    Body,
    /// The value of this header (lowercase).
    Header(&'static str),
}

/// One extra request that confirms an app and may reveal its version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Probe {
    /// Path and query to `GET`, starting with `/`.
    pub path: &'static str,
    /// All of these must hold for the probe to count.
    pub checks: &'static [Check],
    /// Score added when every check holds.
    pub weight: u32,
    /// Where to read the version from, when the checks hold.
    pub version: Option<VersionSource>,
}

/// One app the scanner can recognise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppDef {
    /// Stable lowercase id, matching gethomepage's widget type where one exists.
    pub id: &'static str,
    /// Display name, used as the default card name.
    pub name: &'static str,
    /// The dashboard widget type for this app, if there is one.
    pub widget: Option<&'static str>,
    /// Icon name in the dashboard-icons set.
    pub icon: &'static str,
    /// Ports the app listens on by default.
    pub ports: &'static [u16],
    /// Docker image names that ship the app, without tag. A plain name is
    /// compared with the image's last path segment (`sonarr`); a name with a
    /// `/` must match the end of the image path (`vaultwarden/server`). The
    /// app's `id` always counts as a plain name.
    pub images: &'static [&'static str],
    /// Clues on the root page.
    pub signals: &'static [Signal],
    /// Confirming requests.
    pub probes: &'static [Probe],
}

/// A set of [`AppDef`]s to match against.
#[derive(Debug, Clone)]
pub struct Catalogue {
    apps: Vec<AppDef>,
}

impl Catalogue {
    /// Builds a catalogue from a custom list (useful for tests or plugins).
    pub fn new(apps: Vec<AppDef>) -> Self {
        Self { apps }
    }

    /// The built-in catalogue.
    pub fn builtin() -> &'static Catalogue {
        static BUILTIN: OnceLock<Catalogue> = OnceLock::new();
        BUILTIN.get_or_init(|| Catalogue::new(BUILTIN_APPS.to_vec()))
    }

    /// Every app, in catalogue order.
    pub fn apps(&self) -> &[AppDef] {
        &self.apps
    }

    /// Looks up an app by id.
    pub fn get(&self, id: &str) -> Option<&AppDef> {
        self.apps.iter().find(|a| a.id == id)
    }

    /// Every default port from every app (may repeat).
    pub fn ports(&self) -> impl Iterator<Item = u16> + '_ {
        self.apps.iter().flat_map(|a| a.ports.iter().copied())
    }

    /// Finds the app shipped by a Docker image reference such as
    /// `lscr.io/linuxserver/sonarr:4.0.0` or `jellyfin/jellyfin@sha256:...`.
    pub fn by_image(&self, image: &str) -> Option<&AppDef> {
        let path = image_path(image);
        let name = path.rsplit('/').next().unwrap_or(&path);
        self.apps.iter().find(|a| {
            a.id == name
                || a.images.iter().any(|i| {
                    let i = i.to_ascii_lowercase();
                    if i.contains('/') {
                        path == i || path.ends_with(&format!("/{i}"))
                    } else {
                        i == name
                    }
                })
        })
    }
}

/// Reduces an image reference to its lowercase path without tag or digest:
/// `ghcr.io/immich-app/immich-server:v1.2` → `ghcr.io/immich-app/immich-server`.
fn image_path(image: &str) -> String {
    let no_digest = image.split('@').next().unwrap_or(image);
    // A tag is a `:` after the last `/`; a `:` before it is a registry port.
    let no_tag = match no_digest.rfind(':') {
        Some(i) if !no_digest[i..].contains('/') => &no_digest[..i],
        _ => no_digest,
    };
    no_tag.to_ascii_lowercase()
}

const fn probe(
    path: &'static str,
    checks: &'static [Check],
    weight: u32,
    version: Option<VersionSource>,
) -> Probe {
    Probe {
        path,
        checks,
        weight,
        version,
    }
}

use Check::*;
use Signal::*;
use VersionSource as V;

/// *arr apps answer their status endpoint with 401 when no API key is sent.
/// That alone is weak, so it only backs up a title match.
const ARR_V3: &[Probe] = &[probe("/api/v3/system/status", &[Status(401)], 20, None)];
const ARR_V1: &[Probe] = &[probe("/api/v1/system/status", &[Status(401)], 20, None)];

/// The built-in app list. Order matters only for ties.
pub static BUILTIN_APPS: &[AppDef] = &[
    AppDef {
        id: "plex",
        name: "Plex",
        widget: Some("plex"),
        icon: "plex",
        ports: &[32400],
        images: &["pms-docker", "plex"],
        signals: &[Header("x-plex-protocol", ""), TitleIs("Plex")],
        probes: &[probe(
            "/identity",
            &[Success, Check::BodyContains("machineIdentifier")],
            90,
            Some(V::XmlAttr("version")),
        )],
    },
    AppDef {
        id: "jellyfin",
        name: "Jellyfin",
        widget: Some("jellyfin"),
        icon: "jellyfin",
        ports: &[8096, 8920],
        images: &[],
        signals: &[TitleIs("Jellyfin")],
        probes: &[probe(
            "/System/Info/Public",
            &[Success, JsonEquals("/ProductName", "Jellyfin Server")],
            90,
            Some(V::Json("/Version")),
        )],
    },
    AppDef {
        id: "emby",
        name: "Emby",
        widget: Some("emby"),
        icon: "emby",
        ports: &[8096, 8920],
        images: &["embyserver"],
        signals: &[TitleIs("Emby")],
        probes: &[probe(
            "/emby/System/Info/Public",
            &[Success, JsonHas("/ServerName"), JsonHas("/Version")],
            40,
            Some(V::Json("/Version")),
        )],
    },
    AppDef {
        id: "sonarr",
        name: "Sonarr",
        widget: Some("sonarr"),
        icon: "sonarr",
        ports: &[8989],
        images: &[],
        signals: &[TitleIs("Sonarr")],
        probes: ARR_V3,
    },
    AppDef {
        id: "radarr",
        name: "Radarr",
        widget: Some("radarr"),
        icon: "radarr",
        ports: &[7878],
        images: &[],
        signals: &[TitleIs("Radarr")],
        probes: ARR_V3,
    },
    AppDef {
        id: "lidarr",
        name: "Lidarr",
        widget: Some("lidarr"),
        icon: "lidarr",
        ports: &[8686],
        images: &[],
        signals: &[TitleIs("Lidarr")],
        probes: ARR_V1,
    },
    AppDef {
        id: "readarr",
        name: "Readarr",
        widget: Some("readarr"),
        icon: "readarr",
        ports: &[8787],
        images: &[],
        signals: &[TitleIs("Readarr")],
        probes: ARR_V1,
    },
    AppDef {
        id: "prowlarr",
        name: "Prowlarr",
        widget: Some("prowlarr"),
        icon: "prowlarr",
        ports: &[9696],
        images: &[],
        signals: &[TitleIs("Prowlarr")],
        probes: ARR_V1,
    },
    AppDef {
        id: "bazarr",
        name: "Bazarr",
        widget: Some("bazarr"),
        icon: "bazarr",
        ports: &[6767],
        images: &[],
        signals: &[TitleIs("Bazarr")],
        probes: &[],
    },
    AppDef {
        id: "qbittorrent",
        name: "qBittorrent",
        widget: Some("qbittorrent"),
        icon: "qbittorrent",
        ports: &[8080],
        images: &["qbittorrent-nox"],
        signals: &[TitleContains("qBittorrent")],
        probes: &[probe(
            "/api/v2/app/version",
            &[Success, BodyIsVersion],
            40,
            Some(V::Body),
        )],
    },
    AppDef {
        id: "transmission",
        name: "Transmission",
        widget: Some("transmission"),
        icon: "transmission",
        ports: &[9091],
        images: &[],
        signals: &[TitleContains("Transmission")],
        probes: &[probe(
            "/transmission/rpc",
            &[Status(409), HeaderPresent("x-transmission-session-id")],
            90,
            None,
        )],
    },
    AppDef {
        id: "deluge",
        name: "Deluge",
        widget: Some("deluge"),
        icon: "deluge",
        ports: &[8112],
        images: &[],
        signals: &[TitleContains("Deluge")],
        probes: &[],
    },
    AppDef {
        id: "sabnzbd",
        name: "SABnzbd",
        widget: Some("sabnzbd"),
        icon: "sabnzbd",
        ports: &[8080, 8085],
        images: &[],
        signals: &[TitleContains("SABnzbd")],
        probes: &[probe(
            "/api?mode=version&output=json",
            &[Success, JsonHas("/version")],
            45,
            Some(V::Json("/version")),
        )],
    },
    AppDef {
        id: "nzbget",
        name: "NZBGet",
        widget: Some("nzbget"),
        icon: "nzbget",
        ports: &[6789],
        images: &[],
        signals: &[
            TitleContains("NZBGet"),
            Header("www-authenticate", "nzbget"),
        ],
        probes: &[],
    },
    AppDef {
        id: "seerr",
        name: "Seerr",
        widget: Some("seerr"),
        icon: "seerr",
        ports: &[5055],
        images: &[],
        signals: &[TitleIs("Seerr")],
        probes: &[probe(
            "/api/v1/status",
            &[Success, JsonHas("/version"), JsonHas("/commitTag")],
            40,
            Some(V::Json("/version")),
        )],
    },
    AppDef {
        id: "jellyseerr",
        name: "Jellyseerr",
        widget: Some("jellyseerr"),
        icon: "jellyseerr",
        ports: &[5055],
        images: &[],
        signals: &[TitleIs("Jellyseerr")],
        probes: &[probe(
            "/api/v1/status",
            &[Success, JsonHas("/version"), JsonHas("/commitTag")],
            40,
            Some(V::Json("/version")),
        )],
    },
    AppDef {
        id: "overseerr",
        name: "Overseerr",
        widget: Some("overseerr"),
        icon: "overseerr",
        ports: &[5055],
        images: &[],
        signals: &[TitleIs("Overseerr")],
        probes: &[probe(
            "/api/v1/status",
            &[Success, JsonHas("/version"), JsonHas("/commitTag")],
            40,
            Some(V::Json("/version")),
        )],
    },
    AppDef {
        id: "tautulli",
        name: "Tautulli",
        widget: Some("tautulli"),
        icon: "tautulli",
        ports: &[8181],
        images: &[],
        signals: &[TitleContains("Tautulli")],
        probes: &[],
    },
    AppDef {
        id: "proxmox",
        name: "Proxmox VE",
        widget: Some("proxmox"),
        icon: "proxmox",
        ports: &[8006],
        images: &[],
        signals: &[
            TitleContains("Proxmox Virtual Environment"),
            Header("server", "pve-api-daemon"),
        ],
        probes: &[],
    },
    AppDef {
        id: "proxmoxbackupserver",
        name: "Proxmox Backup Server",
        widget: Some("proxmoxbackupserver"),
        icon: "proxmox",
        ports: &[8007],
        images: &[],
        signals: &[
            TitleContains("Proxmox Backup Server"),
            Header("server", "proxmox-backup"),
        ],
        probes: &[],
    },
    AppDef {
        id: "portainer",
        name: "Portainer",
        widget: Some("portainer"),
        icon: "portainer",
        ports: &[9000, 9443],
        images: &["portainer-ce", "portainer-ee"],
        signals: &[TitleIs("Portainer")],
        probes: &[probe(
            "/api/status",
            &[Success, JsonHas("/Version")],
            40,
            Some(V::Json("/Version")),
        )],
    },
    AppDef {
        id: "immich",
        name: "Immich",
        widget: Some("immich"),
        icon: "immich",
        ports: &[2283],
        images: &["immich-server"],
        signals: &[TitleIs("Immich")],
        probes: &[
            probe(
                "/api/server/ping",
                &[Success, JsonEquals("/res", "pong")],
                60,
                None,
            ),
            probe(
                "/api/server/version",
                &[Success, JsonHas("/major")],
                20,
                Some(V::JsonParts("/major", "/minor", "/patch")),
            ),
        ],
    },
    AppDef {
        id: "pihole",
        name: "Pi-hole",
        widget: Some("pihole"),
        icon: "pi-hole",
        ports: &[80, 443],
        images: &[],
        signals: &[TitleContains("Pi-hole")],
        probes: &[probe(
            "/admin/",
            &[Check::BodyContains("pi-hole")],
            50,
            None,
        )],
    },
    AppDef {
        id: "adguard",
        name: "AdGuard Home",
        widget: Some("adguard"),
        icon: "adguard-home",
        ports: &[3000, 80],
        images: &["adguardhome"],
        signals: &[TitleContains("AdGuard Home")],
        probes: &[probe(
            "/login.html",
            &[Success, Check::BodyContains("AdGuard Home")],
            50,
            None,
        )],
    },
    AppDef {
        id: "uptimekuma",
        name: "Uptime Kuma",
        widget: Some("uptimekuma"),
        icon: "uptime-kuma",
        ports: &[3001],
        images: &["uptime-kuma"],
        signals: &[TitleContains("Uptime Kuma")],
        probes: &[probe(
            "/api/entry-page",
            &[Success, JsonHas("/type")],
            30,
            None,
        )],
    },
    AppDef {
        id: "homeassistant",
        name: "Home Assistant",
        widget: Some("homeassistant"),
        icon: "home-assistant",
        ports: &[8123],
        images: &["home-assistant"],
        signals: &[TitleIs("Home Assistant")],
        probes: &[probe(
            "/manifest.json",
            &[Success, Check::BodyContains("Home Assistant")],
            60,
            None,
        )],
    },
    AppDef {
        id: "grafana",
        name: "Grafana",
        widget: Some("grafana"),
        icon: "grafana",
        ports: &[3000],
        images: &["grafana-oss", "grafana-enterprise"],
        signals: &[TitleIs("Grafana")],
        probes: &[probe(
            "/api/health",
            &[Success, JsonHas("/database"), JsonHas("/version")],
            60,
            Some(V::Json("/version")),
        )],
    },
    AppDef {
        id: "nextcloud",
        name: "Nextcloud",
        widget: Some("nextcloud"),
        icon: "nextcloud",
        ports: &[80, 443],
        images: &[],
        signals: &[TitleContains("Nextcloud")],
        probes: &[probe(
            "/status.php",
            &[
                Success,
                JsonHas("/installed"),
                JsonEquals("/productname", "Nextcloud"),
            ],
            80,
            Some(V::Json("/versionstring")),
        )],
    },
    AppDef {
        id: "gitea",
        name: "Gitea",
        widget: Some("gitea"),
        icon: "gitea",
        ports: &[3000],
        images: &[],
        signals: &[TitleContains("Gitea")],
        probes: &[probe(
            "/api/v1/version",
            &[Success, JsonHas("/version")],
            30,
            Some(V::Json("/version")),
        )],
    },
    AppDef {
        id: "forgejo",
        name: "Forgejo",
        widget: Some("gitea"),
        icon: "forgejo",
        ports: &[3000],
        images: &[],
        signals: &[TitleContains("Forgejo")],
        probes: &[probe(
            "/api/v1/version",
            &[Success, JsonHas("/version")],
            30,
            Some(V::Json("/version")),
        )],
    },
    AppDef {
        id: "traefik",
        name: "Traefik",
        widget: Some("traefik"),
        icon: "traefik",
        ports: &[8080],
        images: &[],
        signals: &[TitleIs("Traefik")],
        probes: &[probe(
            "/api/version",
            &[Success, JsonHas("/Version"), JsonHas("/Codename")],
            70,
            Some(V::Json("/Version")),
        )],
    },
    AppDef {
        id: "npm",
        name: "Nginx Proxy Manager",
        widget: Some("npm"),
        icon: "nginx-proxy-manager",
        ports: &[81],
        images: &["nginx-proxy-manager"],
        signals: &[TitleContains("Nginx Proxy Manager")],
        probes: &[probe(
            "/api/",
            &[
                Success,
                JsonEquals("/status", "OK"),
                JsonHas("/version/major"),
            ],
            60,
            Some(V::JsonParts(
                "/version/major",
                "/version/minor",
                "/version/revision",
            )),
        )],
    },
    AppDef {
        id: "navidrome",
        name: "Navidrome",
        widget: Some("navidrome"),
        icon: "navidrome",
        ports: &[4533],
        images: &[],
        signals: &[TitleIs("Navidrome")],
        probes: &[],
    },
    AppDef {
        id: "audiobookshelf",
        name: "Audiobookshelf",
        widget: Some("audiobookshelf"),
        icon: "audiobookshelf",
        ports: &[13378],
        images: &[],
        signals: &[TitleContains("audiobookshelf")],
        probes: &[probe(
            "/status",
            &[Success, JsonEquals("/app", "audiobookshelf")],
            80,
            Some(V::Json("/serverVersion")),
        )],
    },
    AppDef {
        id: "paperlessngx",
        name: "Paperless-ngx",
        widget: Some("paperlessngx"),
        icon: "paperless-ngx",
        ports: &[8000],
        images: &["paperless-ngx"],
        signals: &[TitleContains("Paperless-ngx")],
        probes: &[probe(
            "/api/",
            &[HeaderPresent("x-api-version")],
            50,
            Some(V::Header("x-version")),
        )],
    },
    AppDef {
        id: "frigate",
        name: "Frigate",
        widget: Some("frigate"),
        icon: "frigate",
        ports: &[5000, 8971],
        images: &[],
        signals: &[TitleIs("Frigate")],
        probes: &[probe(
            "/api/version",
            &[Success, BodyIsVersion],
            30,
            Some(V::Body),
        )],
    },
    AppDef {
        id: "prometheus",
        name: "Prometheus",
        widget: Some("prometheus"),
        icon: "prometheus",
        ports: &[9090],
        images: &[],
        signals: &[TitleContains("Prometheus")],
        probes: &[probe(
            "/api/v1/status/buildinfo",
            &[
                Success,
                JsonEquals("/status", "success"),
                JsonHas("/data/version"),
            ],
            70,
            Some(V::Json("/data/version")),
        )],
    },
    AppDef {
        id: "mealie",
        name: "Mealie",
        widget: Some("mealie"),
        icon: "mealie",
        ports: &[9000, 9925],
        images: &[],
        signals: &[TitleContains("Mealie")],
        probes: &[probe(
            "/api/app/about",
            &[Success, JsonHas("/production"), JsonHas("/version")],
            60,
            Some(V::Json("/version")),
        )],
    },
    AppDef {
        id: "glances",
        name: "Glances",
        widget: Some("glances"),
        icon: "glances",
        ports: &[61208],
        images: &[],
        signals: &[TitleContains("Glances")],
        probes: &[],
    },
    AppDef {
        id: "unifi",
        name: "UniFi Network",
        widget: Some("unifi"),
        icon: "unifi",
        ports: &[8443],
        images: &["unifi-network-application", "unifi-controller"],
        signals: &[TitleContains("UniFi")],
        probes: &[],
    },
    AppDef {
        id: "truenas",
        name: "TrueNAS",
        widget: Some("truenas"),
        icon: "truenas",
        ports: &[80, 443],
        images: &[],
        signals: &[TitleContains("TrueNAS")],
        probes: &[],
    },
    AppDef {
        id: "opnsense",
        name: "OPNsense",
        widget: Some("opnsense"),
        icon: "opnsense",
        ports: &[80, 443],
        images: &[],
        signals: &[TitleContains("OPNsense"), Signal::BodyContains("opnsense")],
        probes: &[],
    },
    AppDef {
        id: "pfsense",
        name: "pfSense",
        widget: Some("pfsense"),
        icon: "pfsense",
        ports: &[80, 443],
        images: &[],
        signals: &[TitleContains("pfSense"), Signal::BodyContains("pfsense")],
        probes: &[],
    },
    AppDef {
        id: "syncthing",
        name: "Syncthing",
        widget: None,
        icon: "syncthing",
        ports: &[8384],
        images: &[],
        signals: &[TitleContains("Syncthing")],
        probes: &[probe(
            "/rest/noauth/health",
            &[Success, JsonEquals("/status", "OK")],
            30,
            None,
        )],
    },
    AppDef {
        id: "vaultwarden",
        name: "Vaultwarden",
        widget: None,
        icon: "vaultwarden",
        ports: &[80],
        images: &["vaultwarden/server"],
        signals: &[TitleContains("Vaultwarden")],
        probes: &[],
    },
    AppDef {
        id: "codeserver",
        name: "code-server",
        widget: None,
        icon: "code-server",
        ports: &[8443],
        images: &["code-server"],
        signals: &[TitleContains("code-server")],
        probes: &[],
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn ids_are_unique_and_lowercase() {
        let mut seen = HashSet::new();
        for app in BUILTIN_APPS {
            assert!(seen.insert(app.id), "duplicate id {}", app.id);
            assert_eq!(app.id, app.id.to_ascii_lowercase());
            assert!(
                !app.signals.is_empty() || !app.probes.is_empty(),
                "{} has no fingerprint",
                app.id
            );
            for p in app.probes {
                assert!(p.path.starts_with('/'), "{} probe {}", app.id, p.path);
                assert!(!p.checks.is_empty());
            }
        }
    }

    #[test]
    fn finds_apps_by_image() {
        let cat = Catalogue::builtin();
        let id = |img: &str| cat.by_image(img).map(|a| a.id);
        assert_eq!(id("lscr.io/linuxserver/sonarr:latest"), Some("sonarr"));
        assert_eq!(id("plexinc/pms-docker"), Some("plex"));
        assert_eq!(
            id("ghcr.io/immich-app/immich-server:v1.120.0"),
            Some("immich")
        );
        assert_eq!(id("jellyfin/jellyfin@sha256:abcd"), Some("jellyfin"));
        assert_eq!(id("portainer/portainer-ce:2.21"), Some("portainer"));
        assert_eq!(id("vaultwarden/server:latest"), Some("vaultwarden"));
        assert_eq!(id("registry.lan:5000/media/radarr"), Some("radarr"));
        assert_eq!(id("someone/server"), None);
        assert_eq!(id("redis:7"), None);
    }
}
