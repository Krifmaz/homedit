//! The drawer's forms for cards, groups and bookmarks. Every change goes straight into the working copy (the
//! dashboard behind shows it live) under one key per form, so a whole form is one undo step and Cancel undoes
//! it.

use crate::dash::Icon;
use crate::model::{
    Bookmark, Check, Doc, Group, Layout, LayoutStyle, Service, WidgetConfig, is_secret_placeholder,
    new_id,
};
use crate::ops;
use crate::state::{App, app};
use kit_leptos::{api, rt};
use leptos::prelude::*;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

/// One form's edits: one undo step.
#[derive(Clone, Copy)]
pub struct Session {
    app: App,
    key: StoredValue<String>,
    changed: RwSignal<bool>,
}

impl Session {
    /// A new session.
    pub fn new() -> Session {
        Session {
            app: app(),
            key: StoredValue::new(new_id("form")),
            changed: RwSignal::new(false),
        }
    }

    /// Applies `f` to the working copy.
    pub fn edit(&self, f: impl FnOnce(&Doc) -> Doc) {
        self.changed.set(true);
        let k = self.key.get_value();
        self.app.edit(Some(&k), f);
    }

    /// Whether anything changed.
    pub fn touched(&self) -> bool {
        self.changed.get()
    }

    /// Keeps the changes and closes.
    pub fn done(&self) {
        self.app.target.set(None);
    }

    /// Undoes this form's changes and closes.
    pub fn cancel(&self) {
        if self.changed.get_untracked() {
            self.app.undo.update(|u| u.undo());
        }
        self.app.target.set(None);
    }
}

/// Whether `v` is an absolute http(s) URL.
pub fn is_http_url(v: &str) -> bool {
    let v = v.trim();
    (v.len() > 7 && v[..7].eq_ignore_ascii_case("http://")
        || v.len() > 8 && v[..8].eq_ignore_ascii_case("https://"))
        && !v.contains(char::is_whitespace)
}

/// `None` for an empty or blank text.
pub fn optional(v: String) -> Option<String> {
    if v.trim().is_empty() { None } else { Some(v) }
}

/// Every group with its path as the label (`Media / Downloads`), leaving out `exclude` and its sub-groups.
pub fn group_options(groups: &[Group], exclude: Option<&str>) -> Vec<(String, String)> {
    fn walk(list: &[Group], prefix: &str, exclude: Option<&str>, out: &mut Vec<(String, String)>) {
        for g in list {
            if Some(g.id.as_str()) == exclude {
                continue;
            }
            let name = if g.name.is_empty() {
                "(unnamed)"
            } else {
                &g.name
            };
            let label = if prefix.is_empty() {
                name.to_string()
            } else {
                format!("{prefix} / {name}")
            };
            out.push((g.id.clone(), label.clone()));
            walk(&g.groups, &label, exclude, out);
        }
    }
    let mut out = Vec::new();
    walk(groups, "", exclude, &mut out);
    out
}

/// A group's cards, its sub-groups' included.
pub fn count_cards(g: &Group) -> usize {
    g.items.len() + g.groups.iter().map(count_cards).sum::<usize>()
}

fn fstr(f: &Value, k: &str) -> String {
    f.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

fn missing(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => true,
        Some(Value::String(s)) => s.trim().is_empty(),
        _ => false,
    }
}

/// Checks a widget's values against its catalogue entry; errors by field key.
pub fn validate_widget(schema: &Value, values: &Map<String, Value>) -> BTreeMap<String, String> {
    let mut errors = BTreeMap::new();
    for f in schema
        .get("fields")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (key, label) = (fstr(f, "key"), fstr(f, "label"));
        let v = values.get(&key);
        if missing(v) {
            if f.get("required").and_then(Value::as_bool).unwrap_or(false) {
                errors.insert(key, format!("{label} is required"));
            }
            continue;
        }
        let v = v.expect("present");
        match fstr(f, "kind").as_str() {
            "url" if !v.as_str().is_some_and(is_http_url) => {
                errors.insert(key, format!("{label} must start with http:// or https://"));
            }
            "number"
                if !(v.as_f64().is_some()
                    || v.as_str().is_some_and(|s| s.trim().parse::<f64>().is_ok())) =>
            {
                errors.insert(key, format!("{label} must be a number"));
            }
            "select" => {
                let opts: Vec<String> = f
                    .get("options")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|o| o.as_str().map(str::to_owned))
                    .collect();
                let s = v
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| v.to_string());
                if !opts.is_empty() && !opts.contains(&s) {
                    errors.insert(key, format!("{label} must be one of {}", opts.join(", ")));
                }
            }
            _ => {}
        }
    }
    errors
}

/// A new widget's values: each field's default, false for switches, a required select's first option.
pub fn default_values(schema: &Value) -> Map<String, Value> {
    let mut out = Map::new();
    for f in schema
        .get("fields")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let v = if let Some(d) = f.get("default").filter(|d| !d.is_null()) {
            d.clone()
        } else if fstr(f, "kind") == "bool" {
            json!(false)
        } else if fstr(f, "kind") == "select"
            && f.get("required").and_then(Value::as_bool).unwrap_or(false)
        {
            f.get("options")
                .and_then(|o| o.get(0))
                .cloned()
                .unwrap_or(Value::Null)
        } else {
            Value::Null
        };
        out.insert(fstr(f, "key"), v);
    }
    out
}

/// Checks the parts of a card the form edits; errors by `name`, `href`, `expectStatus`, `widget.<key>`.
pub fn validate_service(s: &Service, catalog: &[Value]) -> BTreeMap<String, String> {
    let mut e = BTreeMap::new();
    if s.name.trim().is_empty() {
        e.insert("name".into(), "Name is required".into());
    }
    if let Some(h) = &s.href
        && !is_http_url(h)
        && !h.starts_with('/')
    {
        e.insert(
            "href".into(),
            "Link must start with http://, https:// or /".into(),
        );
    }
    if let Some(c) = s.check.as_ref().and_then(|c| c.expect_status)
        && !(100..=599).contains(&c)
    {
        e.insert(
            "expectStatus".into(),
            "Expected status must be a number from 100 to 599".into(),
        );
    }
    if let Some(w) = &s.widget
        && let Some(schema) = catalog.iter().find(|c| fstr(c, "type") == w.kind)
    {
        for (k, v) in validate_widget(schema, &w.fields) {
            e.insert(format!("widget.{k}"), v);
        }
    }
    e
}

/// Checks a bookmark: a name, an http(s) link, at most two letters.
pub fn validate_bookmark(b: &Bookmark) -> BTreeMap<String, String> {
    let mut e = BTreeMap::new();
    if b.name.trim().is_empty() {
        e.insert("name".into(), "Name is required".into());
    }
    if b.href.trim().is_empty() {
        e.insert("href".into(), "Link is required".into());
    } else if !is_http_url(&b.href) {
        e.insert(
            "href".into(),
            "Link must start with http:// or https://".into(),
        );
    }
    if b.abbr.as_ref().is_some_and(|a| a.chars().count() > 2) {
        e.insert("abbr".into(), "At most 2 characters".into());
    }
    e
}

/// The catalogue by category, both sorted.
fn by_category(catalog: &[Value]) -> Vec<(String, Vec<(String, String)>)> {
    let mut m: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for w in catalog {
        m.entry(fstr(w, "category"))
            .or_default()
            .push((fstr(w, "type"), fstr(w, "name")));
    }
    m.into_iter()
        .map(|(c, mut l)| {
            l.sort_by_key(|a| a.1.to_lowercase());
            (c, l)
        })
        .collect()
}

/// A field's error, shown once the form was touched.
fn error_line(
    errors: Memo<BTreeMap<String, String>>,
    shown: Signal<bool>,
    key: &'static str,
) -> impl IntoView {
    move || {
        shown
            .get()
            .then(|| errors.with(|e| e.get(key).cloned()))
            .flatten()
            .map(|m| view! { <small class="err" role="alert">{m}</small> })
    }
}

/// The form's buttons: Done (when valid), Cancel, and Delete for an existing item.
fn footer(s: Session, valid: Signal<bool>, delete: Option<Callback<()>>) -> impl IntoView {
    view! {
        <div class="drawer-footer row">
            <button type="submit" class="btn primary" disabled=move || !valid.get()>"Done"</button>
            <button type="button" class="btn" on:click=move |_| s.cancel()>"Cancel"</button>
            {delete.map(|d| view! { <button type="button" class="btn danger" on:click=move |_| d.run(())>"Delete"</button> })}
        </div>
    }
}

/// An icon reference with a preview, a search over the icon sets and uploaded icons, and Upload.
#[component]
pub fn IconPicker(
    #[prop(into)] value: Signal<Option<String>>,
    #[prop(into)] name: Signal<String>,
    on_change: Callback<Option<String>>,
) -> impl IntoView {
    let query = RwSignal::new(String::new());
    let results = RwSignal::new(Vec::<Value>::new());
    let answered = RwSignal::new(String::new());
    let searching = RwSignal::new(false);
    let uploading = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let file = NodeRef::<leptos::html::Input>::new();
    let seq = StoredValue::new(0u32);
    let search = move |q: String| {
        query.set(q.clone());
        seq.update_value(|n| *n += 1);
        let mine = seq.get_value();
        let q = q.trim().to_string();
        if q.is_empty() {
            results.set(vec![]);
            searching.set(false);
            return;
        }
        rt::spawn(async move {
            rt::sleep(250).await;
            if seq.get_value() != mine {
                return;
            }
            searching.set(true);
            let r = api::get(&format!("/icons?q={}", crate::model::js_encode(&q))).await;
            if seq.get_value() != mine {
                return;
            }
            searching.set(false);
            match r {
                Ok(v) => {
                    results.set(v.as_array().cloned().unwrap_or_default());
                    answered.set(q);
                    error.set(None);
                }
                Err(e) => {
                    results.set(vec![]);
                    error.set(Some(e.message));
                }
            }
        });
    };
    let upload = move |_| {
        let Some(input) = file.get_untracked() else {
            return;
        };
        let Some(f) = input.files().and_then(|l| l.get(0)) else {
            return;
        };
        uploading.set(true);
        rt::spawn(async move {
            let Ok(form) = web_sys::FormData::new() else {
                return;
            };
            let _ = form.append_with_blob("file", &f);
            match api::post_form("/icons", form).await {
                Ok(v) => {
                    on_change.run(v.get("name").and_then(Value::as_str).map(str::to_owned));
                    error.set(None);
                }
                Err(e) => error.set(Some(e.message)),
            }
            uploading.set(false);
            input.set_value("");
        });
    };
    view! {
        <div class="field icon-picker">
            <span>"Icon"</span>
            <div class="row">
                <span class="icon-picker-preview">
                    {move || view! { <Icon icon=value.get() name=name.get() size=32/> }}
                </span>
                <input class="input grow" type="text" spellcheck="false" placeholder="sonarr.png, sh-plex, mdi-server or a URL"
                    prop:value=move || value.get().unwrap_or_default()
                    on:input=move |e| on_change.run(optional(event_target_value(&e)))/>
            </div>
            <small>"A dashboard-icons name, a selfh.st name (sh-…), mdi-… or si-…, an uploaded icon, or a URL."</small>
            <div class="row">
                <input class="input grow" type="search" placeholder="Search icons…" aria-label="Search icons"
                    prop:value=move || query.get() on:input=move |e| search(event_target_value(&e))/>
                <input type="file" accept="image/*" hidden node_ref=file on:change=upload aria-label="Upload icon file"/>
                <button type="button" class="btn" disabled=move || uploading.get()
                    on:click=move |_| { if let Some(i) = file.get_untracked() { i.click(); } }>
                    {move || if uploading.get() { "Uploading…" } else { "Upload" }}
                </button>
            </div>
            {move || error.get().map(|e| view! { <small class="err" role="alert">{e}</small> })}
            {move || searching.get().then(|| view! { <small>"Searching…"</small> })}
            {move || {
                let r = results.get();
                (!r.is_empty()).then(|| view! {
                    <div class="icon-grid" role="group" aria-label="Icon search results">
                        {r.into_iter().map(|x| {
                            let n = fstr(&x, "name");
                            let n2 = n.clone();
                            let on = move || value.get().as_deref() == Some(n2.as_str());
                            view! {
                                <button type="button" class="btn icon-choice" class:selected=on.clone() aria-pressed=move || on().to_string()
                                    title=format!("{n} ({})", fstr(&x, "source")) aria-label=format!("Use icon {n}")
                                    on:click={ let n = n.clone(); move |_| on_change.run(Some(n.clone())) }>
                                    <img src=fstr(&x, "url") alt="" width="28" height="28" loading="lazy"/>
                                </button>
                            }
                        }).collect_view()}
                    </div>
                })
            }}
            {move || (!searching.get() && !query.with(|q| q.trim().is_empty()) && answered.get() == query.get().trim() && results.with(Vec::is_empty))
                .then(|| view! { <small>"No icons found."</small> })}
        </div>
    }
}

/// The inputs of a widget's catalogue entry. A stored secret shows as dots with Replace and Clear; while it is
/// being replaced, Keep puts it back.
#[component]
fn WidgetFields(
    schema: Value,
    #[prop(into)] values: Signal<Map<String, Value>>,
    errors: Memo<BTreeMap<String, String>>,
    #[prop(into)] shown: Signal<bool>,
    on_change: Callback<Map<String, Value>>,
) -> impl IntoView {
    let replacing = RwSignal::new(Vec::<String>::new());
    let fields = schema
        .get("fields")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let set = move |k: String, v: Value| {
        let mut m = values.get_untracked();
        m.insert(k, v);
        on_change.run(m);
    };
    fields
        .into_iter()
        .map(|f| {
            let key = fstr(&f, "key");
            let label = fstr(&f, "label");
            let kind = fstr(&f, "kind");
            let required = f.get("required").and_then(Value::as_bool).unwrap_or(false);
            let help = fstr(&f, "help");
            let k1 = key.clone();
            let val = Memo::new(move |_| values.with(|m| m.get(&k1).cloned().unwrap_or(Value::Null)));
            let ek = format!("widget.{key}");
            let err = move || shown.get().then(|| errors.with(|e| e.get(&ek).cloned())).flatten().map(|m| view! { <small class="err" role="alert">{m}</small> });
            let help = (!help.is_empty()).then(|| view! { <small>{help}</small> });
            let lbl = if required { format!("{label} *") } else { label.clone() };
            let input = match kind.as_str() {
                "bool" => {
                    let k = key.clone();
                    return view! {
                        <div class="field">
                            <label class="row"><span class="switch"><input type="checkbox" prop:checked=move || val.get() == json!(true)
                                on:change=move |e| set(k.clone(), json!(event_target_checked(&e)))/><i></i></span><span>{lbl}</span></label>
                            {help}{err}
                        </div>
                    }
                    .into_any();
                }
                "number" => {
                    let k = key.clone();
                    view! { <input class="input" type="number" prop:value=move || match val.get() { Value::Null => String::new(), Value::String(s) => s, v => v.to_string() }
                        on:input=move |e| {
                            let raw = event_target_value(&e);
                            let v = if raw.is_empty() { Value::Null } else { raw.parse::<f64>().ok().and_then(|n| serde_json::Number::from_f64(n).map(Value::Number)).unwrap_or(Value::String(raw)) };
                            set(k.clone(), v)
                        }/> }
                    .into_any()
                }
                "select" => {
                    let k = key.clone();
                    let opts: Vec<String> = f.get("options").and_then(Value::as_array).into_iter().flatten().filter_map(|o| o.as_str().map(str::to_owned)).collect();
                    view! {
                        <select class="input" prop:value=move || val.get().as_str().unwrap_or("").to_string()
                            on:change=move |e| { let v = event_target_value(&e); set(k.clone(), if v.is_empty() { Value::Null } else { json!(v) }) }>
                            {(!required).then(|| view! { <option value="">"(none)"</option> })}
                            {required.then(|| view! { <option value="" disabled>"Choose…"</option> })}
                            {opts.into_iter().map(|o| view! { <option value=o.clone()>{o.clone()}</option> }).collect_view()}
                        </select>
                    }
                    .into_any()
                }
                "secret" => {
                    let (k, k2, k3, k4) = (key.clone(), key.clone(), key.clone(), key.clone());
                    let stored = Memo::new(move |_| is_secret_placeholder(&val.get()));
                    view! {
                        {move || if stored.get() {
                            let (k, k2) = (k.clone(), k2.clone());
                            view! {
                                <div class="row">
                                    <input class="input grow" type="password" value="" placeholder="••••••••" readonly/>
                                    <button type="button" class="btn" on:click=move |_| { replacing.update(|r| r.push(k.clone())); set(k.clone(), json!("")); }>"Replace"</button>
                                    <button type="button" class="btn danger" on:click=move |_| set(k2.clone(), Value::Null)>"Clear"</button>
                                </div>
                            }.into_any()
                        } else {
                            let (k3, k4) = (k3.clone(), k4.clone());
                            let k5 = k4.clone();
                            view! {
                                <div class="row">
                                    <input class="input grow" type="password" autocomplete="new-password" prop:value=move || val.get().as_str().unwrap_or("").to_string()
                                        on:input=move |e| set(k3.clone(), json!(event_target_value(&e)))/>
                                    {move || replacing.with(|r| r.contains(&k5)).then(|| {
                                        let k = k4.clone();
                                        view! { <button type="button" class="btn" on:click=move |_| {
                                            replacing.update(|r| r.retain(|x| *x != k));
                                            set(k.clone(), json!({ "$secret": "set" }));
                                        }>"Keep"</button> }
                                    })}
                                </div>
                            }.into_any()
                        }}
                    }
                    .into_any()
                }
                other => {
                    let k = key.clone();
                    let t = if other == "url" { "url" } else { "text" };
                    view! { <input class="input" type=t spellcheck="false"
                        prop:value=move || match val.get() { Value::String(s) => s, Value::Number(n) => n.to_string(), _ => String::new() }
                        on:input=move |e| { let v = event_target_value(&e); set(k.clone(), if v.is_empty() { Value::Null } else { json!(v) }) }/> }
                    .into_any()
                }
            };
            view! { <label class="field"><span>{lbl}</span>{input}{help}{err}</label> }.into_any()
        })
        .collect_view()
}

/// Adds or edits a card: details, icon, group, status check and widget (with Test connection).
#[component]
pub fn ServiceForm(group: String, service: Option<String>) -> impl IntoView {
    let a = app();
    let s = Session::new();
    let editing_existing = service.is_some();
    let id = service.clone().unwrap_or_else(|| new_id("s"));
    let start_group = group.clone();
    let doc = Memo::new(move |_| a.undo.with(|u| u.present.clone()));
    let idc = id.clone();
    let svc = Memo::new(move |_| {
        doc.with(|d| ops::find_service(d, &idc).cloned())
            .unwrap_or_else(|| Service {
                id: idc.clone(),
                ..Service::default()
            })
    });
    let idc = id.clone();
    let exists = Memo::new(move |_| doc.with(|d| ops::find_service(d, &idc).is_some()));
    let idc = id.clone();
    let cur_group = Memo::new(move |_| {
        doc.with(|d| ops::find_service_group(d, &idc).map(|g| g.id.clone()))
            .unwrap_or_else(|| start_group.clone())
    });
    let errors = Memo::new(move |_| a.catalog.with(|c| validate_service(&svc.get(), c)));
    let shown = Signal::derive(move || editing_existing || s.touched());
    let valid = Signal::derive(move || errors.with(BTreeMap::is_empty));
    let change = move |f: Box<dyn FnOnce(&mut Service)>, to: Option<String>| {
        let mut next = svc.get_untracked();
        f(&mut next);
        let g = to.unwrap_or_else(|| cur_group.get_untracked());
        s.edit(|d| ops::upsert_service(d, &g, next));
    };
    let test = RwSignal::new(None::<(bool, String)>);
    let testing = RwSignal::new(false);
    let has_check = Memo::new(move |_| svc.with(|s| s.check.as_ref().map(|c| c.kind.clone())));
    let wtype = Memo::new(move |_| svc.with(|s| s.widget.as_ref().map(|w| w.kind.clone())));
    let schema = Memo::new(move |_| {
        wtype.get().and_then(|t| {
            a.catalog
                .with(|c| c.iter().find(|w| fstr(w, "type") == t).cloned())
        })
    });
    let service_id = service.clone();
    let run_test = move |_| {
        let Some(w) = svc.get_untracked().widget else {
            return;
        };
        testing.set(true);
        test.set(None);
        let sid = service_id.clone();
        rt::spawn(async move {
            let body = json!({ "type": w.kind, "fields": w.fields, "serviceId": sid.as_deref().map(crate::model::to_wire_id) });
            match api::post("/widgets/test", body).await {
                Ok(v) => test.set(Some((
                    v.get("ok").and_then(Value::as_bool).unwrap_or(false),
                    fstr(&v, "message"),
                ))),
                Err(e) => test.set(Some((false, e.message))),
            }
            testing.set(false);
        });
    };
    let delete = editing_existing.then(|| {
        let id = id.clone();
        Callback::new(move |_| {
            let id = id.clone();
            let name = svc.get_untracked().name;
            s.edit(|d| ops::delete_service(d, &id));
            s.done();
            kit_leptos::toasts::toast_action(
                &format!("{} deleted", if name.is_empty() { "Card" } else { &name }),
                "Undo",
                6000,
                move || a.undo.update(|u| u.undo()),
            );
        })
    });
    let delete = delete.filter(|_| exists.get_untracked());
    view! {
        <form class="editor-form" on:submit=move |e| { e.prevent_default(); if valid.get_untracked() { s.done(); } }>
            <h2>{if editing_existing { "Edit service" } else { "Add service" }}</h2>
            <label class="field"><span>"Name *"</span>
                <input class="input" type="text" autofocus prop:value=move || svc.get().name
                    on:input=move |e| { let v = event_target_value(&e); change(Box::new(move |s| s.name = v), None) }/>
                {error_line(errors, shown, "name")}
            </label>
            <label class="field"><span>"Link"</span>
                <input class="input" type="url" inputmode="url" spellcheck="false" placeholder="http://sonarr.lan:8989"
                    prop:value=move || svc.get().href.unwrap_or_default()
                    on:input=move |e| { let v = optional(event_target_value(&e)); change(Box::new(move |s| s.href = v), None) }/>
                {error_line(errors, shown, "href")}
            </label>
            <label class="field"><span>"Description"</span>
                <input class="input" type="text" prop:value=move || svc.get().description.unwrap_or_default()
                    on:input=move |e| { let v = optional(event_target_value(&e)); change(Box::new(move |s| s.description = v), None) }/>
            </label>
            <IconPicker value=Signal::derive(move || svc.get().icon) name=Signal::derive(move || svc.get().name)
                on_change=Callback::new(move |v: Option<String>| change(Box::new(move |s| { s.icon = v; s.icon_mask = false; }), None))/>
            <label class="field"><span>"Group"</span>
                <select class="input" prop:value=move || cur_group.get() on:change=move |e| change(Box::new(|_| {}), Some(event_target_value(&e)))>
                    {move || doc.with(|d| group_options(&d.groups, None)).into_iter().map(|(id, label)| view! { <option value=id>{label}</option> }).collect_view()}
                </select>
            </label>
            <fieldset class="form-section">
                <legend>"Status check"</legend>
                <label class="field"><span>"Check"</span>
                    <select class="input" prop:value=move || has_check.get().unwrap_or_default() on:change=move |e| {
                        let k = event_target_value(&e);
                        change(Box::new(move |s| {
                            s.check = if k.is_empty() { None } else {
                                let old = s.check.take();
                                Some(Check {
                                    expect_status: if k == "http" { old.as_ref().and_then(|c| c.expect_status) } else { None },
                                    target: old.and_then(|c| c.target),
                                    kind: k,
                                })
                            };
                        }), None)
                    }>
                        <option value="">"None"</option>
                        <option value="http">"HTTP"</option>
                        <option value="tcp">"TCP connect"</option>
                        <option value="ping">"Ping"</option>
                    </select>
                </label>
                {move || has_check.get().map(|kind| {
                    let http = kind == "http";
                    view! {
                        <label class="field"><span>"Target"</span>
                            <input class="input" type="text" spellcheck="false"
                                placeholder=move || if http { svc.get().href.unwrap_or_else(|| "http://host:port/health".into()) } else { "host:port".into() }
                                prop:value=move || svc.get().check.and_then(|c| c.target).unwrap_or_default()
                                on:input=move |e| { let v = optional(event_target_value(&e)); change(Box::new(move |s| if let Some(c) = &mut s.check { c.target = v; }), None) }/>
                            <small>{if http { "URL to request; the link when empty." } else { "Host (and port for TCP); the link's host when empty." }}</small>
                        </label>
                        {http.then(|| view! {
                            <label class="field"><span>"Expected status"</span>
                                <input class="input" type="number" min="100" max="599" placeholder="any 2xx/3xx"
                                    prop:value=move || svc.get().check.and_then(|c| c.expect_status).map(|c| c.to_string()).unwrap_or_default()
                                    on:input=move |e| {
                                        let raw = event_target_value(&e);
                                        let v = if raw.is_empty() { None } else { Some(raw.parse::<u16>().unwrap_or(0)) };
                                        change(Box::new(move |s| if let Some(c) = &mut s.check { c.expect_status = v; }), None)
                                    }/>
                                {error_line(errors, shown, "expectStatus")}
                            </label>
                        })}
                    }
                })}
            </fieldset>
            <fieldset class="form-section">
                <legend>"Widget"</legend>
                <label class="field"><span>"Widget type"</span>
                    <select class="input" prop:value=move || wtype.get().unwrap_or_default() on:change=move |e| {
                        test.set(None);
                        let t = event_target_value(&e);
                        let ws = a.catalog.with_untracked(|c| c.iter().find(|w| fstr(w, "type") == t).cloned());
                        change(Box::new(move |s| {
                            if t.is_empty() {
                                s.widget = None;
                                return;
                            }
                            let mut fields = ws.as_ref().map(default_values).unwrap_or_default();
                            if fields.contains_key("url") && fields["url"].is_null() && let Some(h) = &s.href {
                                fields.insert("url".into(), json!(h));
                            }
                            s.widget = Some(WidgetConfig { kind: t, fields });
                        }), None)
                    }>
                        <option value="">"None"</option>
                        {move || (wtype.get().is_some() && schema.with(Option::is_none)).then(|| { let t = wtype.get().unwrap_or_default(); view! { <option value=t.clone()>{t.clone()}</option> } })}
                        {move || a.catalog.with(|c| by_category(c)).into_iter().map(|(cat, list)| view! {
                            <optgroup label=cat>{list.into_iter().map(|(t, n)| view! { <option value=t>{n}</option> }).collect_view()}</optgroup>
                        }).collect_view()}
                    </select>
                    {move || (wtype.get().is_some() && schema.with(Option::is_none)).then(|| view! { <small>"This widget type is not in the catalogue; its settings are kept as they are."</small> })}
                </label>
                {move || schema.get().map(|sc| view! {
                    <WidgetFields schema=sc values=Signal::derive(move || svc.get().widget.map(|w| w.fields).unwrap_or_default())
                        errors=errors shown=shown
                        on_change=Callback::new(move |fields: Map<String, Value>| {
                            test.set(None);
                            change(Box::new(move |s| if let Some(w) = &mut s.widget { w.fields = fields; }), None)
                        })/>
                    <div class="row">
                        <button type="button" class="btn" disabled=move || testing.get() on:click=run_test.clone()>
                            {move || if testing.get() { "Testing…" } else { "Test connection" }}
                        </button>
                    </div>
                    {move || test.get().map(|(ok, m)| view! {
                        <p class="test-result" class:ok=ok class:err=!ok role="status">{if ok { "✓ " } else { "✗ " }}{m}</p>
                    })}
                })}
            </fieldset>
            {footer(s, valid, delete)}
        </form>
    }
}

/// Adds or edits a group: name, visibility, layout and, for a new one, its parent.
#[component]
pub fn GroupForm(group: Option<String>) -> impl IntoView {
    let a = app();
    let s = Session::new();
    let existing_id = group.is_some();
    let id = group.unwrap_or_else(|| new_id("g"));
    let parent = RwSignal::new(String::new());
    let doc = Memo::new(move |_| a.undo.with(|u| u.present.clone()));
    let idc = id.clone();
    let head = Memo::new(move |_| {
        doc.with(|d| {
            ops::find_group(d, &idc).map(|g| Group {
                items: vec![],
                groups: vec![],
                ..g.clone()
            })
        })
        .unwrap_or_else(|| Group {
            id: idc.clone(),
            ..Group::default()
        })
    });
    let name_error = Memo::new(move |_| head.with(|h| h.name.trim().is_empty()));
    let shown = Signal::derive(move || existing_id || s.touched());
    let idc = id.clone();
    let change = move |f: Box<dyn FnOnce(&mut Group)>, to_parent: Option<String>| {
        let mut next = head.get_untracked();
        f(&mut next);
        let id = idc.clone();
        let p = to_parent.clone().unwrap_or_else(|| parent.get_untracked());
        s.edit(|d| {
            let exists = ops::find_group(d, &id).is_some();
            // A new group can still move to another parent: take it out and put it in again.
            if !existing_id && exists && to_parent.is_some() {
                let items = ops::find_group(d, &id)
                    .map(|g| g.items.clone())
                    .unwrap_or_default();
                let base = ops::delete_group(d, &id);
                ops::upsert_group(
                    &base,
                    Group { items, ..next },
                    (!p.is_empty()).then_some(p.as_str()),
                )
            } else {
                ops::upsert_group(d, next, (!p.is_empty()).then_some(p.as_str()))
            }
        });
    };
    let idc = id.clone();
    let delete = existing_id.then(|| {
        Callback::new(move |_| {
            let Some(g) = doc.with_untracked(|d| ops::find_group(d, &idc).cloned()) else {
                return;
            };
            let id = idc.clone();
            rt::spawn(async move {
                if kit_leptos::modal::confirm(crate::words::delete_group_question(
                    &g.name,
                    count_cards(&g),
                ))
                .await
                {
                    s.edit(|d| ops::delete_group(d, &id));
                    s.done();
                }
            });
        })
    });
    let idx = id.clone();
    let change2 = change.clone();
    let change3 = change.clone();
    let change4 = change.clone();
    let change5 = change.clone();
    view! {
        <form class="editor-form" on:submit=move |e| { e.prevent_default(); if !name_error.get_untracked() { s.done(); } }>
            <h2>{if existing_id { "Edit group" } else { "Add group" }}</h2>
            <label class="field"><span>"Name *"</span>
                <input class="input" type="text" autofocus prop:value=move || head.get().name
                    on:input=move |e| { let v = event_target_value(&e); change(Box::new(move |g| g.name = v), None) }/>
                {move || (shown.get() && name_error.get()).then(|| view! { <small class="err" role="alert">"Name is required"</small> })}
            </label>
            {(!existing_id).then(|| view! {
                <label class="field"><span>"Parent group"</span>
                    <select class="input" prop:value=move || parent.get() on:change=move |e| {
                        let p = event_target_value(&e);
                        parent.set(p.clone());
                        if s.touched() { change2(Box::new(|_| {}), Some(p)); }
                    }>
                        <option value="">"Top level"</option>
                        {move || doc.with(|d| group_options(&d.groups, Some(&idx))).into_iter().map(|(id, l)| view! { <option value=id>{l}</option> }).collect_view()}
                    </select>
                </label>
            })}
            <div class="field">
                <label class="row"><span class="switch"><input type="checkbox" prop:checked=move || head.get().hidden
                    on:change=move |e| { let v = event_target_checked(&e); change3(Box::new(move |g| g.hidden = v), None) }/><i></i></span>
                    <span>"Hidden from viewers"</span></label>
                <small>"Only admins see this group (dimmed)."</small>
            </div>
            <label class="field"><span>"Columns"</span>
                <input class="input" type="number" min="1" max="8" step="1" prop:value=move || head.get().layout.columns.to_string()
                    on:input=move |e| {
                        let Ok(n) = event_target_value(&e).parse::<f64>() else { return };
                        let n = n.round().clamp(1.0, 8.0) as u32;
                        change4(Box::new(move |g| g.layout.columns = n), None)
                    }/>
            </label>
            <label class="field"><span>"Layout"</span>
                <select class="input" prop:value=move || head.get().layout.style.as_str().to_string()
                    on:change=move |e| { let st = LayoutStyle::parse(&event_target_value(&e)); change5(Box::new(move |g| g.layout = Layout { style: st, ..g.layout }), None) }>
                    <option value="grid">"Grid"</option>
                    <option value="row">"Single row"</option>
                    <option value="column">"Single column"</option>
                </select>
            </label>
            {footer(s, Signal::derive(move || !name_error.get()), delete)}
        </form>
    }
}

/// Adds or renames a bookmark list.
#[component]
pub fn BookmarkGroupForm(group: Option<String>) -> impl IntoView {
    let a = app();
    let s = Session::new();
    let existing_id = group.is_some();
    let id = group.unwrap_or_else(|| new_id("bg"));
    let idc = id.clone();
    let list = Memo::new(move |_| {
        a.undo
            .with(|u| u.present.bookmarks.iter().find(|b| b.id == idc).cloned())
    });
    let error =
        Memo::new(move |_| list.with(|l| l.as_ref().is_none_or(|l| l.name.trim().is_empty())));
    let shown = Signal::derive(move || existing_id || s.touched());
    let idc = id.clone();
    let delete = existing_id.then(|| {
        Callback::new(move |_| {
            let Some(l) = list.get_untracked() else {
                return;
            };
            let id = idc.clone();
            rt::spawn(async move {
                if kit_leptos::modal::confirm(crate::words::delete_list_question(&l.name)).await {
                    s.edit(|d| ops::delete_bookmark_group(d, &id));
                    s.done();
                }
            });
        })
    });
    view! {
        <form class="editor-form" on:submit=move |e| { e.prevent_default(); if !error.get_untracked() { s.done(); } }>
            <h2>{if existing_id { "Edit bookmark list" } else { "Add bookmark list" }}</h2>
            <label class="field"><span>"Name *"</span>
                <input class="input" type="text" autofocus prop:value=move || list.get().map(|l| l.name).unwrap_or_default()
                    on:input=move |e| { let v = event_target_value(&e); let id = id.clone(); s.edit(|d| ops::upsert_bookmark_group(d, &id, &v)) }/>
                {move || (shown.get() && error.get()).then(|| view! { <small class="err" role="alert">"Name is required"</small> })}
            </label>
            {footer(s, Signal::derive(move || !error.get()), delete)}
        </form>
    }
}

/// Adds or edits a bookmark: name, link, letters, icon and list.
#[component]
pub fn BookmarkForm(group: String, bookmark: Option<String>) -> impl IntoView {
    let a = app();
    let s = Session::new();
    let existing_id = bookmark.is_some();
    let id = bookmark.unwrap_or_else(|| new_id("b"));
    let doc = Memo::new(move |_| a.undo.with(|u| u.present.clone()));
    let idc = id.clone();
    let start = group.clone();
    let holder = Memo::new(move |_| {
        doc.with(|d| {
            d.bookmarks
                .iter()
                .find(|g| g.items.iter().any(|b| b.id == idc))
                .map(|g| g.id.clone())
        })
        .unwrap_or_else(|| start.clone())
    });
    let idc = id.clone();
    let bm = Memo::new(move |_| {
        doc.with(|d| {
            d.bookmarks
                .iter()
                .flat_map(|g| g.items.iter())
                .find(|b| b.id == idc)
                .cloned()
        })
        .unwrap_or_else(|| Bookmark {
            id: idc.clone(),
            ..Bookmark::default()
        })
    });
    let errors = Memo::new(move |_| validate_bookmark(&bm.get()));
    let shown = Signal::derive(move || existing_id || s.touched());
    let change = move |f: Box<dyn FnOnce(&mut Bookmark)>, to: Option<String>| {
        let mut next = bm.get_untracked();
        f(&mut next);
        let g = to.unwrap_or_else(|| holder.get_untracked());
        s.edit(|d| ops::upsert_bookmark(d, &g, next));
    };
    let idc = id.clone();
    let delete = existing_id.then(|| {
        Callback::new(move |_| {
            let id = idc.clone();
            let name = bm.get_untracked().name;
            s.edit(|d| ops::delete_bookmark(d, &id));
            s.done();
            kit_leptos::toasts::toast_action(&format!("{name} deleted"), "Undo", 6000, move || {
                a.undo.update(|u| u.undo())
            });
        })
    });
    view! {
        <form class="editor-form" on:submit=move |e| { e.prevent_default(); if errors.with_untracked(BTreeMap::is_empty) { s.done(); } }>
            <h2>{if existing_id { "Edit bookmark" } else { "Add bookmark" }}</h2>
            <label class="field"><span>"Name *"</span>
                <input class="input" type="text" autofocus prop:value=move || bm.get().name
                    on:input=move |e| { let v = event_target_value(&e); change(Box::new(move |b| b.name = v), None) }/>
                {error_line(errors, shown, "name")}
            </label>
            <label class="field"><span>"Link *"</span>
                <input class="input" type="url" inputmode="url" spellcheck="false" placeholder="https://example.com" prop:value=move || bm.get().href
                    on:input=move |e| { let v = event_target_value(&e); change(Box::new(move |b| b.href = v), None) }/>
                {error_line(errors, shown, "href")}
            </label>
            <label class="field"><span>"Letters"</span>
                <input class="input" type="text" maxlength="2" size="3" prop:value=move || bm.get().abbr.unwrap_or_default()
                    on:input=move |e| { let v = optional(event_target_value(&e)); change(Box::new(move |b| b.abbr = v), None) }/>
                <small>"Up to 2 characters, shown when there is no icon."</small>
                {error_line(errors, shown, "abbr")}
            </label>
            <IconPicker value=Signal::derive(move || bm.get().icon) name=Signal::derive(move || bm.get().name)
                on_change=Callback::new(move |v: Option<String>| change(Box::new(move |b| { b.icon = v; b.icon_mask = false; }), None))/>
            {move || (doc.with(|d| d.bookmarks.len()) > 1).then(|| view! {
                <label class="field"><span>"Bookmark list"</span>
                    <select class="input" prop:value=move || holder.get() on:change=move |e| change(Box::new(|_| {}), Some(event_target_value(&e)))>
                        {move || doc.with(|d| d.bookmarks.iter().map(|g| (g.id.clone(), if g.name.is_empty() { "(unnamed)".to_string() } else { g.name.clone() })).collect::<Vec<_>>())
                            .into_iter().map(|(id, n)| view! { <option value=id>{n}</option> }).collect_view()}
                    </select>
                </label>
            })}
            {footer(s, Signal::derive(move || errors.with(BTreeMap::is_empty)), delete)}
        </form>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> Value {
        json!({ "type": "sonarr", "name": "Sonarr", "category": "Media", "fields": [
            { "key": "url", "label": "URL", "kind": "url", "required": true },
            { "key": "key", "label": "API key", "kind": "secret", "required": true },
            { "key": "n", "label": "Limit", "kind": "number", "required": false, "default": 5 },
            { "key": "mode", "label": "Mode", "kind": "select", "required": true, "options": ["a", "b"] },
            { "key": "on", "label": "On", "kind": "bool", "required": false }
        ]})
    }

    #[test]
    fn widget_rules() {
        let d = default_values(&schema());
        assert_eq!(d["n"], json!(5));
        assert_eq!(d["mode"], json!("a"));
        assert_eq!(d["on"], json!(false));
        assert_eq!(d["url"], Value::Null);
        let e = validate_widget(&schema(), &d);
        assert_eq!(e.keys().collect::<Vec<_>>(), ["key", "url"]);
        let mut v = d.clone();
        v.insert("url".into(), json!("ftp://x"));
        v.insert("key".into(), json!({ "$secret": "set" }));
        v.insert("n".into(), json!("x"));
        v.insert("mode".into(), json!("c"));
        let e = validate_widget(&schema(), &v);
        assert_eq!(e["url"], "URL must start with http:// or https://");
        assert_eq!(e["n"], "Limit must be a number");
        assert_eq!(e["mode"], "Mode must be one of a, b");
        assert!(!e.contains_key("key"));
    }

    #[test]
    fn service_and_bookmark_rules() {
        let mut s = Service {
            name: " ".into(),
            href: Some("sonarr".into()),
            ..Service::default()
        };
        s.check = Some(Check {
            kind: "http".into(),
            target: None,
            expect_status: Some(42),
        });
        s.widget = Some(WidgetConfig {
            kind: "sonarr".into(),
            fields: Map::new(),
        });
        let e = validate_service(&s, &[schema()]);
        assert!(
            e.contains_key("name")
                && e.contains_key("href")
                && e.contains_key("expectStatus")
                && e.contains_key("widget.url")
        );
        s.href = Some("/local".into());
        assert!(!validate_service(&s, &[]).contains_key("href"));
        let b = Bookmark {
            name: "x".into(),
            href: "https://a.b".into(),
            abbr: Some("abc".into()),
            ..Bookmark::default()
        };
        assert_eq!(validate_bookmark(&b).keys().collect::<Vec<_>>(), ["abbr"]);
        assert!(
            is_http_url("HTTPS://x.y") && !is_http_url("http://a b") && !is_http_url("http://")
        );
    }

    #[test]
    fn group_pickers() {
        let g = |id: &str, name: &str, groups: Vec<Group>| Group {
            id: id.into(),
            name: name.into(),
            groups,
            ..Group::default()
        };
        let groups = vec![
            g("a", "Media", vec![g("b", "Downloads", vec![])]),
            g("c", "", vec![]),
        ];
        assert_eq!(
            group_options(&groups, None),
            [
                ("a".into(), "Media".into()),
                ("b".into(), "Media / Downloads".into()),
                ("c".into(), "(unnamed)".into())
            ]
        );
        assert_eq!(group_options(&groups, Some("a")).len(), 1);
        assert_eq!(
            by_category(&[schema()]),
            [(
                "Media".to_string(),
                vec![("sonarr".to_string(), "Sonarr".to_string())]
            )]
        );
    }
}
