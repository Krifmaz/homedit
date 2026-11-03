/**
 * Edit-mode toolbar: add, undo/redo, history, settings, save and exit.
 * Also binds the keyboard shortcuts (Ctrl/Cmd+Z, Shift+Ctrl/Cmd+Z, Ctrl+Y,
 * Ctrl/Cmd+S) and warns before leaving the page with unsaved changes.
 */
import { useEffect } from "react";
import { useApp } from "./context";

/** Whether a keyboard event comes from a text input (where undo is native). */
function inTextField(e: KeyboardEvent): boolean {
  const t = e.target as HTMLElement | null;
  return !!t?.closest("input, textarea, select, [contenteditable]");
}

/** The sticky toolbar shown in edit mode. */
export function Toolbar() {
  const app = useApp();
  const { undo, redo, save, dirty, saving } = app;

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.ctrlKey || e.metaKey;
      if (!mod) return;
      const k = e.key.toLowerCase();
      if (k === "s") {
        e.preventDefault();
        if (dirty && !saving) void save();
        return;
      }
      if (inTextField(e)) return;
      if (k === "z" && !e.shiftKey) {
        e.preventDefault();
        undo();
      } else if ((k === "z" && e.shiftKey) || k === "y") {
        e.preventDefault();
        redo();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [undo, redo, save, dirty, saving]);

  useEffect(() => {
    if (!dirty) return;
    const onUnload = (e: BeforeUnloadEvent) => e.preventDefault();
    window.addEventListener("beforeunload", onUnload);
    return () => window.removeEventListener("beforeunload", onUnload);
  }, [dirty]);

  const exit = () => {
    if (dirty && !window.confirm("Discard unsaved changes?")) return;
    app.stopEditing();
  };

  return (
    <div className="toolbar" role="toolbar" aria-label="Editor">
      <strong>Editing</strong>
      <button type="button" className="btn" onClick={() => app.open({ kind: "group" })}>
        + Group
      </button>
      <button type="button" className="btn" onClick={() => app.open({ kind: "infoWidgets" })}>
        Info widgets
      </button>
      <button type="button" className="btn" onClick={() => app.open({ kind: "settings" })}>
        Appearance
      </button>
      <button type="button" className="btn" onClick={() => app.open({ kind: "scan" })}>
        Find services
      </button>
      <button type="button" className="btn" onClick={() => app.open({ kind: "backup" })}>
        Import / export
      </button>
      <button type="button" className="btn" onClick={() => app.open({ kind: "history" })}>
        History
      </button>
      {app.me?.authEnabled && (
        <button type="button" className="btn" onClick={() => app.open({ kind: "users" })}>
          Users
        </button>
      )}
      <span className="btn-row">
        <button
          type="button"
          className="btn"
          onClick={undo}
          disabled={!app.canUndo}
          title="Undo (Ctrl+Z)"
        >
          ↶ Undo
        </button>
        <button
          type="button"
          className="btn"
          onClick={redo}
          disabled={!app.canRedo}
          title="Redo (Ctrl+Shift+Z)"
        >
          ↷ Redo
        </button>
      </span>
      <span className="spacer" style={{ flex: 1 }} />
      <span className="status-text" aria-live="polite">
        {saving ? "Saving…" : dirty ? "Unsaved changes" : "All changes saved"}
      </span>
      <button
        type="button"
        className="btn btn-primary"
        onClick={() => void save()}
        disabled={!dirty || saving}
        title="Save (Ctrl+S)"
      >
        Save
      </button>
      <button type="button" className="btn" onClick={exit}>
        Done
      </button>
    </div>
  );
}
