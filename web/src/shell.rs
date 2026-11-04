//! The page around the dashboard: the background, the header (title, search, Edit, the account menu), the
//! editor's toolbar and drawer, the sign-in page, and the kit's pages (`#/settings`, `#/bug-reports`).

use crate::client;
use crate::dash::Dashboard;
use crate::model::Me;
use crate::state::{App, Target, app};
use crate::theme;
use crate::widgets::InfoStrip;
use kit_leptos::rt;
use leptos::prelude::*;
use serde_json::Value;
use wasm_bindgen::JsCast;

/// The page the address names: `#/settings?s=diagnostics` is path `/settings`, query `s=diagnostics`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Route {
    /// After `#`, before `?`.
    pub path: String,
    /// After `?`.
    pub query: String,
}

impl Route {
    /// The route of a `location.hash`.
    pub fn parse(hash: &str) -> Route {
        let h = hash.trim_start_matches('#');
        let (path, query) = h.split_once('?').unwrap_or((h, ""));
        Route {
            path: if path.is_empty() {
                "/".into()
            } else {
                path.to_string()
            },
            query: query.to_string(),
        }
    }

    /// The value of `key` in the query, "" when missing.
    pub fn param(&self, key: &str) -> String {
        self.query
            .split('&')
            .filter_map(|p| p.split_once('='))
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.to_string())
            .unwrap_or_default()
    }
}

fn current_route() -> Route {
    Route::parse(
        &rt::window()
            .and_then(|w| w.location().hash().ok())
            .unwrap_or_default(),
    )
}

/// Opens `path` of the app (`/settings?s=whats-new`).
pub fn navigate(path: &str) {
    if let Some(w) = rt::window() {
        let _ = w.location().set_hash(path);
    }
}

/// The whole page.
#[component]
pub fn Root() -> impl IntoView {
    let a = app();
    let (route, set_route) = signal(current_route());
    rt::on_window("hashchange", move |_| set_route.set(current_route()));
    // The look follows the settings, live while they are edited.
    Effect::new(move |_| {
        if let Some(d) = a.doc() {
            theme::apply(&d.settings);
        }
    });
    if let Some(mq) = rt::window().and_then(|w| {
        w.match_media("(prefers-color-scheme: light)")
            .ok()
            .flatten()
    }) {
        rt::listen_forever(&mq, "change", move |_| {
            if let Some(d) = a.doc() {
                theme::apply(&d.settings);
            }
        });
    }
    // Only signing in and the page rebuild the view, not every change of the session.
    let login = Memo::new(move |_| a.needs_login());
    move || {
        if login.get() {
            return view! { <crate::login::Login/> }.into_any();
        }
        let r = route.get();
        match r.path.as_str() {
            "/settings" | "/bug-reports" => {
                view! { <crate::kitpages::KitPage route=r/> }.into_any()
            }
            _ => view! { <Home/> }.into_any(),
        }
    }
}

/// The dashboard page.
#[component]
fn Home() -> impl IntoView {
    let a = app();
    let bg = Memo::new(move |_| {
        a.doc()
            .and_then(|d| d.settings.background)
            .filter(|b| !b.url.is_empty())
    });
    let info = Memo::new(move |_| a.doc().map(|d| d.info_widgets).unwrap_or_default());
    // The notices draw into this page's slot.
    Effect::new(|_| kit_leptos::notice::mount());
    let title = move || {
        a.doc()
            .map(|d| d.settings.title)
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| "homedit".into())
    };
    view! {
        {move || bg.get().map(|b| {
            let filter = theme::background_filter(&b);
            let mut style = format!("background-image:{};opacity:{}", theme::css_url(&b.url), b.opacity);
            if !filter.is_empty() {
                style.push_str(&format!(";filter:{filter}"));
            }
            if b.blur > 0.0 {
                // Blur fades the edges; overscan hides that.
                style.push_str(";transform:scale(1.06)");
            }
            view! { <div class="page-bg" aria-hidden="true" style=style></div> }
        })}
        <div class="app" class:editing=move || a.editing.get() class:with-drawer=move || a.target.with(Option::is_some)>
            {move || a.editing.get().then(|| view! { <Toolbar/> })}
            <div id="server-notice"></div>
            <header class="header">
                <h1>{title}</h1>
                <crate::search::SearchBar/>
                <span class="spacer"></span>
                {move || (a.is_admin() && !a.editing.get()).then(|| view! {
                    <button type="button" class="btn" on:click=move |_| { rt::spawn(async move { a.start_editing().await; }); }>"✎ Edit"</button>
                })}
                <AccountMenu/>
            </header>
            {move || {
                let w = info.get();
                (!w.is_empty()).then(|| view! { <InfoStrip widgets=w/> })
            }}
            <main><Dashboard/></main>
        </div>
        {move || (a.editing.get() || a.target.get() == Some(Target::Password)).then(|| view! { <crate::drawer::Drawer/> })}
    }
}

fn is_local(me: &Me) -> bool {
    me.source.as_deref().is_none_or(|s| s == "local")
}

/// Who is signed in, with Password, Settings, Report a bug, What's new and Sign out; Sign in for visitors.
#[component]
fn AccountMenu() -> impl IntoView {
    let a = app();
    let open = RwSignal::new(false);
    if let Some(d) = rt::document() {
        scoped(vec![rt::listen(&d, "click", move |e| {
            if open.get_untracked() && rt::closest(e, ".account").is_none() {
                open.set(false);
            }
        })]);
    }
    move || {
        let Some(me) = a.me.get() else {
            return ().into_any();
        };
        if me.auth_enabled && me.user.is_none() {
            return view! { <button type="button" class="btn" on:click=move |_| a.want_login.set(true)>"Sign in"</button> }.into_any();
        }
        let admin = me.is_admin();
        let name = me.user.clone().unwrap_or_else(|| "Menu".into());
        let local = me.auth_enabled && me.user.is_some() && is_local(&me);
        let signed_in = me.auth_enabled && me.user.is_some();
        let go = move |path: &'static str| {
            move |_| {
                open.set(false);
                navigate(path);
            }
        };
        view! {
            <span class="account">
                <button type="button" class="btn ghost" aria-haspopup="menu" aria-expanded=move || open.get().to_string()
                    title=format!("{name} ({})", me.role.map(|r| r.as_str()).unwrap_or("no role"))
                    on:click=move |_| open.update(|o| *o = !*o)>
                    <span class="ellipsis account-name">{name.clone()}</span>" ▾"
                </button>
                <div class="menu card" role="menu" hidden=move || !open.get()>
                    {local.then(|| view! { <button type="button" class="menu-item" role="menuitem"
                        on:click=move |_| { open.set(false); a.target.set(Some(Target::Password)); }>"Change password"</button> })}
                    {admin.then(|| view! {
                        <button type="button" class="menu-item" role="menuitem" on:click=go("/settings")>"Settings"</button>
                        <button type="button" class="menu-item" role="menuitem" on:click=go("/settings?s=diagnostics")>"Diagnostics"</button>
                    })}
                    <button type="button" class="menu-item" role="menuitem" on:click=go("/settings?s=whats-new")>"What's new"</button>
                    <button type="button" class="menu-item" role="menuitem" on:click=go("/bug-reports")>"Bug reports"</button>
                    {admin.then(|| view! { <a class="menu-item" role="menuitem" href="/api/docs" target="_blank" rel="noopener noreferrer">"API docs"</a> })}
                    {signed_in.then(|| view! { <button type="button" class="menu-item" role="menuitem"
                        on:click=move |_| { open.set(false); a.logout(); }>"Sign out"</button> })}
                </div>
            </span>
        }
        .into_any()
    }
}

fn in_text_field(e: &web_sys::Event) -> bool {
    rt::closest(e, "input, textarea, select, [contenteditable]").is_some()
}

/// The editor's toolbar: what to edit, Undo and Redo, Save and Done, with Ctrl+Z, Ctrl+Shift+Z, Ctrl+Y and
/// Ctrl+S, and a warning before leaving the page with unsaved changes.
#[component]
fn Toolbar() -> impl IntoView {
    let a = app();
    let keys = rt::listen(&rt::window().expect("window").into(), "keydown", move |e| {
        let Some(k) = e.dyn_ref::<web_sys::KeyboardEvent>() else {
            return;
        };
        if !(k.ctrl_key() || k.meta_key()) {
            return;
        }
        let key = k.key().to_lowercase();
        if key == "s" {
            e.prevent_default();
            a.save();
            return;
        }
        if in_text_field(e) {
            return;
        }
        if key == "z" && !k.shift_key() {
            e.prevent_default();
            a.undo.update(|u| u.undo());
        } else if (key == "z" && k.shift_key()) || key == "y" {
            e.prevent_default();
            a.undo.update(|u| u.redo());
        }
    });
    let unload = rt::listen(
        &rt::window().expect("window").into(),
        "beforeunload",
        move |e| {
            if a.dirty() {
                e.prevent_default();
                if let Some(b) = e.dyn_ref::<web_sys::BeforeUnloadEvent>() {
                    b.set_return_value("unsaved");
                }
            }
        },
    );
    scoped(vec![keys, unload]);
    let open = move |t: Target| move |_| a.target.set(Some(t.clone()));
    let auth = move || a.me.with(|m| m.as_ref().is_some_and(|m| m.auth_enabled));
    view! {
        <div class="toolbar" role="toolbar" aria-label="Editor">
            <strong>"Editing"</strong>
            <button type="button" class="btn sm" on:click=open(Target::Group { group: None })>"+ Group"</button>
            <button type="button" class="btn sm" on:click=open(Target::InfoWidgets)>"Info widgets"</button>
            <button type="button" class="btn sm" on:click=open(Target::Appearance)>"Appearance"</button>
            <button type="button" class="btn sm" on:click=open(Target::Scan)>"Find services"</button>
            <button type="button" class="btn sm" on:click=open(Target::Backup)>"Import / export"</button>
            <button type="button" class="btn sm" on:click=open(Target::History)>"History"</button>
            {move || auth().then(|| view! { <button type="button" class="btn sm" on:click=open(Target::Users)>"Users"</button> })}
            <span class="row">
                <button type="button" class="btn sm" title="Undo (Ctrl+Z)" disabled=move || a.undo.with(|u| u.past.is_empty())
                    on:click=move |_| a.undo.update(|u| u.undo())>"↶ Undo"</button>
                <button type="button" class="btn sm" title="Redo (Ctrl+Shift+Z)" disabled=move || a.undo.with(|u| u.future.is_empty())
                    on:click=move |_| a.undo.update(|u| u.redo())>"↷ Redo"</button>
            </span>
            <span class="spacer"></span>
            <span class="status-text small muted" aria-live="polite">
                {move || if a.saving.get() { "Saving…" } else if a.dirty() { "Unsaved changes" } else { "All changes saved" }}
            </span>
            <button type="button" class="btn sm primary" title="Save (Ctrl+S)" disabled=move || !a.dirty() || a.saving.get()
                on:click=move |_| a.save()>"Save"</button>
            <button type="button" class="btn sm" on:click=move |_| a.stop_editing()>"Done"</button>
        </div>
    }
}

/// Keeps listeners until the component that made them goes.
pub fn scoped(l: Vec<rt::Listener>) {
    let keep = send_wrapper::SendWrapper::new(l);
    on_cleanup(move || drop(keep));
}

/// Starts the app's own parts: the live state, the drags, the session.
pub fn start(a: App) {
    crate::dnd::install(a);
    a.load();
    // Kit calls that answer 401 (an expired session) show sign-in.
    rt::on_window(&format!("{}:unauthorized", client::APP_ID), move |_| {
        a.unauthorized.set(true)
    });
}

/// A string field, "" when missing.
pub fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::Route;

    #[test]
    fn routes() {
        assert_eq!(Route::parse("").path, "/");
        let r = Route::parse("#/settings?s=diagnostics");
        assert_eq!(
            (
                r.path.as_str(),
                r.param("s").as_str(),
                r.param("x").as_str()
            ),
            ("/settings", "diagnostics", "")
        );
    }
}
