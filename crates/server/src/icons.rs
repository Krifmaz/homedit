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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slugify("My NAS (main)"), "my-nas-main");
        assert_eq!(slugify("../../etc/passwd"), "etc-passwd");
        assert_eq!(slugify("  "), "");
        assert_eq!(slugify("Jellyfin"), "jellyfin");
    }
}
