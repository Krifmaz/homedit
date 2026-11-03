/**
 * Up/down indicator with the last response time and a sparkline.
 */
import type { Check, ServiceStatus } from "../api/types";
import { Sparkline } from "./Sparkline";

/** Props of {@link StatusBadge}. */
export interface StatusBadgeProps {
  /** The service's check; nothing is shown without one. */
  check: Check | null;
  /** Latest status, if any has arrived. */
  status?: ServiceStatus;
}

/** Human text for a status, e.g. "Up · 200 · 42 ms". Exported for tests. */
export function statusText(check: Check, status?: ServiceStatus): string {
  if (!status) return "Checking…";
  const parts = [status.up ? "Up" : "Down"];
  if (check.kind === "http" && status.code !== undefined) parts.push(String(status.code));
  if (status.up && status.latencyMs !== undefined) parts.push(`${Math.round(status.latencyMs)} ms`);
  return parts.join(" · ");
}

/** Status dot, text and history sparkline for a card. */
export function StatusBadge({ check, status }: StatusBadgeProps) {
  if (!check) return null;
  const text = statusText(check, status);
  const cls = status ? (status.up ? "up" : "down") : "";
  return (
    <div className="status" title={`${check.kind.toUpperCase()} check: ${text}`}>
      <span style={{ display: "flex", alignItems: "center", gap: 5 }}>
        <span className={`status-dot ${cls}`} aria-hidden />
        <span>{text}</span>
      </span>
      {status && <Sparkline samples={status.history} />}
    </div>
  );
}
