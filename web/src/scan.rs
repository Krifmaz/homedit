//! Find services: scan networks for open ports, watch progress, and add what was found to the dashboard with
//! the link and widget type filled in. Scans are plain TCP connects, rate limited and logged on the server.

use crate::model::{Check, Group, Service, WidgetConfig, new_id};
use crate::ops;
use crate::state::{Target, app};
use kit_leptos::{api, modal, rt};
use kit_rules::dialogs::Confirm;
use leptos::prelude::*;
use serde_json::{Map, Value, json};

/// How often a running scan is asked about.
const POLL_MS: u64 = 1000;

/// One service a scan found, flattened for display.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    /// `ip:port`, the key of its suggestion.
    pub key: String,
    /// The hostname when known, else the address.
    pub host: String,
    /// The address.
    pub ip: String,
    /// The port.
    pub port: u64,
    /// The recognised app, the page title, or "Port N".
    pub name: String,
    /// The app id from the catalogue, when recognised.
    pub app_id: Option<String>,
    /// The widget type for it, if any.
    pub widget: Option<String>,
    /// The link to open.
    pub href: Option<String>,
    /// Whether a card already points here.
    pub on_dashboard: bool,
    /// The pre-filled card.
    pub suggestion: Option<Value>,
}

fn st(v: &Value, k: &str) -> Option<String> {
    v.get(k)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn service_name(svc: &Value) -> String {
    let http = svc.get("http");
    svc.get("app")
        .and_then(|a| st(a, "name"))
        .or_else(|| http.and_then(|h| st(h, "title")))
        .or_else(|| http.and_then(|h| st(h, "server")))
        .or_else(|| svc.get("container").and_then(|c| st(c, "name")))
        .or_else(|| st(svc, "guess"))
        .unwrap_or_else(|| {
            format!(
                "Port {}",
                svc.get("port").and_then(Value::as_u64).unwrap_or(0)
            )
        })
}

/// Numeric-aware order of two addresses (`10.0.0.9` before `10.0.0.10`).
fn ip_order(a: &str, b: &str) -> std::cmp::Ordering {
    let parts = |s: &str| {
        s.split('.')
            .map(|p| p.parse::<u64>().unwrap_or(u64::MAX))
            .collect::<Vec<_>>()
    };
    parts(a).cmp(&parts(b)).then_with(|| a.cmp(b))
}

/// The services of a finished scan: recognised apps first, then by address and port.
pub fn found_services(job: &Value) -> Vec<Found> {
    let empty = Map::new();
    let sugg = job
        .get("suggestions")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let mut out = Vec::new();
    let hosts = job
        .get("report")
        .and_then(|r| r.get("hosts"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for h in &hosts {
        let ip = st(h, "ip").unwrap_or_default();
        let host = st(h, "hostname")
            .or_else(|| st(h, "name"))
            .unwrap_or_else(|| ip.clone());
        for s in h
            .get("services")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let port = s.get("port").and_then(Value::as_u64).unwrap_or(0);
            let key = format!("{ip}:{port}");
            let sug = sugg.get(&key).cloned();
            let app = s.get("app");
            out.push(Found {
                name: sug
                    .as_ref()
                    .and_then(|g| st(g, "name"))
                    .unwrap_or_else(|| service_name(s)),
                app_id: app.and_then(|a| st(a, "id")),
                widget: app.and_then(|a| st(a, "widget")),
                href: sug.as_ref().and_then(|g| st(g, "href")),
                on_dashboard: s
                    .get("on_dashboard")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                suggestion: sug,
                key,
                host: host.clone(),
                ip: ip.clone(),
                port,
            });
        }
    }
    out.sort_by(|a, b| {
        a.app_id
            .is_none()
            .cmp(&b.app_id.is_none())
            .then_with(|| ip_order(&a.ip, &b.ip))
            .then(a.port.cmp(&b.port))
    });
    out
}

/// A card from a suggestion; its form then only needs the secrets.
pub fn suggestion_to_service(s: &Value, id: String) -> Service {
    let widget = s.get("widget").and_then(Value::as_object).and_then(|w| {
        let kind = w
            .get("type")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())?
            .to_string();
        let fields = w
            .iter()
            .filter(|(k, v)| {
                *k != "type" && (v.is_null() || v.is_string() || v.is_number() || v.is_boolean())
            })
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        Some(WidgetConfig { kind, fields })
    });
    Service {
        id,
        name: st(s, "name").unwrap_or_default(),
        href: st(s, "href"),
        icon: st(s, "icon").or_else(|| st(s, "favicon")),
        description: st(s, "description"),
        check: Some(Check {
            kind: "http".into(),
            target: None,
            expect_status: None,
        }),
        widget,
        ..Service::default()
    }
}

/// A running scan's progress from 0 to 1, `None` when unknown.
pub fn scan_progress(job: &Value) -> Option<f64> {
    for e in job
        .get("events")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .rev()
    {
        let n = |k: &str| e.get(k).and_then(Value::as_f64).unwrap_or(0.0);
        match e.get("event").and_then(Value::as_str) {
            Some("finished") => return Some(1.0),
            Some("progress") if n("total") > 0.0 => return Some((n("done") / n("total")).min(1.0)),
            Some("started") => return Some(0.0),
            _ => {}
        }
    }
    None
}

/// What a running scan has found so far, for the live progress line.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Live {
    /// Still finding live hosts (`done` of `total` addresses checked).
    pub discovering: bool,
    /// Steps done in the current phase.
    pub done: u64,
    /// Steps planned in the current phase.
    pub total: u64,
    /// Live hosts found.
    pub hosts: usize,
    /// Apps found: `(ip:port, name)`, in the order they turned up.
    pub apps: Vec<(String, String)>,
}

impl Live {
    /// "Finding hosts: 120 of 254 checked" or "Checking ports: 43%".
    pub fn phase_text(&self) -> String {
        if self.discovering {
            format!("Finding hosts: {} of {} checked", self.done, self.total)
        } else if let Some(pct) = (self.done * 100).checked_div(self.total) {
            format!("Checking ports: {}%", pct.min(100))
        } else {
            "Starting…".to_string()
        }
    }

    /// "3 hosts · 5 apps".
    pub fn counts_text(&self) -> String {
        let n =
            |k: usize, one: &str, many: &str| format!("{k} {}", if k == 1 { one } else { many });
        format!(
            "{} · {}",
            n(self.hosts, "host", "hosts"),
            n(self.apps.len(), "app", "apps")
        )
    }
}

/// Hosts and apps a running scan has found so far, from its events.
pub fn live_progress(job: &Value) -> Live {
    let mut live = Live::default();
    let mut hosts = std::collections::BTreeSet::new();
    for e in job
        .get("events")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let n = |k: &str| e.get(k).and_then(Value::as_u64).unwrap_or(0);
        match e.get("event").and_then(Value::as_str) {
            Some("progress") => {
                live.discovering = e.get("phase").and_then(Value::as_str) == Some("discover");
                live.done = n("done");
                live.total = n("total");
                live.hosts = live.hosts.max(n("hosts") as usize);
            }
            Some("host_up") => {
                hosts.insert(st(e, "ip").unwrap_or_default());
            }
            Some("service") => {
                let Some(svc) = e.get("service") else {
                    continue;
                };
                let key = format!(
                    "{}:{}",
                    st(e, "ip").unwrap_or_default(),
                    svc.get("port").and_then(Value::as_u64).unwrap_or(0)
                );
                let is_app = svc.get("app").is_some_and(|a| !a.is_null())
                    || svc.get("http").is_some_and(|h| !h.is_null());
                let name = service_name(svc);
                match live.apps.iter_mut().find(|(k, _)| *k == key) {
                    Some(slot) => slot.1 = name,
                    None if is_app => live.apps.push((key, name)),
                    None => {}
                }
            }
            _ => {}
        }
    }
    live.hosts = live.hosts.max(hosts.len());
    live
}

/// The default targets: the server's networks, LAN ones first.
pub fn network_targets(networks: &Value) -> Vec<String> {
    networks
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|n| st(n, "cidr"))
        .collect()
}

/// "Your networks: 10.0.1.0/24 (eth0), 172.18.0.0/24 (Docker)".
pub fn networks_text(networks: &Value) -> Option<String> {
    let list: Vec<String> = networks
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|n| {
            let cidr = st(n, "cidr")?;
            let docker = n.get("docker").and_then(Value::as_bool).unwrap_or(false);
            Some(if docker {
                format!("{cidr} (Docker)")
            } else {
                match st(n, "interface") {
                    Some(i) => format!("{cidr} ({i})"),
                    None => cidr,
                }
            })
        })
        .collect();
    (!list.is_empty()).then(|| format!("This server is on {}.", list.join(", ")))
}

/// The targets in the text box: one per line or comma separated.
pub fn parse_targets(text: &str) -> Vec<String> {
    text.split(['\n', ','])
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

fn is_bare_network(t: &str) -> bool {
    let o: Vec<&str> = t.split('.').collect();
    o.len() == 4
        && o[3] == "0"
        && o.iter().all(|p| {
            !p.is_empty()
                && p.len() <= 3
                && p.bytes().all(|b| b.is_ascii_digit())
                && p.parse::<u16>().is_ok_and(|n| n <= 255)
        })
}

/// Targets written as a network without its prefix length (`10.0.1.0` rather than `10.0.1.0/24`), which scan
/// just that one, usually unused, address.
pub fn bare_networks(text: &str) -> Vec<String> {
    parse_targets(text)
        .into_iter()
        .filter(|t| is_bare_network(t))
        .collect()
}

/// `text` with every bare network address written as a /24 range.
pub fn widen_bare_networks(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    let mut word = String::new();
    let flush = |w: &mut String, out: &mut String| {
        if is_bare_network(w) {
            w.push_str("/24");
        }
        out.push_str(w);
        w.clear();
    };
    for c in text.chars() {
        if c.is_whitespace() || c == ',' {
            flush(&mut word, &mut out);
            out.push(c);
        } else {
            word.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

/// The question asked before scanning bare network addresses, and its OK button: scan their /24 ranges
/// instead of the single addresses.
pub fn widen_question(bare: &[String]) -> (String, String) {
    if let [one] = bare {
        (
            format!(
                "{one} is a network address, so only that one address would be scanned. Scan {one}/24 (the whole network) instead?"
            ),
            format!("Scan {one}/24"),
        )
    } else {
        (
            format!(
                "{} are network addresses, so only those addresses would be scanned. Scan their /24 ranges (the whole networks) instead?",
                bare.join(", ")
            ),
            "Scan the /24 ranges".to_string(),
        )
    }
}

/// Words for a rescan interval in hours ("day", "3 days", "week").
pub fn interval_label(hours: u64) -> String {
    let unit = |n: u64, one: &str, many: &str| {
        if n == 1 {
            one.to_string()
        } else {
            format!("{n} {many}")
        }
    };
    if hours.is_multiple_of(168) && hours > 0 {
        unit(hours / 168, "week", "weeks")
    } else if hours.is_multiple_of(24) && hours > 0 {
        unit(hours / 24, "day", "days")
    } else {
        unit(hours, "hour", "hours")
    }
}

/// "Last run … Next run …" for a schedule; one that never ran is due at once (`nextRunAt: 0`).
pub fn schedule_times(s: &Value, now: f64, when: impl Fn(f64) -> String) -> String {
    let mut parts = Vec::new();
    let t = |k: &str| s.get(k).and_then(Value::as_f64);
    if let Some(l) = t("lastRunAt").filter(|l| *l > 0.0) {
        parts.push(format!("Last run {}.", when(l)));
    }
    if s.get("enabled").and_then(Value::as_bool).unwrap_or(false)
        && let Some(n) = t("nextRunAt")
    {
        parts.push(if n <= now {
            "Next run within a minute.".to_string()
        } else {
            format!("Next run {}.", when(n))
        });
    }
    parts.join(" ")
}

fn local_time(ms: f64) -> String {
    js_sys::Date::new(&ms.into())
        .to_locale_string("default", &js_sys::Object::new())
        .into()
}

fn ports_json(kind: &str, list: &str) -> Value {
    if kind == "list" {
        json!({"kind": "list", "ports": list})
    } else {
        json!({"kind": kind})
    }
}

/// The scan panel in the editor's drawer.
#[component]
pub fn ScanPanel() -> impl IntoView {
    let a = app();
    let targets = RwSignal::new(String::new());
    let networks = RwSignal::new(Value::Null);
    let port_kind = RwSignal::new("common".to_string());
    let port_list = RwSignal::new("80,443,8000-9000".to_string());
    let fingerprint = RwSignal::new(true);
    let job = RwSignal::new(None::<Value>);
    let error = RwSignal::new(None::<String>);
    let hide_known = RwSignal::new(true);
    let group_id = RwSignal::new(String::new());
    let schedule = RwSignal::new(None::<Value>);
    let schedule_msg = RwSignal::new(None::<String>);
    let schedule_at = RwSignal::new(js_sys::Date::now());
    let alive = RwSignal::new(true);
    on_cleanup(move || alive.set(false));

    let refresh = move || async move {
        match api::get("/scan").await {
            Ok(v) => job.set(Some(v)),
            Err(e) => error.set(Some(e.message)),
        }
    };
    let running = Memo::new(move |_| {
        job.with(|j| {
            j.as_ref()
                .and_then(|j| j.get("running"))
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
    });
    // While a scan runs, ask again every second.
    let watch = move || {
        rt::spawn(async move {
            loop {
                refresh().await;
                if !alive.try_get_untracked().unwrap_or(false)
                    || !running.try_get_untracked().unwrap_or(false)
                {
                    break;
                }
                rt::sleep(POLL_MS).await;
            }
        })
    };
    watch();
    rt::spawn(async move {
        // Older servers have no schedule; the section stays hidden.
        if let Ok(s) = api::get("/scan/schedule").await {
            let r = s.get("request").cloned().unwrap_or(Value::Null);
            let ts: Vec<String> = r
                .get("targets")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
            if !ts.is_empty() {
                targets.set(ts.join("\n"));
                let p = r.get("ports").cloned().unwrap_or(Value::Null);
                if let Some(k) = p.get("kind").and_then(Value::as_str) {
                    port_kind.set(k.to_string());
                }
                if let Some(l) = p.get("ports").and_then(Value::as_str) {
                    port_list.set(l.to_string());
                }
                fingerprint.set(
                    r.get("fingerprint")
                        .and_then(Value::as_bool)
                        .unwrap_or(true),
                );
            }
            schedule.set(Some(s));
        }
        // The default targets: the server's own networks (older servers
        // can't tell, so the usual home range stands in).
        let nets = api::get("/scan/networks").await.unwrap_or(Value::Null);
        let list = network_targets(&nets);
        networks.set(nets);
        if targets.get_untracked().trim().is_empty() {
            targets.set(if list.is_empty() {
                "192.168.1.0/24".to_string()
            } else {
                list.join("\n")
            });
        }
    });

    let request = move || {
        json!({
            "targets": parse_targets(&targets.get_untracked()),
            "ports": ports_json(&port_kind.get_untracked(), &port_list.get_untracked()),
            "fingerprint": fingerprint.get_untracked(),
        })
    };
    let save_schedule = move || {
        let Some(s) = schedule.get_untracked() else {
            return;
        };
        schedule_msg.set(None);
        let body = json!({
            "enabled": s.get("enabled").and_then(Value::as_bool).unwrap_or(false),
            "intervalHours": s.get("intervalHours").and_then(Value::as_u64).unwrap_or(24),
            "request": request(),
        });
        rt::spawn(async move {
            match api::put("/scan/schedule", body).await {
                Ok(saved) => {
                    schedule.set(Some(saved));
                    schedule_at.set(js_sys::Date::now());
                    schedule_msg.set(Some("Schedule saved.".into()));
                }
                Err(e) => schedule_msg.set(Some(e.message)),
            }
        });
    };
    let set_schedule = move |k: &'static str, v: Value| {
        schedule.update(|s| {
            if let Some(Value::Object(m)) = s {
                m.insert(k.into(), v);
            }
        })
    };

    // Scans `text` (the targets box) with the form's ports.
    let scan = move |text: String| async move {
        let mut req = request();
        req["targets"] = json!(parse_targets(&text));
        if req["targets"].as_array().is_none_or(Vec::is_empty) {
            return;
        }
        if port_kind.get_untracked() == "full"
            && !modal::confirm(
                Confirm::new("Scan all 65,535 ports?")
                    .text("It is slow.")
                    .ok("Scan"),
            )
            .await
        {
            return;
        }
        error.set(None);
        match api::post("/scan", req).await {
            Ok(_) => watch(),
            Err(e) => error.set(Some(e.message)),
        }
    };
    // Starts the scan, first offering the /24 ranges of bare network addresses.
    let start = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        rt::spawn(async move {
            let mut text = targets.get_untracked();
            let bare_now = bare_networks(&text);
            if !bare_now.is_empty() {
                let (title, ok) = widen_question(&bare_now);
                let q = Confirm::new(title).ok(ok).cancel("Scan as typed");
                if modal::confirm(q).await {
                    text = widen_bare_networks(&text);
                    targets.set(text.clone());
                }
            }
            scan(text).await;
        });
    };
    // Scans again with the bare network addresses widened to /24.
    let rescan_widened = move || {
        let text = widen_bare_networks(&targets.get_untracked());
        targets.set(text.clone());
        rt::spawn(scan(text));
    };

    let groups = Memo::new(move |_| {
        a.undo.with(|u| {
            ops::all_groups(&u.present.groups)
                .into_iter()
                .map(|g| (g.id.clone(), g.name.clone()))
                .collect::<Vec<_>>()
        })
    });
    let add = move |f: Found| {
        let Some(sug) = f.suggestion else { return };
        let mut gid = group_id.get_untracked();
        if gid.is_empty() {
            gid = groups
                .get_untracked()
                .first()
                .map(|g| g.0.clone())
                .unwrap_or_default();
        }
        let svc = suggestion_to_service(&sug, new_id("s"));
        let sid = svc.id.clone();
        let target_group = if gid.is_empty() {
            new_id("g")
        } else {
            gid.clone()
        };
        let tg = target_group.clone();
        a.edit(None, move |d| {
            let mut doc = d.clone();
            if gid.is_empty() {
                doc = ops::upsert_group(
                    &doc,
                    Group {
                        id: tg.clone(),
                        name: "Discovered".into(),
                        ..Group::default()
                    },
                    None,
                );
            }
            ops::upsert_service(&doc, &tg, svc)
        });
        // Open the card's form, so the API key can be added and tested.
        a.target.set(Some(Target::Service {
            group: target_group,
            service: Some(sid),
        }));
    };

    let found =
        Memo::new(move |_| job.with(|j| j.as_ref().map(found_services).unwrap_or_default()));
    let bare = Memo::new(move |_| bare_networks(&targets.get()));
    // The bare network addresses a finished scan that found nothing was given, still in the box: it can
    // offer the whole network instead.
    let scanned_bare = Memo::new(move |_| {
        if running.get() || !found.with(Vec::is_empty) {
            return Vec::new();
        }
        let scanned = job.with(|j| {
            j.as_ref()
                .and_then(|j| j.get("report"))
                .and_then(|r| r.get("targets"))
                .and_then(Value::as_array)
                .map(|t| {
                    t.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join("\n")
                })
        });
        let Some(scanned) = scanned else {
            return Vec::new();
        };
        let now = bare.get();
        bare_networks(&scanned)
            .into_iter()
            .filter(|t| now.contains(t))
            .collect()
    });
    let has_targets = Memo::new(move |_| !parse_targets(&targets.get()).is_empty());

    view! {
        <div class="scan">
            <h2>"Find services"</h2>
            <form on:submit=start>
                <label class="field">
                    <span>"Networks and hosts"</span>
                    <textarea class="input scan-targets" bind:value=targets></textarea>
                    <small>"One per line: a CIDR range (192.168.1.0/24), an address, or a hostname. Only private ranges are allowed unless the server is configured otherwise."</small>
                    {move || networks.with(networks_text).map(|t| view! { <small class="muted">{t}" Ranges skip the server's own addresses, and only hosts that answer are port-scanned."</small> })}
                </label>
                {move || {
                    let b = bare.get();
                    (!b.is_empty() && scanned_bare.with(Vec::is_empty)).then(|| {
                        let (text, button) = if b.len() == 1 {
                            (format!("{0} is a network address, so only that one address is scanned. To scan the whole network, use {0}/24.", b[0]), format!("Scan {}/24", b[0]))
                        } else {
                            (format!("{} are network addresses, so only those addresses are scanned. To scan the whole networks, add /24 to each.", b.join(", ")), "Scan the /24 ranges".to_string())
                        };
                        view! {
                            <div class="scan-hint" role="status">
                                <span>{text}</span>
                                <button type="button" class="btn sm" on:click=move |_| targets.update(|t| *t = widen_bare_networks(t))>{button}</button>
                            </div>
                        }
                    })
                }}
                <label class="field">
                    <span>"Ports"</span>
                    <select class="input" prop:value=move || port_kind.get() on:change=move |e| port_kind.set(event_target_value(&e))>
                        <option value="common">"Common web ports and known apps (default)"</option>
                        <option value="catalogue">"Known apps only (fastest)"</option>
                        <option value="list">"Custom list"</option>
                        <option value="full">"All ports 1–65535 (slow)"</option>
                    </select>
                </label>
                {move || (port_kind.get() == "list").then(|| view! {
                    <label class="field">
                        <span>"Port list"</span>
                        <input class="input" bind:value=port_list/>
                        <small>"Comma separated, ranges allowed: 80,443,8000-8100"</small>
                    </label>
                })}
                <label class="row check-row">
                    <input type="checkbox" bind:checked=fingerprint/>
                    <span>"Identify apps (page title, favicon, known API paths)"</span>
                </label>
                <button type="submit" class="btn primary" disabled=move || running.get() || !has_targets.get()>
                    {move || if running.get() { "Scanning…" } else { "Start scan" }}
                </button>
            </form>

            {move || error.get().map(|e| view! { <div class="error-box" role="alert">{e}</div> })}
            {move || job.with(|j| j.as_ref().and_then(|j| st(j, "error"))).map(|e| view! { <div class="error-box" role="alert">"Scan failed: "{e}</div> })}

            {move || running.get().then(|| {
                let p = job.with(|j| j.as_ref().and_then(scan_progress));
                let live = job.with(|j| j.as_ref().map(live_progress).unwrap_or_default());
                let apps = live.apps.clone();
                view! {
                    <div class="scan-live" role="status" aria-live="polite">
                        <div class="scan-progress">
                            <progress max="1" value=p.map(|p| p.to_string()) aria-label="Scan progress"></progress>
                            <small class="muted">{live.phase_text()}</small>
                        </div>
                        <strong class="scan-counts">{live.counts_text()}</strong>
                        {(!apps.is_empty()).then(|| view! {
                            <ul class="scan-live-apps">
                                {apps.into_iter().map(|(key, name)| view! { <li><span>{name}</span><small class="muted">{key}</small></li> }).collect_view()}
                            </ul>
                        })}
                    </div>
                }
            })}

            {move || {
                if running.get() {
                    return None;
                }
                let diff = job.with(|j| j.as_ref().and_then(|j| j.get("diff")).filter(|d| d.is_object()).cloned())?;
                let list = |k: &str| diff.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
                let (appeared, gone, changed) = (list("appeared"), list("disappeared"), list("changed"));
                if appeared.len() + gone.len() + changed.len() == 0 {
                    return None;
                }
                let at = |c: &Value| {
                    let svc = c.get("service").cloned().unwrap_or(Value::Null);
                    let host = st(c, "hostname").or_else(|| st(c, "ip")).unwrap_or_default();
                    (svc.clone(), format!("{host}:{}", svc.get("port").and_then(Value::as_u64).unwrap_or(0)), format!("{}:{}", st(c, "ip").unwrap_or_default(), svc.get("port").and_then(Value::as_u64).unwrap_or(0)))
                };
                let all = found.get();
                Some(view! {
                    <section class="scan-results" aria-label="Changes since the last scan">
                        <h3>"Since the last scan"</h3>
                        <ul class="scan-list scan-diff">
                            {appeared.iter().map(|c| {
                                let (svc, place, key) = at(c);
                                let f = all.iter().find(|x| x.key == key).cloned();
                                let name = f.as_ref().map(|f| f.name.clone()).unwrap_or_else(|| service_name(&svc));
                                let addable = f.clone().filter(|f| f.suggestion.is_some() && !f.on_dashboard);
                                view! {
                                    <li class="appeared">
                                        <strong>{name}</strong>
                                        <small class="muted">"New on "{place}</small>
                                        {addable.map(|f| view! { <button type="button" class="btn sm" on:click=move |_| add(f.clone())>"+ Add to dashboard"</button> })}
                                    </li>
                                }
                            }).collect_view()}
                            {gone.iter().map(|c| {
                                let (svc, place, _) = at(c);
                                let carded = svc.get("on_dashboard").and_then(Value::as_bool).unwrap_or(false);
                                view! {
                                    <li class="disappeared">
                                        <strong>{service_name(&svc)}</strong>
                                        <small class="muted">"No longer answers on "{place}{carded.then_some(" (it has a card on the dashboard)")}</small>
                                    </li>
                                }
                            }).collect_view()}
                            {changed.iter().map(|c| {
                                let (svc, place, _) = at(c);
                                let ver = svc.get("app").and_then(|a| st(a, "version")).map(|v| format!(" (now v{v})"));
                                view! {
                                    <li>
                                        <strong>{service_name(&svc)}</strong>
                                        <small class="muted">"Changed on "{place}{ver}</small>
                                    </li>
                                }
                            }).collect_view()}
                        </ul>
                    </section>
                })
            }}

            {move || {
                let has_report = job.with(|j| j.as_ref().and_then(|j| j.get("report")).is_some_and(Value::is_object));
                (has_report && !running.get()).then(|| {
                    let all = found.get();
                    let known = all.iter().filter(|f| f.on_dashboard).count();
                    let n = all.len();
                    view! {
                        <section class="scan-results" aria-label="Scan results">
                            <h3>
                                {format!("Found {n} open {}", if n == 1 { "port" } else { "ports" })}
                                {(known > 0).then(|| view! { <small class="muted">" · "{known}" already on the dashboard"</small> })}
                            </h3>
                            <label class="row check-row">
                                <input type="checkbox" bind:checked=hide_known/>
                                <span>"Hide ones already on the dashboard"</span>
                            </label>
                            {move || {
                                let gs = groups.get();
                                (!gs.is_empty()).then(|| view! {
                                    <label class="field">
                                        <span>"Add to group"</span>
                                        <select class="input" prop:value=move || { let g = group_id.get(); if g.is_empty() { groups.with(|gs| gs.first().map(|g| g.0.clone()).unwrap_or_default()) } else { g } }
                                            on:change=move |e| group_id.set(event_target_value(&e))>
                                            {gs.into_iter().map(|(id, name)| view! { <option value=id>{name}</option> }).collect_view()}
                                        </select>
                                    </label>
                                })
                            }}
                            {move || {
                                let shown: Vec<Found> = found.get().into_iter().filter(|f| !hide_known.get() || !f.on_dashboard).collect();
                                if shown.is_empty() {
                                    let sb = scanned_bare.get();
                                    let hint = (!sb.is_empty()).then(|| {
                                        let (text, button) = if sb.len() == 1 {
                                            (format!("Only the single address {} was scanned. The devices are likely elsewhere on its network.", sb[0]), format!("Scan {}/24", sb[0]))
                                        } else {
                                            (format!("Only the single addresses {} were scanned. The devices are likely elsewhere on their networks.", sb.join(", ")), "Scan the /24 ranges".to_string())
                                        };
                                        view! {
                                            <div class="scan-hint" role="status">
                                                <span>{text}</span>
                                                <button type="button" class="btn sm" disabled=move || running.get() on:click=move |_| rescan_widened()>{button}</button>
                                            </div>
                                        }
                                    });
                                    return view! { <p class="muted small">"Nothing new found."</p>{hint} }.into_any();
                                }
                                view! {
                                    <ul class="scan-list">
                                        {shown.into_iter().map(|f| {
                                            let label = format!("Add {} on {}:{} to the dashboard", f.name, f.host, f.port);
                                            let place = format!("{}:{}", f.host, f.port);
                                            let fa = f.clone();
                                            view! {
                                                <li>
                                                    <div class="scan-item-main">
                                                        <strong>{f.name.clone()}</strong>
                                                        {f.widget.clone().map(|w| view! { <span class="badge">"widget: "{w}</span> })}
                                                        {f.on_dashboard.then(|| view! { <span class="badge">"on dashboard"</span> })}
                                                    </div>
                                                    <small class="muted">
                                                        {match f.href.clone() {
                                                            Some(h) => { let t = h.clone(); view! { <a href=h target="_blank" rel="noopener noreferrer">{t}</a> }.into_any() }
                                                            None => place.into_any(),
                                                        }}
                                                        {f.app_id.is_none().then_some(" · not recognised")}
                                                    </small>
                                                    {f.suggestion.is_some().then(|| view! {
                                                        <button type="button" class="btn sm" aria-label=label on:click=move |_| add(fa.clone())>"+ Add to dashboard"</button>
                                                    })}
                                                </li>
                                            }
                                        }).collect_view()}
                                    </ul>
                                }
                                .into_any()
                            }}
                        </section>
                    }
                })
            }}

            {move || schedule.get().map(|s| {
                let enabled = s.get("enabled").and_then(Value::as_bool).unwrap_or(false);
                let hours = s.get("intervalHours").and_then(Value::as_u64).unwrap_or(24);
                let mut choices = vec![1, 6, 12, 24, 72, 168, 720, hours];
                choices.sort_unstable();
                choices.dedup();
                let ran = s.get("lastRunAt").and_then(Value::as_f64).is_some_and(|l| l > 0.0);
                view! {
                    <section class="scan-schedule" aria-label="Rescan schedule">
                        <h3>"Rescan automatically"</h3>
                        <p class="muted small">"Rescans use the networks and ports above and tell admins what appeared or disappeared."</p>
                        <label class="row check-row">
                            <input type="checkbox" prop:checked=enabled on:change=move |e| set_schedule("enabled", json!(event_target_checked(&e)))/>
                            <span>"Rescan on a schedule"</span>
                        </label>
                        <label class="field">
                            <span>"Every"</span>
                            <select class="input" prop:value=hours.to_string()
                                on:change=move |e| set_schedule("intervalHours", json!(event_target_value(&e).parse::<u64>().unwrap_or(24)))>
                                {choices.into_iter().map(|h| view! { <option value=h.to_string()>{interval_label(h)}</option> }).collect_view()}
                            </select>
                        </label>
                        {(ran || enabled).then(|| view! { <p class="muted small">{schedule_times(&s, schedule_at.get_untracked(), local_time)}</p> })}
                        <button type="button" class="btn" disabled=move || enabled && !has_targets.get() on:click=move |_| save_schedule()>"Save schedule"</button>
                        {move || schedule_msg.get().map(|m| view! { <p class="muted small" role="status">{m}</p> })}
                    </section>
                }
            })}

            <div class="drawer-footer row">
                <button type="button" class="btn" on:click=move |_| a.target.set(None)>"Close"</button>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_and_bare_networks() {
        assert_eq!(
            parse_targets(" 10.0.0.0/24 ,\nnas\n\n"),
            vec!["10.0.0.0/24", "nas"]
        );
        assert_eq!(
            bare_networks("10.0.1.0\n10.0.2.0/24, 300.1.1.0, 10.0.3.5"),
            vec!["10.0.1.0"]
        );
        assert_eq!(
            widen_bare_networks("10.0.1.0\n192.168.5.0, nas"),
            "10.0.1.0/24\n192.168.5.0/24, nas"
        );
    }

    #[test]
    fn widen_questions() {
        assert_eq!(widen_question(&["10.0.1.0".into()]).1, "Scan 10.0.1.0/24");
        assert!(
            widen_question(&["10.0.1.0".into(), "10.0.2.0".into()])
                .0
                .contains("10.0.1.0, 10.0.2.0 are network addresses")
        );
    }

    #[test]
    fn intervals() {
        assert_eq!(interval_label(1), "hour");
        assert_eq!(interval_label(6), "6 hours");
        assert_eq!(interval_label(24), "day");
        assert_eq!(interval_label(72), "3 days");
        assert_eq!(interval_label(168), "week");
        assert_eq!(interval_label(720), "30 days");
    }

    #[test]
    fn schedule_words() {
        let w = |ms: f64| format!("@{ms}");
        assert_eq!(
            schedule_times(&json!({"enabled": true, "nextRunAt": 0}), 5.0, w),
            "Next run within a minute."
        );
        assert_eq!(
            schedule_times(
                &json!({"enabled": true, "lastRunAt": 1, "nextRunAt": 9}),
                5.0,
                w
            ),
            "Last run @1. Next run @9."
        );
        assert_eq!(
            schedule_times(&json!({"enabled": false, "nextRunAt": 9}), 5.0, w),
            ""
        );
    }

    #[test]
    fn found_and_progress() {
        let job = json!({
            "running": false,
            "events": [{"event": "started"}, {"event": "progress", "done": 1, "total": 4}],
            "report": {"hosts": [
                {"ip": "10.0.0.10", "services": [{"port": 80, "on_dashboard": false, "http": {"title": "Box"}}]},
                {"ip": "10.0.0.9", "hostname": "nas", "services": [
                    {"port": 8989, "on_dashboard": true, "app": {"id": "sonarr", "name": "Sonarr", "widget": "sonarr"}}
                ]}
            ]},
            "suggestions": {"10.0.0.9:8989": {"name": "Sonarr", "href": "http://nas:8989",
                "widget": {"type": "sonarr", "url": "http://nas:8989", "nested": {"x": 1}}, "favicon": "data:x"}}
        });
        let f = found_services(&job);
        assert_eq!(
            f.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
            ["Sonarr", "Box"]
        );
        assert_eq!((f[0].host.as_str(), f[0].on_dashboard), ("nas", true));
        assert_eq!(scan_progress(&job), Some(0.25));
        let s = suggestion_to_service(f[0].suggestion.as_ref().unwrap(), "s1".into());
        assert_eq!(s.icon.as_deref(), Some("data:x"));
        let w = s.widget.unwrap();
        assert_eq!((w.kind.as_str(), w.fields.len()), ("sonarr", 1));
        assert_eq!(ip_order("10.0.0.9", "10.0.0.10"), std::cmp::Ordering::Less);
    }

    #[test]
    fn live_hosts_and_apps() {
        let job = json!({"running": true, "events": [
            {"event": "started"},
            {"event": "service", "ip": "10.0.0.5", "service": {"port": 8096, "app": {"name": "Jellyfin"}, "source": "docker"}},
            {"event": "progress", "phase": "discover", "done": 120, "total": 254, "hosts": 2},
            {"event": "host_up", "ip": "10.0.0.5"},
            {"event": "host_up", "ip": "10.0.0.7", "name": "nas.lan"},
            {"event": "host_up", "ip": "10.0.0.9"},
            {"event": "service", "ip": "10.0.0.7", "service": {"port": 22, "http": null}},
            {"event": "service", "ip": "10.0.0.7", "service": {"port": 8989, "http": {"title": "Sonarr"}}},
            {"event": "service", "ip": "10.0.0.5", "service": {"port": 8096, "app": {"name": "Jellyfin 10.9"}}}
        ]});
        let live = live_progress(&job);
        assert!(live.discovering);
        assert_eq!(live.phase_text(), "Finding hosts: 120 of 254 checked");
        assert_eq!(live.counts_text(), "3 hosts · 2 apps");
        assert_eq!(
            live.apps,
            vec![
                ("10.0.0.5:8096".to_string(), "Jellyfin 10.9".to_string()),
                ("10.0.0.7:8989".to_string(), "Sonarr".to_string())
            ]
        );
        let ports = json!({"events": [{"event": "progress", "phase": "ports", "done": 50, "total": 200, "hosts": 1}]});
        let live = live_progress(&ports);
        assert_eq!(live.phase_text(), "Checking ports: 25%");
        assert_eq!(live.counts_text(), "1 host · 0 apps");
        // Unidentified services are named after the app usually on the port.
        assert_eq!(
            service_name(&json!({"port": 8989, "guess": "Sonarr"})),
            "Sonarr"
        );
    }

    #[test]
    fn default_targets_from_networks() {
        let nets = json!([
            {"cidr": "10.0.1.0/24", "interface": "eth0", "address": "10.0.1.5", "docker": false},
            {"cidr": "172.18.0.0/24", "interface": "br-1", "address": "172.18.0.1", "docker": true}
        ]);
        assert_eq!(network_targets(&nets), ["10.0.1.0/24", "172.18.0.0/24"]);
        assert_eq!(
            networks_text(&nets).unwrap(),
            "This server is on 10.0.1.0/24 (eth0), 172.18.0.0/24 (Docker)."
        );
        assert_eq!(networks_text(&Value::Null), None);
    }
}
