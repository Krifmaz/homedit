//! API documentation.
//!
//! * `GET /api/openapi.yaml` serves the OpenAPI 3.1 description of every
//!   endpoint (`crates/server/openapi.yaml`, with the version filled in).
//! * `GET /api/docs` renders the same document as a plain, self-contained
//!   HTML page, so it works offline with no script or CDN.
//!
//! Both are public: they describe the API and hold no data.

use std::fmt::Write as _;
use std::sync::LazyLock;

use axum::http::header;
use axum::response::{Html, IntoResponse, Response};
use serde_json::Value;

const SPEC_SOURCE: &str = include_str!("../openapi.yaml");
const METHODS: [&str; 5] = ["get", "post", "put", "patch", "delete"];

/// The OpenAPI document as served.
pub static SPEC: LazyLock<String> = LazyLock::new(|| {
    SPEC_SOURCE.replacen(
        "version: VERSION",
        &format!("version: {}", env!("CARGO_PKG_VERSION")),
        1,
    )
});

static PAGE: LazyLock<String> = LazyLock::new(|| render(&SPEC).unwrap_or_else(|e| esc(&e)));

/// `GET /api/openapi.yaml`.
pub async fn spec() -> Response {
    (
        [(header::CONTENT_TYPE, "application/yaml; charset=utf-8")],
        SPEC.as_str(),
    )
        .into_response()
}

/// `GET /api/docs`.
pub async fn page() -> Html<&'static str> {
    Html(PAGE.as_str())
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Escapes text and turns `code` spans and **bold** into HTML, keeping
/// paragraphs.
fn prose(s: &str) -> String {
    let mut out = String::new();
    for para in s.split("\n\n").filter(|p| !p.trim().is_empty()) {
        let text = esc(para.trim());
        let mut html = String::new();
        for (i, part) in text.split('`').enumerate() {
            if i % 2 == 1 {
                let _ = write!(html, "<code>{part}</code>");
            } else {
                for (j, b) in part.split("**").enumerate() {
                    if j % 2 == 1 {
                        let _ = write!(html, "<strong>{b}</strong>");
                    } else {
                        html.push_str(b);
                    }
                }
            }
        }
        let _ = write!(out, "<p>{html}</p>");
    }
    out
}

/// Renders the document as HTML.
pub fn render(yaml: &str) -> Result<String, String> {
    let doc: Value = serde_yaml_ng::from_str(yaml).map_err(|e| e.to_string())?;
    let info = &doc["info"];
    let title = esc(info["title"].as_str().unwrap_or("API"));
    let version = esc(info["version"].as_str().unwrap_or(""));
    let mut body = String::new();
    let _ = write!(
        body,
        "<header><h1>{title} <small>{version}</small></h1><p><a href=\"/api/openapi.yaml\">openapi.yaml</a></p></header>{}",
        prose(info["description"].as_str().unwrap_or(""))
    );
    let paths = doc["paths"].as_object().cloned().unwrap_or_default();
    let tags: Vec<String> = doc["tags"]
        .as_array()
        .map(|t| {
            t.iter()
                .filter_map(|t| t["name"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let _ = write!(body, "<nav>");
    for t in &tags {
        let _ = write!(body, "<a href=\"#{0}\">{0}</a>", esc(t));
    }
    let _ = write!(body, "<a href=\"#schemas\">schemas</a></nav>");
    for tag in &tags {
        let _ = write!(body, "<h2 id=\"{0}\">{0}</h2>", esc(tag));
        for (path, item) in &paths {
            for m in METHODS {
                let Some(op) = item.get(m) else { continue };
                let in_tag = op["tags"]
                    .as_array()
                    .is_some_and(|t| t.iter().any(|x| x.as_str() == Some(tag)));
                if !in_tag {
                    continue;
                }
                let _ = write!(
                    body,
                    "<details><summary><span class=\"m {m}\">{}</span> <code>{}</code> {}</summary>{}",
                    m.to_uppercase(),
                    esc(path),
                    esc(op["summary"].as_str().unwrap_or("")),
                    prose(op["description"].as_str().unwrap_or(""))
                );
                let params: Vec<&Value> = item["parameters"]
                    .as_array()
                    .into_iter()
                    .chain(op["parameters"].as_array())
                    .flatten()
                    .collect();
                if !params.is_empty() {
                    let _ = write!(body, "<h4>Parameters</h4><ul>");
                    for p in params {
                        let _ = write!(
                            body,
                            "<li><code>{}</code> ({}) {}</li>",
                            esc(p["name"].as_str().unwrap_or("")),
                            esc(p["in"].as_str().unwrap_or("")),
                            esc(p["description"].as_str().unwrap_or(""))
                        );
                    }
                    let _ = write!(body, "</ul>");
                }
                if let Some(content) = op["requestBody"]["content"].as_object() {
                    for (ty, c) in content {
                        let _ = write!(
                            body,
                            "<h4>Body <small>{}</small></h4><pre>{}</pre>",
                            esc(ty),
                            esc(&serde_json::to_string_pretty(&c["schema"]).unwrap_or_default())
                        );
                    }
                }
                if let Some(resp) = op["responses"].as_object() {
                    let _ = write!(body, "<h4>Responses</h4><ul>");
                    for (code, r) in resp {
                        let _ = write!(
                            body,
                            "<li><code>{}</code> {}</li>",
                            esc(code),
                            esc(r["description"].as_str().unwrap_or(""))
                        );
                    }
                    let _ = write!(body, "</ul>");
                }
                let _ = write!(body, "</details>");
            }
        }
    }
    let _ = write!(body, "<h2 id=\"schemas\">schemas</h2>");
    if let Some(schemas) = doc["components"]["schemas"].as_object() {
        for (name, s) in schemas {
            let _ = write!(
                body,
                "<details id=\"schema-{0}\"><summary><code>{0}</code></summary><pre>{1}</pre></details>",
                esc(name),
                esc(&serde_json::to_string_pretty(s).unwrap_or_default())
            );
        }
    }
    Ok(format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{title}</title><style>{CSS}</style></head><body><main>{body}</main></body></html>"
    ))
}

const CSS: &str = ":root{--bg:#fafafa;--fg:#1c1c1e;--muted:#6b6b70;--card:#fff;--line:#e3e3e6;--code:#f1f1f4}
@media (prefers-color-scheme:dark){:root{--bg:#141416;--fg:#ececef;--muted:#9a9aa2;--card:#1d1d20;--line:#2e2e33;--code:#26262b}}
body{margin:0;background:var(--bg);color:var(--fg);font:15px/1.55 system-ui,sans-serif}
main{max-width:880px;margin:0 auto;padding:24px 16px 64px}
h1 small,h4 small{color:var(--muted);font-weight:400;font-size:.6em}
h2{margin-top:2em;text-transform:capitalize;border-bottom:1px solid var(--line);padding-bottom:4px}
h4{margin:1em 0 .3em}
nav{display:flex;flex-wrap:wrap;gap:8px 14px;margin:1em 0}
a{color:inherit}
code,pre{font:13px/1.45 ui-monospace,monospace;background:var(--code);border-radius:4px}
code{padding:1px 4px}
pre{padding:10px;overflow-x:auto}
details{background:var(--card);border:1px solid var(--line);border-radius:8px;margin:6px 0;padding:8px 12px}
summary{cursor:pointer;overflow-wrap:anywhere}
.m{display:inline-block;min-width:54px;font:600 12px ui-monospace,monospace;text-align:center;border-radius:4px;padding:2px 0;color:#fff}
.get{background:#2f7d4f}.post{background:#2f5fae}.put{background:#a5641c}.patch{background:#7a4fb0}.delete{background:#b33a3a}";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_parses_and_every_operation_is_described() {
        let doc: Value = serde_yaml_ng::from_str(&SPEC).unwrap();
        assert_eq!(doc["info"]["version"], env!("CARGO_PKG_VERSION"));
        let tags: Vec<&str> = doc["tags"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        for (path, item) in doc["paths"].as_object().unwrap() {
            for m in METHODS {
                if let Some(op) = item.get(m) {
                    assert!(op["summary"].is_string(), "{m} {path} has no summary");
                    assert!(op["responses"].is_object(), "{m} {path} has no responses");
                    let tag = op["tags"][0].as_str().unwrap_or_default();
                    assert!(tags.contains(&tag), "{m} {path} has unknown tag {tag}");
                }
            }
        }
    }

    /// Every route registered in the code is in the document.
    #[test]
    fn spec_covers_every_route() {
        let doc: Value = serde_yaml_ng::from_str(&SPEC).unwrap();
        let paths = doc["paths"].as_object().unwrap();
        let sources = [
            ("/api", include_str!("api.rs")),
            ("/api/auth", include_str!("../../auth/src/routes.rs")),
        ];
        let mut seen = 0;
        for (prefix, src) in sources {
            for part in src.split(".route(\"").skip(1) {
                let route = part.split('"').next().unwrap();
                let full = format!("{prefix}{route}");
                assert!(
                    paths.contains_key(&full),
                    "{full} is missing from openapi.yaml"
                );
                seen += 1;
            }
        }
        assert!(seen > 25, "found only {seen} routes");
    }

    #[test]
    fn page_escapes_and_formats() {
        assert_eq!(
            prose("a `<b>` **c**"),
            "<p>a <code>&lt;b&gt;</code> <strong>c</strong></p>"
        );
        let html = render(&SPEC).unwrap();
        assert!(html.contains("/api/scan/schedule") && html.contains("<details"));
    }
}
