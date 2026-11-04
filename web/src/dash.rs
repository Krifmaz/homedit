//! The dashboard: groups of service cards and the bookmark lists, editable in edit mode.

use crate::dnd::{self, BM, BMDROP, DROP, GRP, SVC};
use crate::model::{Bookmark, BookmarkGroup, Check, Doc, Group, Sample, Service, Status};
use crate::ops;
use crate::state::{App, Target, app};
use crate::{fmt, icons};
use kit_leptos::toasts;
use leptos::prelude::*;

/// An icon from a name or URL; the name's initials when there is none or it does not load.
#[component]
pub fn Icon(
    #[prop(into)] icon: Option<String>,
    #[prop(default = false)] mask: bool,
    #[prop(into)] name: String,
    #[prop(default = 36)] size: u32,
    #[prop(default = "card-icon")] class: &'static str,
) -> impl IntoView {
    let url = icon.as_deref().and_then(icons::url);
    let mask = mask || icon.as_deref().is_some_and(icons::is_mask);
    let failed = RwSignal::new(false);
    let style = format!("width:{size}px;height:{size}px;flex-basis:{size}px");
    move || {
        match url.clone() {
        Some(u) if !failed.get() => {
            if mask {
                let m = format!("url(\"{u}\") center / contain no-repeat");
                view! { <span class=class role="img" aria-label=name.clone()
                    style=format!("{style};background-color:currentColor;mask:{m};-webkit-mask:{m}")></span> }
                .into_any()
            } else {
                view! { <img class=class src=u alt="" width=size height=size loading="lazy" style=style.clone()
                    on:error=move |_| failed.set(true)/> }
                .into_any()
            }
        }
        _ => view! { <span class=format!("{class} fallback") aria-hidden="true"
            style=format!("{style};font-size:{}px", (f64::from(size) * 0.38).round())>{fmt::initials(&name)}</span> }
        .into_any(),
    }
    }
}

/// Recent response times, with down samples marked; nothing below two samples.
fn sparkline(samples: &[Sample]) -> impl IntoView + use<> {
    let (w, h) = (96.0, 22.0);
    let n = samples.len();
    (n >= 2).then(|| {
        let step = w / (n - 1) as f64;
        let up = samples.iter().filter(|s| s.up).count();
        let label = format!("Up {}% of the last {n} checks", (up as f64 / n as f64 * 100.0).round());
        let downs: Vec<f64> = samples.iter().enumerate().filter(|(_, s)| !s.up).map(|(i, _)| i as f64 * step - 1.0).collect();
        let path = fmt::spark_path(samples, w, h);
        view! {
            <svg class="sparkline" width=w height=h viewBox=format!("0 0 {w} {h}") role="img" aria-label=label.clone()>
                <title>{label.clone()}</title>
                {downs.into_iter().map(|x| view! { <rect x=x y="0" width="2" height=h fill="var(--err)" opacity="0.7"></rect> }).collect_view()}
                <path d=path fill="none" stroke="currentColor" stroke-width="1.5"></path>
            </svg>
        }
    })
}

/// The status dot, words and sparkline of a card with a check.
#[component]
fn StatusBadge(check: Check, id: String) -> impl IntoView {
    let a = app();
    let status = Memo::new(move |_| a.status.with(|m| m.get(&id).cloned()));
    move || {
        let s: Option<Status> = status.get();
        let text = fmt::status_text(&check, s.as_ref());
        let cls = match &s {
            Some(s) if s.up => "status-dot up",
            Some(_) => "status-dot down",
            None => "status-dot",
        };
        view! {
            <div class="status" title=format!("{} check: {text}", check.kind.to_uppercase())>
                <span class="status-line"><span class=cls aria-hidden="true"></span><span>{text.clone()}</span></span>
                {s.map(|s| sparkline(&s.history))}
            </div>
        }
    }
}

/// A service card: a link outside edit mode, with edit buttons inside it.
#[component]
fn Card(service: Service, group: String) -> impl IntoView {
    let a = app();
    let id = service.id.clone();
    let drag_id = format!("{SVC}:{id}");
    let body = {
        let s = service.clone();
        move || {
            let s = s.clone();
            view! {
                <div class="svc-main">
                    <Icon icon=s.icon.clone() mask=s.icon_mask name=s.name.clone()/>
                    <div class="svc-text">
                        <div class="svc-name">{s.name.clone()}</div>
                        {s.description.clone().filter(|d| !d.is_empty()).map(|d| view! { <div class="svc-desc">{d}</div> })}
                    </div>
                    {s.check.clone().map(|c| view! { <StatusBadge check=c id=s.id.clone()/> })}
                </div>
                {s.widget.is_some().then(|| view! { <crate::widgets::ServiceWidget id=s.id.clone()/> })}
            }
        }
    };
    let edit = {
        let (id, name, group) = (id.clone(), service.name.clone(), group.clone());
        move || {
            a.editing.get().then(|| {
                let (id2, group2) = (id.clone(), group.clone());
                let open = move |_| a.target.set(Some(Target::Service { group: group2.clone(), service: Some(id2.clone()) }));
                let del = {
                    let (id, name) = (id.clone(), name.clone());
                    move |_| delete_with_undo(a, &format!("{name} deleted"), |d| ops::delete_service(d, &id))
                };
                view! {
                    <div class="edit-actions" data-nodrag="">
                        <button type="button" class="btn ghost sm icon drag-handle" data-handle=format!("{SVC}:{id}")
                            aria-label=format!("Move {name}") title="Drag, or Alt and the arrow keys">"⠿"</button>
                        <button type="button" class="btn ghost sm icon" aria-label=format!("Edit {name}") on:click=open>"✎"</button>
                        <button type="button" class="btn ghost sm icon danger" aria-label=format!("Delete {name}") on:click=del>"✕"</button>
                    </div>
                }
            })
        }
    };
    let dragging = {
        let d = drag_id.clone();
        move || dnd::dragging().with(|x| x.as_deref() == Some(d.as_str()))
    };
    let href = service.href.clone().filter(|h| !h.is_empty());
    let name = service.name.clone();
    move || {
        if a.editing.get() {
            view! {
                <div class="svc" class:dragging=dragging.clone() data-drag=drag_id.clone() tabindex="0"
                    aria-roledescription="draggable card" aria-label=name.clone()>
                    {body()}
                    {edit.clone()}
                </div>
            }
            .into_any()
        } else if let Some(h) = href.clone() {
            view! { <a class="svc" href=h target="_blank" rel="noopener noreferrer">{body()}</a> }
                .into_any()
        } else {
            view! { <div class="svc">{body()}</div> }.into_any()
        }
    }
}

/// Applies a delete and offers Undo in a toast.
pub fn delete_with_undo(a: App, what: &str, f: impl FnOnce(&Doc) -> Doc) {
    a.edit(None, f);
    toasts::toast_action(what, "Undo", 6000, move || a.undo.update(|u| u.undo()));
}

/// A card's identity for keyed lists: a changed card is drawn again, an unchanged one keeps its live parts.
fn card_key(s: &Service) -> String {
    format!("{s:?}")
}

/// One group, its cards and its sub-groups.
#[component]
fn GroupSection(id: String, depth: u32, groups: Memo<Vec<Group>>) -> impl IntoView {
    let a = app();
    let gid = id.clone();
    let group = Memo::new(move |_| {
        groups.with(|gs| {
            ops::all_groups(gs)
                .into_iter()
                .find(|g| g.id == gid)
                .cloned()
        })
    });
    let head = Memo::new(move |_| group.get().map(|g| (g.name, g.icon, g.hidden, g.layout)));
    let items = move || group.with(|g| g.as_ref().map(|g| g.items.clone()).unwrap_or_default());
    let subs = move || {
        group.with(|g| {
            g.as_ref()
                .map(|g| g.groups.iter().map(|c| c.id.clone()).collect::<Vec<_>>())
                .unwrap_or_default()
        })
    };
    let drag = format!("{GRP}:{id}");
    let dragging = {
        let d = drag.clone();
        move || dnd::dragging().with(|x| x.as_deref() == Some(d.as_str()))
    };
    let (id1, id2, id3) = (id.clone(), id.clone(), id.clone());
    view! {
        <section class="group" class:is-hidden=move || head.get().is_some_and(|h| h.2) class:dragging=dragging
            data-drag=(depth == 0).then(|| drag.clone())>
            {move || head.get().map(|(name, icon, hidden, _)| {
                let id = id1.clone();
                let handle = format!("{GRP}:{id}");
                view! {
                    <div class="group-head">
                        {(a.editing.get() && depth == 0).then(|| view! {
                            <button type="button" class="btn ghost sm icon drag-handle" data-handle=handle.clone()
                                aria-label=format!("Move group {name}") title="Drag, or Alt and the arrow keys">"⠿"</button>
                        })}
                        {icon.filter(|i| !i.is_empty()).map(|i| view! { <Icon icon=Some(i) name=name.clone() size=20 class="group-icon"/> })}
                        {if depth == 0 { view! { <h2>{name.clone()}</h2> }.into_any() } else { view! { <h3>{name.clone()}</h3> }.into_any() }}
                        {hidden.then(|| view! { <span class="badge">"Hidden from viewers"</span> })}
                        {a.editing.get().then(|| {
                            let id = id.clone();
                            view! { <button type="button" class="btn ghost sm icon" aria-label=format!("Edit group {name}")
                                on:click=move |_| a.target.set(Some(Target::Group { group: Some(id.clone()) }))>"✎"</button> }
                        })}
                    </div>
                }
            })}
            <div class=move || format!("cards style-{}", head.get().map(|h| h.3.style.as_str()).unwrap_or("grid"))
                style=move || format!("--cols:{}", head.get().map(|h| h.3.columns.clamp(1, 8)).unwrap_or(3))
                data-drop=format!("{DROP}:{id}")>
                <For each=items key=card_key children=move |s| view! { <Card service=s group=id2.clone()/> }/>
                {move || a.editing.get().then(|| {
                    let id = id3.clone();
                    view! { <button type="button" class="add-tile" on:click=move |_| a.target.set(Some(Target::Service { group: id.clone(), service: None }))>"+ Add service"</button> }
                })}
            </div>
            {move || {
                let s = subs();
                (!s.is_empty()).then(|| view! {
                    <div class="subgroups">
                        <For each=move || subs() key=|g| g.clone() children=move |g| view! { <GroupSection id=g depth=depth + 1 groups=groups/> }.into_any()/>
                    </div>
                })
            }}
        </section>
    }
}

/// The groups and bookmarks.
#[component]
pub fn Dashboard() -> impl IntoView {
    let a = app();
    let doc = Memo::new(move |_| a.doc());
    let filtering = move || !a.filter.with(|f| f.trim().is_empty());
    let groups = Memo::new(move |_| {
        doc.with(|d| {
            d.as_ref()
                .map(|d| ops::filter_groups(&d.groups, &a.filter.get()))
                .unwrap_or_default()
        })
    });
    let top = move || groups.with(|g| g.iter().map(|g| g.id.clone()).collect::<Vec<_>>());
    let lists = Memo::new(move |_| {
        doc.with(|d| d.as_ref().map(|d| d.bookmarks.clone()).unwrap_or_default())
    });
    // Only loading (or not) rebuilds the page: edits update the groups in place.
    let loaded = Memo::new(move |_| doc.with(Option::is_some));
    move || {
        if !loaded.get() {
            return view! { <div class="empty"><span class="spinner"></span></div> }.into_any();
        }
        view! {
            <div class="groups">
                <For each=top key=|g| g.clone() children=move |g| view! { <GroupSection id=g depth=0 groups=groups/> }/>
                {move || a.editing.get().then(|| view! {
                    <button type="button" class="add-tile" on:click=move |_| a.target.set(Some(Target::Group { group: None }))>"+ Add group"</button>
                })}
                {move || (!a.editing.get() && filtering() && groups.with(Vec::is_empty)).then(|| view! {
                    <p class="muted small">"No cards match. Press Enter to search the web."</p>
                })}
                {move || (!a.editing.get() && !filtering() && groups.with(Vec::is_empty) && lists.with(Vec::is_empty)).then(|| view! {
                    <div class="empty"><p class="muted">"Nothing here yet."</p>
                        {a.is_admin().then(|| view! { <p class="muted small">"Press Edit to add groups and cards, or Find services to scan your network."</p> })}
                    </div>
                })}
            </div>
            {move || (!filtering()).then(|| view! { <Bookmarks lists=lists/> })}
        }
        .into_any()
    }
}

/// The bookmark lists under the groups.
#[component]
fn Bookmarks(lists: Memo<Vec<BookmarkGroup>>) -> impl IntoView {
    let a = app();
    let shown = Memo::new(move |_| !lists.with(Vec::is_empty) || a.editing.get());
    move || {
        if !shown.get() {
            return ().into_any();
        }
        view! {
            <div class="bookmarks">
                <For each=move || lists.get() key=|l| format!("{l:?}") children=move |l| view! { <BookmarkList list=l/> }/>
                {a.editing.get().then(|| view! {
                    <button type="button" class="add-tile" on:click=move |_| a.target.set(Some(Target::BookmarkGroup { group: None }))>"+ Add bookmark list"</button>
                })}
            </div>
        }
        .into_any()
    }
}

/// One bookmark list.
#[component]
fn BookmarkList(list: BookmarkGroup) -> impl IntoView {
    let a = app();
    let (lid, lid2) = (list.id.clone(), list.id.clone());
    let name = list.name.clone();
    view! {
        <section class="bookmark-group">
            <h3>
                {list.name.clone()}
                {move || a.editing.get().then(|| {
                    let id = lid.clone();
                    view! { <button type="button" class="btn ghost sm icon" aria-label=format!("Edit bookmark list {name}")
                        on:click=move |_| a.target.set(Some(Target::BookmarkGroup { group: Some(id.clone()) }))>"✎"</button> }
                })}
            </h3>
            <div data-drop=format!("{BMDROP}:{}", list.id)>
                {list.items.iter().cloned().map(|b| view! { <BookmarkItem bookmark=b list=lid2.clone()/> }).collect_view()}
                {move || a.editing.get().then(|| {
                    let id = lid2.clone();
                    view! { <button type="button" class="add-tile small-tile"
                        on:click=move |_| a.target.set(Some(Target::Bookmark { group: id.clone(), bookmark: None }))>"+ Add bookmark"</button> }
                })}
            </div>
        </section>
    }
}

/// One bookmark, draggable in edit mode.
#[component]
fn BookmarkItem(bookmark: Bookmark, list: String) -> impl IntoView {
    let a = app();
    let b = bookmark;
    let icon = {
        let b = b.clone();
        move || {
            match b.icon.clone().filter(|i| !i.is_empty()) {
            Some(i) => view! { <Icon icon=Some(i) mask=b.icon_mask name=b.name.clone() size=26 class="bookmark-icon"/> }.into_any(),
            None => {
                let abbr = b.abbr.clone().filter(|x| !x.is_empty()).unwrap_or_else(|| b.name.chars().take(2).collect());
                view! { <span class="bookmark-abbr" aria-hidden="true">{abbr.to_uppercase()}</span> }.into_any()
            }
        }
        }
    };
    let drag = format!("{BM}:{}", b.id);
    move || {
        if !a.editing.get() {
            return view! { <a class="bookmark" href=b.href.clone() target="_blank" rel="noopener noreferrer">{icon()}<span>{b.name.clone()}</span></a> }.into_any();
        }
        let (id, id2, name, list) = (b.id.clone(), b.id.clone(), b.name.clone(), list.clone());
        let d = drag.clone();
        view! {
            <div class="bookmark" class:dragging=move || dnd::dragging().with(|x| x.as_deref() == Some(d.as_str()))
                data-drag=drag.clone() tabindex="0">
                {icon()}
                <span class="grow">{b.name.clone()}</span>
                <span class="row" data-nodrag="">
                    <button type="button" class="btn ghost sm icon drag-handle" data-handle=drag.clone() aria-label=format!("Move bookmark {name}")>"⠿"</button>
                    <button type="button" class="btn ghost sm icon" aria-label=format!("Edit bookmark {name}")
                        on:click=move |_| a.target.set(Some(Target::Bookmark { group: list.clone(), bookmark: Some(id.clone()) }))>"✎"</button>
                    <button type="button" class="btn ghost sm icon danger" aria-label=format!("Delete bookmark {name}")
                        on:click={
                            let name = name.clone();
                            move |_| delete_with_undo(a, &format!("{name} deleted"), |d| ops::delete_bookmark(d, &id2))
                        }>"✕"</button>
                </span>
            </div>
        }
        .into_any()
    }
}
