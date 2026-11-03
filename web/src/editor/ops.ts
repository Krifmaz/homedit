/**
 * Pure edit operations on a {@link ConfigDoc}.
 *
 * Every function returns a new document and never mutates its input, so the
 * results can be pushed straight onto the undo stack.
 */
import type { Bookmark, BookmarkGroup, ConfigDoc, Group, Service, Settings } from "../api/types";

/** Generates a short random identifier with a readable prefix. */
export function newId(prefix: string): string {
  const rand =
    typeof crypto !== "undefined" && "randomUUID" in crypto
      ? crypto.randomUUID().slice(0, 8)
      : Math.random().toString(36).slice(2, 10);
  return `${prefix}-${rand}`;
}

/** Returns a deep copy of `doc` with `recipe` applied to the copy. */
export function produce(doc: ConfigDoc, recipe: (draft: ConfigDoc) => void): ConfigDoc {
  const draft = structuredClone(doc);
  recipe(draft);
  return draft;
}

/** Moves an element of `list` from index `from` to index `to` in place. */
function move<T>(list: T[], from: number, to: number): void {
  const [item] = list.splice(from, 1);
  if (item !== undefined) list.splice(Math.max(0, Math.min(to, list.length)), 0, item);
}

/** Every group in the document, nested ones included, depth first. */
export function allGroups(doc: Pick<ConfigDoc, "groups">): Group[] {
  const out: Group[] = [];
  const walk = (list: Group[]) => {
    for (const g of list) {
      out.push(g);
      if (g.groups) walk(g.groups);
    }
  };
  walk(doc.groups);
  return out;
}

/** Finds a group (at any depth) by id. */
export function findGroup(doc: ConfigDoc, groupId: string): Group | undefined {
  return allGroups(doc).find((g) => g.id === groupId);
}

/** Finds the list (top level or a parent's `groups`) that holds a group. */
function siblingsOf(doc: ConfigDoc, groupId: string): Group[] | undefined {
  if (doc.groups.some((g) => g.id === groupId)) return doc.groups;
  return allGroups(doc).find((g) => g.groups?.some((c) => c.id === groupId))?.groups;
}

/** Finds the group holding a service. */
export function findServiceGroup(doc: ConfigDoc, serviceId: string): Group | undefined {
  return allGroups(doc).find((g) => g.items.some((s) => s.id === serviceId));
}

/** Finds a service by id. */
export function findService(doc: ConfigDoc, serviceId: string): Service | undefined {
  for (const g of allGroups(doc)) {
    const s = g.items.find((x) => x.id === serviceId);
    if (s) return s;
  }
  return undefined;
}

/** Reorders a group among its siblings to `toIndex`. */
export function moveGroup(doc: ConfigDoc, groupId: string, toIndex: number): ConfigDoc {
  return produce(doc, (d) => {
    const list = siblingsOf(d, groupId);
    if (!list) return;
    const from = list.findIndex((g) => g.id === groupId);
    if (from >= 0) move(list, from, toIndex);
  });
}

/** Moves a service to `toIndex` within `toGroupId` (possibly another group). */
export function moveService(
  doc: ConfigDoc,
  serviceId: string,
  toGroupId: string,
  toIndex: number,
): ConfigDoc {
  return produce(doc, (d) => {
    const src = findServiceGroup(d, serviceId);
    const dst = findGroup(d, toGroupId);
    if (!src || !dst) return;
    const from = src.items.findIndex((s) => s.id === serviceId);
    if (src === dst) {
      move(src.items, from, toIndex);
      return;
    }
    const [svc] = src.items.splice(from, 1);
    if (svc) dst.items.splice(Math.max(0, Math.min(toIndex, dst.items.length)), 0, svc);
  });
}

/** Inserts `service` into `groupId`, or replaces it if the id already exists anywhere. */
export function upsertService(doc: ConfigDoc, groupId: string, service: Service): ConfigDoc {
  return produce(doc, (d) => {
    const current = findServiceGroup(d, service.id);
    if (current && current.id === groupId) {
      current.items = current.items.map((s) => (s.id === service.id ? service : s));
      return;
    }
    if (current) current.items = current.items.filter((s) => s.id !== service.id);
    findGroup(d, groupId)?.items.push(service);
  });
}

/** Removes a service. */
export function deleteService(doc: ConfigDoc, serviceId: string): ConfigDoc {
  return produce(doc, (d) => {
    for (const g of allGroups(d)) g.items = g.items.filter((s) => s.id !== serviceId);
  });
}

/**
 * Inserts a group at the end (of the top level, or of `parentId`'s
 * sub-groups), or updates an existing group's name, icon, layout and
 * visibility while keeping its cards and sub-groups.
 */
export function upsertGroup(
  doc: ConfigDoc,
  group: Omit<Group, "items" | "groups"> & { items?: Service[] },
  parentId?: string,
): ConfigDoc {
  return produce(doc, (d) => {
    const existing = findGroup(d, group.id);
    if (existing) {
      Object.assign(existing, { ...group, items: group.items ?? existing.items });
      return;
    }
    const fresh: Group = { ...group, items: group.items ?? [] };
    const parent = parentId ? findGroup(d, parentId) : undefined;
    if (parent) (parent.groups ??= []).push(fresh);
    else d.groups.push(fresh);
  });
}

/** Removes a group (at any depth) with all its cards and sub-groups. */
export function deleteGroup(doc: ConfigDoc, groupId: string): ConfigDoc {
  return produce(doc, (d) => {
    const strip = (list: Group[]): Group[] =>
      list
        .filter((g) => g.id !== groupId)
        .map((g) => (g.groups ? { ...g, groups: strip(g.groups) } : g));
    d.groups = strip(d.groups);
  });
}

/** Inserts or replaces a bookmark group (name only when it exists). */
export function upsertBookmarkGroup(
  doc: ConfigDoc,
  group: Omit<BookmarkGroup, "items">,
): ConfigDoc {
  return produce(doc, (d) => {
    const existing = d.bookmarks.find((b) => b.id === group.id);
    if (existing) existing.name = group.name;
    else d.bookmarks.push({ ...group, items: [] });
  });
}

/** Removes a bookmark group. */
export function deleteBookmarkGroup(doc: ConfigDoc, groupId: string): ConfigDoc {
  return produce(doc, (d) => {
    d.bookmarks = d.bookmarks.filter((b) => b.id !== groupId);
  });
}

/** Inserts or replaces a bookmark in `groupId`. */
export function upsertBookmark(doc: ConfigDoc, groupId: string, bookmark: Bookmark): ConfigDoc {
  return produce(doc, (d) => {
    for (const g of d.bookmarks) {
      if (g.id !== groupId) g.items = g.items.filter((b) => b.id !== bookmark.id);
    }
    const g = d.bookmarks.find((b) => b.id === groupId);
    if (!g) return;
    const i = g.items.findIndex((b) => b.id === bookmark.id);
    if (i >= 0) g.items[i] = bookmark;
    else g.items.push(bookmark);
  });
}

/** Removes a bookmark. */
export function deleteBookmark(doc: ConfigDoc, bookmarkId: string): ConfigDoc {
  return produce(doc, (d) => {
    for (const g of d.bookmarks) g.items = g.items.filter((b) => b.id !== bookmarkId);
  });
}

/** Moves a bookmark to `toIndex` within `toGroupId`. */
export function moveBookmark(
  doc: ConfigDoc,
  bookmarkId: string,
  toGroupId: string,
  toIndex: number,
): ConfigDoc {
  return produce(doc, (d) => {
    const src = d.bookmarks.find((g) => g.items.some((b) => b.id === bookmarkId));
    const dst = d.bookmarks.find((g) => g.id === toGroupId);
    if (!src || !dst) return;
    const from = src.items.findIndex((b) => b.id === bookmarkId);
    const [bm] = src.items.splice(from, 1);
    if (bm) dst.items.splice(Math.max(0, Math.min(toIndex, dst.items.length)), 0, bm);
  });
}

/** Merges `patch` into the dashboard settings. */
export function updateSettings(doc: ConfigDoc, patch: Partial<Settings>): ConfigDoc {
  return produce(doc, (d) => {
    d.settings = { ...d.settings, ...patch };
  });
}
