/**
 * Helper for drawer forms: live-preview edits as one undo step.
 */
import { useCallback, useRef } from "react";
import { useApp } from "../../app/context";
import type { ConfigDoc } from "../../api/types";
import { newId } from "../ops";

/** Controls returned by {@link useFormSession}. */
export interface FormSession {
  /** Current working copy. */
  doc: ConfigDoc;
  /** Apply a change to the working copy; all changes in a session coalesce. */
  update(next: ConfigDoc): void;
  /** Keep the changes and close. */
  done(): void;
  /** Revert this session's changes and close. */
  cancel(): void;
}

/**
 * Starts a form session. Every `update` replaces the working copy under one
 * coalesce key, so the dashboard previews the edit live and the whole
 * session is a single undo step; `cancel` undoes it.
 */
export function useFormSession(onClose: () => void): FormSession {
  const app = useApp();
  const key = useRef(newId("form")).current;
  const changed = useRef(false);
  const update = useCallback(
    (next: ConfigDoc) => {
      changed.current = true;
      app.apply(next, key);
    },
    [app, key],
  );
  return {
    doc: app.doc!,
    update,
    done: onClose,
    cancel: () => {
      if (changed.current) app.undo();
      onClose();
    },
  };
}
