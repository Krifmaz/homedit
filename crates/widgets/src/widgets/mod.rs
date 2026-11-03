//! Built-in service widgets, one module per app (or family of apps).
//!
//! Each module keeps HTTP calls in the [`crate::Widget`] impl and response
//! parsing in pure `parse_*` functions, which are unit tested against
//! recorded API responses in `tests/fixtures/<kind>/`.

use std::sync::Arc;

use crate::schema::{FieldSpec, FieldType};
use crate::widget::Widget;

pub mod adguard;
pub mod arr;
pub mod customapi;
pub mod docker;
pub mod immich;
pub mod jellyfin;
pub mod pihole;
pub mod plex;
pub mod portainer;
pub mod proxmox;
pub mod qbittorrent;
pub mod sabnzbd;
pub mod seerr;
pub mod uptimekuma;

/// Every built-in widget.
pub fn all() -> Vec<Arc<dyn Widget>> {
    vec![
        Arc::new(arr::Sonarr),
        Arc::new(arr::Radarr),
        Arc::new(arr::Prowlarr),
        Arc::new(plex::Plex),
        Arc::new(jellyfin::Jellyfin),
        Arc::new(jellyfin::Emby),
        Arc::new(immich::Immich),
        Arc::new(qbittorrent::QBittorrent),
        Arc::new(sabnzbd::Sabnzbd),
        Arc::new(proxmox::Proxmox),
        Arc::new(portainer::Portainer),
        Arc::new(docker::Docker),
        Arc::new(pihole::PiHole),
        Arc::new(adguard::AdGuard),
        Arc::new(uptimekuma::UptimeKuma),
        Arc::new(seerr::Overseerr),
        Arc::new(seerr::Jellyseerr),
        Arc::new(seerr::Seerr),
        Arc::new(customapi::CustomApi),
    ]
}

/// The common `key` field holding an API key.
pub(crate) fn api_key_field(help: &str) -> FieldSpec {
    FieldSpec::new("key", "API key", FieldType::Secret)
        .required()
        .help(help)
}

/// Optional `username` field.
pub(crate) fn username_field(required: bool) -> FieldSpec {
    let f = FieldSpec::new("username", "Username", FieldType::Text);
    if required { f.required() } else { f }
}

/// Optional `password` field.
pub(crate) fn password_field(required: bool) -> FieldSpec {
    let f = FieldSpec::new("password", "Password", FieldType::Secret);
    if required { f.required() } else { f }
}

/// The `insecure_tls` checkbox (see [`crate::context::INSECURE_TLS_FIELD`]).
pub(crate) fn insecure_tls_field() -> FieldSpec {
    FieldSpec::new(
        crate::context::INSECURE_TLS_FIELD,
        "Allow self-signed certificate",
        FieldType::Bool,
    )
    .default_value(false)
    .help("Skip TLS certificate verification for this service only.")
}

/// Loads a recorded API response from `tests/fixtures`.
#[cfg(test)]
pub(crate) fn fixture(path: &str) -> serde_json::Value {
    let full = format!("{}/tests/fixtures/{path}", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("{full}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{full}: {e}"))
}
