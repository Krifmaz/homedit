//! Custom icon uploads and icon search.
//!
//! Uploaded files live in `$HOMEDIT_DATA_DIR/icons/` and are served at
//! `/icons/custom/<file>`. They are referenced from the config as
//! `custom/<file>`. Every response from that folder carries a strict
//! Content-Security-Policy with `sandbox`, so an uploaded SVG opened
//! directly can never run script on the dashboard's origin.

use std::path::Path;

use axum::Json;
use axum::extract::{Multipart, Query, State};
use axum::http::{HeaderValue, header};
use homedit_core::icons;
use serde::{Deserialize, Serialize};
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;

use crate::auth::RequireAdmin;
use crate::{ApiError, AppState};

/// Largest accepted upload.
pub const MAX_ICON_BYTES: usize = 2 * 1024 * 1024;
/// Accepted file extensions.
pub const ALLOWED_EXTENSIONS: &[&str] = &["png", "svg", "webp", "jpg", "jpeg", "gif", "ico"];

/// Router that serves uploaded icons from `dir`.
pub fn serve_router(dir: &Path) -> axum::Router<AppState> {
    axum::Router::new()
        .fallback_service(ServeDir::new(dir))
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'none'; style-src 'unsafe-inline'; sandbox"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
}

/// Query for [`search`].
#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    /// Text to look for.
    #[serde(default)]
    pub q: String,
}

/// One icon search result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IconHit {
    /// Name to store in the config.
    pub name: String,
    /// `dashboard-icons`, `selfhst`, or `custom`.
    pub source: &'static str,
    /// URL for a preview.
    pub url: String,
}

/// Turns arbitrary text into an icon slug: lowercase ASCII letters, digits
/// and dashes.
pub fn slugify(s: &str) -> String {
    let mut out = String::new();
    for c in s.trim().chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_owned()
}

/// `GET /api/icons?q=` – custom uploads whose name contains `q`, plus the
/// matching names from the dashboard-icons and selfh.st sets. The browser
/// hides results whose preview fails to load.
pub async fn search(
    RequireAdmin(_): RequireAdmin,
    State(state): State<AppState>,
    Query(q): Query<SearchQuery>,
) -> Result<Json<Vec<IconHit>>, ApiError> {
    let needle = slugify(&q.q);
    let mut hits = Vec::new();
    let mut dir = match tokio::fs::read_dir(state.icon_dir()).await {
        Ok(d) => Some(d),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(ApiError::internal(e)),
    };
    if let Some(dir) = dir.as_mut() {
        while let Some(entry) = dir.next_entry().await.map_err(ApiError::internal)? {
            let name = entry.file_name().to_string_lossy().into_owned();
            if needle.is_empty() || name.to_lowercase().contains(&needle) {
                let r = format!("custom/{name}");
                if let Some(res) = icons::resolve(&r) {
                    hits.push(IconHit {
                        name: r,
                        source: "custom",
                        url: res.url,
                    });
                }
            }
        }
    }
    hits.sort_by(|a, b| a.name.cmp(&b.name));
    hits.truncate(50);
    if !needle.is_empty() {
        for (name, source) in [
            (format!("{needle}.png"), "dashboard-icons"),
            (format!("{needle}.svg"), "dashboard-icons"),
            (format!("sh-{needle}.png"), "selfhst"),
        ] {
            if let Some(r) = icons::resolve(&name) {
                hits.push(IconHit {
                    name,
                    source,
                    url: r.url,
                });
            }
        }
    }
    Ok(Json(hits))
}

/// Result of an upload.
#[derive(Debug, Serialize)]
pub struct Uploaded {
    /// Name to store in the config (`custom/<file>`).
    pub name: String,
    /// URL of the stored file.
    pub url: String,
}

/// `POST /api/icons` (multipart, field `file`) – stores an uploaded icon.
/// The stored file name is derived from the uploaded name; an existing file
/// with the same name is replaced.
pub async fn upload(
    RequireAdmin(_): RequireAdmin,
    State(state): State<AppState>,
    mut form: Multipart,
) -> Result<Json<Uploaded>, ApiError> {
    while let Some(field) = form
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?
    {
        if field.name() != Some("file") {
            continue;
        }
        let original = field.file_name().unwrap_or("icon.png").to_owned();
        let (stem, ext) = original.rsplit_once('.').unwrap_or((&original, ""));
        let ext = ext.to_ascii_lowercase();
        if !ALLOWED_EXTENSIONS.contains(&ext.as_str()) {
            return Err(ApiError::bad_request(format!(
                "unsupported file type; use one of {}",
                ALLOWED_EXTENSIONS.join(", ")
            )));
        }
        let stem = slugify(stem);
        let stem = if stem.is_empty() { "icon".into() } else { stem };
        let file = format!("{stem}.{ext}");
        let bytes = field
            .bytes()
            .await
            .map_err(|e| ApiError::bad_request(e.to_string()))?;
        if bytes.len() > MAX_ICON_BYTES {
            return Err(ApiError::bad_request("icon is larger than 2 MB"));
        }
        tokio::fs::create_dir_all(state.icon_dir())
            .await
            .map_err(ApiError::internal)?;
        tokio::fs::write(state.icon_dir().join(&file), &bytes)
            .await
            .map_err(ApiError::internal)?;
        let name = format!("custom/{file}");
        let url = icons::resolve(&name)
            .map(|r| r.url)
            .ok_or_else(|| ApiError::internal("generated icon name did not resolve"))?;
        return Ok(Json(Uploaded { name, url }));
    }
    Err(ApiError::bad_request("missing form field \"file\""))
}

/// The file extension for an inline icon's media type.
fn ext_for_mime(mime: &str) -> Option<&'static str> {
    Some(match mime.trim().to_ascii_lowercase().as_str() {
        "image/png" => "png",
        "image/svg+xml" => "svg",
        "image/webp" => "webp",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/x-icon" | "image/vnd.microsoft.icon" | "image/ico" => "ico",
        _ => return None,
    })
}

/// Decodes a `data:image/...` URL into its extension and bytes. Returns
/// `None` for anything that is not an image this server accepts as an
/// upload, or that is larger than [`MAX_ICON_BYTES`].
pub fn decode_data_url(url: &str) -> Option<(&'static str, Vec<u8>)> {
    use base64::Engine as _;
    let rest = url.trim().strip_prefix("data:")?;
    let (meta, payload) = rest.split_once(',')?;
    let mut parts = meta.split(';');
    let ext = ext_for_mime(parts.next()?)?;
    let base64 = parts.any(|p| p.trim().eq_ignore_ascii_case("base64"));
    let bytes = if base64 {
        let clean: String = payload.chars().filter(|c| !c.is_whitespace()).collect();
        base64::engine::general_purpose::STANDARD
            .decode(clean.trim_end_matches('='))
            .or_else(|_| {
                base64::engine::general_purpose::STANDARD_NO_PAD.decode(clean.trim_end_matches('='))
            })
            .ok()?
    } else {
        percent_decode(payload)?
    };
    (!bytes.is_empty() && bytes.len() <= MAX_ICON_BYTES).then_some((ext, bytes))
}

fn percent_decode(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    Some(out)
}

/// FNV-1a, so the same image always gets the same file name.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Writes one inline icon to `dir` and returns its config name
/// (`custom/<file>`), or `None` when `icon` is not a usable `data:` image.
fn store_one(dir: &Path, label: &str, icon: &str) -> std::io::Result<Option<String>> {
    let Some((ext, bytes)) = decode_data_url(icon) else {
        return Ok(None);
    };
    let stem = slugify(label);
    let stem = if stem.is_empty() { "icon".into() } else { stem };
    let file = format!("{stem}-{:08x}.{ext}", fnv1a(&bytes) as u32);
    let path = dir.join(&file);
    if !path.exists() {
        std::fs::create_dir_all(dir)?;
        std::fs::write(&path, &bytes)?;
    }
    Ok(Some(format!("custom/{file}")))
}

/// Turns every `data:` image icon in `dash` (cards, groups, bookmarks) into
/// an uploaded icon in `dir`, referenced as `custom/<file>`, the same as an
/// icon uploaded from the editor. The dashboard only loads icons it can
/// resolve, so an inline icon would show in the editor but fall back to
/// initials on the card. Returns whether anything changed.
pub fn store_inline_icons(dash: &mut homedit_core::Dashboard, dir: &Path) -> std::io::Result<bool> {
    fn fix(
        dir: &Path,
        label: &str,
        icon: &mut Option<String>,
        changed: &mut bool,
    ) -> std::io::Result<()> {
        if let Some(i) = icon.as_deref()
            && i.trim_start().starts_with("data:")
            && let Some(name) = store_one(dir, label, i)?
        {
            *icon = Some(name);
            *changed = true;
        }
        Ok(())
    }
    fn walk(
        dir: &Path,
        groups: &mut [homedit_core::Group],
        changed: &mut bool,
    ) -> std::io::Result<()> {
        for g in groups {
            fix(dir, &g.name, &mut g.layout.icon, changed)?;
            for s in &mut g.services {
                fix(dir, &s.name, &mut s.icon, changed)?;
            }
            walk(dir, &mut g.groups, changed)?;
        }
        Ok(())
    }
    let mut changed = false;
    walk(dir, &mut dash.groups, &mut changed)?;
    for bg in &mut dash.bookmark_groups {
        fix(dir, &bg.name, &mut bg.layout.icon, &mut changed)?;
        for b in &mut bg.bookmarks {
            fix(dir, &b.name, &mut b.icon, &mut changed)?;
        }
    }
    Ok(changed)
}

/// Moves `data:` icons left in the stored dashboard (saved before icons were
/// stored as files) into the icon folder. Runs at startup.
pub async fn store_inline_icons_stored(state: &AppState) -> anyhow::Result<()> {
    let (version, mut dash) = state.db.load_versioned().await?;
    if store_inline_icons(&mut dash, &state.icon_dir())? {
        state
            .db
            .save_dashboard(
                &dash,
                Some(version),
                Some("system"),
                Some("stored inline icons as files"),
            )
            .await?;
        tracing::info!("moved inline (data:) icons to the icon folder");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_data_urls() {
        let (ext, b) = decode_data_url("data:image/png;base64,UE5H").unwrap();
        assert_eq!((ext, b.as_slice()), ("png", &b"PNG"[..]));
        let (ext, b) = decode_data_url("data:image/svg+xml,%3Csvg%2F%3E").unwrap();
        assert_eq!((ext, b.as_slice()), ("svg", &b"<svg/>"[..]));
        assert_eq!(
            decode_data_url("data:image/x-icon;base64,AAAB").unwrap().0,
            "ico"
        );
        assert!(decode_data_url("data:text/html;base64,PGI+").is_none());
        assert!(decode_data_url("data:image/png;base64,").is_none());
        assert!(decode_data_url("sonarr.png").is_none());
    }

    #[test]
    fn stores_inline_icons_everywhere() {
        use homedit_core::{Bookmark, BookmarkGroup, Dashboard, Group, Service};
        let dir = tempfile::tempdir().unwrap();
        let mut svc = Service::new("Login | OPNsense", None);
        svc.icon = Some("data:image/png;base64,UE5H".into());
        let mut keep = Service::new("Sonarr", None);
        keep.icon = Some("sonarr.png".into());
        let mut bad = Service::new("Bad", None);
        bad.icon = Some("data:text/html,x".into());
        let mut g = Group {
            name: "Net".into(),
            services: vec![svc, keep, bad],
            ..Group::default()
        };
        g.layout.icon = Some("data:image/svg+xml,%3Csvg%2F%3E".into());
        let mut d = Dashboard {
            groups: vec![g],
            bookmark_groups: vec![BookmarkGroup {
                name: "B".into(),
                bookmarks: vec![Bookmark {
                    name: "Docs".into(),
                    icon: Some("data:image/png;base64,UE5H".into()),
                    ..Bookmark::default()
                }],
                ..BookmarkGroup::default()
            }],
            ..Dashboard::default()
        };
        assert!(store_inline_icons(&mut d, dir.path()).unwrap());
        let s = &d.groups[0].services;
        let name = s[0].icon.clone().unwrap();
        assert!(
            name.starts_with("custom/login-opnsense-") && name.ends_with(".png"),
            "{name}"
        );
        assert!(icons::resolve(&name).is_some());
        let file = dir.path().join(name.trim_start_matches("custom/"));
        assert_eq!(std::fs::read(file).unwrap(), b"PNG");
        assert_eq!(s[1].icon.as_deref(), Some("sonarr.png"));
        assert_eq!(
            s[2].icon.as_deref(),
            Some("data:text/html,x"),
            "non-images are left alone"
        );
        assert!(
            d.groups[0]
                .layout
                .icon
                .as_deref()
                .unwrap()
                .ends_with(".svg")
        );
        assert!(
            d.bookmark_groups[0].bookmarks[0]
                .icon
                .as_deref()
                .unwrap()
                .starts_with("custom/docs-")
        );
        assert!(
            !store_inline_icons(&mut d, dir.path()).unwrap(),
            "a second pass changes nothing"
        );
    }

    #[test]
    fn slugs() {
        assert_eq!(slugify("My NAS (main)"), "my-nas-main");
        assert_eq!(slugify("../../etc/passwd"), "etc-passwd");
        assert_eq!(slugify("  "), "");
        assert_eq!(slugify("Jellyfin"), "jellyfin");
    }
}
