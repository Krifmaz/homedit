/**
 * Version history panel of the editor drawer: lists saved versions, shows
 * what restoring one would change, and restores it.
 */
import { useCallback, useEffect, useState } from "react";
import type { ConfigDoc, Group, HistoryEntry } from "../api/types";
import { useApp } from "../app/context";
import "./history-panel.css";

/** Added / removed / changed counts for one kind of item. */
export interface ChangeCounts {
  added: number;
  removed: number;
  changed: number;
}

/** Result of {@link diffSummary}. */
export interface DiffSummary {
  /** Service groups, including nested sub-groups (their own fields only). */
  groups: ChangeCounts;
  /** Service cards; a card moved to another group counts as changed. */
  services: ChangeCounts;
  /** Individual bookmarks; a bookmark moved to another list counts as changed. */
  bookmarks: ChangeCounts;
  /** Whether the dashboard settings differ. */
  settings: boolean;
  /** Whether the info widgets differ (any add, remove, edit or reorder). */
  infoWidgets: boolean;
}

/** Structural equality for JSON-like values, independent of key order. */
function same(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (typeof a !== "object" || typeof b !== "object" || a === null || b === null) return false;
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  if (Array.isArray(a) && Array.isArray(b)) {
    return a.length === b.length && a.every((v, i) => same(v, b[i]));
  }
  const ao = a as Record<string, unknown>;
  const bo = b as Record<string, unknown>;
  // Keys whose value is undefined are treated as absent.
  const ak = Object.keys(ao).filter((k) => ao[k] !== undefined);
  const bk = Object.keys(bo).filter((k) => bo[k] !== undefined);
  return ak.length === bk.length && ak.every((k) => same(ao[k], bo[k]));
}

/** Counts additions, removals and changes between two id-keyed maps. */
function countChanges(a: Map<string, unknown>, b: Map<string, unknown>): ChangeCounts {
  const out: ChangeCounts = { added: 0, removed: 0, changed: 0 };
  for (const [id, va] of a) {
    if (!b.has(id)) out.removed++;
    else if (!same(va, b.get(id))) out.changed++;
  }
  for (const id of b.keys()) if (!a.has(id)) out.added++;
  return out;
}

/** Flattens groups and sub-groups, recording each group's parent id. */
function walkGroups(
  groups: Group[],
  parent: string | null,
  visit: (g: Group, parent: string | null) => void,
) {
  for (const g of groups) {
    visit(g, parent);
    walkGroups(g.groups ?? [], g.id, visit);
  }
}

/**
 * Summarises how config `b` differs from config `a` (the changes that turn
 * `a` into `b`), matching groups, services and bookmarks by id.
 *
 * A group counts as changed when its own fields (name, icon, visibility,
 * layout, parent) differ, or when the order of its cards changed; edits to
 * the cards themselves are counted under `services` only. The `version`
 * field is ignored.
 *
 * @param a the reference document (e.g. the current config).
 * @param b the document to compare with it (e.g. a past version).
 */
export function diffSummary(a: ConfigDoc, b: ConfigDoc): DiffSummary {
  const index = (doc: ConfigDoc) => {
    const groups = new Map<string, unknown>();
    const services = new Map<string, unknown>();
    const bookmarks = new Map<string, unknown>();
    walkGroups(doc.groups, null, (g, parent) => {
      // eslint-disable-next-line @typescript-eslint/no-unused-vars -- dropped: sub-groups are indexed on their own
      const { items, groups: _sub, ...own } = g;
      groups.set(g.id, { ...own, parent, order: items.map((s) => s.id) });
      for (const s of items) services.set(s.id, { ...s, group: g.id });
    });
    for (const bg of doc.bookmarks) {
      for (const bm of bg.items) bookmarks.set(bm.id, { ...bm, list: bg.id });
    }
    return { groups, services, bookmarks };
  };
  const ia = index(a);
  const ib = index(b);
  return {
    groups: countChanges(ia.groups, ib.groups),
    services: countChanges(ia.services, ib.services),
    bookmarks: countChanges(ia.bookmarks, ib.bookmarks),
    settings: !same(a.settings, b.settings),
    infoWidgets: !same(a.infoWidgets, b.infoWidgets),
  };
}

/** Whether a {@link DiffSummary} records no difference at all. */
export function isEmptyDiff(d: DiffSummary): boolean {
  const zero = (c: ChangeCounts) => c.added + c.removed + c.changed === 0;
  return zero(d.groups) && zero(d.services) && zero(d.bookmarks) && !d.settings && !d.infoWidgets;
}

/**
 * Formats a timestamp relative to `now`: "just now", "5 min ago",
 * "3 h ago", "2 days ago", and the local date for anything older than 30
 * days. Future times (clock skew) read as "just now".
 *
 * @param iso ISO 8601 timestamp.
 * @param now reference time in milliseconds since the epoch.
 */
export function relativeTime(iso: string, now: number): string {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return iso;
  const sec = Math.round((now - t) / 1000);
  if (sec < 45) return "just now";
  const min = Math.round(sec / 60);
  if (min < 60) return `${min} min ago`;
  const h = Math.round(min / 60);
  if (h < 24) return `${h} h ago`;
  const days = Math.round(h / 24);
  if (days <= 30) return days === 1 ? "yesterday" : `${days} days ago`;
  return new Date(t).toLocaleDateString();
}

/** Renders one "N added, M removed, K changed" line, or nothing if all zero. */
function CountsLine({ label, c }: { label: string; c: ChangeCounts }) {
  const parts = [
    c.added > 0 && `${c.added} added`,
    c.removed > 0 && `${c.removed} removed`,
    c.changed > 0 && `${c.changed} changed`,
  ].filter(Boolean);
  if (parts.length === 0) return null;
  return (
    <li>
      <strong>{label}:</strong> {parts.join(", ")}
    </li>
  );
}

/** What the selected entry would change, versus the current document. */
type Preview =
  | { version: string; state: "loading" }
  | { version: string; state: "error"; error: string }
  | { version: string; state: "ready"; diff: DiffSummary };

/** Props of {@link HistoryPanel}. */
export interface HistoryPanelProps {
  /** Closes the drawer. */
  onClose: () => void;
}

/**
 * Lists saved config versions, newest first. Selecting one shows what
 * restoring it would change compared with the document on screen; "Restore"
 * (after a confirmation, which warns about unsaved edits) restores it on the
 * server and reloads the editor.
 */
export function HistoryPanel({ onClose }: HistoryPanelProps) {
  const { api, doc, dirty, reload } = useApp();
  const [entries, setEntries] = useState<HistoryEntry[] | null>(null);
  const [listError, setListError] = useState<string | null>(null);
  const [preview, setPreview] = useState<Preview | null>(null);
  const [restoring, setRestoring] = useState<string | null>(null);
  const [restoreError, setRestoreError] = useState<string | null>(null);
  const [now, setNow] = useState(() => Date.now());

  const refresh = useCallback(async () => {
    try {
      const list = await api.history();
      setEntries(list);
      setListError(null);
      setNow(Date.now());
    } catch (e) {
      setListError(e instanceof Error ? e.message : String(e));
    }
  }, [api]);

  useEffect(() => {
    let alive = true;
    api.history().then(
      (list) => alive && setEntries(list),
      (e: unknown) => alive && setListError(e instanceof Error ? e.message : String(e)),
    );
    const timer = setInterval(() => setNow(Date.now()), 60_000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [api]);

  const select = async (version: string) => {
    if (preview?.version === version) {
      setPreview(null);
      return;
    }
    setPreview({ version, state: "loading" });
    try {
      const past = await api.historyVersion(version);
      const diff = doc ? diffSummary(doc, past) : null;
      setPreview((p) =>
        p?.version !== version
          ? p
          : diff
            ? { version, state: "ready", diff }
            : { version, state: "error", error: "Current config not loaded" },
      );
    } catch (e) {
      const error = e instanceof Error ? e.message : String(e);
      setPreview((p) => (p?.version === version ? { version, state: "error", error } : p));
    }
  };

  const restore = async (entry: HistoryEntry) => {
    const warning = dirty ? "\n\nYou have unsaved changes; they will be lost." : "";
    if (!window.confirm(`Restore version ${entry.version} ("${entry.summary}")?${warning}`)) return;
    setRestoring(entry.version);
    setRestoreError(null);
    try {
      await api.restore(entry.version);
      await reload();
      setPreview(null);
      await refresh();
    } catch (e) {
      setRestoreError(e instanceof Error ? e.message : String(e));
    } finally {
      setRestoring(null);
    }
  };

  return (
    <div className="history-panel">
      <h2>Version history</h2>
      {listError && (
        <p className="field-error" role="alert">
          Could not load history: {listError}
        </p>
      )}
      {restoreError && (
        <p className="field-error" role="alert">
          Restore failed: {restoreError}
        </p>
      )}
      {!entries && !listError && <p className="field-help">Loading…</p>}
      {entries && entries.length === 0 && <p className="field-help">No saved versions yet.</p>}
      {entries && entries.length > 0 && (
        <ol className="history-list">
          {entries.map((entry, i) => {
            const current = i === 0;
            const selected = preview?.version === entry.version;
            const when = new Date(entry.createdAt);
            return (
              <li
                key={entry.version}
                className={`history-entry${selected ? " is-selected" : ""}`}
                data-testid={`history-${entry.version}`}
              >
                <button
                  type="button"
                  className="history-select"
                  aria-expanded={selected}
                  onClick={() => void select(entry.version)}
                >
                  <span className="history-summary">{entry.summary}</span>
                  <span className="history-meta">
                    {entry.author} ·{" "}
                    <time
                      dateTime={entry.createdAt}
                      title={Number.isNaN(when.getTime()) ? entry.createdAt : when.toLocaleString()}
                    >
                      {relativeTime(entry.createdAt, now)}
                    </time>{" "}
                    · <code>{entry.version}</code>
                    {current && <span className="badge history-current">current</span>}
                  </span>
                </button>
                {selected && <PreviewBox preview={preview} />}
                <div className="btn-row">
                  <button
                    type="button"
                    className="btn"
                    disabled={current || restoring !== null}
                    title={current ? "This is the current version" : undefined}
                    onClick={() => void restore(entry)}
                  >
                    {restoring === entry.version ? "Restoring…" : "Restore"}
                  </button>
                </div>
              </li>
            );
          })}
        </ol>
      )}
      <div className="drawer-footer btn-row">
        <button type="button" className="btn" onClick={onClose}>
          Close
        </button>
      </div>
    </div>
  );
}

/** The diff summary box under a selected entry. */
function PreviewBox({ preview }: { preview: Preview }) {
  if (preview.state === "loading") return <p className="field-help">Comparing…</p>;
  if (preview.state === "error") {
    return <p className="field-error">Could not load this version: {preview.error}</p>;
  }
  const d = preview.diff;
  if (isEmptyDiff(d)) {
    return <p className="field-help history-diff">Same as the current config.</p>;
  }
  return (
    <div className="history-diff">
      <p className="field-help">Restoring this version would change:</p>
      <ul>
        <CountsLine label="Groups" c={d.groups} />
        <CountsLine label="Services" c={d.services} />
        <CountsLine label="Bookmarks" c={d.bookmarks} />
        {d.settings && (
          <li>
            <strong>Settings</strong> changed
          </li>
        )}
        {d.infoWidgets && (
          <li>
            <strong>Info widgets</strong> changed
          </li>
        )}
      </ul>
    </div>
  );
}
