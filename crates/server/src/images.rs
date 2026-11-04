//! Background image uploads.
//!
//! Uploaded files live in `$HOMEDIT_DATA_DIR/images/` and are served at
//! `/images/<file>`, the same path gethomepage serves its `public/images`
//! folder from. A gethomepage `settings.yaml` with
//! `background: /images/wallpaper.jpg` therefore works as soon as the file
//! is copied (or uploaded) into that folder. Responses carry the same
//! sandboxing headers as uploaded icons.

use axum::Json;
use axum::extract::{DefaultBodyLimit, Multipart, State};
use axum::routing::{MethodRouter, get};
use serde::Serialize;

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
        let name = format!("{stem}.{ext}");
        let bytes = field
            .bytes()
            .await
            .map_err(|e| ApiError::bad_request(e.to_string()))?;
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(ApiError::bad_request("image is larger than 20 MB"));
        }
        tokio::fs::create_dir_all(state.image_dir())
            .await
            .map_err(ApiError::internal)?;
        tokio::fs::write(state.image_dir().join(&name), &bytes)
            .await
            .map_err(ApiError::internal)?;
        return Ok(Json(ImageFile {
            url: format!("{IMAGE_PREFIX}{name}"),
            name,
        }));
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
}
