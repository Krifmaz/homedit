//! homedit's web app: the dashboard, its editor, and the kit's pages, in Leptos (client-side, WebAssembly).
//! The server serves `index.html` with the dashboard inline (`#homedit-data`), so the first paint needs no
//! call; everything after is `/api`.

mod appearance;
mod backup;
mod client;
mod dash;
mod dnd;
mod drawer;
mod fmt;
mod forms;
mod icons;
mod kitpages;
mod login;
mod model;
mod ops;
mod panels;
mod scan;
mod search;
mod shell;
mod state;
mod theme;
mod undo;
mod widgets;
mod words;

use kit_leptos::config::{self, AppConfig};
use kit_leptos::notice::{self, NoticeSource};
use kit_rules::app::App as KitApp;
use kit_rules::notices::CheckWords;
use leptos::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

thread_local! {
    static STARTED: Cell<bool> = const { Cell::new(false) };
}

/// The kit's parts that need the session: notices for admins, What's new once signed in. Runs on every load of
/// the session; What's new asks once per page.
pub fn kit_start(admin: bool, signed_in: bool) {
    let source = NoticeSource::default()
        .words_for(
            "data",
            CheckWords {
                failing: "The data folder still can't be written",
                checking: "Still checking the data folder",
                fixed: "The data folder is writable again",
            },
        )
        .words_for(
            "docker",
            CheckWords {
                failing: "Docker still doesn't answer",
                checking: "Still checking Docker",
                fixed: "Docker answers again",
            },
        );
    notice::start(source, admin);
    if signed_in && !STARTED.with(|s| s.replace(true)) {
        kit_leptos::whats_new::start(Rc::new(shell::navigate));
    }
}

fn main() {
    config::set(AppConfig::new(
        KitApp::named(kitpages::APP_NAME).with_version(env!("CARGO_PKG_VERSION")),
    ));
    kit_leptos::api::set_backend(client::backend());
    kit_leptos::crash::install();
    kit_leptos::toasts::mount();
    kit_leptos::details::init();
    kit_leptos::dropzone::init();
    kit_leptos::offline::watch_connection();
    let a = state::App::new(client::inline_data());
    kit_leptos::update::start(
        kit_leptos::update::UpdateConfig::from_config()
            .hold("It reloads when you leave edit mode", move || {
                a.editing.get_untracked()
            }),
    );
    let Some(root) = kit_leptos::rt::by_id("app") else {
        return;
    };
    let root: web_sys::HtmlElement = wasm_bindgen::JsCast::unchecked_into(root);
    let handle = leptos::mount::mount_to(root, move || {
        provide_context(a);
        shell::start(a);
        view! { <shell::Root/> }
    });
    handle.forget();
}
