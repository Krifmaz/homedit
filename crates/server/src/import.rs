//! Importing a gethomepage config folder.
//!
//! * At startup, when `HOMEDIT_IMPORT_DIR` points at a gethomepage config
//!   folder and nothing has been saved yet, it is imported once.
//! * `POST /api/import` (admin, multipart) takes the YAML files
//!   (`services.yaml`, `bookmarks.yaml`, `widgets.yaml`, `settings.yaml`,
//!   `docker.yaml`, `custom.css`) and replaces the dashboard with them. The
//!   previous dashboard stays in the version history, so the import can be
//!   undone with one click.
//!
//! `{{HOMEPAGE_VAR_*}}` and `{{HOMEPAGE_FILE_*}}` placeholders are resolved
//! from the server's environment. Imported secrets are encrypted before
//! they are stored.

use std::path::Path;

use axum::Json;
use axum::extract::{Multipart, State};
use homedit_import::{EnvResolver, ImportSources, ImportWarning, import_dashboard};
use serde::Serialize;

use crate::auth::RequireAdmin;
use crate::status::LiveEvent;
use crate::{ApiError, AppState, secrets};

/// Largest accepted file in an import upload.
pub const MAX_FILE_BYTES: usize = 1024 * 1024;

/// Result of an import.
#[derive(Debug, Serialize)]
pub struct Imported {
    /// The new version.
    pub version: i64,
    /// Non-fatal problems, one per entry.
    pub warnings: Vec<ImportWarning>,
}

/// Imports `dir` when the database is still empty. Returns the new version
/// when something was imported.
pub async fn import_on_start(state: &AppState, dir: &Path) -> anyhow::Result<Option<i64>> {
    if state.db.current_version().await? > 0 {
        tracing::info!(dir = %dir.display(), "dashboard already saved; skipping HOMEDIT_IMPORT_DIR");
        return Ok(None);
    }
    let report = homedit_import::import_dashboard_dir(dir)?;
    for w in &report.warnings {
        tracing::warn!(file = %w.file, path = %w.path, "import: {}", w.message);
    }
    let mut dash = report.dashboard;
    secrets::seal(&mut dash, state.auth.secrets());
    let summary = format!("imported from {}", dir.display());
    let (version, _) = state
        .db
        .save_dashboard(&dash, Some(0), Some("import"), Some(&summary))
        .await?;
    tracing::info!(version, "imported gethomepage config");
    Ok(Some(version))
}

/// `POST /api/import`.
pub async fn upload(
    RequireAdmin(admin): RequireAdmin,
    State(state): State<AppState>,
    mut form: Multipart,
) -> Result<Json<Imported>, ApiError> {
    let mut sources = ImportSources::default();
    let mut any = false;
    while let Some(field) = form
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?
    {
        let name = field
            .file_name()
            .or(field.name())
            .unwrap_or_default()
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or_default()
            .to_owned();
        let bytes = field
            .bytes()
            .await
            .map_err(|e| ApiError::bad_request(e.to_string()))?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err(ApiError::bad_request(format!("{name} is larger than 1 MB")));
        }
        let text = String::from_utf8(bytes.to_vec())
            .map_err(|_| ApiError::bad_request(format!("{name} is not UTF-8 text")))?;
        if sources.set_by_name(&name, text) {
            any = true;
        }
    }
    if !any {
        return Err(ApiError::bad_request(
            "no gethomepage files found; send services.yaml, bookmarks.yaml, widgets.yaml, settings.yaml, docker.yaml or custom.css",
        ));
    }
    let report = import_dashboard(&sources, &EnvResolver)
        .map_err(|e| ApiError::new(axum::http::StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;
    let mut dash = report.dashboard;
    secrets::seal(&mut dash, state.auth.secrets());
    let (version, _) = state
        .db
        .save_dashboard(
            &dash,
            None,
            Some(&admin.author()),
            Some("imported gethomepage config"),
        )
        .await?;
    state.status.publish(LiveEvent::Config { version });
    Ok(Json(Imported {
        version,
        warnings: report.warnings,
    }))
}
