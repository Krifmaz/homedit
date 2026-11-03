/**
 * Test helpers for the editor forms: mounts a form inside an
 * {@link AppProvider} backed by {@link MockApi}, already in edit mode.
 */
import { render, screen } from "@testing-library/react";
import { useEffect, type ReactNode } from "react";
import { AppProvider, useApp, type AppState } from "../../app/context";
import type { Api } from "../../api/client";
import { MockApi } from "../../api/mock";

/** Latest app state seen by {@link renderForm}'s probe; read it in assertions. */
export const probe: { app: AppState | null } = { app: null };

/** Enters edit mode, then renders `children` once the working copy is loaded. */
function Editing({ children }: { children: ReactNode }) {
  const app = useApp();
  const { startEditing } = app;
  useEffect(() => {
    void startEditing();
  }, [startEditing]);
  useEffect(() => {
    probe.app = app;
  });
  return app.editing ? <>{children}</> : null;
}

/**
 * Renders `form` in edit mode against a fresh mock API and waits for it to
 * appear (detected by its heading).
 *
 * @returns the render result plus the API instance in use.
 */
export async function renderForm(form: ReactNode, api: Api = new MockApi({ liveEvents: false })) {
  probe.app = null;
  const result = render(
    <AppProvider api={api}>
      <Editing>{form}</Editing>
    </AppProvider>,
  );
  await screen.findByRole("heading", { level: 2 });
  return { ...result, api };
}

/** The current working copy; throws if the app has not loaded. */
export function currentDoc() {
  const doc = probe.app?.doc;
  if (!doc) throw new Error("no document loaded");
  return doc;
}
