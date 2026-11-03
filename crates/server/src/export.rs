//! YAML export, also used as a backup.
//!
//! `GET /api/export` (admin) downloads the whole dashboard as
//! `homedit.yaml`. Secret widget fields are left out unless
//! `?secrets=true` is given, in which case they are written in plain text
//! (decrypted), so the file works as a full backup on another server with a
//! different key. Restore by uploading the file to `POST /api/import`,
//! which recognises `homedit.yaml` next to the gethomepage files.

use axum::extract::{Query, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use homedit_core::secrets::is_secret_key;
use homedit_core::{Dashboard, Group};
use serde::Deserialize;
use serde_json::Value;

use crate::auth::RequireAdmin;
use crate::{ApiError, AppState, secrets};

/// File name of an exported dashboard.
pub const FILE_NAME: &str = "homedit.yaml";

/// Query for [`download`].
#[derive(Debug, Default, Deserialize)]
pub struct ExportQuery {
    /// Include decrypted secrets.
    #[serde(default)]
    pub secrets: bool,
}

/// Removes secret fields from every widget, or decrypts them when `keep`.
pub fn prepare(
    dash: &mut Dashboard,
    keep: bool,
    sb: &homedit_auth::secrets::SecretBox,
) -> Result<(), String> {
    fn walk(
        groups: &mut [Group],
        keep: bool,
        sb: &homedit_auth::secrets::SecretBox,
    ) -> Result<(), String> {
        for g in groups {
            for s in &mut g.services {
                if let Some(w) = s.widget.as_mut() {
                    if keep {
                        *w = secrets::open_widget(w, sb)?;
                    } else if let Value::Object(map) = w {
                        map.retain(|k, _| !is_secret_key(k));
                    }
                }
            }
            walk(&mut g.groups, keep, sb)?;
        }
        Ok(())
    }
    walk(&mut dash.groups, keep, sb)
}

/// Serialises a dashboard as commented YAML.
pub fn to_yaml(dash: &Dashboard, with_secrets: bool) -> Result<String, String> {
    let body = serde_yaml_ng::to_string(dash).map_err(|e| e.to_string())?;
    let note = if with_secrets {
        "# Contains API keys and passwords in plain text. Store it safely.\n"
    } else {
        "# Secrets (API keys, passwords) are not included; re-enter them after import.\n"
    };
    Ok(format!(
        "# homedit dashboard export, version {}\n{note}{body}",
        env!("CARGO_PKG_VERSION")
    ))
}

/// Parses an exported file back into a dashboard.
pub fn from_yaml(text: &str) -> Result<Dashboard, String> {
    serde_yaml_ng::from_str(text).map_err(|e| e.to_string())
}

/// `GET /api/export`.
pub async fn download(
    RequireAdmin(admin): RequireAdmin,
    State(state): State<AppState>,
    Query(q): Query<ExportQuery>,
) -> Result<Response, ApiError> {
    let mut dash = state.db.load_dashboard().await?;
    prepare(&mut dash, q.secrets, state.auth.secrets()).map_err(ApiError::internal)?;
    let yaml = to_yaml(&dash, q.secrets).map_err(ApiError::internal)?;
    tracing::info!(by = %admin.author(), secrets = q.secrets, "dashboard exported");
    Ok((
        [
            (header::CONTENT_TYPE, "application/yaml; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"homedit.yaml\"",
            ),
            (header::CACHE_CONTROL, "no-store"),
        ],
        yaml,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use homedit_auth::secrets::SecretBox;
    use homedit_core::Service;
    use serde_json::json;

    fn dash(sb: &SecretBox) -> Dashboard {
        let mut s = Service::new("Sonarr", Some("http://s"));
        s.widget = Some(json!({"type":"sonarr","url":"http://s","key":"K"}));
        let mut d = Dashboard {
            groups: vec![Group {
                name: "Media".into(),
                services: vec![s],
                ..Group::default()
            }],
            ..Dashboard::default()
        };
        secrets::seal(&mut d, sb);
        d
    }

    #[test]
    fn without_and_with_secrets() {
        let sb = SecretBox::from_base64(&SecretBox::generate_key()).unwrap();
        let mut a = dash(&sb);
        prepare(&mut a, false, &sb).unwrap();
        let y = to_yaml(&a, false).unwrap();
        assert!(!y.contains("key:") && !y.contains("enc:"), "{y}");
        let mut b = dash(&sb);
        prepare(&mut b, true, &sb).unwrap();
        let y = to_yaml(&b, true).unwrap();
        assert!(y.contains("key: K"), "{y}");
        let back = from_yaml(&y).unwrap();
        assert_eq!(
            back.groups[0].services[0].widget.as_ref().unwrap()["key"],
            "K"
        );
        assert_eq!(back, b);
    }
}
