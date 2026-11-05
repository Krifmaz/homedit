//! Live data on the dashboard: a card's widget stats (Sonarr queue, Plex streams…) and the info strip (clock,
//! weather, host resources, Docker). The browser only talks to homedit; the server calls the services.

use crate::fmt;
use crate::model::InfoWidget;
use kit_leptos::api;
use leptos::prelude::*;
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// How often a card's widget refreshes.
pub const SERVICE_MS: u64 = 15_000;
/// How often host resources refresh.
pub const RESOURCES_MS: u64 = 10_000;
/// How often the Docker counts refresh.
pub const DOCKER_MS: u64 = 30_000;
/// How often the weather refreshes.
pub const WEATHER_MS: u64 = 600_000;
/// Most stats on one card.
pub const MAX_STATS: usize = 4;

/// The last answer of a polled path.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Poll {
    /// No answer yet.
    pub loading: bool,
    /// The last good answer.
    pub data: Option<Value>,
    /// The last failure.
    pub error: Option<String>,
}

fn hidden() -> bool {
    kit_leptos::rt::document().is_some_and(|d| d.hidden())
}

/// Asks `GET /api<path>` now and every `ms` after each answer, while the component lives. A hidden tab asks
/// nothing and asks again as soon as it shows.
pub fn poll(path: String, ms: u64) -> ReadSignal<Poll> {
    let (get, set) = signal(Poll {
        loading: true,
        ..Poll::default()
    });
    let alive = Arc::new(AtomicBool::new(true));
    let a = alive.clone();
    on_cleanup(move || a.store(false, Ordering::Relaxed));
    leptos::task::spawn_local(async move {
        while alive.load(Ordering::Relaxed) {
            if hidden() {
                kit_leptos::rt::sleep(1000).await;
                continue;
            }
            let r = api::get(&path).await;
            if !alive.load(Ordering::Relaxed) {
                break;
            }
            match r {
                Ok(v) => set.set(Poll {
                    loading: false,
                    data: Some(v),
                    error: None,
                }),
                Err(e) => set.update(|p| {
                    p.loading = false;
                    p.error = Some(e.message);
                }),
            }
            let mut waited = 0;
            while waited < ms && alive.load(Ordering::Relaxed) && !hidden() {
                kit_leptos::rt::sleep(1000.min(ms - waited)).await;
                waited += 1000;
            }
            while alive.load(Ordering::Relaxed) && hidden() {
                kit_leptos::rt::sleep(1000).await;
            }
        }
    });
    get
}

/// A card's widget stats (`GET /api/widgets/{id}/data`).
#[component]
pub fn ServiceWidget(id: String) -> impl IntoView {
    let p = poll(
        format!("/widgets/{}/data", crate::model::js_encode(&id)),
        SERVICE_MS,
    );
    move || {
        let p = p.get();
        if p.loading {
            return view! {
                <div class="widget-stats" aria-busy="true" aria-label="Loading widget">
                    {(0..3).map(|_| view! { <div class="stat skeleton"><div class="stat-value">"\u{a0}"</div><div class="stat-label">"\u{a0}"</div></div> }).collect_view()}
                </div>
            }
            .into_any();
        }
        let problem = problem(&p);
        let d = p.data.unwrap_or(Value::Null);
        let stats: Vec<Value> = d
            .get("fields")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .take(MAX_STATS)
            .collect();
        if stats.is_empty() && problem.is_none() {
            return ().into_any();
        }
        view! {
            <div class="service-widget">
                {(!stats.is_empty()).then(|| view! {
                    <div class="widget-stats">
                        {stats.iter().map(|s| {
                            let unit = s.get("unit").and_then(Value::as_str).unwrap_or("").to_string();
                            view! {
                                <div class="stat">
                                    <div class="stat-value">
                                        {fmt::stat_value(s.get("value").unwrap_or(&Value::Null))}
                                        {(!unit.is_empty()).then(|| view! { <span class="stat-unit">" "{unit.clone()}</span> })}
                                    </div>
                                    <div class="stat-label">{s.get("label").and_then(Value::as_str).unwrap_or("").to_string()}</div>
                                </div>
                            }
                        }).collect_view()}
                    </div>
                })}
                {problem.map(|m| view! { <WidgetError label="Widget" message=m/> })}
            </div>
        }
        .into_any()
    }
}

/// The info widgets, in order.
#[component]
pub fn InfoStrip(widgets: Vec<InfoWidget>) -> impl IntoView {
    view! {
        <div class="info-strip">
            {widgets.into_iter().map(|w| match w.kind.as_str() {
                "clock" => view! { <Clock widget=w/> }.into_any(),
                "weather" => view! { <Weather widget=w/> }.into_any(),
                "resources" => view! { <Resources widget=w/> }.into_any(),
                "docker" => view! { <Docker widget=w/> }.into_any(),
                _ => ().into_any(),
            }).collect_view()}
        </div>
    }
}

fn opt_str(w: &InfoWidget, k: &str) -> String {
    w.options
        .get(k)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn locale(date: &js_sys::Date, time: bool, hour12: bool) -> String {
    let o = js_sys::Object::new();
    let set = |k: &str, v: &wasm_bindgen::JsValue| {
        let _ = js_sys::Reflect::set(&o, &k.into(), v);
    };
    if time {
        for k in ["hour", "minute", "second"] {
            set(k, &"2-digit".into());
        }
        set("hour12", &hour12.into());
        date.to_locale_time_string_with_options("default", &o)
            .into()
    } else {
        set("weekday", &"short".into());
        set("day", &"numeric".into());
        set("month", &"short".into());
        date.to_locale_date_string("default", &o).into()
    }
}

/// The local time and date.
#[component]
fn Clock(widget: InfoWidget) -> impl IntoView {
    let (now, set_now) = signal(js_sys::Date::now());
    let alive = Arc::new(AtomicBool::new(true));
    let a = alive.clone();
    on_cleanup(move || a.store(false, Ordering::Relaxed));
    leptos::task::spawn_local(async move {
        while alive.load(Ordering::Relaxed) {
            kit_leptos::rt::sleep(1000).await;
            set_now.set(js_sys::Date::now());
        }
    });
    let hour12 = opt_str(&widget, "format") == "12h";
    view! {
        <div class="info-widget info-clock">
            {move || {
                let d = js_sys::Date::new(&now.get().into());
                view! {
                    <time class="info-clock-time">{locale(&d, true, hour12)}</time>
                    <span class="info-sub">{locale(&d, false, false)}</span>
                }
            }}
        </div>
    }
}

fn num(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(Value::as_f64).filter(|n| n.is_finite())
}

fn info_poll(w: &InfoWidget, ms: u64) -> ReadSignal<Poll> {
    poll(format!("/info/{}", crate::model::js_encode(&w.id)), ms)
}

/// What went wrong for a widget: the server's `{"error": …}` (its upstream failed), else the call's own failure.
pub fn problem(p: &Poll) -> Option<String> {
    p.data
        .as_ref()
        .and_then(|d| d.get("error"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| p.error.clone())
}

/// A widget's problem in plain words, with the kit's Copy button; a click on the sentence opens the whole text
/// (both through the kit's page listener, [`kit_leptos::details::init`]). A click here never follows the card's
/// link around it.
#[component]
pub fn WidgetError(
    /// What the widget is ("Weather"), the details' title.
    label: &'static str,
    /// The sentence.
    message: String,
) -> impl IntoView {
    let title = format!("{label} unavailable");
    view! {
        <div class="widget-error" role="status" on:click=|e| e.prevent_default()>
            <span class="widget-error-text" data-details=message.clone() data-details-title=title>
                {message.clone()}
            </span>
            {kit_leptos::html::raw(kit_leptos::details::copy_button_html(&message))}
        </div>
    }
}

fn loading_or_failed(label: &'static str, p: &Poll) -> AnyView {
    if p.loading {
        view! { <span class="info-loading" aria-busy="true">{label}" …"</span> }.into_any()
    } else {
        let message = problem(p).unwrap_or_else(|| "Unexpected response".into());
        view! { <div class="info-widget info-failed"><WidgetError label=label message=message/></div> }
            .into_any()
    }
}

/// The temperature and condition (`units: imperial` for °F).
#[component]
fn Weather(widget: InfoWidget) -> impl IntoView {
    let p = info_poll(&widget, WEATHER_MS);
    let imperial = opt_str(&widget, "units") == "imperial";
    let location = opt_str(&widget, "location");
    move || {
        let p = p.get();
        let d = p.data.clone().unwrap_or(Value::Null);
        let (Some(t), Some(cond)) = (num(&d, "tempC"), d.get("condition").and_then(Value::as_str))
        else {
            return loading_or_failed("Weather", &p);
        };
        let temp = if imperial { t * 9.0 / 5.0 + 32.0 } else { t };
        let sub = if location.is_empty() {
            cond.to_string()
        } else {
            format!("{cond} · {location}")
        };
        view! {
            <div class="info-widget info-weather">
                <span class="info-main">{format!("{}°{}", temp.round(), if imperial { "F" } else { "C" })}</span>
                <span class="info-sub">{sub}</span>
            </div>
        }
        .into_any()
    }
}

fn meter(label: &'static str, ratio: f64, text: String) -> impl IntoView {
    let pct = (ratio * 100.0).clamp(0.0, 100.0);
    view! {
        <div class="info-meter-row">
            <span class="info-meter-label">{label}</span>
            <span class="info-meter" class:is-high={pct >= 90.0} role="meter" aria-label=label aria-valuemin="0" aria-valuemax="100"
                aria-valuenow=pct.round().to_string() aria-valuetext=text.clone()>
                <span class="info-meter-fill" style=format!("width:{pct}%")></span>
            </span>
            <span class="info-meter-text">{text}</span>
        </div>
    }
}

/// Host CPU, memory and disk.
#[component]
fn Resources(widget: InfoWidget) -> impl IntoView {
    let p = info_poll(&widget, RESOURCES_MS);
    move || {
        let p = p.get();
        let d = p.data.clone().unwrap_or(Value::Null);
        let vals = ["cpu", "memUsed", "memTotal", "diskUsed", "diskTotal"].map(|k| num(&d, k));
        let [Some(cpu), Some(mu), Some(mt), Some(du), Some(dt)] = vals else {
            return loading_or_failed("Resources", &p);
        };
        let r = |u: f64, t: f64| if t > 0.0 { u / t } else { 0.0 };
        view! {
            <div class="info-widget info-resources">
                {meter("CPU", cpu / 100.0, format!("{}%", cpu.round()))}
                {meter("RAM", r(mu, mt), format!("{} / {}", fmt::bytes(mu), fmt::bytes(mt)))}
                {meter("Disk", r(du, dt), format!("{} / {}", fmt::bytes(du), fmt::bytes(dt)))}
            </div>
        }
        .into_any()
    }
}

/// Running and stopped containers.
#[component]
fn Docker(widget: InfoWidget) -> impl IntoView {
    let p = info_poll(&widget, DOCKER_MS);
    move || {
        let p = p.get();
        let d = p.data.clone().unwrap_or(Value::Null);
        let (Some(run), Some(stop)) = (num(&d, "running"), num(&d, "stopped")) else {
            return loading_or_failed("Docker", &p);
        };
        view! {
            <div class="info-widget info-docker">
                <span class="info-main"><span class="info-count is-up">{run}</span>" running"</span>
                <span class="info-sub"><span class="info-count" class:is-down={stop > 0.0}>{stop}</span>" stopped"</span>
            </div>
        }
        .into_any()
    }
}
