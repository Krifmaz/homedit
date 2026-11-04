//! The side drawer that holds the open form or panel. Forms edit the working copy directly, so the dashboard
//! behind shows each change; a new target starts a fresh form (and its own undo step).

use crate::appearance::{AppearanceForm, InfoWidgetsForm};
use crate::backup::BackupPanel;
use crate::forms::{BookmarkForm, BookmarkGroupForm, GroupForm, ServiceForm};
use crate::panels::{HistoryPanel, PasswordForm, UsersPanel};
use crate::scan::ScanPanel;
use crate::state::{Target, app};
use leptos::prelude::*;

/// The drawer, when something is open.
#[component]
pub fn Drawer() -> impl IntoView {
    let a = app();
    // Only a different target rebuilds the form; edits behind it don't.
    let target = Memo::new(move |_| a.target.get());
    move || {
        let t = target.get()?;
        let body = match t {
            Target::Service { group, service } => {
                view! { <ServiceForm group=group service=service/> }.into_any()
            }
            Target::Group { group } => view! { <GroupForm group=group/> }.into_any(),
            Target::BookmarkGroup { group } => {
                view! { <BookmarkGroupForm group=group/> }.into_any()
            }
            Target::Bookmark { group, bookmark } => {
                view! { <BookmarkForm group=group bookmark=bookmark/> }.into_any()
            }
            Target::InfoWidgets => view! { <InfoWidgetsForm/> }.into_any(),
            Target::Appearance => view! { <AppearanceForm/> }.into_any(),
            Target::History => view! { <HistoryPanel/> }.into_any(),
            Target::Users => view! { <UsersPanel/> }.into_any(),
            Target::Scan => view! { <ScanPanel/> }.into_any(),
            Target::Backup => view! { <BackupPanel/> }.into_any(),
            Target::Password => view! { <PasswordForm/> }.into_any(),
        };
        Some(view! {
            <aside class="drawer card" role="dialog" aria-modal="false" aria-label="Editor">
                <div class="drawer-body">{body}</div>
            </aside>
        })
    }
}
