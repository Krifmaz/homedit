//! homedit's own API calls, on the kit's `fetch` client ([`kit_leptos::api`]).
//!
//! The kit's client adds homedit's headers to every call ([`backend`]): the CSRF token from `GET /api/me` on
//! everything but `GET`, and `If-Match` on the save of the configuration. A save refused because the
//! configuration changed elsewhere answers 409 ([`is_conflict`]).

use crate::model::{Doc, HistoryEntry, Me, StatusMap, history_from_wire, status_from_wire};
use kit_leptos::api::{self, ApiError, HttpBackend};
use serde_json::{Value, json};
use std::cell::RefCell;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

thread_local! {
    static CSRF: RefCell<String> = const { RefCell::new(String::new()) };
    static IF_MATCH: RefCell<Option<String>> = const { RefCell::new(None) };
    static EVENTS: RefCell<Option<web_sys::EventSource>> = const { RefCell::new(None) };
}

/// The project id: `X-Requested-With`, the `x-homedit` mark on the server's answers, the events' prefix.
pub const APP_ID: &str = "homedit";

/// The kit's client with homedit's headers.
pub fn backend() -> HttpBackend {
    HttpBackend::new(APP_ID).with_headers(|method, path| {
        let mut h = Vec::new();
        if method != "GET" {
            let t = CSRF.with(|c| c.borrow().clone());
            h.push(("X-CSRF-Token".to_string(), t));
        }
        if method == "PUT"
            && path == "/config"
            && let Some(v) = IF_MATCH.with(|m| m.borrow().clone())
        {
            h.push(("If-Match".to_string(), v));
        }
        h
    })
}

/// Remembers the CSRF token the server gave.
pub fn set_csrf(token: &str) {
    CSRF.with(|c| *c.borrow_mut() = token.to_string());
}

/// Whether a failed call was refused because the configuration changed elsewhere.
pub fn is_conflict(e: &ApiError) -> bool {
    e.status == 409
}

/// `GET /api/me`.
pub async fn me() -> Result<Me, ApiError> {
    let v = api::get("/me").await?;
    let me = Me::from_wire(&v);
    set_csrf(&me.csrf_token);
    Ok(me)
}

/// `GET /api/dashboard`: what visitors see (icons resolved, hidden groups only for admins).
pub async fn dashboard() -> Result<Doc, ApiError> {
    Ok(Doc::from_wire(&api::get("/dashboard").await?))
}

/// `GET /api/status`.
pub async fn status() -> Result<StatusMap, ApiError> {
    Ok(status_from_wire(&api::get("/status").await?))
}

/// `GET /api/config`: the editable document.
pub async fn config() -> Result<Doc, ApiError> {
    Ok(Doc::from_wire(&api::get("/config").await?))
}

/// `PUT /api/config` with `If-Match`; answers the new version.
pub async fn save_config(doc: &Doc) -> Result<String, ApiError> {
    IF_MATCH.with(|m| *m.borrow_mut() = Some(doc.version.clone()));
    let r = api::put("/config", doc.to_wire()).await;
    IF_MATCH.with(|m| *m.borrow_mut() = None);
    Ok(version_of(&r?))
}

fn version_of(v: &Value) -> String {
    match v.get("version") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(x) => x.to_string(),
    }
}

/// `GET /api/widgets/catalog`.
pub async fn catalog() -> Result<Vec<Value>, ApiError> {
    Ok(api::get("/widgets/catalog")
        .await?
        .as_array()
        .cloned()
        .unwrap_or_default())
}

/// `GET /api/history`.
pub async fn history() -> Result<Vec<HistoryEntry>, ApiError> {
    Ok(history_from_wire(&api::get("/history").await?))
}

/// `GET /api/history/{version}`.
pub async fn history_version(v: &str) -> Result<Doc, ApiError> {
    Ok(Doc::from_wire(
        &api::get(&format!("/history/{}", crate::model::js_encode(v))).await?,
    ))
}

/// `POST /api/history/{version}/restore`; answers the new version.
pub async fn restore(v: &str) -> Result<String, ApiError> {
    Ok(version_of(
        &api::post_empty(&format!("/history/{}/restore", crate::model::js_encode(v))).await?,
    ))
}

/// Signs in (`/auth/login`) or creates the first admin (`/auth/setup`); keeps the new CSRF token.
pub async fn session(path: &str, username: &str, password: &str) -> Result<Value, ApiError> {
    let v = api::post(path, json!({ "username": username, "password": password })).await?;
    if let Some(t) = v.get("csrfToken").and_then(Value::as_str) {
        set_csrf(t);
    }
    Ok(v)
}

/// `POST /api/auth/logout`; answers where to go next (the proxy's sign-out page), if anywhere.
pub async fn logout() -> Result<Option<String>, ApiError> {
    let v = api::post_empty("/auth/logout").await?;
    Ok(v.get("logoutUrl")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned))
}

/// `POST /api/auth/password`; keeps the new CSRF token.
pub async fn change_password(current: &str, new: &str) -> Result<(), ApiError> {
    let v = api::post(
        "/auth/password",
        json!({ "currentPassword": current, "newPassword": new }),
    )
    .await?;
    if let Some(t) = v.get("csrfToken").and_then(Value::as_str) {
        set_csrf(t);
    }
    Ok(())
}

/// What the live events tell the app.
pub struct Handlers {
    /// A `status` event's data.
    pub status: Box<dyn Fn(Value)>,
    /// The configuration was saved (its new version).
    pub config: Box<dyn Fn(String)>,
    /// A rescan found changes (one sentence each).
    pub changes: Box<dyn Fn(Vec<String>)>,
}

/// Opens `GET /api/events` (closing the stream before, so a new sign-in gets its own).
pub fn subscribe(h: Handlers) {
    EVENTS.with(|e| {
        if let Some(old) = e.borrow_mut().take() {
            old.close();
        }
    });
    let Ok(es) = web_sys::EventSource::new("/api/events") else {
        return;
    };
    let data = |e: &web_sys::Event| -> Option<Value> {
        let m = e.dyn_ref::<web_sys::MessageEvent>()?;
        serde_json::from_str(&m.data().as_string()?).ok()
    };
    let Handlers {
        status,
        config,
        changes,
    } = h;
    let on_status = Closure::<dyn Fn(web_sys::Event)>::new(move |e: web_sys::Event| {
        if let Some(v) = data(&e) {
            status(v);
        }
    });
    let on_config = Closure::<dyn Fn(web_sys::Event)>::new(move |e: web_sys::Event| {
        if let Some(v) = data(&e) {
            config(version_of(&v));
        }
    });
    let on_scan = Closure::<dyn Fn(web_sys::Event)>::new(move |e: web_sys::Event| {
        let Some(v) = data(&e) else { return };
        let d = v.get("data").cloned().unwrap_or(Value::Null);
        if d.get("event").and_then(Value::as_str) == Some("changes") {
            let msgs: Vec<String> = d
                .get("messages")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|m| m.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            if !msgs.is_empty() {
                changes(msgs);
            }
        }
    });
    let _ = es.add_event_listener_with_callback("status", on_status.as_ref().unchecked_ref());
    let _ = es.add_event_listener_with_callback("config", on_config.as_ref().unchecked_ref());
    let _ = es.add_event_listener_with_callback("scan", on_scan.as_ref().unchecked_ref());
    // The listeners live as long as the stream; a new stream gets new ones.
    on_status.forget();
    on_config.forget();
    on_scan.forget();
    EVENTS.with(|e| *e.borrow_mut() = Some(es));
}

/// The data the server put in the page for the first paint, if any.
pub fn inline_data() -> Option<(Doc, StatusMap)> {
    let el = kit_leptos::rt::by_id("homedit-data")?;
    let raw: Value = serde_json::from_str(&el.text_content()?).ok()?;
    crate::model::inline_from_wire(&raw)
}
