/**
 * Undo/redo state for the editor.
 *
 * The state holds immutable snapshots of the config document. Consecutive
 * edits that share a `coalesceKey` (for example typing in one text field)
 * collapse into a single undo step.
 */
import type { ConfigDoc } from "../api/types";

/** Maximum number of undo steps kept. */
export const MAX_UNDO = 100;

/** Undo/redo stacks around the current document. */
export interface UndoState {
  /** Earlier snapshots, oldest first. */
  past: ConfigDoc[];
  /** The document being edited. */
  present: ConfigDoc;
  /** Snapshots undone, most recently undone last. */
  future: ConfigDoc[];
  /** Snapshot last saved to (or loaded from) the server. */
  saved: ConfigDoc;
  /** Key of the last edit, for coalescing. */
  lastKey: string | null;
}

/** Actions accepted by {@link undoReducer}. */
export type UndoAction =
  | { type: "reset"; doc: ConfigDoc }
  | { type: "apply"; doc: ConfigDoc; coalesceKey?: string }
  | { type: "undo" }
  | { type: "redo" }
  | { type: "saved"; version: string }
  | { type: "synced"; doc: ConfigDoc };

/** Creates the initial state for `doc`. */
export function initUndo(doc: ConfigDoc): UndoState {
  return { past: [], present: doc, future: [], saved: doc, lastKey: null };
}

/** Reducer for the undo/redo state. */
export function undoReducer(state: UndoState, action: UndoAction): UndoState {
  switch (action.type) {
    case "reset":
      return initUndo(action.doc);
    case "apply": {
      if (action.doc === state.present) return state;
      const key = action.coalesceKey ?? null;
      if (key !== null && key === state.lastKey && state.past.length > 0) {
        return { ...state, present: action.doc, future: [] };
      }
      const past = [...state.past, state.present].slice(-MAX_UNDO);
      return { ...state, past, present: action.doc, future: [], lastKey: key };
    }
    case "undo": {
      const prev = state.past[state.past.length - 1];
      if (!prev) return state;
      return {
        ...state,
        past: state.past.slice(0, -1),
        present: prev,
        future: [...state.future, state.present],
        lastKey: null,
      };
    }
    case "redo": {
      const next = state.future[state.future.length - 1];
      if (!next) return state;
      return {
        ...state,
        past: [...state.past, state.present],
        present: next,
        future: state.future.slice(0, -1),
        lastKey: null,
      };
    }
    case "synced":
      // The server's copy of what was just saved (with server-assigned ids).
      return { ...state, present: action.doc, saved: action.doc, lastKey: null };
    case "saved": {
      const stamp = (d: ConfigDoc) => ({ ...d, version: action.version });
      const present = stamp(state.present);
      return {
        past: state.past.map(stamp),
        present,
        future: state.future.map(stamp),
        saved: present,
        lastKey: null,
      };
    }
  }
}

/** Whether the present document differs from the last saved one. */
export function isDirty(state: UndoState): boolean {
  if (state.present === state.saved) return false;
  return JSON.stringify(state.present) !== JSON.stringify(state.saved);
}
