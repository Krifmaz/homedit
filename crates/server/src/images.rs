//! Background image uploads.
//!
//! Uploaded files live in `$HOMEDIT_DATA_DIR/images/` and are served at
//! `/images/<file>`, the same path gethomepage serves its `public/images`
//! folder from. A gethomepage `settings.yaml` with
//! `background: /images/wallpaper.jpg` therefore works as soon as the file
//! is copied (or uploaded) into that folder. Responses carry the same
//! sandboxing headers as uploaded icons.

use std::time::Duration;

use axum::Json;
use axum::extract::{DefaultBodyLimit, Multipart, State};
use axum::routing::{MethodRouter, get};
use homedit_auth::ssrf::{ClientOptions, GuardedClient, SsrfGuard};
use serde::{Deserialize, Serialize};

use crate::auth::RequireAdmin;
use crate::icons::{serve_router, slugify};
use crate::{ApiError, AppState};

/// Largest accepted image.
pub const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;
/// Accepted file extensions.
pub const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "avif", "gif", "svg"];
/// URL prefix uploaded images are served under.
pub const IMAGE_PREFIX: &str = "/images/";

/// Router that serves uploaded images.
pub fn router(state: &AppState) -> axum::Router<AppState> {
    serve_router(&state.image_dir())
}

/// `GET` and `POST` handlers for `/api/images`, with a body limit large
/// enough for wallpapers.
pub fn api_route() -> MethodRouter<AppState> {
    get(list)
        .post(upload)
        .layer(DefaultBodyLimit::max(MAX_IMAGE_BYTES + 64 * 1024))
}

/// `POST /api/images/fetch` – downloads the image at `url` into the images
/// folder. Many image hosts refuse hotlinked images, so a copy on this
/// server is the dependable way to use a picture from the web.
pub async fn fetch(
    RequireAdmin(_): RequireAdmin,
    State(state): State<AppState>,
    Json(req): Json<FetchRequest>,
) -> Result<Json<ImageFile>, ApiError> {
    let url = req.url.trim();
    // Only the host the admin typed may be reached, and addresses such as
    // cloud metadata endpoints stay blocked by the guard.
    let client = GuardedClient::new(
        SsrfGuard::from_urls([url]),
        ClientOptions {
            timeout: Duration::from_secs(30),
            user_agent: format!("Mozilla/5.0 (compatible; homedit/{})", crate::VERSION),
            ..ClientOptions::default()
        },
    )
    .map_err(ApiError::internal)?;
    let request = client
        .get(url)
        .map_err(|e| ApiError::bad_request(format!("cannot fetch that URL: {e}")))?;
    let mut resp = request
        .send()
        .await
        .map_err(|e| ApiError::bad_request(format!("download failed: {e}")))?;
    if !resp.status().is_success() {
        return Err(ApiError::bad_request(format!(
            "the image host answered {}",
            resp.status()
        )));
    }
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let (stem, ext) = file_name_for(resp.url(), &content_type)?;
    let mut bytes = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| ApiError::bad_request(format!("download failed: {e}")))?
    {
        if bytes.len() + chunk.len() > MAX_IMAGE_BYTES {
            return Err(ApiError::bad_request("image is larger than 20 MB"));
        }
        bytes.extend_from_slice(&chunk);
    }
    store(&state, &stem, &ext, &bytes).await.map(Json)
}

/// Body of [`fetch`].
#[derive(Debug, Deserialize)]
pub struct FetchRequest {
    /// `http(s)` URL of the image.
    pub url: String,
}

/// Picks a file stem and extension for a downloaded image from its URL and
/// `Content-Type`, refusing anything that is not an image.
fn file_name_for(url: &reqwest::Url, content_type: &str) -> Result<(String, String), ApiError> {
    let last = url
        .path_segments()
        .and_then(|mut s| s.next_back())
        .map(percent_decode)
        .unwrap_or_default();
    let (stem, url_ext) = match last.rsplit_once('.') {
        Some((stem, ext)) => (stem.to_owned(), ext.to_ascii_lowercase()),
        None => (last.clone(), String::new()),
    };
    let mime = content_type.split(';').next().unwrap_or("").trim();
    let ext = match mime {
        "image/jpeg" | "image/jpg" | "image/pjpeg" => "jpg",
        "image/png" => "png",
        "image/webp" => "webp",
        "image/avif" => "avif",
        "image/gif" => "gif",
        "image/svg+xml" => "svg",
        // Some hosts send a generic type; trust a known extension then.
        "" | "application/octet-stream" | "binary/octet-stream"
            if IMAGE_EXTENSIONS.contains(&url_ext.as_str()) =>
        {
            if url_ext == "jpeg" {
                "jpg"
            } else {
                url_ext.as_str()
            }
        }
        other => {
            let what = if other.is_empty() {
                "of unknown type"
            } else {
                other
            };
            return Err(ApiError::bad_request(format!(
                "that URL is not an image (it is {what})"
            )));
        }
    };
    // Long booru-style names are trimmed so the gallery stays readable.
    let mut stem = slugify(&stem);
    stem.truncate(80);
    let stem = stem.trim_end_matches('-');
    let stem = if stem.is_empty() { "image" } else { stem };
    Ok((stem.to_owned(), ext.to_owned()))
}

/// Decodes `%XX` escapes in a URL path segment; invalid ones are kept.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(b)) => {
                out.push(b);
                i += 3;
            }
            (b, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Writes an image into the images folder and returns its entry.
async fn store(
    state: &AppState,
    stem: &str,
    ext: &str,
    bytes: &[u8],
) -> Result<ImageFile, ApiError> {
    let name = format!("{stem}.{ext}");
    tokio::fs::create_dir_all(state.image_dir())
        .await
        .map_err(ApiError::internal)?;
    tokio::fs::write(state.image_dir().join(&name), bytes)
        .await
        .map_err(ApiError::internal)?;
    Ok(ImageFile {
        url: format!("{IMAGE_PREFIX}{name}"),
        name,
    })
}

/// One stored image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImageFile {
    /// File name inside the images folder.
    pub name: String,
    /// URL to use as a background.
    pub url: String,
}

fn is_image(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| IMAGE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

/// `GET /api/images` – the stored images, by name.
pub async fn list(
    RequireAdmin(_): RequireAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<ImageFile>>, ApiError> {
    let mut out = Vec::new();
    let mut dir = match tokio::fs::read_dir(state.image_dir()).await {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Json(out)),
        Err(e) => return Err(ApiError::internal(e)),
    };
    while let Some(entry) = dir.next_entry().await.map_err(ApiError::internal)? {
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_file = entry.file_type().await.is_ok_and(|t| t.is_file());
        if is_file && !name.starts_with('.') && is_image(&name) {
            out.push(ImageFile {
                url: format!("{IMAGE_PREFIX}{name}"),
                name,
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Json(out))
}

/// `POST /api/images` (multipart, field `file`) – stores an uploaded image.
/// An existing file with the same name is replaced.
pub async fn upload(
    RequireAdmin(_): RequireAdmin,
    State(state): State<AppState>,
    mut form: Multipart,
) -> Result<Json<ImageFile>, ApiError> {
    while let Some(field) = form
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?
    {
        if field.name() != Some("file") {
            continue;
        }
        let original = field.file_name().unwrap_or("image.jpg").to_owned();
        let (stem, ext) = original.rsplit_once('.').unwrap_or((&original, ""));
        let ext = ext.to_ascii_lowercase();
        if !IMAGE_EXTENSIONS.contains(&ext.as_str()) {
            return Err(ApiError::bad_request(format!(
                "unsupported file type; use one of {}",
                IMAGE_EXTENSIONS.join(", ")
            )));
        }
        let stem = slugify(stem);
        let stem = if stem.is_empty() {
            "image".into()
        } else {
            stem
        };
        let bytes = field
            .bytes()
            .await
            .map_err(|e| ApiError::bad_request(e.to_string()))?;
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(ApiError::bad_request("image is larger than 20 MB"));
        }
        return store(&state, &stem, &ext, &bytes).await.map(Json);
    }
    Err(ApiError::bad_request("missing form field \"file\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_extensions() {
        assert!(is_image("wall.JPG"));
        assert!(is_image("a.webp"));
        assert!(!is_image("notes.txt"));
        assert!(!is_image("noext"));
    }

    #[test]
    fn downloaded_file_names() {
        let url = |s: &str| reqwest::Url::parse(s).unwrap();
        let name = |u: &str, ct: &str| file_name_for(&url(u), ct).ok();
        assert_eq!(
            name(
                "https://cdn.example/s/__aris_and_hibiki__sample-ce9e.jpg",
                "image/jpeg"
            ),
            Some(("aris-and-hibiki-sample-ce9e".into(), "jpg".into()))
        );
        assert_eq!(
            name("https://x.example/a%20b.webp?x=1", ""),
            Some(("a-b".into(), "webp".into()))
        );
        assert_eq!(
            name("https://x.example/", "image/png"),
            Some(("image".into(), "png".into()))
        );
        assert_eq!(name("https://x.example/page.html", "text/html"), None);
        assert_eq!(name("https://x.example/noext", ""), None);
    }
}
