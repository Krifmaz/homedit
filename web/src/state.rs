//! The app's state: the session, the dashboard, live statuses and the editor (undo and redo, saving, the open
//! form). One [`App`] of signals, given to every component through Leptos' context ([`app`]).

use crate::client;
use crate::model::{Doc, Me, StatusMap, apply_status_event};
use crate::undo::Undo;
use kit_leptos::api::ApiError;
use kit_leptos::{rt, toasts};
use leptos::prelude::*;
use serde_json::Value;

/// What the editor drawer shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// A card in a group (none: a new card).
    Service {
        group: String,
        service: Option<String>,
    },
    /// A group (none: a new one).
    Group { group: Option<String> },
    /// A bookmark list.
    BookmarkGroup { group: Option<String> },
    /// A bookmark in a list.
    Bookmark {
        group: String,
        bookmark: Option<String>,
    },
    /// The info widgets.
    InfoWidgets,
    /// Appearance.
    Appearance,
    /// Version history.
    History,
    /// Accounts.
    Users,
    /// Find services.
    Scan,
    /// Import and export.
    Backup,
    /// Change password.
    Password,
}

impl Target {
    /// Whether it edits one item of the document (closed after a save, whose ids may change).
    pub fn is_item(&self) -> bool {
        matches!(
            self,
            Target::Service { .. }
                | Target::Group { .. }
                | Target::BookmarkGroup { .. }
                | Target::Bookmark { .. }
        )
    }
}

/// Every signal the components read.
#[derive(Clone, Copy)]
pub struct App {
    /// The session, `None` until loaded.
    pub me: RwSignal<Option<Me>>,
    /// What visitors see (`/api/dashboard`).
    pub view: RwSignal<Option<Doc>>,
    /// Live statuses.
    pub status: RwSignal<StatusMap>,
    /// The widget catalogue, loaded when editing starts.
    pub catalog: RwSignal<Vec<Value>>,
    /// Whether edit mode is on.
    pub editing: RwSignal<bool>,
    /// The editor's undo stacks around the working copy.
    pub undo: RwSignal<Undo>,
    /// Whether a save is running.
    pub saving: RwSignal<bool>,
    /// The drawer's form.
    pub target: RwSignal<Option<Target>>,
    /// The server answered 401: sign in.
    pub unauthorized: RwSignal<bool>,
    /// The sign-in page was asked for while browsing anonymously.
    pub want_login: RwSignal<bool>,
    /// The search box.
    pub filter: RwSignal<String>,
}

/// The app's state.
pub fn app() -> App {
    expect_context::<App>()
}

impl App {
    /// New state, with what the server put in the page.
    pub fn new(initial: Option<(Doc, StatusMap)>) -> App {
        let (doc, status) = initial.map(|(d, s)| (Some(d), s)).unwrap_or_default();
        App {
            me: RwSignal::new(None),
            undo: RwSignal::new(Undo::new(doc.clone().unwrap_or_default())),
            view: RwSignal::new(doc),
            status: RwSignal::new(status),
            catalog: RwSignal::new(Vec::new()),
            editing: RwSignal::new(false),
            saving: RwSignal::new(false),
            target: RwSignal::new(None),
            unauthorized: RwSignal::new(false),
            want_login: RwSignal::new(false),
            filter: RwSignal::new(String::new()),
        }
    }

    /// The document on screen: the working copy while editing.
    pub fn doc(&self) -> Option<Doc> {
        if self.editing.get() {
            Some(self.undo.with(|u| u.present.clone()))
        } else {
            self.view.get()
        }
    }

    /// Whether the working copy has unsaved changes.
    pub fn dirty(&self) -> bool {
        self.editing.get() && self.undo.with(Undo::dirty)
    }

    /// Whether this visitor may edit.
    pub fn is_admin(&self) -> bool {
        self.me.with(|m| m.as_ref().is_some_and(Me::is_admin))
    }

    /// Whether the sign-in (or first-run setup) page shows.
    pub fn needs_login(&self) -> bool {
        let me = self.me.get();
        self.want_login.get()
            || self.unauthorized.get()
            || me
                .as_ref()
                .is_some_and(|m| m.setup_required || (m.auth_enabled && m.role.is_none()))
    }

    /// Whether the sign-in page was opened by choice and can be left.
    pub fn login_optional(&self) -> bool {
        let me = self.me.get();
        self.want_login.get()
            && !self.unauthorized.get()
            && me
                .as_ref()
                .is_some_and(|m| !m.setup_required && m.role.is_some())
    }

    /// Replaces the working copy (one undo step, or the same as the last edit with the same `key`).
    pub fn apply(&self, next: Doc, key: Option<&str>) {
        self.undo.update(|u| u.apply(next, key));
    }

    /// Applies `f` to the working copy.
    pub fn edit(&self, key: Option<&str>, f: impl FnOnce(&Doc) -> Doc) {
        let next = self.undo.with_untracked(|u| f(&u.present));
        self.apply(next, key);
    }

    /// Shows a failed call: a 401 sends to sign-in, a conflict offers Reload, the rest is an error toast with
    /// Copy.
    pub fn fail(&self, e: &ApiError) {
        if e.status == 401 {
            self.unauthorized.set(true);
        } else if client::is_conflict(e) && self.editing.get_untracked() {
            let me = *self;
            toasts::toast_action(
                "The configuration was changed elsewhere. Reload to get the latest version.",
                "Reload",
                30_000,
                move || me.reload(),
            );
        } else {
            toasts::toast(&e.message, "err");
        }
    }

    /// Loads the session, the dashboard and the statuses, and opens the live events.
    pub fn load(&self) {
        let me = *self;
        rt::spawn(async move {
            match client::me().await {
                Ok(m) => {
                    let admin = m.is_admin();
                    let signed_in = m.user.is_some() || !m.auth_enabled;
                    me.me.set(Some(m));
                    crate::kit_start(admin, signed_in);
                }
                Err(e) => me.fail(&e),
            }
        });
        rt::spawn(async move {
            match client::dashboard().await {
                Ok(d) => {
                    me.view.set(Some(d));
                    me.unauthorized.set(false);
                }
                Err(e) => me.fail(&e),
            }
        });
        rt::spawn(async move {
            if let Ok(s) = client::status().await {
                me.status.set(s);
            }
        });
        client::subscribe(client::Handlers {
            status: Box::new(move |v| {
                let now = js_sys::Date::now();
                me.status.update(|s| {
                    apply_status_event(s, &v, now);
                });
            }),
            config: Box::new(move |_| {
                rt::spawn(async move {
                    if let Ok(d) = client::dashboard().await {
                        me.view.set(Some(d));
                    }
                });
            }),
            changes: Box::new(move |msgs| me.scan_changes(msgs)),
        });
    }

    /// A rescan found changes: a toast with the first sentences, Review opens Find services for admins.
    fn scan_changes(&self, msgs: Vec<String>) {
        if self.target.get_untracked() == Some(Target::Scan) {
            return;
        }
        let text = crate::fmt::changes_text(&msgs);
        if self.is_admin() {
            let me = *self;
            toasts::toast_action(&text, "Review", 30_000, move || me.open_scan());
        } else {
            toasts::toast(&text, "info");
        }
    }

    /// Opens Find services, starting edit mode first.
    pub fn open_scan(&self) {
        let me = *self;
        if self.editing.get_untracked() {
            self.target.set(Some(Target::Scan));
        } else {
            rt::spawn(async move {
                if me.start_editing().await {
                    me.target.set(Some(Target::Scan));
                }
            });
        }
    }

    /// Signs in again from the start (after signing in or out).
    pub fn refresh_session(&self) {
        self.editing.set(false);
        self.target.set(None);
        self.want_login.set(false);
        self.unauthorized.set(false);
        self.load();
    }

    /// Signs out: to the proxy's sign-out page, or back to the anonymous view or sign-in.
    pub fn logout(&self) {
        let me = *self;
        rt::spawn(async move {
            if !kit_leptos::modal::confirm(crate::words::sign_out_question()).await {
                return;
            }
            match client::logout().await {
                Ok(Some(url)) => {
                    if let Some(w) = rt::window() {
                        let _ = w.location().assign(&url);
                    }
                }
                Ok(None) => {
                    me.view.set(None);
                    me.me.set(None);
                    me.refresh_session();
                }
                Err(e) => me.fail(&e),
            }
        });
    }

    /// Starts edit mode with the editable document and the widget catalogue. False when it failed.
    pub async fn start_editing(&self) -> bool {
        let (doc, cat) = (client::config().await, client::catalog().await);
        match (doc, cat) {
            (Ok(d), Ok(c)) => {
                self.undo.set(Undo::new(d));
                self.catalog.set(c);
                self.editing.set(true);
                true
            }
            (Err(e), _) | (_, Err(e)) => {
                self.fail(&e);
                false
            }
        }
    }

    /// Leaves edit mode (asking first when there are unsaved changes).
    pub fn stop_editing(&self) {
        let me = *self;
        rt::spawn(async move {
            if me.dirty() && !kit_leptos::modal::confirm(crate::words::discard_question()).await {
                return;
            }
            me.editing.set(false);
            me.target.set(None);
        });
    }

    /// Loads the editable document again (after a conflict or a restore).
    pub fn reload(&self) {
        let me = *self;
        rt::spawn(async move {
            match client::config().await {
                Ok(d) => me.undo.set(Undo::new(d)),
                Err(e) => return me.fail(&e),
            }
            if let Ok(v) = client::dashboard().await {
                me.view.set(Some(v));
            }
        });
    }

    /// Saves the working copy, then picks up the ids the server gave new items (undo history is kept).
    pub fn save(&self) {
        if self.saving.get_untracked() || !self.undo.with_untracked(Undo::dirty) {
            return;
        }
        let me = *self;
        self.saving.set(true);
        rt::spawn(async move {
            let doc = me.undo.with_untracked(|u| u.present.clone());
            match client::save_config(&doc).await {
                Ok(v) => {
                    me.undo.update(|u| u.saved_as(&v));
                    if let (Ok(fresh), Ok(view)) =
                        (client::config().await, client::dashboard().await)
                    {
                        me.undo.update(|u| u.synced(fresh));
                        me.view.set(Some(view));
                    }
                    if me.target.get_untracked().is_some_and(|t| t.is_item()) {
                        me.target.set(None);
                    }
                    toasts::toast("Saved", "ok");
                }
                Err(e) => me.fail(&e),
            }
            me.saving.set(false);
        });
    }
}
