//! The embedded React UI.
//!
//! Files under `web/dist` are compiled into the binary (in debug builds they
//! are read from disk, so a running `vite build --watch` is picked up).
//! Unknown paths outside `/api` fall back to `index.html` so client-side
//! routes work. Before `index.html` is sent, the literal placeholder
//! `<!--HOMEDIT_DATA-->` is replaced with the dashboard view as JSON in a
//! `<script id="homedit-data" type="application/json">` tag, so the first
//! paint needs no API round trip.

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

use crate::AppState;
use crate::auth::CurrentUser;
use crate::view::{self, Mode};

/// The built frontend.
#[derive(RustEmbed)]
#[folder = "../../web/dist/"]
#[exclude = ".gitkeep"]
struct Ui;

/// The names of the web app's files built into this binary.
pub fn file_names() -> impl Iterator<Item = String> {
    Ui::iter().map(|n| n.into_owned())
}

/// Placeholder in `index.html` replaced by the initial data.
pub const DATA_PLACEHOLDER: &str = "<!--HOMEDIT_DATA-->";

/// Page served when the frontend was not built into this binary.
const FALLBACK_INDEX: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>homedit</title></head><body>
<p>The homedit UI was not built into this binary. Run <code>npm ci &amp;&amp; npm run build</code> in <code>web/</code> and rebuild.
The API is available under <a href="/api/dashboard">/api/dashboard</a>.</p>
<!--HOMEDIT_DATA--></body></html>"#;

/// Serves a static file, or `index.html` with inlined data.
pub async fn serve(State(state): State<AppState>, user: CurrentUser, uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path.starts_with("api/") || path == "api" {
        return crate::ApiError::not_found().into_response();
    }
    if !path.is_empty()
        && path != "index.html"
        && let Some(file) = Ui::get(path)
    {
        let mut resp = Response::new(Body::from(file.data.into_owned()));
        let mime = file.metadata.mimetype();
        if let Ok(v) = HeaderValue::from_str(mime) {
            resp.headers_mut().insert(header::CONTENT_TYPE, v);
        }
        // Vite puts content-hashed files under assets/.
        let cache = if path.starts_with("assets/") {
            "public, max-age=31536000, immutable"
        } else {
            "public, max-age=3600"
        };
        resp.headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
        return resp;
    }
    // A missing file with an extension is a real 404, not a client route.
    if path
        .rsplit('/')
        .next()
        .is_some_and(|last| last.contains('.'))
        && path != "index.html"
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    index(&state, &user).await
}

async fn index(state: &AppState, user: &CurrentUser) -> Response {
    let template = Ui::get("index.html")
        .map(|f| String::from_utf8_lossy(&f.data).into_owned())
        .unwrap_or_else(|| FALLBACK_INDEX.to_owned());
    let data = if !user.can_view_dashboard() {
        // Not signed in and anonymous viewing is off: the UI shows login.
        serde_json::Value::Null
    } else {
        match state.db.load_versioned().await {
            Ok((version, dash)) => {
                let doc = view::to_view(
                    &dash,
                    version,
                    Mode::Dashboard {
                        show_hidden: user.can_view_hidden(),
                    },
                );
                serde_json::json!({ "dashboard": doc, "status": state.status.snapshot() })
            }
            Err(e) => {
                tracing::error!(error = %e, "cannot load dashboard for first paint");
                serde_json::Value::Null
            }
        }
    };
    let html = template.replacen(DATA_PLACEHOLDER, &inline_json(&data), 1);
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        html,
    )
        .into_response()
}

/// Renders `data` as a JSON script tag that is safe to place in HTML: `<`,
/// `>` and `&` are escaped so the content can never close the tag.
pub fn inline_json(data: &serde_json::Value) -> String {
    let json = serde_json::to_string(data)
        .unwrap_or_else(|_| "null".into())
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    format!(r#"<script id="homedit-data" type="application/json">{json}</script>"#)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_json_cannot_break_out() {
        let v = serde_json::json!({"title": "</script><script>alert(1)</script>"});
        let out = inline_json(&v);
        assert_eq!(out.matches("</script>").count(), 1, "{out}");
        let inner = out
            .trim_start_matches(r#"<script id="homedit-data" type="application/json">"#)
            .trim_end_matches("</script>");
        let back: serde_json::Value = serde_json::from_str(inner).unwrap();
        assert_eq!(back, v);
    }
}
