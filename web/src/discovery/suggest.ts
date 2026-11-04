/**
 * Turns scanner results into dashboard cards.
 */
import type { FieldValue, HostResult, ScanJob, Service, ServiceSuggestion } from "../api/types";

/** One service found by a scan, flattened for display. */
export interface FoundService {
  /** `ip:port`, the key of its suggestion. */
  key: string;
  /** Hostname if known, else the address. */
  host: string;
  ip: string;
  port: number;
  /** Recognised app name, page title, or a fallback. */
  name: string;
  /** App id from the catalogue, when recognised. */
  appId?: string;
  /** Widget type available for it, if any. */
  widget?: string;
  /** Confidence of the app match, 0 to 100. */
  confidence?: number;
  /** URL to open. */
  href?: string;
  /** Whether a card already points here. */
  onDashboard: boolean;
  /** Pre-filled card values. */
  suggestion?: ServiceSuggestion;
}

/** Flattens a finished scan, recognised apps first, then by host and port. */
export function foundServices(job: ScanJob): FoundService[] {
  const hosts: HostResult[] = job.report?.hosts ?? [];
  const out: FoundService[] = [];
  for (const h of hosts) {
    for (const s of h.services) {
      const key = `${h.ip}:${s.port}`;
      const sug = job.suggestions[key];
      out.push({
        key,
        host: h.hostname || h.ip,
        ip: h.ip,
        port: s.port,
        name: sug?.name ?? s.app?.name ?? s.http?.title ?? `Port ${s.port}`,
        appId: s.app?.id,
        widget: s.app?.widget ?? undefined,
        confidence: s.app?.confidence,
        href: sug?.href,
        onDashboard: s.on_dashboard,
        suggestion: sug,
      });
    }
  }
  return out.sort(
    (a, b) =>
      Number(!a.appId) - Number(!b.appId) ||
      a.ip.localeCompare(b.ip, undefined, { numeric: true }) ||
      a.port - b.port,
  );
}

/** Builds a card from a suggestion; the widget form then only needs secrets. */
export function suggestionToService(s: ServiceSuggestion, id: string): Service {
  let widget: Service["widget"] = null;
  if (s.widget?.type) {
    const fields: Record<string, FieldValue> = {};
    for (const [k, v] of Object.entries(s.widget)) {
      if (k === "type") continue;
      if (v === null || ["string", "number", "boolean"].includes(typeof v))
        fields[k] = v as FieldValue;
    }
    widget = { type: s.widget.type, fields };
  }
  return {
    id,
    name: s.name,
    href: s.href,
    icon: s.icon ?? s.favicon ?? undefined,
    description: s.description ?? undefined,
    check: { kind: "http" },
    widget,
  };
}

/** Progress of a running scan from 0 to 1, or `null` when unknown. */
export function scanProgress(job: ScanJob): number | null {
  for (let i = job.events.length - 1; i >= 0; i--) {
    const e = job.events[i]!;
    if (e.event === "finished") return 1;
    if (e.event === "progress" && e.total > 0) return Math.min(1, e.done / e.total);
    if (e.event === "started") return 0;
  }
  return null;
}

/** Splits the targets text box into targets (one per line, or comma separated). */
export function parseTargets(text: string): string[] {
  return text
    .split(/[\n,]/)
    .map((t) => t.trim())
    .filter(Boolean);
}

/** A bare IPv4 address whose last octet is 0, such as `10.0.1.0`. */
const NETWORK_ADDRESS = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.0$/;

/**
 * Targets that look like a network written without its prefix length
 * (`10.0.1.0` rather than `10.0.1.0/24`). Scanning one of those checks just
 * that single, usually unused, address.
 */
export function bareNetworks(text: string): string[] {
  return parseTargets(text).filter((t) => {
    const m = NETWORK_ADDRESS.exec(t);
    return m !== null && m.slice(1).every((o) => Number(o) <= 255);
  });
}

/** Rewrites every bare network address in `text` as a /24 range. */
export function widenBareNetworks(text: string): string {
  const bare = new Set(bareNetworks(text));
  return text.replace(/[^\s,]+/g, (t) => (bare.has(t) ? `${t}/24` : t));
}

/**
 * The question asked before scanning bare network addresses: offer their /24
 * ranges instead of the single addresses.
 */
export function widenQuestion(bare: string[]): string {
  return bare.length === 1
    ? `${bare[0]} is a network address, so only that one address would be scanned. Scan ${bare[0]}/24 (the whole network) instead?`
    : `${bare.join(", ")} are network addresses, so only those addresses would be scanned. Scan their /24 ranges (the whole networks) instead?`;
}
