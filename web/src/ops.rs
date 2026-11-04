//! Edit operations on a [`Doc`]. Each takes the document and returns the edited copy, so the result goes
//! straight onto the undo stack.

use crate::model::{Bookmark, BookmarkGroup, Doc, Group, Service};

fn clamp_insert<T>(list: &mut Vec<T>, at: usize, item: T) {
    let at = at.min(list.len());
    list.insert(at, item);
}

/// Every group, nested ones included, depth first.
pub fn all_groups(groups: &[Group]) -> Vec<&Group> {
    let mut out = Vec::new();
    fn walk<'a>(list: &'a [Group], out: &mut Vec<&'a Group>) {
        for g in list {
            out.push(g);
            walk(&g.groups, out);
        }
    }
    walk(groups, &mut out);
    out
}

/// Calls `f` on every group, nested ones included, until it returns true.
fn find_mut<'a>(list: &'a mut [Group], pred: &dyn Fn(&Group) -> bool) -> Option<&'a mut Group> {
    for g in list.iter_mut() {
        if pred(g) {
            return Some(g);
        }
        if let Some(x) = find_mut(&mut g.groups, pred) {
            return Some(x);
        }
    }
    None
}

/// The group with `id`, at any depth.
pub fn find_group<'a>(doc: &'a Doc, id: &str) -> Option<&'a Group> {
    all_groups(&doc.groups).into_iter().find(|g| g.id == id)
}

/// The group holding the service `id`.
pub fn find_service_group<'a>(doc: &'a Doc, id: &str) -> Option<&'a Group> {
    all_groups(&doc.groups)
        .into_iter()
        .find(|g| g.items.iter().any(|s| s.id == id))
}

/// The service `id`.
pub fn find_service<'a>(doc: &'a Doc, id: &str) -> Option<&'a Service> {
    all_groups(&doc.groups)
        .into_iter()
        .flat_map(|g| g.items.iter())
        .find(|s| s.id == id)
}

/// The list (top level or a parent's groups) holding the group `id`.
fn siblings_mut<'a>(groups: &'a mut Vec<Group>, id: &str) -> Option<&'a mut Vec<Group>> {
    if groups.iter().any(|g| g.id == id) {
        return Some(groups);
    }
    for g in groups.iter_mut() {
        if let Some(l) = siblings_mut(&mut g.groups, id) {
            return Some(l);
        }
    }
    None
}

/// Moves the group `id` to `to` among its siblings.
pub fn move_group(doc: &Doc, id: &str, to: usize) -> Doc {
    let mut d = doc.clone();
    if let Some(list) = siblings_mut(&mut d.groups, id)
        && let Some(from) = list.iter().position(|g| g.id == id)
    {
        let g = list.remove(from);
        clamp_insert(list, to, g);
    }
    d
}

/// Moves the service `id` to `to` within the group `to_group` (possibly another group).
pub fn move_service(doc: &Doc, id: &str, to_group: &str, to: usize) -> Doc {
    let mut d = doc.clone();
    if find_group(&d, to_group).is_none() {
        return d;
    }
    let Some(src) = find_mut(&mut d.groups, &|g| g.items.iter().any(|s| s.id == id)) else {
        return d;
    };
    let from = src
        .items
        .iter()
        .position(|s| s.id == id)
        .expect("found above");
    let svc = src.items.remove(from);
    if let Some(dst) = find_mut(&mut d.groups, &|g| g.id == to_group) {
        clamp_insert(&mut dst.items, to, svc);
    }
    d
}

/// Puts `service` in `group`: replaces it in place when it is there already, else moves or adds it at the end.
pub fn upsert_service(doc: &Doc, group: &str, service: Service) -> Doc {
    let mut d = doc.clone();
    let id = service.id.clone();
    if let Some(cur) = find_mut(&mut d.groups, &|g| g.items.iter().any(|s| s.id == id)) {
        if cur.id == group {
            if let Some(s) = cur.items.iter_mut().find(|s| s.id == id) {
                *s = service;
            }
            return d;
        }
        cur.items.retain(|s| s.id != id);
    }
    if let Some(g) = find_mut(&mut d.groups, &|g| g.id == group) {
        g.items.push(service);
    }
    d
}

/// Removes the service `id`.
pub fn delete_service(doc: &Doc, id: &str) -> Doc {
    let mut d = doc.clone();
    fn strip(list: &mut [Group], id: &str) {
        for g in list {
            g.items.retain(|s| s.id != id);
            strip(&mut g.groups, id);
        }
    }
    strip(&mut d.groups, id);
    d
}

/// Updates the group's name, icon, layout and visibility (keeping its cards and sub-groups), or adds it at the
/// end of the top level or of `parent`'s sub-groups.
pub fn upsert_group(doc: &Doc, group: Group, parent: Option<&str>) -> Doc {
    let mut d = doc.clone();
    let id = group.id.clone();
    if let Some(g) = find_mut(&mut d.groups, &|g| g.id == id) {
        g.name = group.name;
        g.icon = group.icon;
        g.hidden = group.hidden;
        g.layout = group.layout;
        if !group.items.is_empty() {
            g.items = group.items;
        }
        return d;
    }
    match parent.and_then(|p| find_mut(&mut d.groups, &|g| g.id == p)) {
        Some(p) => p.groups.push(group),
        None => d.groups.push(group),
    }
    d
}

/// Removes the group `id` with its cards and sub-groups.
pub fn delete_group(doc: &Doc, id: &str) -> Doc {
    let mut d = doc.clone();
    fn strip(list: &mut Vec<Group>, id: &str) {
        list.retain(|g| g.id != id);
        for g in list {
            strip(&mut g.groups, id);
        }
    }
    strip(&mut d.groups, id);
    d
}

/// Renames the bookmark list, or adds it empty.
pub fn upsert_bookmark_group(doc: &Doc, id: &str, name: &str) -> Doc {
    let mut d = doc.clone();
    match d.bookmarks.iter_mut().find(|b| b.id == id) {
        Some(b) => b.name = name.into(),
        None => d.bookmarks.push(BookmarkGroup {
            id: id.into(),
            name: name.into(),
            items: vec![],
        }),
    }
    d
}

/// Removes a bookmark list.
pub fn delete_bookmark_group(doc: &Doc, id: &str) -> Doc {
    let mut d = doc.clone();
    d.bookmarks.retain(|b| b.id != id);
    d
}

/// Puts `bm` in the list `group`: in place when it is there, else at the end (taken out of any other list).
pub fn upsert_bookmark(doc: &Doc, group: &str, bm: Bookmark) -> Doc {
    let mut d = doc.clone();
    if !d.bookmarks.iter().any(|g| g.id == group) {
        return d;
    }
    for g in d.bookmarks.iter_mut().filter(|g| g.id != group) {
        g.items.retain(|b| b.id != bm.id);
    }
    let g = d
        .bookmarks
        .iter_mut()
        .find(|g| g.id == group)
        .expect("checked");
    match g.items.iter_mut().find(|b| b.id == bm.id) {
        Some(b) => *b = bm,
        None => g.items.push(bm),
    }
    d
}

/// Removes a bookmark.
pub fn delete_bookmark(doc: &Doc, id: &str) -> Doc {
    let mut d = doc.clone();
    for g in &mut d.bookmarks {
        g.items.retain(|b| b.id != id);
    }
    d
}

/// Moves a bookmark to `to` in the list `to_group`.
pub fn move_bookmark(doc: &Doc, id: &str, to_group: &str, to: usize) -> Doc {
    let mut d = doc.clone();
    if !d.bookmarks.iter().any(|g| g.id == to_group) {
        return d;
    }
    let Some(src) = d
        .bookmarks
        .iter_mut()
        .find(|g| g.items.iter().any(|b| b.id == id))
    else {
        return d;
    };
    let from = src.items.iter().position(|b| b.id == id).expect("found");
    let bm = src.items.remove(from);
    let dst = d
        .bookmarks
        .iter_mut()
        .find(|g| g.id == to_group)
        .expect("checked");
    clamp_insert(&mut dst.items, to, bm);
    d
}

/// The groups whose cards (or sub-groups' cards) match `filter` (name, description or link, any case), with
/// only the matching cards. Empty filter: every group.
pub fn filter_groups(groups: &[Group], filter: &str) -> Vec<Group> {
    let q = filter.trim().to_lowercase();
    if q.is_empty() {
        return groups.to_vec();
    }
    let hit = |s: &Service| {
        [Some(&s.name), s.description.as_ref(), s.href.as_ref()]
            .into_iter()
            .flatten()
            .any(|t| t.to_lowercase().contains(&q))
    };
    groups
        .iter()
        .filter_map(|g| {
            let items: Vec<Service> = g.items.iter().filter(|s| hit(s)).cloned().collect();
            let subs = filter_groups(&g.groups, filter);
            (!items.is_empty() || !subs.is_empty()).then(|| Group {
                items,
                groups: subs,
                ..g.clone()
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Layout;

    fn svc(id: &str) -> Service {
        Service {
            id: id.into(),
            name: id.to_uppercase(),
            ..Service::default()
        }
    }

    fn doc() -> Doc {
        Doc {
            groups: vec![
                Group {
                    id: "a".into(),
                    name: "A".into(),
                    items: vec![svc("s1"), svc("s2")],
                    groups: vec![Group {
                        id: "a1".into(),
                        name: "A1".into(),
                        items: vec![svc("s4")],
                        ..Group::default()
                    }],
                    ..Group::default()
                },
                Group {
                    id: "b".into(),
                    name: "B".into(),
                    items: vec![svc("s3")],
                    ..Group::default()
                },
            ],
            bookmarks: vec![
                BookmarkGroup {
                    id: "x".into(),
                    name: "X".into(),
                    items: vec![Bookmark {
                        id: "k1".into(),
                        ..Bookmark::default()
                    }],
                },
                BookmarkGroup {
                    id: "y".into(),
                    name: "Y".into(),
                    items: vec![],
                },
            ],
            ..Doc::default()
        }
    }

    fn ids(g: &Group) -> Vec<&str> {
        g.items.iter().map(|s| s.id.as_str()).collect()
    }

    #[test]
    fn moves_services_within_and_across_groups() {
        let d = move_service(&doc(), "s1", "a", 1);
        assert_eq!(ids(&d.groups[0]), ["s2", "s1"]);
        let d = move_service(&doc(), "s1", "a1", 0);
        assert_eq!(ids(&d.groups[0]), ["s2"]);
        assert_eq!(ids(&d.groups[0].groups[0]), ["s1", "s4"]);
        let d = move_service(&doc(), "s1", "b", 99);
        assert_eq!(ids(&d.groups[1]), ["s3", "s1"]);
        // Unknown target: nothing changes.
        assert_eq!(move_service(&doc(), "s1", "zz", 0), doc());
    }

    #[test]
    fn moves_groups_among_siblings() {
        let d = move_group(&doc(), "b", 0);
        assert_eq!(d.groups[0].id, "b");
        let d = move_group(&doc(), "a1", 5);
        assert_eq!(d.groups[0].groups[0].id, "a1");
    }

    #[test]
    fn upserts_and_deletes_services() {
        let mut s = svc("s1");
        s.name = "Renamed".into();
        let d = upsert_service(&doc(), "a", s.clone());
        assert_eq!(find_service(&d, "s1").unwrap().name, "Renamed");
        assert_eq!(ids(&d.groups[0]), ["s1", "s2"]);
        let d = upsert_service(&doc(), "b", s);
        assert_eq!(ids(&d.groups[1]), ["s3", "s1"]);
        assert_eq!(find_service_group(&d, "s1").unwrap().id, "b");
        let d = delete_service(&doc(), "s4");
        assert!(find_service(&d, "s4").is_none());
    }

    #[test]
    fn upserts_and_deletes_groups() {
        let g = Group {
            id: "a".into(),
            name: "New name".into(),
            layout: Layout {
                columns: 5,
                ..Layout::default()
            },
            ..Group::default()
        };
        let d = upsert_group(&doc(), g, None);
        assert_eq!(
            (d.groups[0].name.as_str(), d.groups[0].layout.columns),
            ("New name", 5)
        );
        assert_eq!(ids(&d.groups[0]), ["s1", "s2"]);
        assert_eq!(d.groups[0].groups.len(), 1);
        let d = upsert_group(
            &doc(),
            Group {
                id: "n".into(),
                ..Group::default()
            },
            Some("a"),
        );
        assert_eq!(d.groups[0].groups[1].id, "n");
        let d = delete_group(&doc(), "a1");
        assert!(find_group(&d, "a1").is_none());
        assert_eq!(all_groups(&doc().groups).len(), 3);
    }

    #[test]
    fn bookmarks() {
        let d = upsert_bookmark_group(&doc(), "x", "Renamed");
        assert_eq!(
            (d.bookmarks[0].name.as_str(), d.bookmarks[0].items.len()),
            ("Renamed", 1)
        );
        let d = upsert_bookmark(
            &doc(),
            "y",
            Bookmark {
                id: "k1".into(),
                name: "moved".into(),
                ..Bookmark::default()
            },
        );
        assert!(d.bookmarks[0].items.is_empty());
        assert_eq!(d.bookmarks[1].items[0].name, "moved");
        let d = move_bookmark(&doc(), "k1", "y", 0);
        assert_eq!(d.bookmarks[1].items.len(), 1);
        let d = delete_bookmark(&doc(), "k1");
        assert!(d.bookmarks[0].items.is_empty());
        assert_eq!(delete_bookmark_group(&doc(), "x").bookmarks.len(), 1);
    }

    #[test]
    fn filters_cards() {
        let f = filter_groups(&doc().groups, "s4");
        assert_eq!(f.len(), 1);
        assert!(f[0].items.is_empty());
        assert_eq!(ids(&f[0].groups[0]), ["s4"]);
        assert_eq!(filter_groups(&doc().groups, " ").len(), 2);
        assert!(filter_groups(&doc().groups, "nothing").is_empty());
    }
}
