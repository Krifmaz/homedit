//! The drawer's Appearance form (title, theme, accent, background, cards, search, custom CSS) and the info
//! widgets form. Both preview live and are one undo step.

use crate::forms::Session;
use crate::model::{Background, InfoWidget, Search, Settings, ThemeMode, new_id};
use crate::state::app;
use crate::theme;
use kit_leptos::{api, rt};
use leptos::prelude::*;
use serde_json::{Map, Value, json};

/// The info widget types with their names, in the picker's order.
pub const INFO_TYPES: [(&str, &str); 4] = [
    ("clock", "Clock"),
    ("weather", "Weather"),
    ("resources", "Host resources"),
    ("docker", "Docker"),
];

/// A new info widget's options.
pub fn default_info_options(kind: &str) -> Map<String, Value> {
    let mut m = Map::new();
    match kind {
        "clock" => {
            m.insert("format".into(), json!("24h"));
        }
        "weather" => {
            m.insert("units".into(), json!("metric"));
        }
        _ => {}
    }
    m
}

fn label_of(kind: &str) -> &'static str {
    INFO_TYPES
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, l)| *l)
        .unwrap_or("Widget")
}

/// `#abc` and `#aabbcc` as a colour input's `#aabbcc`, else the kit's cream.
pub fn color_input(v: &str) -> String {
    match theme::hex_rgb(v) {
        Some((r, g, b)) => format!("#{r:02x}{g:02x}{b:02x}"),
        None => "#f0ead6".into(),
    }
}

/// Why the settings can't be kept, if they can't: a custom search needs `%s`, an accent must be a colour.
pub fn settings_error(s: &Settings) -> Option<&'static str> {
    if let Search::Custom(u) = &s.search
        && !u.contains("%s")
    {
        return Some("The search URL must contain %s where the query goes");
    }
    if !s.accent.trim().is_empty() && theme::hex_rgb(&s.accent).is_none() {
        return Some("The accent must be a colour like #10b981");
    }
    None
}

/// A change to the info widget list.
type EditList = Box<dyn FnOnce(&mut Vec<InfoWidget>)>;

/// The info widgets: add, remove, reorder, and each one's options.
#[component]
pub fn InfoWidgetsForm() -> impl IntoView {
    let a = app();
    let s = Session::new();
    let add_kind = RwSignal::new("clock".to_string());
    let list = Memo::new(move |_| a.undo.with(|u| u.present.info_widgets.clone()));
    let edit = move |f: EditList| {
        s.edit(|d| {
            let mut d = d.clone();
            f(&mut d.info_widgets);
            d
        })
    };
    let ids = move || {
        list.with(|l| {
            l.iter()
                .map(|w| (w.id.clone(), w.kind.clone()))
                .collect::<Vec<_>>()
        })
    };
    view! {
        <form class="editor-form" on:submit=move |e| { e.prevent_default(); s.done(); }>
            <h2>"Info widgets"</h2>
            {move || list.with(Vec::is_empty).then(|| view! { <p class="muted small">"No info widgets yet."</p> })}
            <ol class="info-widget-list">
                <For each=ids key=|(id, _)| id.clone() children=move |(id, kind)| {
                    let opt = {
                        let id = id.clone();
                        move |k: &'static str| {
                            let id = id.clone();
                            Signal::derive(move || list.with(|l| l.iter().find(|w| w.id == id).and_then(|w| w.options.get(k).cloned()).map(|v| match v {
                                Value::String(s) => s,
                                v => v.to_string(),
                            }).unwrap_or_default()))
                        }
                    };
                    let set = {
                        let id = id.clone();
                        move |k: &'static str, numeric: bool| {
                            let id = id.clone();
                            move |e: leptos::ev::Event| {
                                let raw = event_target_value(&e);
                                let id = id.clone();
                                edit(Box::new(move |l| {
                                    let Some(w) = l.iter_mut().find(|w| w.id == id) else { return };
                                    if raw.trim().is_empty() {
                                        w.options.remove(k);
                                    } else if numeric && let Ok(n) = raw.trim().parse::<f64>() {
                                        w.options.insert(k.into(), json!(n));
                                    } else {
                                        w.options.insert(k.into(), json!(raw));
                                    }
                                }))
                            }
                        }
                    };
                    let text = {
                        let (opt, set) = (opt.clone(), set.clone());
                        move |k: &'static str, label: &'static str, ph: &'static str, numeric: bool| view! {
                            <label class="field"><span>{label}</span>
                                <input class="input" type=if numeric { "number" } else { "text" } step="any" placeholder=ph prop:value=opt(k) on:input=set(k, numeric)/></label>
                        }
                    };
                    let select = {
                        let (opt, set) = (opt.clone(), set.clone());
                        move |k: &'static str, label: &'static str, choices: &'static [(&'static str, &'static str)], fallback: &'static str| {
                            let v = opt(k);
                            view! {
                                <label class="field"><span>{label}</span>
                                    <select class="input" prop:value=move || { let x = v.get(); if x.is_empty() { fallback.to_string() } else { x } } on:change=set(k, false)>
                                        {choices.iter().map(|(v, l)| view! { <option value=*v>{*l}</option> }).collect_view()}
                                    </select></label>
                            }
                        }
                    };
                    let fields = match kind.as_str() {
                        "clock" => select("format", "Time format", &[("24h", "24-hour"), ("12h", "12-hour")], "24h").into_any(),
                        "weather" => view! {
                            {text("location", "Location name", "Home", false)}
                            {text("latitude", "Latitude", "52.37", true)}
                            {text("longitude", "Longitude", "4.90", true)}
                            {select("units", "Units", &[("metric", "Metric (°C)"), ("imperial", "Imperial (°F)")], "metric")}
                        }.into_any(),
                        "resources" => text("disk", "Disk path", "/", false).into_any(),
                        "docker" => text("host", "Docker host", "local socket", false).into_any(),
                        _ => ().into_any(),
                    };
                    let (i1, i2, i3, i4) = (id.clone(), id.clone(), id.clone(), id.clone());
                    let pos = Memo::new(move |_| list.with(|l| (l.iter().position(|w| w.id == i4).unwrap_or(0), l.len())));
                    let pos = move || pos.get();
                    let mv = move |id: String, by: i64| {
                        edit(Box::new(move |l| {
                            let Some(i) = l.iter().position(|w| w.id == id) else { return };
                            let j = i as i64 + by;
                            if j >= 0 && (j as usize) < l.len() {
                                l.swap(i, j as usize);
                            }
                        }))
                    };
                    let name = label_of(&kind);
                    view! {
                        <li class="info-widget-item">
                            <fieldset class="form-section">
                                <legend>{name}</legend>
                                {fields}
                                <div class="row">
                                    <button type="button" class="btn sm icon" aria-label=format!("Move {name} up") disabled=move || pos().0 == 0 on:click=move |_| mv(i1.clone(), -1)>"↑"</button>
                                    <button type="button" class="btn sm icon" aria-label=format!("Move {name} down") disabled=move || { let (i, n) = pos(); i + 1 >= n } on:click=move |_| mv(i2.clone(), 1)>"↓"</button>
                                    <button type="button" class="btn sm danger" aria-label=format!("Remove {name}")
                                        on:click=move |_| { let id = i3.clone(); edit(Box::new(move |l| l.retain(|w| w.id != id))) }>"Remove"</button>
                                </div>
                            </fieldset>
                        </li>
                    }
                }/>
            </ol>
            <div class="field">
                <span>"Add widget"</span>
                <div class="row">
                    <select class="input" prop:value=move || add_kind.get() on:change=move |e| add_kind.set(event_target_value(&e))>
                        {INFO_TYPES.iter().map(|(k, l)| view! { <option value=*k>{*l}</option> }).collect_view()}
                    </select>
                    <button type="button" class="btn" on:click=move |_| {
                        let k = add_kind.get_untracked();
                        edit(Box::new(move |l| l.push(InfoWidget { id: new_id("iw"), options: default_info_options(&k), kind: k })))
                    }>"Add"</button>
                </div>
            </div>
            <div class="drawer-footer row">
                <button type="submit" class="btn primary">"Done"</button>
                <button type="button" class="btn" on:click=move |_| s.cancel()>"Cancel"</button>
            </div>
        </form>
    }
}

/// A range input with its value beside it.
fn slider(
    label: &'static str,
    (min, max, step): (f64, f64, f64),
    value: Signal<f64>,
    shown: Signal<String>,
    help: Option<&'static str>,
    on: impl Fn(f64) + 'static,
) -> impl IntoView {
    view! {
        <div class="field">
            <span>{label}</span>
            <div class="row">
                <input class="grow" type="range" min=min max=max step=step prop:value=move || value.get().to_string()
                    on:input=move |e| { if let Ok(v) = event_target_value(&e).parse::<f64>() { on(v.clamp(min, max)) } }/>
                <output class="small muted">{move || shown.get()}</output>
            </div>
            {help.map(|h| view! { <small>{h}</small> })}
        </div>
    }
}

/// The dashboard's look.
#[component]
pub fn AppearanceForm() -> impl IntoView {
    let a = app();
    let s = Session::new();
    let st = Memo::new(move |_| a.undo.with(|u| u.present.settings.clone()));
    let set = move |f: Box<dyn FnOnce(&mut Settings)>| {
        s.edit(|d| {
            let mut d = d.clone();
            f(&mut d.settings);
            d
        })
    };
    let set_bg = move |f: Box<dyn FnOnce(&mut Background)>| {
        set(Box::new(move |s| {
            let b = s.background.get_or_insert_with(|| Background::new(""));
            f(b);
        }))
    };
    let error = Memo::new(move |_| st.with(settings_error));
    let has_bg = Memo::new(move |_| st.with(|s| s.background.is_some()));
    let custom = Memo::new(move |_| st.with(|s| matches!(s.search, Search::Custom(_))));
    let bgv = move |f: fn(&Background) -> f64| {
        Signal::derive(move || st.with(|s| s.background.as_ref().map(f).unwrap_or(0.0)))
    };
    view! {
        <form class="editor-form" on:submit=move |e| { e.prevent_default(); if error.get_untracked().is_none() { s.done(); } }>
            <h2>"Appearance"</h2>
            <label class="field"><span>"Title"</span>
                <input class="input" type="text" prop:value=move || st.get().title on:input=move |e| { let v = event_target_value(&e); set(Box::new(move |s| s.title = v)) }/></label>
            <label class="field"><span>"Theme"</span>
                <select class="input" prop:value=move || st.get().theme.as_str().to_string() on:change=move |e| {
                    let t = match event_target_value(&e).as_str() { "light" => ThemeMode::Light, "dark" => ThemeMode::Dark, _ => ThemeMode::System };
                    set(Box::new(move |s| s.theme = t))
                }>
                    <option value="system">"Follow the system"</option>
                    <option value="light">"Light"</option>
                    <option value="dark">"Dark (Encore)"</option>
                </select></label>
            <div class="field"><span>"Accent colour"</span>
                <div class="row">
                    <input type="color" aria-label="Accent colour picker" prop:value=move || color_input(&st.get().accent)
                        on:input=move |e| { let v = event_target_value(&e); set(Box::new(move |s| s.accent = v)) }/>
                    <input class="input grow" type="text" spellcheck="false" placeholder="The theme's own" prop:value=move || st.get().accent
                        on:input=move |e| { let v = event_target_value(&e); set(Box::new(move |s| s.accent = v.trim().to_string())) }/>
                    <button type="button" class="btn sm" disabled=move || st.with(|s| s.accent.is_empty())
                        on:click=move |_| set(Box::new(|s| s.accent.clear()))>"Theme's own"</button>
                </div>
            </div>
            <fieldset class="form-section">
                <legend>"Background image"</legend>
                <label class="row"><span class="switch"><input type="checkbox" prop:checked=move || has_bg.get()
                    on:change=move |e| { let on = event_target_checked(&e); set(Box::new(move |s| s.background = on.then(|| Background::new("")))) }/><i></i></span>
                    <span>"Show a background image"</span></label>
                {move || has_bg.get().then(|| view! {
                    <BackgroundPicker value=Signal::derive(move || st.get().background.unwrap_or_else(|| Background::new("")))
                        on_url=Callback::new(move |u: String| set_bg(Box::new(move |b| b.url = u)))/>
                    {slider("Blur (px)", (0.0, 30.0, 1.0), bgv(|b| b.blur), Signal::derive(move || st.with(|s| s.background.as_ref().map(|b| b.blur.to_string()).unwrap_or_default())), None,
                        move |v| set_bg(Box::new(move |b| b.blur = v)))}
                    {slider("Opacity", (0.0, 1.0, 0.05), bgv(|b| b.opacity), Signal::derive(move || st.with(|s| s.background.as_ref().map(|b| format!("{}%", (b.opacity * 100.0).round())).unwrap_or_default())), None,
                        move |v| set_bg(Box::new(move |b| b.opacity = v)))}
                    {slider("Saturation", (0.0, 200.0, 5.0), bgv(|b| b.saturate), Signal::derive(move || st.with(|s| s.background.as_ref().map(|b| format!("{}%", b.saturate)).unwrap_or_default())), None,
                        move |v| set_bg(Box::new(move |b| b.saturate = v)))}
                    {slider("Brightness", (0.0, 200.0, 5.0), bgv(|b| b.brightness), Signal::derive(move || st.with(|s| s.background.as_ref().map(|b| format!("{}%", b.brightness)).unwrap_or_default())), None,
                        move |v| set_bg(Box::new(move |b| b.brightness = v)))}
                })}
            </fieldset>
            <fieldset class="form-section">
                <legend>"Cards"</legend>
                {slider("Frosted glass blur (px)", (0.0, 40.0, 1.0), Signal::derive(move || st.get().card_blur),
                    Signal::derive(move || { let b = st.get().card_blur; if b > 0.0 { b.to_string() } else { "Off".into() } }),
                    Some("See-through cards with a blur; best over a background image."),
                    move |v| set(Box::new(move |s| s.card_blur = v)))}
            </fieldset>
            <fieldset class="form-section">
                <legend>"Search"</legend>
                <label class="field"><span>"Search provider"</span>
                    <select class="input" prop:value=move || st.get().search.provider().to_string() on:change=move |e| {
                        let p = event_target_value(&e);
                        set(Box::new(move |s| s.search = match p.as_str() {
                            "google" => Search::Google,
                            "bing" => Search::Bing,
                            "custom" => Search::Custom(String::new()),
                            _ => Search::DuckDuckGo,
                        }))
                    }>
                        <option value="duckduckgo">"DuckDuckGo"</option>
                        <option value="google">"Google"</option>
                        <option value="bing">"Bing"</option>
                        <option value="custom">"Custom"</option>
                    </select></label>
                {move || custom.get().then(|| view! {
                    <label class="field"><span>"Search URL *"</span>
                        <input class="input" type="url" spellcheck="false" placeholder="https://search.example/?q=%s"
                            prop:value=move || match st.get().search { Search::Custom(u) => u, _ => String::new() }
                            on:input=move |e| { let v = event_target_value(&e); set(Box::new(move |s| s.search = Search::Custom(v))) }/>
                        <small>"%s is replaced by the search terms."</small></label>
                })}
            </fieldset>
            <label class="field"><span>"Custom CSS"</span>
                <textarea class="input mono" spellcheck="false" rows="6" prop:value=move || st.get().custom_css
                    on:input=move |e| { let v = event_target_value(&e); set(Box::new(move |s| s.custom_css = v)) }></textarea>
                <small>"Added after the built-in styles. The kit's colours are variables: --bg, --panel, --text, --accent…"</small></label>
            {move || error.get().map(|m| view! { <div class="error-box" role="alert">{m}</div> })}
            <div class="drawer-footer row">
                <button type="submit" class="btn primary" disabled=move || error.get().is_some()>"Done"</button>
                <button type="button" class="btn" on:click=move |_| s.cancel()>"Cancel"</button>
            </div>
        </form>
    }
}

/// Whether `url` is on another website.
pub fn is_external(url: &str) -> bool {
    crate::forms::is_http_url(url)
}

/// The background's URL with a preview, Upload, a copy of an outside image, and the uploaded images.
#[component]
fn BackgroundPicker(
    #[prop(into)] value: Signal<Background>,
    on_url: Callback<String>,
) -> impl IntoView {
    let images = RwSignal::new(Vec::<Value>::new());
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let file = NodeRef::<leptos::html::Input>::new();
    rt::spawn(async move {
        if let Ok(v) = api::get("/images").await {
            images.set(v.as_array().cloned().unwrap_or_default());
        }
    });
    let got = move |r: Result<Value, kit_leptos::api::ApiError>| match r {
        Ok(img) => {
            let url = crate::shell::s(&img, "url");
            let name = crate::shell::s(&img, "name");
            images.update(|l| {
                l.retain(|i| crate::shell::s(i, "name") != name);
                l.push(img);
            });
            on_url.run(url);
            error.set(None);
        }
        Err(e) => error.set(Some(e.message)),
    };
    let upload = move |_| {
        let Some(input) = file.get_untracked() else {
            return;
        };
        let Some(f) = input.files().and_then(|l| l.get(0)) else {
            return;
        };
        busy.set(true);
        rt::spawn(async move {
            if let Ok(form) = web_sys::FormData::new() {
                let _ = form.append_with_blob("file", &f);
                got(api::post_form("/images", form).await);
            }
            busy.set(false);
            input.set_value("");
        });
    };
    let copy = move |_| {
        let url = value.get_untracked().url.trim().to_string();
        busy.set(true);
        rt::spawn(async move {
            got(api::post("/images/fetch", json!({ "url": url })).await);
            busy.set(false);
        });
    };
    view! {
        <div class="field bg-picker">
            <span>"Image URL"</span>
            <div class="bg-preview" aria-hidden="true" style=move || {
                let b = value.get();
                if b.url.is_empty() { String::new() } else { format!("background-image:{};filter:{};opacity:{}", theme::css_url(&b.url), theme::background_filter(&b), b.opacity) }
            }>{move || value.with(|b| b.url.is_empty()).then(|| view! { <span class="muted small">"No image yet"</span> })}</div>
            <div class="row">
                <input class="input grow" type="text" spellcheck="false" placeholder="https://… or /images/wallpaper.jpg"
                    prop:value=move || value.get().url on:input=move |e| on_url.run(event_target_value(&e))/>
                <input type="file" accept="image/*" hidden node_ref=file on:change=upload aria-label="Upload background image file"/>
                <button type="button" class="btn" disabled=move || busy.get() on:click=move |_| { if let Some(i) = file.get_untracked() { i.click(); } }>"Upload"</button>
            </div>
            {move || is_external(&value.get().url).then(|| view! {
                <div class="row"><button type="button" class="btn sm" disabled=move || busy.get() on:click=copy>"Save a copy on this server"</button>
                <small>"Some sites refuse to show their images elsewhere; a copy here avoids that."</small></div>
            })}
            <small>"Uploads are kept in the data folder under images/ and served at /images/."</small>
            {move || error.get().map(|e| view! { <small class="err" role="alert">{e}</small> })}
            {move || {
                let list = images.get();
                (!list.is_empty()).then(|| view! {
                    <div class="bg-grid" role="group" aria-label="Uploaded images">
                        {list.into_iter().map(|img| {
                            let (url, name) = (crate::shell::s(&img, "url"), crate::shell::s(&img, "name"));
                            let (u2, u3) = (url.clone(), url.clone());
                            view! {
                                <button type="button" class="btn bg-choice" class:selected=move || value.with(|b| b.url == u2) title=name.clone()
                                    aria-label=format!("Use image {name}") on:click=move |_| on_url.run(u3.clone())>
                                    <img src=url alt="" loading="lazy"/>
                                </button>
                            }
                        }).collect_view()}
                    </div>
                })
            }}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules() {
        assert_eq!(color_input("#abc"), "#aabbcc");
        assert_eq!(color_input(""), "#f0ead6");
        let mut s = Settings::default();
        assert_eq!(settings_error(&s), None);
        s.search = Search::Custom("https://x/".into());
        assert!(settings_error(&s).is_some());
        s.search = Search::Custom("https://x/?q=%s".into());
        s.accent = "blue".into();
        assert!(settings_error(&s).is_some());
        assert_eq!(default_info_options("clock")["format"], json!("24h"));
        assert!(default_info_options("docker").is_empty());
        assert!(is_external("https://a.b/c.jpg") && !is_external("/images/a.jpg"));
    }
}
