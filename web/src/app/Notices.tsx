/**
 * A dismissible notice in the corner, used for changes found by scheduled
 * rescans ("New: Home Assistant at 192.168.1.30:8123").
 */
import { useApp } from "./context";

/** Shows pending notices; renders nothing when there are none. */
export function Notices() {
  const app = useApp();
  // The Find services panel already lists the changes; don't cover it.
  if (app.notices.length === 0 || app.target?.kind === "scan") return null;
  return (
    <div className="notices" role="status" aria-live="polite">
      <div className="notices-head">
        <strong>Network changes found</strong>
        <button
          type="button"
          className="btn btn-icon"
          aria-label="Dismiss"
          onClick={app.dismissNotices}
        >
          ✕
        </button>
      </div>
      <ul>
        {app.notices.map((m, i) => (
          <li key={i}>{m}</li>
        ))}
      </ul>
      {app.me?.role === "admin" && (
        <button
          type="button"
          className="btn"
          onClick={() => {
            app.dismissNotices();
            const show = () => app.open({ kind: "scan" });
            if (app.editing) show();
            else void app.startEditing().then(show);
          }}
        >
          Review in Find services
        </button>
      )}
    </div>
  );
}
