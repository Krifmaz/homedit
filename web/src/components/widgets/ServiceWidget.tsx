/**
 * Live stats of a service card's widget (Sonarr queue, Plex streams, ...).
 *
 * The browser only ever talks to homedit: the server calls the upstream
 * service with the stored API key and returns plain stats.
 */
import type { Service } from "../../api/types";
import { useApp } from "../../app/context";
import { formatStatValue } from "./format";
import { usePolling } from "./usePolling";
import "./widgets.css";

/** How often a service widget refreshes, in milliseconds. */
export const SERVICE_WIDGET_INTERVAL_MS = 15_000;

/** Most stats shown on one card. */
export const MAX_STATS = 4;

/** Props of {@link ServiceWidget}. */
export interface ServiceWidgetProps {
  /** The service whose widget to show. */
  service: Service;
}

/**
 * Renders the widget stats of `service`, or nothing when it has no widget.
 *
 * Shows skeleton blocks until the first response, at most
 * {@link MAX_STATS} stats, and a compact error line when the upstream call
 * failed. API failures never throw.
 */
export function ServiceWidget({ service }: ServiceWidgetProps) {
  if (!service.widget) return null;
  return <LiveStats serviceId={service.id} />;
}

/** Polling part of {@link ServiceWidget}, mounted only when there is a widget. */
function LiveStats({ serviceId }: { serviceId: string }) {
  const { api } = useApp();
  const { data, error, loading } = usePolling(
    () => api.widgetData(serviceId),
    SERVICE_WIDGET_INTERVAL_MS,
    [serviceId],
  );

  if (loading) {
    return (
      <div className="widget-stats" aria-busy="true" aria-label="Loading widget">
        {[0, 1, 2].map((i) => (
          <div key={i} className="stat stat-skeleton">
            <div className="stat-value">&nbsp;</div>
            <div className="stat-label">&nbsp;</div>
          </div>
        ))}
      </div>
    );
  }

  const stats = (data?.fields ?? []).slice(0, MAX_STATS);
  const problem = error ?? data?.error ?? null;
  if (stats.length === 0 && !problem) return null;

  return (
    <div className="service-widget">
      {stats.length > 0 && (
        <div className="widget-stats">
          {stats.map((s, i) => (
            <div key={`${s.label}-${i}`} className="stat">
              <div className="stat-value">
                {formatStatValue(s.value)}
                {s.unit && <span className="stat-unit"> {s.unit}</span>}
              </div>
              <div className="stat-label">{s.label}</div>
            </div>
          ))}
        </div>
      )}
      {problem && (
        <div className="widget-error" role="status" title={problem}>
          {problem}
        </div>
      )}
    </div>
  );
}
