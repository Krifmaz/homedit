/**
 * The dashboard body: groups of service cards and bookmark lists.
 *
 * In edit mode cards, groups and bookmarks become draggable (pointer, touch
 * and keyboard via dnd-kit). A drag is one undo step: every move during the
 * drag shares a coalesce key, so the dashboard reflows live and Ctrl+Z puts
 * everything back.
 */
/* eslint-disable react-hooks/refs -- dnd-kit returns `setNodeRef` callbacks, which the
   rule mistakes for ref objects read during render. */
import {
  closestCenter,
  DndContext,
  KeyboardSensor,
  PointerSensor,
  TouchSensor,
  useDroppable,
  useSensor,
  useSensors,
  type DragEndEvent,
  type DragOverEvent,
} from "@dnd-kit/core";
import {
  rectSortingStrategy,
  SortableContext,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { useRef, type CSSProperties, type ReactNode } from "react";
import type { Bookmark, BookmarkGroup, ConfigDoc, Group, Service } from "../api/types";
import { useApp } from "../app/context";
import {
  deleteBookmark,
  deleteService,
  findServiceGroup,
  moveBookmark,
  moveGroup,
  moveService,
  newId,
} from "../editor/ops";
import { Icon } from "./Icon";
import { matches } from "./SearchBar";
import { ServiceCard } from "./ServiceCard";

/** Prefixes that keep draggable ids of different kinds apart. */
const SVC = "svc:";
const GRP = "group:";
const DROP = "drop:";
const BM = "bm:";
const BMDROP = "bmdrop:";

/** Stops pointer and key events on buttons inside draggables from starting a drag. */
const noDrag = {
  onPointerDown: (e: { stopPropagation(): void }) => e.stopPropagation(),
  onKeyDown: (e: { stopPropagation(): void }) => e.stopPropagation(),
};

/** Splits a prefixed drag id. Exported for tests. */
export function parseDragId(id: string | number): { kind: string; id: string } {
  const s = String(id);
  const i = s.indexOf(":");
  return { kind: s.slice(0, i + 1), id: s.slice(i + 1) };
}

/** Applies the filter to a group tree, dropping groups with no matching cards. */
export function filterGroups(groups: Group[], q: string): Group[] {
  if (!q.trim()) return groups;
  const out: Group[] = [];
  for (const g of groups) {
    const items = g.items.filter((s) => matches(q, s.name, s.description, s.href));
    const sub = g.groups ? filterGroups(g.groups, q) : undefined;
    if (items.length > 0 || (sub && sub.length > 0)) out.push({ ...g, items, groups: sub });
  }
  return out;
}

/**
 * Works out the move a drag-over or drop implies for a service card.
 * Returns `null` when nothing should change. Exported for tests.
 */
export function serviceMoveFor(
  doc: ConfigDoc,
  serviceId: string,
  over: { kind: string; id: string },
  sameGroupOnly: boolean,
): { groupId: string; index: number } | null {
  const src = findServiceGroup(doc, serviceId);
  if (!src) return null;
  if (over.kind === DROP) {
    if (over.id === src.id || sameGroupOnly) return null;
    return { groupId: over.id, index: Number.MAX_SAFE_INTEGER };
  }
  if (over.kind !== SVC || over.id === serviceId) return null;
  const dst = findServiceGroup(doc, over.id);
  if (!dst) return null;
  if (sameGroupOnly !== (dst.id === src.id)) return null;
  return { groupId: dst.id, index: dst.items.findIndex((s) => s.id === over.id) };
}

/** Props of {@link Dashboard}. */
export interface DashboardProps {
  /** Text in the search box; filters cards. */
  filter: string;
}

/** Groups and bookmarks, editable in edit mode. */
export function Dashboard({ filter }: DashboardProps) {
  const app = useApp();
  const doc = app.doc;
  const dragKey = useRef<string | undefined>(undefined);
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 6 } }),
    useSensor(TouchSensor, { activationConstraint: { delay: 200, tolerance: 6 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  );
  if (!doc) return <p className="field-help">Loading…</p>;

  const filtering = filter.trim() !== "";
  const dnd = app.editing && !filtering;
  const groups = filterGroups(doc.groups, filter);

  const onDragOver = ({ active, over }: DragOverEvent) => {
    if (!over) return;
    const a = parseDragId(active.id);
    if (a.kind !== SVC) return;
    const mv = serviceMoveFor(doc, a.id, parseDragId(over.id), false);
    if (mv) app.apply(moveService(doc, a.id, mv.groupId, mv.index), dragKey.current);
  };

  const onDragEnd = ({ active, over }: DragEndEvent) => {
    const key = dragKey.current;
    dragKey.current = undefined;
    if (!over) return;
    const a = parseDragId(active.id);
    const o = parseDragId(over.id);
    if (a.kind === SVC) {
      const mv = serviceMoveFor(doc, a.id, o, true);
      if (mv) app.apply(moveService(doc, a.id, mv.groupId, mv.index), key);
    } else if (a.kind === GRP && o.kind === GRP && a.id !== o.id) {
      const index = doc.groups.findIndex((g) => g.id === o.id);
      if (index >= 0) app.apply(moveGroup(doc, a.id, index), key);
    } else if (a.kind === BM && (o.kind === BM || o.kind === BMDROP) && a.id !== o.id) {
      const dst =
        o.kind === BMDROP
          ? doc.bookmarks.find((b) => b.id === o.id)
          : doc.bookmarks.find((b) => b.items.some((x) => x.id === o.id));
      if (!dst) return;
      const index =
        o.kind === BMDROP ? dst.items.length : dst.items.findIndex((x) => x.id === o.id);
      app.apply(moveBookmark(doc, a.id, dst.id, index), key);
    }
  };

  return (
    <DndContext
      sensors={sensors}
      collisionDetection={closestCenter}
      onDragStart={() => (dragKey.current = newId("drag"))}
      onDragOver={onDragOver}
      onDragEnd={onDragEnd}
      onDragCancel={() => (dragKey.current = undefined)}
    >
      <SortableContext items={groups.map((g) => GRP + g.id)} strategy={verticalListSortingStrategy}>
        <div className="groups">
          {groups.map((g) => (
            <GroupSection key={g.id} group={g} depth={0} dnd={dnd} />
          ))}
          {app.editing && (
            <button type="button" className="add-tile" onClick={() => app.open({ kind: "group" })}>
              + Add group
            </button>
          )}
          {!app.editing && filtering && groups.length === 0 && (
            <p className="field-help">No cards match. Press Enter to search the web.</p>
          )}
        </div>
      </SortableContext>
      {!filtering && <Bookmarks lists={doc.bookmarks} dnd={dnd} />}
    </DndContext>
  );
}

/** Drag props for an element: sortable when `dnd` is on. */
function useDrag(id: string, dnd: boolean) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id,
    disabled: !dnd,
  });
  const style: CSSProperties = { transform: CSS.Translate.toString(transform), transition };
  return { attributes, listeners, setNodeRef, isDragging, style };
}

/** One group with its cards and sub-groups. */
function GroupSection({ group, depth, dnd }: { group: Group; depth: number; dnd: boolean }) {
  const app = useApp();
  const drag = useDrag(GRP + group.id, dnd && depth === 0);
  const drop = useDroppable({ id: DROP + group.id, disabled: !dnd });
  const Heading = depth === 0 ? "h2" : "h3";
  return (
    <section
      ref={depth === 0 ? drag.setNodeRef : undefined}
      style={depth === 0 ? drag.style : undefined}
      className={`group${group.hidden ? " is-hidden" : ""}${drag.isDragging ? " dragging" : ""}`}
      aria-labelledby={`gh-${group.id}`}
    >
      <div className="group-head">
        {dnd && depth === 0 && (
          <button
            type="button"
            className="btn btn-icon drag-handle"
            aria-label={`Move group ${group.name}`}
            {...drag.attributes}
            {...drag.listeners}
          >
            ⠿
          </button>
        )}
        {group.icon && (
          <Icon icon={group.icon} name={group.name} size={20} className="group-icon" />
        )}
        <Heading id={`gh-${group.id}`}>{group.name}</Heading>
        {group.hidden && <span className="badge">Hidden from viewers</span>}
        {app.editing && (
          <button
            type="button"
            className="btn btn-icon"
            aria-label={`Edit group ${group.name}`}
            onClick={() => app.open({ kind: "group", groupId: group.id })}
          >
            ✎
          </button>
        )}
      </div>
      <SortableContext items={group.items.map((s) => SVC + s.id)} strategy={rectSortingStrategy}>
        <div
          ref={drop.setNodeRef}
          className={`cards style-${group.layout.style}`}
          style={{ "--cols": Math.min(8, Math.max(1, group.layout.columns)) } as CSSProperties}
        >
          {group.items.map((s) => (
            <SortableCard key={s.id} service={s} groupId={group.id} dnd={dnd} />
          ))}
          {app.editing && (
            <button
              type="button"
              className="add-tile"
              onClick={() => app.open({ kind: "service", groupId: group.id })}
            >
              + Add service
            </button>
          )}
        </div>
      </SortableContext>
      {group.groups && group.groups.length > 0 && (
        <div className="subgroups">
          {group.groups.map((g) => (
            <GroupSection key={g.id} group={g} depth={depth + 1} dnd={dnd} />
          ))}
        </div>
      )}
    </section>
  );
}

/** A card that can be dragged in edit mode. */
function SortableCard({
  service,
  groupId,
  dnd,
}: {
  service: Service;
  groupId: string;
  dnd: boolean;
}) {
  const app = useApp();
  const drag = useDrag(SVC + service.id, dnd);
  let actions: ReactNode = null;
  if (app.editing && app.doc) {
    const doc = app.doc;
    actions = (
      <div className="edit-actions">
        <button
          type="button"
          className="btn btn-icon"
          aria-label={`Edit ${service.name}`}
          {...noDrag}
          onClick={() => app.open({ kind: "service", groupId, serviceId: service.id })}
        >
          ✎
        </button>
        <button
          type="button"
          className="btn btn-icon btn-danger"
          aria-label={`Delete ${service.name}`}
          {...noDrag}
          onClick={() => app.apply(deleteService(doc, service.id))}
        >
          ✕
        </button>
      </div>
    );
  }
  return (
    <div
      ref={drag.setNodeRef}
      style={drag.style}
      className={drag.isDragging ? "dragging" : undefined}
      {...(dnd
        ? {
            ...drag.attributes,
            ...drag.listeners,
            "aria-roledescription": "draggable card",
            "aria-label": service.name,
          }
        : {})}
    >
      <ServiceCard
        service={service}
        status={app.status[service.id]}
        editing={app.editing}
        actions={actions}
      />
    </div>
  );
}

/** Bookmark lists below the groups. */
function Bookmarks({ lists, dnd }: { lists: BookmarkGroup[]; dnd: boolean }) {
  const app = useApp();
  if (lists.length === 0 && !app.editing) return null;
  return (
    <div className="bookmarks">
      {lists.map((list) => (
        <BookmarkList key={list.id} list={list} dnd={dnd} />
      ))}
      {app.editing && (
        <button
          type="button"
          className="add-tile"
          onClick={() => app.open({ kind: "bookmarkGroup" })}
        >
          + Add bookmark list
        </button>
      )}
    </div>
  );
}

/** One bookmark list. */
function BookmarkList({ list, dnd }: { list: BookmarkGroup; dnd: boolean }) {
  const app = useApp();
  const drop = useDroppable({ id: BMDROP + list.id, disabled: !dnd });
  return (
    <section className="bookmark-group" aria-labelledby={`bh-${list.id}`}>
      <h3 id={`bh-${list.id}`}>
        {list.name}
        {app.editing && (
          <button
            type="button"
            className="btn btn-icon"
            aria-label={`Edit bookmark list ${list.name}`}
            onClick={() => app.open({ kind: "bookmarkGroup", groupId: list.id })}
          >
            ✎
          </button>
        )}
      </h3>
      <SortableContext
        items={list.items.map((b) => BM + b.id)}
        strategy={verticalListSortingStrategy}
      >
        <div ref={drop.setNodeRef}>
          {list.items.map((b) => (
            <BookmarkItem key={b.id} bookmark={b} groupId={list.id} dnd={dnd} />
          ))}
          {app.editing && (
            <button
              type="button"
              className="add-tile"
              style={{ minHeight: 38, width: "100%" }}
              onClick={() => app.open({ kind: "bookmark", groupId: list.id })}
            >
              + Add bookmark
            </button>
          )}
        </div>
      </SortableContext>
    </section>
  );
}

/** One bookmark link, draggable in edit mode. */
function BookmarkItem({
  bookmark,
  groupId,
  dnd,
}: {
  bookmark: Bookmark;
  groupId: string;
  dnd: boolean;
}) {
  const app = useApp();
  const drag = useDrag(BM + bookmark.id, dnd);
  const icon = bookmark.icon ? (
    <Icon
      icon={bookmark.icon}
      kind={bookmark.iconKind}
      name={bookmark.name}
      size={26}
      className="bookmark-icon"
    />
  ) : (
    <span className="bookmark-abbr" aria-hidden>
      {(bookmark.abbr || bookmark.name.slice(0, 2)).toUpperCase()}
    </span>
  );
  if (!app.editing) {
    return (
      <a className="bookmark" href={bookmark.href} target="_blank" rel="noopener noreferrer">
        {icon}
        <span>{bookmark.name}</span>
      </a>
    );
  }
  const doc = app.doc!;
  return (
    <div
      ref={drag.setNodeRef}
      style={drag.style}
      className={`bookmark${drag.isDragging ? " dragging" : ""}`}
      {...(dnd ? { ...drag.attributes, ...drag.listeners } : {})}
    >
      {icon}
      <span style={{ flex: 1 }}>{bookmark.name}</span>
      <button
        type="button"
        className="btn btn-icon"
        aria-label={`Edit bookmark ${bookmark.name}`}
        {...noDrag}
        onClick={() => app.open({ kind: "bookmark", groupId, bookmarkId: bookmark.id })}
      >
        ✎
      </button>
      <button
        type="button"
        className="btn btn-icon btn-danger"
        aria-label={`Delete bookmark ${bookmark.name}`}
        {...noDrag}
        onClick={() => app.apply(deleteBookmark(doc, bookmark.id))}
      >
        ✕
      </button>
    </div>
  );
}
