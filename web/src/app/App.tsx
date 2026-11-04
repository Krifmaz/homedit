/**
 * The app shell: background, header (title, search, edit button), info
 * widgets, the dashboard, and in edit mode the toolbar and drawer.
 */
import { useMemo, useState } from "react";
import { Dashboard, filterGroups } from "../components/Dashboard";
import { SearchBar } from "../components/SearchBar";
import { InfoWidgetStrip } from "../components/widgets/InfoWidgetStrip";
import { AccountMenu } from "../auth/AccountMenu";
import { LoginPage } from "../auth/LoginPage";
import { Drawer } from "../editor/Drawer";
import { allGroups } from "../editor/ops";
import { useApp } from "./context";
import { Notices } from "./Notices";
import { backgroundFilter, cssUrl, useApplySettings } from "./theme";
import { Toolbar } from "./Toolbar";

/** Root component. */
export function App() {
  const app = useApp();
  const [filter, setFilter] = useState("");
  const doc = app.doc;
  useApplySettings(doc?.settings);

  const singleMatch = useMemo(() => {
    if (!doc || !filter.trim()) return undefined;
    const hits = allGroups({ groups: filterGroups(doc.groups, filter) }).flatMap((g) => g.items);
    return hits.length === 1 ? hits[0]!.href : undefined;
  }, [doc, filter]);

  if (app.needsLogin) return <LoginPage />;

  const bg = doc?.settings.background;
  return (
    <>
      {bg?.url && (
        <div
          className="page-bg"
          aria-hidden
          style={{
            backgroundImage: cssUrl(bg.url),
            filter: backgroundFilter(bg) || undefined,
            opacity: bg.opacity,
            // Blur fades the edges towards transparent; overscan hides that.
            transform: bg.blur ? "scale(1.06)" : undefined,
          }}
        />
      )}
      <div className={`app${app.editing ? " editing" : ""}${app.target ? " with-drawer" : ""}`}>
        {app.editing && <Toolbar />}
        {app.error && (
          <div className="error-banner" role="alert">
            <span style={{ flex: 1 }}>{app.error}</span>
            {app.editing && /changed elsewhere/.test(app.error) && (
              <button
                type="button"
                className="btn"
                onClick={() => void app.reload().then(app.clearError)}
              >
                Reload
              </button>
            )}
            <button
              type="button"
              className="btn btn-icon"
              aria-label="Dismiss"
              onClick={app.clearError}
            >
              ✕
            </button>
          </div>
        )}
        <header className="header">
          <h1>{doc?.settings.title || "homedit"}</h1>
          {doc && (
            <SearchBar
              value={filter}
              onChange={setFilter}
              settings={doc.settings.search}
              singleMatch={singleMatch}
            />
          )}
          <span className="spacer" />
          {app.me?.role === "admin" && !app.editing && (
            <button type="button" className="btn" onClick={() => void app.startEditing()}>
              ✎ Edit
            </button>
          )}
          <AccountMenu />
        </header>
        {doc && doc.infoWidgets.length > 0 && <InfoWidgetStrip widgets={doc.infoWidgets} />}
        <main>
          <Dashboard filter={filter} />
        </main>
      </div>
      {(app.editing || app.target?.kind === "password") && <Drawer />}
      <Notices />
    </>
  );
}
