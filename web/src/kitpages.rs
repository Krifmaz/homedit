//! The kit's pages: `#/settings?s=<section>` (the server's settings from its schema, Diagnostics, What's new)
//! and `#/bug-reports` (the account's reports and New report). Their words and markup are the kit's.

use crate::shell::{Route, navigate, s};
use crate::state::app;
use kit_leptos::api;
use kit_leptos::bug_report::{self, SheetOpts};
use kit_leptos::diagnostics::DiagnosticsPage;
use kit_leptos::icons::icon;
use kit_leptos::schema_form::SettingsCards;
use kit_leptos::schema_form::schema::{Schema, visible_cards};
use kit_leptos::whats_new::ChangelogPage;
use leptos::prelude::*;
use serde_json::Value;
use std::rc::Rc;

/// The app's name as people see it.
pub const APP_NAME: &str = "homedit";

/// One of the kit's pages, under a bar back to the dashboard.
#[component]
pub fn KitPage(route: Route) -> impl IntoView {
    let a = app();
    // The page waits for the session: what it shows depends on the role.
    let role = Memo::new(move |_| a.me.with(|m| m.as_ref().map(|m| m.is_admin())));
    let page = move || {
        role.get()?;
        Some(match route.path.as_str() {
            "/bug-reports" => view! { <BugReports/> }.into_any(),
            _ => view! { <Settings page=route.param("s")/> }.into_any(),
        })
    };
    Effect::new(|_| kit_leptos::notice::mount());
    view! {
        <div class="app kit-page">
            <div id="server-notice"></div>
            <header class="header">
                <a class="btn ghost" href="#/">"← Dashboard"</a>
            </header>
            <main>{page}</main>
        </div>
    }
}

/// Opens the New report sheet, with the server's version.
pub fn new_report(from: &'static str) {
    kit_leptos::rt::spawn(async move {
        let version = api::get("/health")
            .await
            .ok()
            .map(|h| s(&h, "version"))
            .unwrap_or_default();
        let mut o = SheetOpts::new(APP_NAME, from, &version);
        o.on_sent = Some(Rc::new(|_| {
            navigate("/bug-reports");
            kit_leptos::rt::dispatch("homedit:bugs-changed");
        }));
        bug_report::open(o);
    });
}

fn report_row(r: &Value) -> impl IntoView + use<> {
    let n = r.get("message_count").and_then(Value::as_i64).unwrap_or(0);
    let count = match n {
        0 => "No replies".to_string(),
        1 => "1 reply".to_string(),
        n => format!("{n} replies"),
    };
    view! {
        <div class="list-item bug-row" style="flex-wrap:wrap;gap:10px">
            <div class="grow" style="min-width:200px">
                <span class="bug-title" style="font-weight:650">{s(r, "title")}</span>
                <div class="small faint">{format!("by {}", s(r, "username"))}</div>
            </div>
            <span class="badge">{s(r, "status")}</span>
            <span class="small muted bug-count">{count}</span>
        </div>
    }
}

/// `#/bug-reports`.
#[component]
fn BugReports() -> impl IntoView {
    let (tick, set_tick) = signal(0u32);
    kit_leptos::rt::on_window("homedit:bugs-changed", move |_| {
        set_tick.update(|t| *t += 1)
    });
    let list = LocalResource::new(move || {
        tick.track();
        async { api::get("/bug-reports").await.map_err(|e| e.message) }
    });
    let rows = move || match list.get() {
        None => view! { <div class="empty"><span class="spinner"></span></div> }.into_any(),
        Some(Err(e)) => view! { <div class="error-box">{e}</div> }.into_any(),
        Some(Ok(v)) => {
            let reports = v
                .get("reports")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if reports.is_empty() {
                view! { <div class="empty" inner_html=format!(r#"{}<p class="muted">No reports yet. New report writes the first one.</p>"#, icon("bug"))></div> }.into_any()
            } else {
                view! { <div class="list">{reports.iter().map(report_row).collect_view()}</div> }
                    .into_any()
            }
        }
    };
    view! {
        <div class="row wrap between" style="margin-bottom:18px;gap:10px">
            <h1 style="margin:0">"Bug reports"</h1>
            <button class="btn primary" on:click=|_| new_report("#/bug-reports") inner_html=format!("{}New report", icon("plus"))></button>
        </div>
        <div class="card flat mt">{rows}</div>
    }
}

/// `#/settings?s=<page>`: admins get the server's settings and Diagnostics, everyone What's new.
#[component]
fn Settings(page: String) -> impl IntoView {
    let a = app();
    let admin = a.is_admin();
    let data = LocalResource::new(move || async move {
        if !admin {
            return Err(String::new());
        }
        let schema = api::get("/settings/schema").await.map_err(|e| e.message)?;
        let values = api::get("/settings").await.map_err(|e| e.message)?;
        Ok::<_, String>((Schema::from_json(&schema), values))
    });
    let page = if page.is_empty() {
        if admin {
            "signin".to_string()
        } else {
            "whats-new".to_string()
        }
    } else {
        page
    };
    let tabs = {
        let page = page.clone();
        move || {
            let sections: Vec<(String, String)> = match data.get() {
                Some(Ok((schema, _))) => schema
                    .sections
                    .iter()
                    .map(|s| (s.id.clone(), s.title.clone()))
                    .collect(),
                _ => vec![],
            };
            let mut fixed = vec![("whats-new".to_string(), "What's new".to_string())];
            if admin {
                fixed.insert(0, ("diagnostics".to_string(), "Diagnostics".to_string()));
            }
            sections
                .into_iter()
                .chain(fixed)
                .map(|(id, title)| {
                    let on = id == page;
                    view! { <a href=format!("#/settings?s={id}") class:active=on>{title}</a> }
                })
                .collect_view()
        }
    };
    let body = {
        let page = page.clone();
        move || {
            match page.as_str() {
            "diagnostics" => view! { <DiagnosticsPage app=APP_NAME.to_string()/> }.into_any(),
            "whats-new" => view! { <ChangelogPage app=APP_NAME.to_string()/> }.into_any(),
            id => match data.get() {
                None => view! { <div class="empty"><span class="spinner"></span></div> }.into_any(),
                Some(Err(e)) => view! { <div class="error-box">{if e.is_empty() { "Only admins can change the settings.".to_string() } else { e }}</div> }.into_any(),
                Some(Ok((schema, values))) => {
                    let cards = schema.section(id).map(|s| visible_cards(&s.groups, &values)).unwrap_or_default();
                    view! { <SettingsCards cards=cards values=values/> }.into_any()
                }
            },
        }
        }
    };
    view! {
        <h1>"Settings"</h1>
        <div class="tabs">{tabs}</div>
        <div class="mt">{body}</div>
    }
}
