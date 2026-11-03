/**
 * Drawer form for adding or editing a group of service cards.
 */
import { useId, useState } from "react";
import type { Group, GroupLayout } from "../../api/types";
import { deleteGroup, findGroup, newId, upsertGroup } from "../ops";
import { useFormSession } from "./useFormSession";
import { countCards, groupOptions } from "./util";
import "./forms.css";

/** Props of {@link GroupForm}. */
export interface GroupFormProps {
  /** Group being edited; omitted to add a new group. */
  groupId?: string;
  /** Closes the drawer. */
  onClose: () => void;
}

/** Group fields the form edits (cards and sub-groups are left alone). */
type GroupHead = Omit<Group, "items" | "groups">;

/** Layout styles offered in the form, with their labels. */
const STYLES: { value: GroupLayout["style"]; label: string }[] = [
  { value: "grid", label: "Grid" },
  { value: "row", label: "Single row" },
  { value: "column", label: "Single column" },
];

/**
 * Add/edit form for a group: name, visibility, layout and (for new groups)
 * parent group. Changes preview live; Cancel reverts the session.
 */
export function GroupForm({ groupId, onClose }: GroupFormProps) {
  const session = useFormSession(onClose);
  const { doc } = session;
  const uid = useId();
  const [id] = useState(() => groupId ?? newId("g"));
  const [parentId, setParentId] = useState("");
  const [touched, setTouched] = useState(Boolean(groupId));

  const existing = findGroup(doc, id);
  const head: GroupHead = existing
    ? {
        id,
        name: existing.name,
        icon: existing.icon,
        hidden: existing.hidden,
        layout: existing.layout,
      }
    : { id, name: "", hidden: false, layout: { columns: 3, style: "grid" } };
  const nameError = head.name.trim() === "" ? "Name is required" : null;

  const change = (patch: Partial<GroupHead>, parent = parentId) => {
    setTouched(true);
    const next = { ...head, ...patch };
    // A new group can still be re-parented: take it out and insert it again.
    const base = !groupId && existing && parent !== parentId ? deleteGroup(doc, id) : doc;
    session.update(
      upsertGroup(
        base,
        base === doc ? next : { ...next, items: existing?.items },
        parent || undefined,
      ),
    );
  };

  const remove = () => {
    if (!existing) return;
    const n = countCards(existing);
    const what = n === 1 ? "1 card" : `${n} cards`;
    if (!window.confirm(`Delete group "${existing.name}" and its ${what}?`)) return;
    session.update(deleteGroup(doc, id));
    session.done();
  };

  const columns = head.layout.columns;

  return (
    <form
      className="editor-form"
      onSubmit={(e) => {
        e.preventDefault();
        if (!nameError) session.done();
      }}
    >
      <h2>{groupId ? "Edit group" : "Add group"}</h2>

      <div className="field">
        <label htmlFor={`${uid}-name`} className="required">
          Name
        </label>
        <input
          id={`${uid}-name`}
          type="text"
          value={head.name}
          aria-required
          aria-invalid={touched && nameError ? true : undefined}
          onChange={(e) => change({ name: e.target.value })}
        />
        {touched && nameError && (
          <span className="field-error" role="alert">
            {nameError}
          </span>
        )}
      </div>

      {!groupId && (
        <div className="field">
          <label htmlFor={`${uid}-parent`}>Parent group</label>
          <select
            id={`${uid}-parent`}
            value={parentId}
            onChange={(e) => {
              const p = e.target.value;
              if (existing) change({}, p);
              setParentId(p);
            }}
          >
            <option value="">Top level</option>
            {groupOptions(doc.groups, id).map((g) => (
              <option key={g.id} value={g.id}>
                {g.label}
              </option>
            ))}
          </select>
        </div>
      )}

      <div className="field">
        <div className="field-inline">
          <input
            id={`${uid}-hidden`}
            type="checkbox"
            checked={head.hidden}
            aria-describedby={`${uid}-hidden-help`}
            onChange={(e) => change({ hidden: e.target.checked })}
          />
          <label htmlFor={`${uid}-hidden`}>Hidden from viewers</label>
        </div>
        <span id={`${uid}-hidden-help`} className="field-help">
          Only admins see this group (dimmed).
        </span>
      </div>

      <div className="field">
        <label htmlFor={`${uid}-columns`}>Columns</label>
        <input
          id={`${uid}-columns`}
          type="number"
          min={1}
          max={8}
          step={1}
          value={columns}
          onChange={(e) => {
            if (e.target.value === "") return;
            change({ layout: { ...head.layout, columns: clampColumns(e.target.valueAsNumber) } });
          }}
        />
      </div>

      <div className="field">
        <label htmlFor={`${uid}-style`}>Layout</label>
        <select
          id={`${uid}-style`}
          value={head.layout.style}
          onChange={(e) =>
            change({ layout: { ...head.layout, style: e.target.value as GroupLayout["style"] } })
          }
        >
          {STYLES.map((s) => (
            <option key={s.value} value={s.value}>
              {s.label}
            </option>
          ))}
        </select>
      </div>

      <div className="drawer-footer btn-row">
        <button type="submit" className="btn btn-primary" disabled={Boolean(nameError)}>
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

/** Clamps a column count to the supported range 1–8. */
function clampColumns(n: number): number {
  return Number.isFinite(n) ? Math.min(8, Math.max(1, Math.round(n))) : 3;
}
