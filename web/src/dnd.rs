//! Moving cards, groups and bookmarks in edit mode: with the mouse (press and drag the item), on touch (drag its
//! handle) or with the keyboard (Alt and the arrow keys on a focused item).
//!
//! Draggable elements carry `data-drag="<kind>:<id>"`, groups' card areas and bookmark lists `data-drop`. One
//! drag is one undo step: every move during it shares a key, so the dashboard reflows live and Undo puts it all
//! back.

use crate::model::Doc;
use crate::ops;
use crate::state::App;
use leptos::prelude::*;
use std::cell::RefCell;
use wasm_bindgen::JsCast;

/// A card: `svc:<id>`.
pub const SVC: &str = "svc";
/// A top-level group: `group:<id>`.
pub const GRP: &str = "group";
/// A group's cards: `drop:<group id>`.
pub const DROP: &str = "drop";
/// A bookmark: `bm:<id>`.
pub const BM: &str = "bm";
/// A bookmark list: `bmdrop:<list id>`.
pub const BMDROP: &str = "bmdrop";

/// `kind:id` as its two halves.
pub fn parse(s: &str) -> Option<(&str, &str)> {
    s.split_once(':')
}

/// The move that hovering the dragged card `id` over `over` (kind, id) means: (group, index), or `None`.
pub fn service_move(doc: &Doc, id: &str, over: (&str, &str)) -> Option<(String, usize)> {
    let src = ops::find_service_group(doc, id)?;
    match over {
        (DROP, g) if g != src.id => {
            ops::find_group(doc, g).map(|dst| (dst.id.clone(), dst.items.len()))
        }
        (SVC, o) if o != id => {
            let dst = ops::find_service_group(doc, o)?;
            Some((dst.id.clone(), dst.items.iter().position(|s| s.id == o)?))
        }
        _ => None,
    }
}

/// The document after hovering the dragged item `drag` over `over`, `None` when nothing moves.
pub fn moved(doc: &Doc, drag: (&str, &str), over: (&str, &str)) -> Option<Doc> {
    match drag {
        (SVC, id) => service_move(doc, id, over).map(|(g, i)| ops::move_service(doc, id, &g, i)),
        (GRP, id) => match over {
            (GRP, o) if o != id => {
                let i = doc.groups.iter().position(|g| g.id == o)?;
                Some(ops::move_group(doc, id, i))
            }
            _ => None,
        },
        (BM, id) => {
            let (list, index) = match over {
                (BMDROP, l) => {
                    let dst = doc.bookmarks.iter().find(|b| b.id == l)?;
                    if dst.items.iter().any(|b| b.id == id) {
                        return None;
                    }
                    (dst.id.clone(), dst.items.len())
                }
                (BM, o) if o != id => {
                    let dst = doc
                        .bookmarks
                        .iter()
                        .find(|b| b.items.iter().any(|x| x.id == o))?;
                    (dst.id.clone(), dst.items.iter().position(|x| x.id == o)?)
                }
                _ => return None,
            };
            Some(ops::move_bookmark(doc, id, &list, index))
        }
        _ => None,
    }
    .filter(|d| d != doc)
}

/// The document after moving the item `drag` one place back (`-1`) or forward (`1`) among its siblings.
pub fn nudged(doc: &Doc, drag: (&str, &str), by: i32) -> Option<Doc> {
    let step = |i: usize, len: usize| -> Option<usize> {
        let j = i as i64 + i64::from(by);
        (j >= 0 && (j as usize) < len).then_some(j as usize)
    };
    match drag {
        (SVC, id) => {
            let g = ops::find_service_group(doc, id)?;
            let i = g.items.iter().position(|s| s.id == id)?;
            Some(ops::move_service(
                doc,
                id,
                &g.id.clone(),
                step(i, g.items.len())?,
            ))
        }
        (GRP, id) => {
            let i = doc.groups.iter().position(|g| g.id == id)?;
            Some(ops::move_group(doc, id, step(i, doc.groups.len())?))
        }
        (BM, id) => {
            let l = doc
                .bookmarks
                .iter()
                .find(|b| b.items.iter().any(|x| x.id == id))?;
            let i = l.items.iter().position(|x| x.id == id)?;
            Some(ops::move_bookmark(
                doc,
                id,
                &l.id.clone(),
                step(i, l.items.len())?,
            ))
        }
        _ => None,
    }
}

/// A drag under way.
struct Drag {
    item: String,
    start: (i32, i32),
    started: bool,
    key: String,
}

thread_local! {
    static DRAG: RefCell<Option<Drag>> = const { RefCell::new(None) };
}

/// The item being dragged now, for its `dragging` class.
pub fn dragging() -> ArcRwSignal<Option<String>> {
    // Reference counted, so it outlives whichever component asked first.
    thread_local! {
        static SIG: ArcRwSignal<Option<String>> = ArcRwSignal::new(None);
    }
    SIG.with(Clone::clone)
}

fn attr_at(x: i32, y: i32) -> Option<String> {
    let el = kit_leptos::rt::document()?.element_from_point(x as f32, y as f32)?;
    let hit = el.closest("[data-drag],[data-drop]").ok()??;
    hit.get_attribute("data-drag")
        .or_else(|| hit.get_attribute("data-drop"))
}

/// Listens for drags on the whole page (once, at start).
pub fn install(app: App) {
    use kit_leptos::rt::on_document;
    on_document("pointerdown", move |e| {
        if !app.editing.get_untracked() || !app.filter.with_untracked(|f| f.trim().is_empty()) {
            return;
        }
        let Some(pe) = e.dyn_ref::<web_sys::PointerEvent>() else {
            return;
        };
        if pe.button() != 0 {
            return;
        }
        let Some(t) = kit_leptos::rt::target_el(e) else {
            return;
        };
        let handle = t.closest("[data-handle]").ok().flatten();
        // Touch drags only from a handle, so the page still scrolls; a mouse drags the whole item.
        if pe.pointer_type() == "touch" && handle.is_none() {
            return;
        }
        if handle.is_none()
            && t.closest("button, a, input, select, textarea, [data-nodrag]")
                .ok()
                .flatten()
                .is_some()
        {
            return;
        }
        let item = match &handle {
            Some(h) => h.get_attribute("data-handle"),
            None => t
                .closest("[data-drag]")
                .ok()
                .flatten()
                .and_then(|d| d.get_attribute("data-drag")),
        };
        let Some(item) = item else { return };
        // Groups move by their handle only.
        if handle.is_none() && item.starts_with("group:") {
            return;
        }
        if handle.is_some() {
            e.prevent_default();
        }
        DRAG.with(|d| {
            *d.borrow_mut() = Some(Drag {
                item,
                start: (pe.client_x(), pe.client_y()),
                started: false,
                key: crate::model::new_id("drag"),
            })
        });
    });
    on_document("pointermove", move |e| {
        let Some(pe) = e.dyn_ref::<web_sys::PointerEvent>() else {
            return;
        };
        let (x, y) = (pe.client_x(), pe.client_y());
        let step = DRAG.with(|d| {
            let mut d = d.borrow_mut();
            let drag = d.as_mut()?;
            if !drag.started {
                let (dx, dy) = (x - drag.start.0, y - drag.start.1);
                if dx * dx + dy * dy < 36 {
                    return None;
                }
                drag.started = true;
                dragging().set(Some(drag.item.clone()));
            }
            Some((drag.item.clone(), drag.key.clone()))
        });
        let Some((item, key)) = step else { return };
        e.prevent_default();
        let Some(over) = attr_at(x, y) else { return };
        let (Some(a), Some(o)) = (parse(&item), parse(&over)) else {
            return;
        };
        let next = app.undo.with_untracked(|u| moved(&u.present, a, o));
        if let Some(next) = next {
            app.apply(next, Some(&key));
        }
    });
    let end = move |_: &web_sys::Event| {
        if DRAG
            .with(|d| d.borrow_mut().take())
            .is_some_and(|d| d.started)
        {
            dragging().set(None);
        }
    };
    on_document("pointerup", end);
    on_document("pointercancel", end);
    on_document("keydown", move |e| {
        if !app.editing.get_untracked() {
            return;
        }
        let Some(k) = e.dyn_ref::<web_sys::KeyboardEvent>() else {
            return;
        };
        if !k.alt_key() {
            return;
        }
        let by = match k.key().as_str() {
            "ArrowUp" | "ArrowLeft" => -1,
            "ArrowDown" | "ArrowRight" => 1,
            _ => return,
        };
        let Some(t) = kit_leptos::rt::target_el(e) else {
            return;
        };
        let Some(item) = t
            .get_attribute("data-drag")
            .or_else(|| t.get_attribute("data-handle"))
        else {
            return;
        };
        let Some(a) = parse(&item) else { return };
        let next = app.undo.with_untracked(|u| nudged(&u.present, a, by));
        if let Some(next) = next {
            e.prevent_default();
            app.apply(next, None);
            // Keep the focus on the moved item once it is drawn again.
            let sel = format!("[data-drag=\"{item}\"],[data-handle=\"{item}\"]");
            kit_leptos::rt::after(0, move || {
                if let Some(el) =
                    kit_leptos::rt::qs(&sel).and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
                {
                    let _ = el.focus();
                }
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Bookmark, BookmarkGroup, Group, Service};

    fn doc() -> Doc {
        let s = |id: &str| Service {
            id: id.into(),
            ..Service::default()
        };
        Doc {
            groups: vec![
                Group {
                    id: "a".into(),
                    items: vec![s("1"), s("2")],
                    ..Group::default()
                },
                Group {
                    id: "b".into(),
                    items: vec![s("3")],
                    ..Group::default()
                },
            ],
            bookmarks: vec![
                BookmarkGroup {
                    id: "x".into(),
                    items: vec![Bookmark {
                        id: "k".into(),
                        ..Bookmark::default()
                    }],
                    ..BookmarkGroup::default()
                },
                BookmarkGroup {
                    id: "y".into(),
                    ..BookmarkGroup::default()
                },
            ],
            ..Doc::default()
        }
    }

    #[test]
    fn card_moves() {
        assert_eq!(service_move(&doc(), "1", (SVC, "2")), Some(("a".into(), 1)));
        assert_eq!(service_move(&doc(), "1", (SVC, "3")), Some(("b".into(), 0)));
        assert_eq!(
            service_move(&doc(), "1", (DROP, "b")),
            Some(("b".into(), 1))
        );
        assert_eq!(service_move(&doc(), "1", (DROP, "a")), None);
        assert_eq!(service_move(&doc(), "1", (SVC, "1")), None);
        let d = moved(&doc(), (SVC, "1"), (SVC, "3")).unwrap();
        assert_eq!(d.groups[1].items[0].id, "1");
    }

    #[test]
    fn group_and_bookmark_moves() {
        let d = moved(&doc(), (GRP, "b"), (GRP, "a")).unwrap();
        assert_eq!(d.groups[0].id, "b");
        assert!(moved(&doc(), (GRP, "a"), (SVC, "3")).is_none());
        let d = moved(&doc(), (BM, "k"), (BMDROP, "y")).unwrap();
        assert_eq!(d.bookmarks[1].items[0].id, "k");
        assert!(moved(&doc(), (BM, "k"), (BMDROP, "x")).is_none());
    }

    #[test]
    fn keyboard_nudges() {
        let d = nudged(&doc(), (SVC, "1"), 1).unwrap();
        assert_eq!(d.groups[0].items[1].id, "1");
        assert!(nudged(&doc(), (SVC, "1"), -1).is_none());
        assert_eq!(nudged(&doc(), (GRP, "a"), 1).unwrap().groups[0].id, "b");
        assert!(nudged(&doc(), (BM, "k"), 1).is_none());
        assert_eq!(parse("svc:a:b"), Some(("svc", "a:b")));
    }
}
