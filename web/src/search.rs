//! The search box: filters the cards as you type; Enter searches the web, or opens the card when exactly one
//! matches. `/` anywhere focuses it, Escape clears it.

use crate::ops;
use crate::state::app;
use kit_leptos::rt;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

/// The search box.
#[component]
pub fn SearchBar() -> impl IntoView {
    let a = app();
    let input = NodeRef::<leptos::html::Input>::new();
    if let Some(w) = rt::window() {
        crate::shell::scoped(vec![rt::listen(&w, "keydown", move |e| {
            let Some(k) = e.dyn_ref::<web_sys::KeyboardEvent>() else {
                return;
            };
            if k.key() != "/"
                || rt::closest(e, "input, textarea, select, [contenteditable]").is_some()
            {
                return;
            }
            e.prevent_default();
            if let Some(i) = input.get_untracked() {
                let _ = i.focus();
            }
        })]);
    }
    let submit = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        let q = a.filter.get_untracked().trim().to_string();
        let Some(doc) = a.doc() else { return };
        if q.is_empty() {
            return;
        }
        let hits: Vec<Option<String>> = ops::all_groups(&ops::filter_groups(&doc.groups, &q))
            .into_iter()
            .flat_map(|g| g.items.iter().map(|s| s.href.clone()))
            .collect();
        let url = match hits.as_slice() {
            [Some(h)] if !h.is_empty() => h.clone(),
            _ => doc.settings.search.url_for(&q),
        };
        if let Some(w) = rt::window() {
            let _ = w.open_with_url_and_target_and_features(&url, "_blank", "noopener,noreferrer");
        }
    };
    view! {
        <form class="search dash-search" role="search" on:submit=submit>
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true">
                <circle cx="11" cy="11" r="7"></circle><path d="m20 20-3.5-3.5"></path>
            </svg>
            <input class="input" type="search" node_ref=input aria-label="Search services or the web"
                placeholder="Search services or the web  ( / )"
                prop:value=move || a.filter.get()
                on:input=move |e| a.filter.set(event_target_value(&e))
                on:keydown=move |e| if e.key() == "Escape" { a.filter.set(String::new()) }/>
        </form>
    }
}
