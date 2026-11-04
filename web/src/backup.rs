//! Import from a gethomepage config folder or a homedit backup, and download the config as YAML.

use crate::state::app;
use kit_leptos::{api, modal, rt};
use kit_rules::dialogs::Confirm;
use leptos::html;
use leptos::prelude::*;
use serde_json::Value;

/// The files the import understands.
pub const IMPORT_ACCEPT: &str = ".yaml,.yml,.css";

/// What an import said: the new version and its warnings (`file`, `path`, `message`).
#[derive(Clone, Debug, PartialEq)]
pub struct Imported {
    /// The version the import was saved as.
    pub version: String,
    /// Problems found, one line each.
    pub warnings: Vec<(String, String, String)>,
}

impl Imported {
    /// Reads `POST /api/import`'s answer.
    pub fn from_wire(v: &Value) -> Imported {
        let s = |w: &Value, k: &str| w.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        Imported {
            version: match v.get("version") {
                Some(Value::String(s)) => s.clone(),
                Some(x) => x.to_string(),
                None => String::new(),
            },
            warnings: v
                .get("warnings")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|w| (s(w, "file"), s(w, "path"), s(w, "message")))
                .collect(),
        }
    }
}

/// Where the config downloads from.
pub fn export_url(secrets: bool) -> String {
    format!("/api/export?secrets={secrets}")
}

/// The import / export panel in the editor's drawer.
#[component]
pub fn BackupPanel() -> impl IntoView {
    let a = app();
    let input = NodeRef::<html::Input>::new();
    let names = RwSignal::new(Vec::<String>::new());
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let result = RwSignal::new(None::<Imported>);

    let picked = move |_| {
        let list = input.get_untracked().and_then(|i| i.files());
        names.set(
            list.map(|l| {
                (0..l.length())
                    .filter_map(|i| l.get(i))
                    .map(|f| f.name())
                    .collect()
            })
            .unwrap_or_default(),
        );
    };
    let upload = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        let Some(files) = input
            .get_untracked()
            .and_then(|i| i.files())
            .filter(|l| l.length() > 0)
        else {
            return;
        };
        rt::spawn(async move {
            let mut q = crate::words::import_question();
            if a.dirty() {
                q = q.text("The import replaces the dashboard, and your unsaved changes are lost. The current one stays in the history.");
            }
            if !modal::confirm(q).await {
                return;
            }
            let Ok(form) = web_sys::FormData::new() else {
                return;
            };
            for i in 0..files.length() {
                if let Some(f) = files.get(i) {
                    let _ = form.append_with_blob_and_filename("file", &f, &f.name());
                }
            }
            busy.set(true);
            error.set(None);
            result.set(None);
            match api::post_form("/import", form).await {
                Ok(v) => {
                    result.set(Some(Imported::from_wire(&v)));
                    a.reload();
                }
                Err(e) => error.set(Some(e.message)),
            }
            busy.set(false);
        });
    };
    let with_secrets = move |e: leptos::ev::MouseEvent| {
        e.prevent_default();
        rt::spawn(async move {
            let q = Confirm::new("Download with secrets?")
                .text("The file holds API keys and passwords in plain text. Keep it safe.")
                .ok("Download");
            if modal::confirm(q).await
                && let Some(w) = rt::window()
            {
                let _ = w.location().set_href(&export_url(true));
            }
        });
    };

    view! {
        <div class="backup-panel">
            <h2>"Import and export"</h2>
            <form on:submit=upload>
                <h3>"Import"</h3>
                <p class="muted small">
                    "Pick files from a gethomepage config folder (services.yaml, bookmarks.yaml, widgets.yaml, settings.yaml, docker.yaml, custom.css), or a homedit.yaml backup to restore. The current version stays in the history, so you can undo an import there."
                </p>
                <label class="field">
                    <span>"Files"</span>
                    <input class="input" type="file" multiple accept=IMPORT_ACCEPT node_ref=input on:change=picked/>
                    {move || { let n = names.get(); (!n.is_empty()).then(|| view! { <small>{n.join(", ")}</small> }) }}
                </label>
                <button type="submit" class="btn primary" disabled=move || busy.get() || names.with(Vec::is_empty)>
                    {move || if busy.get() { "Importing…" } else { "Import" }}
                </button>
            </form>
            {move || error.get().map(|e| view! { <div class="error-box" role="alert">{e}</div> })}
            {move || result.get().map(|r| view! {
                <div role="status">
                    <p>"Imported as version "{r.version.clone()}"."{r.warnings.is_empty().then_some(" No problems found.")}</p>
                    {(!r.warnings.is_empty()).then(|| view! {
                        <ul class="import-warnings">
                            {r.warnings.iter().map(|(f, p, m)| view! {
                                <li><strong>{f.clone()}</strong>{(!p.is_empty()).then(|| view! { " "<code>{p.clone()}</code> })}": "{m.clone()}</li>
                            }).collect_view()}
                        </ul>
                    })}
                </div>
            })}

            <h3>"Export"</h3>
            <p class="muted small">"Download the whole config as homedit.yaml, for backups or to move to another server."</p>
            <div class="row wrap">
                <a class="btn" href=export_url(false) download="homedit.yaml">"Download (without secrets)"</a>
                <a class="btn" href=export_url(true) download="homedit.yaml" on:click=with_secrets>"Download with secrets"</a>
            </div>

            <div class="drawer-footer row">
                <button type="button" class="btn" on:click=move |_| a.target.set(None)>"Close"</button>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_import_answer() {
        let r = Imported::from_wire(
            &json!({"version": 7, "warnings": [{"file": "services.yaml", "path": "0.x", "message": "bad"}]}),
        );
        assert_eq!(r.version, "7");
        assert_eq!(
            r.warnings,
            vec![("services.yaml".into(), "0.x".into(), "bad".into())]
        );
        assert_eq!(export_url(false), "/api/export?secrets=false");
    }
}
