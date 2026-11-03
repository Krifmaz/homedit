/**
 * Drawer forms for bookmark groups and single bookmarks.
 */
import { useId, useState } from "react";
import type { Bookmark } from "../../api/types";
import {
  deleteBookmark,
  deleteBookmarkGroup,
  newId,
  upsertBookmark,
  upsertBookmarkGroup,
} from "../ops";
import { IconPicker } from "./IconPicker";
import { useFormSession } from "./useFormSession";
import { isHttpUrl, optional } from "./util";
import "./forms.css";

/** Props of {@link BookmarkGroupForm}. */
export interface BookmarkGroupFormProps {
  /** Bookmark group being edited; omitted to add a new one. */
  groupId?: string;
  /** Closes the drawer. */
  onClose: () => void;
}

/** Add/edit form for a bookmark group (its name), with delete. */
export function BookmarkGroupForm({ groupId, onClose }: BookmarkGroupFormProps) {
  const session = useFormSession(onClose);
  const { doc } = session;
  const uid = useId();
  const [id] = useState(() => groupId ?? newId("bg"));
  const [touched, setTouched] = useState(Boolean(groupId));
  const existing = doc.bookmarks.find((g) => g.id === id);
  const name = existing?.name ?? "";
  const error = name.trim() === "" ? "Name is required" : null;

  const remove = () => {
    if (!existing) return;
    const n = existing.items.length;
    if (
      !window.confirm(
        `Delete "${existing.name}" and its ${n === 1 ? "1 bookmark" : `${n} bookmarks`}?`,
      )
    ) {
      return;
    }
    session.update(deleteBookmarkGroup(doc, id));
    session.done();
  };

  return (
    <form
      className="editor-form"
      onSubmit={(e) => {
        e.preventDefault();
        if (!error) session.done();
      }}
    >
      <h2>{groupId ? "Edit bookmark group" : "Add bookmark group"}</h2>
      <div className="field">
        <label htmlFor={`${uid}-name`} className="required">
          Name
        </label>
        <input
          id={`${uid}-name`}
          type="text"
          value={name}
          aria-required
          aria-invalid={touched && error ? true : undefined}
          onChange={(e) => {
            setTouched(true);
            session.update(upsertBookmarkGroup(doc, { id, name: e.target.value }));
          }}
        />
        {touched && error && (
          <span className="field-error" role="alert">
            {error}
          </span>
        )}
      </div>
      <div className="drawer-footer btn-row">
        <button type="submit" className="btn btn-primary" disabled={Boolean(error)}>
          Done
        </button>
        <button type="button" className="btn" onClick={session.cancel}>
          Cancel
        </button>
        {groupId && existing && (
          <button type="button" className="btn btn-danger" onClick={remove}>
            Delete
          </button>
        )}
      </div>
    </form>
  );
}

/** Props of {@link BookmarkForm}. */
export interface BookmarkFormProps {
  /** Bookmark group the bookmark is in (or is added to). */
  groupId: string;
  /** Bookmark being edited; omitted to add a new one. */
  bookmarkId?: string;
  /** Closes the drawer. */
  onClose: () => void;
}

/**
 * Validates a bookmark: name required, link required and http(s), and an
 * abbreviation of at most 2 characters.
 *
 * @returns error messages keyed by `name`, `href` or `abbr`.
 */
export function validateBookmark(b: Bookmark): Record<string, string> {
  const errors: Record<string, string> = {};
  if (b.name.trim() === "") errors.name = "Name is required";
  if (b.href.trim() === "") errors.href = "Link is required";
  else if (!isHttpUrl(b.href)) errors.href = "Link must start with http:// or https://";
  if (b.abbr && [...b.abbr].length > 2) errors.abbr = "At most 2 characters";
  return errors;
}

/** Add/edit form for a bookmark: name, link, abbreviation, icon and group. */
export function BookmarkForm({ groupId, bookmarkId, onClose }: BookmarkFormProps) {
  const session = useFormSession(onClose);
  const { doc } = session;
  const uid = useId();
  const [id] = useState(() => bookmarkId ?? newId("b"));
  const [touched, setTouched] = useState(Boolean(bookmarkId));

  const holder = doc.bookmarks.find((g) => g.items.some((b) => b.id === id));
  const existing = holder?.items.find((b) => b.id === id);
  const bm: Bookmark = existing ?? { id, name: "", href: "" };
  const currentGroup = holder?.id ?? groupId;
  const errors = validateBookmark(bm);
  const shown = touched ? errors : {};
  const valid = Object.keys(errors).length === 0;

  const change = (patch: Partial<Bookmark>, toGroup = currentGroup) => {
    setTouched(true);
    session.update(upsertBookmark(doc, toGroup, { ...bm, ...patch }));
  };

  const remove = () => {
    if (!window.confirm(`Delete bookmark "${bm.name}"?`)) return;
    session.update(deleteBookmark(doc, id));
    session.done();
  };

  const error = (key: string) =>
    shown[key] ? (
      <span id={`${uid}-${key}-err`} className="field-error" role="alert">
        {shown[key]}
      </span>
    ) : null;
  const a11y = (key: string, required = false) => ({
    id: `${uid}-${key}`,
    "aria-required": required || undefined,
    "aria-invalid": shown[key] ? true : undefined,
    "aria-describedby": shown[key] ? `${uid}-${key}-err` : undefined,
  });

  return (
    <form
      className="editor-form"
      onSubmit={(e) => {
        e.preventDefault();
        if (valid) session.done();
      }}
    >
      <h2>{bookmarkId ? "Edit bookmark" : "Add bookmark"}</h2>
      <div className="field">
        <label htmlFor={`${uid}-name`} className="required">
          Name
        </label>
        <input
          {...a11y("name", true)}
          type="text"
          value={bm.name}
          onChange={(e) => change({ name: e.target.value })}
        />
        {error("name")}
      </div>
      <div className="field">
        <label htmlFor={`${uid}-href`} className="required">
          Link
        </label>
        <input
          {...a11y("href", true)}
          type="url"
          inputMode="url"
          spellCheck={false}
          placeholder="https://example.com"
          value={bm.href}
          onChange={(e) => change({ href: e.target.value })}
        />
        {error("href")}
      </div>
      <div className="field">
        <label htmlFor={`${uid}-abbr`}>Abbreviation</label>
        <input
          {...a11y("abbr")}
          type="text"
          maxLength={2}
          size={3}
          value={bm.abbr ?? ""}
          onChange={(e) => change({ abbr: optional(e.target.value) })}
        />
        <span className="field-help">Up to 2 characters, shown when there is no icon.</span>
        {error("abbr")}
      </div>
      <IconPicker
        value={bm.icon}
        name={bm.name}
        onChange={(icon) => change({ icon, iconKind: undefined })}
      />
      {doc.bookmarks.length > 1 && (
        <div className="field">
          <label htmlFor={`${uid}-group`}>Bookmark group</label>
          <select
            id={`${uid}-group`}
            value={currentGroup}
            onChange={(e) => change({}, e.target.value)}
          >
            {doc.bookmarks.map((g) => (
              <option key={g.id} value={g.id}>
                {g.name || "(unnamed)"}
              </option>
            ))}
          </select>
        </div>
      )}
      <div className="drawer-footer btn-row">
        <button type="submit" className="btn btn-primary" disabled={!valid}>
          Done
        </button>
        <button type="button" className="btn" onClick={session.cancel}>
          Cancel
        </button>
        {bookmarkId && existing && (
          <button type="button" className="btn btn-danger" onClick={remove}>
            Delete
          </button>
        )}
      </div>
    </form>
  );
}
