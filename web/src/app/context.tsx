/**
 * Application-wide state: API client, session, dashboard document, live
 * status, and the editor (undo/redo, save, open form).
 */
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useReducer,
  useState,
  type ReactNode,
} from "react";
import { ApiError, type Api, type InlineData } from "../api/client";
import type { ConfigDoc, DashboardData, Me, StatusMap, WidgetSchema } from "../api/types";
import { initUndo, isDirty, undoReducer } from "../editor/history";

/** What the editor drawer is currently showing. */
export type EditTarget =
  | { kind: "service"; groupId: string; serviceId?: string }
  | { kind: "group"; groupId?: string }
  | { kind: "bookmarkGroup"; groupId?: string }
  | { kind: "bookmark"; groupId: string; bookmarkId?: string }
  | { kind: "infoWidgets" }
  | { kind: "settings" }
  | { kind: "history" }
  | { kind: "users" }
  | { kind: "password" };

/** Everything components read from {@link useApp}. */
export interface AppState {
  /** API client in use (HTTP or mock). */
  api: Api;
  /** Current session, `null` until loaded. */
  me: Me | null;
  /** Document rendered on screen: the editor's working copy in edit mode. */
  doc: DashboardData | null;
  /** Live status of services by id. */
  status: StatusMap;
  /** Widget catalogue, loaded on first entry into edit mode. */
  catalog: WidgetSchema[];
  /** Whether edit mode is on. */
  editing: boolean;
  /** Whether the working copy has unsaved changes. */
  dirty: boolean;
  /** Whether undo / redo are possible. */
  canUndo: boolean;
  canRedo: boolean;
  /** Last error to show in the toolbar, if any. */
  error: string | null;
  /** Whether a save is running. */
  saving: boolean;
  /** Form or panel open in the editor drawer. */
  target: EditTarget | null;
  /** Enter edit mode (admins only); loads the full config and catalogue. */
  startEditing(): Promise<void>;
  /** Leave edit mode, discarding unsaved changes. */
  stopEditing(): void;
  /**
   * Replace the working copy with `next`.
   * @param coalesceKey edits with the same key collapse into one undo step.
   */
  apply(next: ConfigDoc, coalesceKey?: string): void;
  undo(): void;
  redo(): void;
  /** Save the working copy to the server. */
  save(): Promise<void>;
  /** Reload the working copy from the server (e.g. after a restore). */
  reload(): Promise<void>;
  /** Open a form or panel in the drawer, or close it with `null`. */
  open(target: EditTarget | null): void;
  /** Clear the error message. */
  clearError(): void;
  /** Whether the visitor must sign in (or run first-time setup) first. */
  needsLogin: boolean;
  /** Show (true) or leave (false) the sign-in page while browsing anonymously. */
  requestLogin(show: boolean): void;
  /** Whether the sign-in page was opened voluntarily (it can be left). */
  loginOptional: boolean;
  /** Refetch the session and dashboard, e.g. after signing in. */
  refreshSession(): void;
  /** Sign out and return to the anonymous view or the login page. */
  logout(): Promise<void>;
}

const Ctx = createContext<AppState | null>(null);

/** Access the application state. Must be used inside {@link AppProvider}. */
export function useApp(): AppState {
  const v = useContext(Ctx);
  if (!v) throw new Error("useApp must be used inside <AppProvider>");
  return v;
}

/** Props of {@link AppProvider}. */
export interface AppProviderProps {
  api: Api;
  /** Data the server inlined into the page for the first paint. */
  initial?: InlineData | null;
  children: ReactNode;
}

/** Provides {@link AppState} to the tree. */
export function AppProvider({ api, initial = null, children }: AppProviderProps) {
  const [me, setMe] = useState<Me | null>(null);
  const [view, setView] = useState<DashboardData | null>(initial?.dashboard ?? null);
  const [status, setStatus] = useState<StatusMap>(initial?.status ?? {});
  const [catalog, setCatalog] = useState<WidgetSchema[]>([]);
  const [editing, setEditing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [target, setTarget] = useState<EditTarget | null>(null);
  const [undo, dispatch] = useReducer(undoReducer, null, () =>
    initUndo(
      initial?.dashboard ?? {
        version: "",
        settings: {
          title: "",
          theme: "system",
          accent: "#3b6fd8",
          background: null,
          customCss: "",
          search: { provider: "duckduckgo" },
        },
        groups: [],
        bookmarks: [],
        infoWidgets: [],
      },
    ),
  );

  const fail = useCallback((e: unknown) => {
    setError(e instanceof Error ? e.message : String(e));
  }, []);

  const [session, setSession] = useState(0);
  const [unauthorized, setUnauthorized] = useState(false);
  const [wantLogin, setWantLogin] = useState(false);

  useEffect(() => {
    let alive = true;
    const loadFail = (e: unknown) => {
      if (!alive) return;
      if (e instanceof ApiError && e.status === 401) setUnauthorized(true);
      else fail(e);
    };
    api.me().then((m) => alive && setMe(m), loadFail);
    api.dashboard().then((d) => {
      if (!alive) return;
      setView(d);
      setUnauthorized(false);
    }, loadFail);
    api.status().then(
      (s) => alive && setStatus(s),
      () => {},
    );
    const close = api.subscribeStatus(
      (e) =>
        setStatus((prev) => {
          const old = prev[e.serviceId];
          const history = [
            ...(old?.history ?? []).slice(-59),
            { t: e.checkedAt, up: e.up, ms: e.latencyMs },
          ];
          return { ...prev, [e.serviceId]: { ...e, history } };
        }),
      () => api.dashboard().then((d) => alive && setView(d), loadFail),
    );
    return () => {
      alive = false;
      close();
    };
  }, [api, fail, session]);

  const refreshSession = useCallback(() => {
    setEditing(false);
    setTarget(null);
    setError(null);
    setWantLogin(false);
    setSession((n) => n + 1);
  }, []);

  const logout = useCallback(async () => {
    try {
      const url = await api.logout();
      if (url) {
        window.location.assign(url);
        return;
      }
      setView(null);
      refreshSession();
    } catch (e) {
      fail(e);
    }
  }, [api, fail, refreshSession]);

  const startEditing = useCallback(async () => {
    try {
      const [doc, cat] = await Promise.all([api.config(), api.catalog()]);
      dispatch({ type: "reset", doc });
      setCatalog(cat);
      setEditing(true);
    } catch (e) {
      fail(e);
    }
  }, [api, fail]);

  const stopEditing = useCallback(() => {
    setEditing(false);
    setTarget(null);
  }, []);

  const reload = useCallback(async () => {
    try {
      const doc = await api.config();
      dispatch({ type: "reset", doc });
      setView(await api.dashboard());
    } catch (e) {
      fail(e);
    }
  }, [api, fail]);

  const save = useCallback(async () => {
    setSaving(true);
    try {
      const version = await api.saveConfig(undo.present);
      dispatch({ type: "saved", version });
      // Pick up ids the server assigned to new items, so later saves and
      // stored secrets line up with them. Undo history is kept.
      const [fresh, view] = await Promise.all([api.config(), api.dashboard()]);
      dispatch({ type: "synced", doc: fresh });
      setTarget((t) => (t && "groupId" in t ? null : t));
      setView(view);
      setError(null);
    } catch (e) {
      fail(e);
    } finally {
      setSaving(false);
    }
  }, [api, undo.present, fail]);

  const value = useMemo<AppState>(
    () => ({
      api,
      me,
      doc: editing ? undo.present : view,
      status,
      catalog,
      editing,
      dirty: editing && isDirty(undo),
      canUndo: undo.past.length > 0,
      canRedo: undo.future.length > 0,
      error,
      saving,
      target,
      startEditing,
      stopEditing,
      apply: (next, coalesceKey) => dispatch({ type: "apply", doc: next, coalesceKey }),
      undo: () => dispatch({ type: "undo" }),
      redo: () => dispatch({ type: "redo" }),
      save,
      reload,
      open: setTarget,
      clearError: () => setError(null),
      needsLogin:
        wantLogin ||
        unauthorized ||
        !!me?.setupRequired ||
        (!!me && me.authEnabled && me.role === null),
      requestLogin: setWantLogin,
      loginOptional: wantLogin && !unauthorized && !me?.setupRequired && !!me && me.role !== null,
      refreshSession,
      logout,
    }),
    [
      api,
      me,
      editing,
      undo,
      view,
      status,
      catalog,
      error,
      saving,
      target,
      startEditing,
      stopEditing,
      save,
      reload,
      unauthorized,
      wantLogin,
      refreshSession,
      logout,
    ],
  );

  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}
