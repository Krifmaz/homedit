//! The drawer's panels: version history (with what a restore would change), accounts, and Change password.

use crate::client;
use crate::fmt;
use crate::login::{MIN_PASSWORD, login_error};
use crate::model::{Doc, Group, HistoryEntry};
use crate::shell::s;
use crate::state::app;
use kit_leptos::{api, modal, rt, toasts};
use leptos::prelude::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// How many items were added, removed and changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Added.
    pub added: usize,
    /// Removed.
    pub removed: usize,
    /// Changed.
    pub changed: usize,
}

impl Counts {
    fn of(a: &BTreeMap<String, String>, b: &BTreeMap<String, String>) -> Counts {
        let mut c = Counts::default();
        for (id, va) in a {
            match b.get(id) {
                None => c.removed += 1,
                Some(vb) if vb != va => c.changed += 1,
                _ => {}
            }
        }
        c.added = b.keys().filter(|k| !a.contains_key(*k)).count();
        c
    }

    fn line(&self, label: &str) -> Option<String> {
        let parts: Vec<String> = [
            (self.added, "added"),
            (self.removed, "removed"),
            (self.changed, "changed"),
        ]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, w)| format!("{n} {w}"))
        .collect();
        (!parts.is_empty()).then(|| format!("{label}: {}", parts.join(", ")))
    }

    fn is_zero(&self) -> bool {
        self.added + self.removed + self.changed == 0
    }
}

/// What going from `a` to `b` changes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diff {
    /// Groups.
    pub groups: Counts,
    /// Cards.
    pub services: Counts,
    /// Bookmarks.
    pub bookmarks: Counts,
    /// Whether the settings differ.
    pub settings: bool,
    /// Whether the info widgets differ.
    pub info_widgets: bool,
}

impl Diff {
    /// The changes between two documents (ids, places and contents).
    pub fn between(a: &Doc, b: &Doc) -> Diff {
        type Index = (
            BTreeMap<String, String>,
            BTreeMap<String, String>,
            BTreeMap<String, String>,
        );
        fn index(d: &Doc) -> Index {
            let (mut g, mut s, mut b) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
            fn walk(
                list: &[Group],
                parent: &str,
                g: &mut BTreeMap<String, String>,
                s: &mut BTreeMap<String, String>,
            ) {
                for x in list {
                    let order: Vec<&str> = x.items.iter().map(|i| i.id.as_str()).collect();
                    g.insert(
                        x.id.clone(),
                        format!(
                            "{:?}|{:?}|{}|{:?}|{parent}|{order:?}",
                            x.name, x.icon, x.hidden, x.layout
                        ),
                    );
                    for i in &x.items {
                        s.insert(i.id.clone(), format!("{i:?}|{}", x.id));
                    }
                    walk(&x.groups, &x.id, g, s);
                }
            }
            walk(&d.groups, "", &mut g, &mut s);
            for l in &d.bookmarks {
                for x in &l.items {
                    b.insert(x.id.clone(), format!("{x:?}|{}", l.id));
                }
            }
            (g, s, b)
        }
        let (ia, ib) = (index(a), index(b));
        Diff {
            groups: Counts::of(&ia.0, &ib.0),
            services: Counts::of(&ia.1, &ib.1),
            bookmarks: Counts::of(&ia.2, &ib.2),
            settings: a.settings != b.settings,
            info_widgets: a.info_widgets != b.info_widgets,
        }
    }

    /// Whether nothing changes.
    pub fn is_empty(&self) -> bool {
        self.groups.is_zero()
            && self.services.is_zero()
            && self.bookmarks.is_zero()
            && !self.settings
            && !self.info_widgets
    }

    /// One line per kind of change.
    pub fn lines(&self) -> Vec<String> {
        let mut out: Vec<String> = [
            self.groups.line("Groups"),
            self.services.line("Cards"),
            self.bookmarks.line("Bookmarks"),
        ]
        .into_iter()
        .flatten()
        .collect();
        if self.settings {
            out.push("Appearance changed".into());
        }
        if self.info_widgets {
            out.push("Info widgets changed".into());
        }
        out
    }
}

/// Close at the bottom of a panel.
fn close_footer() -> impl IntoView {
    let a = app();
    view! { <div class="drawer-footer row"><button type="button" class="btn" on:click=move |_| a.target.set(None)>"Close"</button></div> }
}

/// The saved versions, newest first: open one to see what a restore would change, then Restore.
#[component]
pub fn HistoryPanel() -> impl IntoView {
    let a = app();
    let entries = RwSignal::new(None::<Result<Vec<HistoryEntry>, String>>);
    let open = RwSignal::new(None::<(String, Option<Result<Diff, String>>)>);
    let restoring = RwSignal::new(None::<String>);
    let load = move || {
        rt::spawn(async move {
            entries.set(Some(client::history().await.map_err(|e| e.message)));
        })
    };
    load();
    let select = move |v: String| {
        if open.with(|o| o.as_ref().is_some_and(|(x, _)| *x == v)) {
            open.set(None);
            return;
        }
        open.set(Some((v.clone(), None)));
        rt::spawn(async move {
            let r = client::history_version(&v)
                .await
                .map_err(|e| e.message)
                .map(|past| a.undo.with_untracked(|u| Diff::between(&u.present, &past)));
            open.update(|o| {
                if let Some((x, d)) = o
                    && *x == v
                {
                    *d = Some(r);
                }
            });
        });
    };
    let restore = move |e: HistoryEntry| {
        rt::spawn(async move {
            let mut q = crate::words::restore_question(&e.version);
            if a.dirty() {
                q = q.text("The dashboard goes back to this version, and your unsaved changes are lost. The current one stays in the history.");
            }
            if !modal::confirm(q).await {
                return;
            }
            restoring.set(Some(e.version.clone()));
            match client::restore(&e.version).await {
                Ok(_) => {
                    a.reload();
                    open.set(None);
                    load();
                    toasts::toast(&format!("Version {} restored", e.version), "ok");
                }
                Err(err) => toasts::toast(&format!("Restore failed: {}", err.message), "err"),
            }
            restoring.set(None);
        });
    };
    view! {
        <div class="history-panel">
            <h2>"Version history"</h2>
            {move || match entries.get() {
                None => view! { <div class="empty"><span class="spinner"></span></div> }.into_any(),
                Some(Err(e)) => view! { <div class="error-box">"Could not load the history: "{e}</div> }.into_any(),
                Some(Ok(list)) if list.is_empty() => view! { <p class="muted small">"No saved versions yet."</p> }.into_any(),
                Some(Ok(list)) => {
                    let now = js_sys::Date::now();
                    view! {
                        <ol class="history-list">
                            {list.into_iter().enumerate().map(|(i, e)| {
                                let current = i == 0;
                                let (v1, v2, v3) = (e.version.clone(), e.version.clone(), e.version.clone());
                                let when = js_sys::Date::new(&e.created_at.into()).to_locale_string("default", &js_sys::Object::new());
                                let entry = e.clone();
                                view! {
                                    <li class="history-entry" class:is-selected=move || open.with(|o| o.as_ref().is_some_and(|(x, _)| *x == v1))>
                                        <button type="button" class="history-select" on:click=move |_| select(v2.clone())>
                                            <span class="history-summary">{e.summary.clone()}</span>
                                            <span class="history-meta small muted">
                                                {e.author.clone()}" · "<time title=String::from(when)>{fmt::ago(e.created_at, now)}</time>" · "<code>{e.version.clone()}</code>
                                                {current.then(|| view! { <span class="badge">"current"</span> })}
                                            </span>
                                        </button>
                                        {move || open.get().filter(|(x, _)| *x == v3).map(|(_, d)| match d {
                                            None => view! { <p class="muted small">"Comparing…"</p> }.into_any(),
                                            Some(Err(e)) => view! { <p class="err small">"Could not load this version: "{e}</p> }.into_any(),
                                            Some(Ok(d)) if d.is_empty() => view! { <p class="muted small history-diff">"Same as now."</p> }.into_any(),
                                            Some(Ok(d)) => view! {
                                                <div class="history-diff"><p class="muted small">"Restoring this version would change:"</p>
                                                    <ul>{d.lines().into_iter().map(|l| view! { <li>{l}</li> }).collect_view()}</ul></div>
                                            }.into_any(),
                                        })}
                                        <div class="row">
                                            <button type="button" class="btn sm" disabled=move || current || restoring.with(Option::is_some)
                                                title=current.then_some("This is the current version") on:click={ let entry = entry.clone(); move |_| restore(entry.clone()) }>
                                                {let v = entry.version.clone(); move || if restoring.get().as_deref() == Some(v.as_str()) { "Restoring…" } else { "Restore" }}
                                            </button>
                                        </div>
                                    </li>
                                }
                            }).collect_view()}
                        </ol>
                    }
                    .into_any()
                }
            }}
            {close_footer()}
        </div>
    }
}

/// The accounts: role, Enable or Disable, Delete, and Add a local user.
#[component]
pub fn UsersPanel() -> impl IntoView {
    let a = app();
    let users = RwSignal::new(None::<Vec<Value>>);
    let error = RwSignal::new(None::<String>);
    let name = RwSignal::new(String::new());
    let pass = RwSignal::new(String::new());
    let role = RwSignal::new("viewer".to_string());
    let load = move || {
        rt::spawn(async move {
            match api::get("/auth/users").await {
                Ok(v) => users.set(Some(v.as_array().cloned().unwrap_or_default())),
                Err(e) => error.set(Some(login_error(&e))),
            }
        })
    };
    load();
    let run = move |f: std::pin::Pin<Box<dyn std::future::Future<Output = api::ApiResult>>>| {
        error.set(None);
        rt::spawn(async move {
            match f.await {
                Ok(_) => load(),
                Err(e) => error.set(Some(login_error(&e))),
            }
        });
    };
    let me = move || a.me.with(|m| m.as_ref().and_then(|m| m.user.clone()));
    let add = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        let body = json!({ "username": name.get_untracked().trim(), "password": pass.get_untracked(), "role": role.get_untracked() });
        name.set(String::new());
        pass.set(String::new());
        run(Box::pin(
            async move { api::post("/auth/users", body).await },
        ));
    };
    view! {
        <div>
            <h2>"Users"</h2>
            {move || error.get().map(|e| view! { <div class="error-box" role="alert">{e}</div> })}
            {move || match users.get() {
                None => view! { <div class="empty"><span class="spinner"></span></div> }.into_any(),
                Some(list) => view! {
                    <ul class="user-list">
                        {list.into_iter().map(|u| {
                            let id = u.get("id").map(|v| v.to_string().trim_matches('"').to_string()).unwrap_or_default();
                            let uname = s(&u, "username");
                            let disabled = u.get("disabled").and_then(Value::as_bool).unwrap_or(false);
                            let you = me().as_deref() == Some(uname.as_str());
                            let (i1, i2, i3, n2) = (id.clone(), id.clone(), id.clone(), uname.clone());
                            view! {
                                <li class:is-disabled=disabled>
                                    <div class="row">
                                        <strong>{uname.clone()}</strong>
                                        {you.then(|| view! { <span class="badge">"you"</span> })}
                                        <span class="badge">{s(&u, "source")}</span>
                                        {disabled.then(|| view! { <span class="badge warn">"disabled"</span> })}
                                    </div>
                                    <div class="row">
                                        <select class="input" aria-label=format!("Role of {uname}") prop:value=s(&u, "role")
                                            on:change=move |e| { let r = event_target_value(&e); let p = format!("/auth/users/{i1}"); run(Box::pin(async move { api::patch(&p, json!({ "role": r })).await })) }>
                                            <option value="viewer">"Viewer"</option>
                                            <option value="admin">"Admin"</option>
                                        </select>
                                        <button type="button" class="btn sm" on:click=move |_| { let p = format!("/auth/users/{i2}"); run(Box::pin(async move { api::patch(&p, json!({ "disabled": !disabled })).await })) }>
                                            {if disabled { "Enable" } else { "Disable" }}</button>
                                        <button type="button" class="btn sm danger" aria-label=format!("Delete {uname}") on:click=move |_| {
                                            let (p, n) = (format!("/auth/users/{i3}"), n2.clone());
                                            rt::spawn(async move {
                                                if modal::confirm(crate::words::delete_user_question(&n)).await {
                                                    run(Box::pin(async move { api::del(&p).await }));
                                                }
                                            });
                                        }>"Delete"</button>
                                    </div>
                                </li>
                            }
                        }).collect_view()}
                    </ul>
                }.into_any(),
            }}
            <form class="user-add" on:submit=add>
                <h3>"Add a local user"</h3>
                <label class="field"><span>"Username"</span>
                    <input class="input" autocomplete="off" prop:value=move || name.get() on:input=move |e| name.set(event_target_value(&e))/></label>
                <label class="field"><span>"Password"</span>
                    <input class="input" type="password" autocomplete="new-password" prop:value=move || pass.get() on:input=move |e| pass.set(event_target_value(&e))/>
                    <small>{format!("At least {MIN_PASSWORD} characters.")}</small></label>
                <label class="field"><span>"Role"</span>
                    <select class="input" prop:value=move || role.get() on:change=move |e| role.set(event_target_value(&e))>
                        <option value="viewer">"Viewer (can only look)"</option>
                        <option value="admin">"Admin (can edit and scan)"</option>
                    </select></label>
                <button type="submit" class="btn primary" disabled=move || name.with(|n| n.trim().is_empty()) || pass.with(|p| p.chars().count() < MIN_PASSWORD)>"Add user"</button>
            </form>
            {close_footer()}
        </div>
    }
}

/// Change password, for accounts kept by homedit.
#[component]
pub fn PasswordForm() -> impl IntoView {
    let a = app();
    let (cur, new, conf) = (
        RwSignal::new(String::new()),
        RwSignal::new(String::new()),
        RwSignal::new(String::new()),
    );
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let ok = move || {
        !cur.with(String::is_empty)
            && new.with(|n| n.chars().count() >= MIN_PASSWORD)
            && new.get() == conf.get()
            && !busy.get()
    };
    let submit = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        if !ok() {
            return;
        }
        busy.set(true);
        error.set(None);
        rt::spawn(async move {
            match client::change_password(&cur.get_untracked(), &new.get_untracked()).await {
                Ok(()) => {
                    toasts::toast("Password changed", "ok");
                    a.target.set(None);
                }
                Err(e) => error.set(Some(login_error(&e))),
            }
            busy.set(false);
        });
    };
    view! {
        <form on:submit=submit>
            <h2>"Change password"</h2>
            {move || error.get().map(|e| view! { <div class="error-box" role="alert">{e}</div> })}
            <label class="field"><span>"Current password"</span>
                <input class="input" type="password" autocomplete="current-password" prop:value=move || cur.get() on:input=move |e| cur.set(event_target_value(&e))/></label>
            <label class="field"><span>"New password"</span>
                <input class="input" type="password" autocomplete="new-password" prop:value=move || new.get() on:input=move |e| new.set(event_target_value(&e))/>
                <small class:err=move || new.with(|n| !n.is_empty() && n.chars().count() < MIN_PASSWORD)>{format!("At least {MIN_PASSWORD} characters.")}</small></label>
            <label class="field"><span>"Confirm new password"</span>
                <input class="input" type="password" autocomplete="new-password" prop:value=move || conf.get() on:input=move |e| conf.set(event_target_value(&e))/>
                {move || (!conf.with(String::is_empty) && conf.get() != new.get()).then(|| view! { <small class="err">"Passwords don't match."</small> })}</label>
            <div class="drawer-footer row">
                <button type="submit" class="btn primary" disabled=move || !ok()>{move || if busy.get() { "Saving…" } else { "Change password" }}</button>
                <button type="button" class="btn" on:click=move |_| a.target.set(None)>"Close"</button>
            </div>
        </form>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Bookmark, BookmarkGroup, Service};

    #[test]
    fn diffs() {
        let svc = |id: &str, n: &str| Service {
            id: id.into(),
            name: n.into(),
            ..Service::default()
        };
        let a = Doc {
            groups: vec![Group {
                id: "g".into(),
                items: vec![svc("1", "a"), svc("2", "b")],
                ..Group::default()
            }],
            bookmarks: vec![BookmarkGroup {
                id: "l".into(),
                items: vec![Bookmark {
                    id: "k".into(),
                    ..Bookmark::default()
                }],
                ..BookmarkGroup::default()
            }],
            ..Doc::default()
        };
        assert!(Diff::between(&a, &a).is_empty());
        let mut b = a.clone();
        b.groups[0].items[0].name = "A".into();
        b.groups[0].items.remove(1);
        b.groups[0].items.push(svc("3", "c"));
        b.bookmarks.clear();
        b.settings.title = "x".into();
        let d = Diff::between(&a, &b);
        assert_eq!(
            d.services,
            Counts {
                added: 1,
                removed: 1,
                changed: 1
            }
        );
        assert_eq!(d.groups.changed, 1);
        assert_eq!(d.bookmarks.removed, 1);
        assert_eq!(
            d.lines(),
            [
                "Groups: 1 changed",
                "Cards: 1 added, 1 removed, 1 changed",
                "Bookmarks: 1 removed",
                "Appearance changed"
            ]
        );
    }
}
