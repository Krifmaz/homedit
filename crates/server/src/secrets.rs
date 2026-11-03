//! Encryption of widget secrets at rest.
//!
//! Secret fields of a service widget (`key`, `password`, `token`, ...) are
//! stored as AES-GCM ciphertext (`enc:v1:...`) produced by the auth crate's
//! [`SecretBox`]. They are decrypted only on the server, right before a
//! widget request, and are never sent to a browser: the dashboard view
//! shows `••••` and the editor document shows `{"$secret":"set"}`.

use homedit_auth::secrets::{SecretBox, decrypt_json_secrets, encrypt_json_secrets};
use homedit_core::{Dashboard, Group};
use serde_json::Value;

use crate::AppState;

/// Scope bound into every widget secret's ciphertext.
pub const WIDGET_SCOPE: &str = "widget";

/// Encrypts every plaintext secret in every service widget of `dash`.
/// Returns whether anything changed.
pub fn seal(dash: &mut Dashboard, sb: &SecretBox) -> bool {
    fn walk(groups: &mut [Group], sb: &SecretBox, changed: &mut bool) {
        for g in groups {
            for s in &mut g.services {
                if let Some(w) = s.widget.as_mut() {
                    let before = w.clone();
                    encrypt_json_secrets(sb, WIDGET_SCOPE, w);
                    *changed |= *w != before;
                }
            }
            walk(&mut g.groups, sb, changed);
        }
    }
    let mut changed = false;
    walk(&mut dash.groups, sb, &mut changed);
    changed
}

/// Decrypts a stored widget block for use by the widget crate.
pub fn open_widget(widget: &Value, sb: &SecretBox) -> Result<Value, String> {
    let mut w = widget.clone();
    decrypt_json_secrets(sb, WIDGET_SCOPE, &mut w).map_err(|e| e.to_string())?;
    Ok(w)
}

/// Encrypts plaintext secrets left in the stored dashboard, for example
/// from a database written before encryption existed. Runs at startup.
pub async fn seal_stored(state: &AppState) -> anyhow::Result<()> {
    let (version, mut dash) = state.db.load_versioned().await?;
    if seal(&mut dash, state.auth.secrets()) {
        state
            .db
            .save_dashboard(
                &dash,
                Some(version),
                Some("system"),
                Some("encrypted stored secrets"),
            )
            .await?;
        tracing::info!("encrypted plaintext widget secrets in the database");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use homedit_core::Service;
    use serde_json::json;

    #[test]
    fn seal_then_open() {
        let sb = SecretBox::from_base64(&SecretBox::generate_key()).unwrap();
        let mut s = Service::new("Sonarr", None);
        s.widget = Some(json!({"type":"sonarr","url":"http://s","key":"abc"}));
        let mut d = Dashboard {
            groups: vec![Group {
                name: "G".into(),
                services: vec![s],
                ..Group::default()
            }],
            ..Dashboard::default()
        };
        assert!(seal(&mut d, &sb));
        let stored = d.groups[0].services[0].widget.clone().unwrap();
        assert_ne!(stored["key"], "abc");
        assert_eq!(stored["url"], "http://s");
        assert!(!seal(&mut d, &sb), "sealing twice changes nothing");
        assert_eq!(open_widget(&stored, &sb).unwrap()["key"], "abc");
    }
}
