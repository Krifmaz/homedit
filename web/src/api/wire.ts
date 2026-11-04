/**
 * Translation between the server's JSON and the shapes the UI works with.
 *
 * The server stores ids as integers (`null` for a new item), versions as
 * integers, timestamps as Unix milliseconds, and keeps gethomepage-style
 * settings (an accent palette name, background blur as a CSS size, opacity
 * in percent, search URL templates with `{q}`). The UI uses string ids it
 * can mint locally, string versions, ISO timestamps, a CSS accent colour, a
 * blur in pixels, an opacity from 0 to 1 and `%s` templates.
 *
 * Every function here is pure, so the mapping is tested without a server.
 */
import type {
  Background,
  BookmarkGroup,
  ConfigDoc,
  FieldValue,
  Group,
  HistoryEntry,
  InfoWidget,
  SearchSettings,
  Service,
  Settings,
  StatusEvent,
  StatusMap,
} from "./types";

/** A JSON object of unknown shape. */
type Obj = Record<string, unknown>;

const isObj = (v: unknown): v is Obj => typeof v === "object" && v !== null && !Array.isArray(v);
const str = (v: unknown): string | undefined => (typeof v === "string" ? v : undefined);
const num = (v: unknown): number | undefined =>
  typeof v === "number" && Number.isFinite(v) ? v : undefined;
const arr = (v: unknown): unknown[] => (Array.isArray(v) ? v : []);

/** Tailwind 500 shades for the accent palette names gethomepage uses. */
export const PALETTE: Record<string, string> = {
  slate: "#64748b",
  gray: "#6b7280",
  zinc: "#71717a",
  neutral: "#737373",
  stone: "#78716c",
  red: "#ef4444",
  orange: "#f97316",
  amber: "#f59e0b",
  yellow: "#eab308",
  lime: "#84cc16",
  green: "#22c55e",
  emerald: "#10b981",
  teal: "#14b8a6",
  cyan: "#06b6d4",
  sky: "#0ea5e9",
  blue: "#3b82f6",
  indigo: "#6366f1",
  violet: "#8b5cf6",
  purple: "#a855f7",
  fuchsia: "#d946ef",
  pink: "#ec4899",
  rose: "#f43f5e",
  white: "#ffffff",
};

/** Accent colour used when the server has none. */
export const DEFAULT_ACCENT = "#3b6fd8";

/** Tailwind blur sizes gethomepage accepts, in pixels. */
const BLUR: Record<string, number> = {
  none: 0,
  sm: 4,
  "": 8,
  md: 12,
  lg: 16,
  xl: 24,
  "2xl": 40,
  "3xl": 64,
};

/** Parses a stored blur (`"sm"`, `"8px"`, `"8"`, `8`) into pixels. */
export function blurToPx(v: unknown): number {
  if (typeof v === "number") return Math.max(0, v);
  if (typeof v !== "string") return 0;
  const s = v.trim().toLowerCase();
  if (s in BLUR) return BLUR[s]!;
  const n = parseFloat(s);
  return Number.isFinite(n) ? Math.max(0, n) : 0;
}

/** Reads a stored percentage (0 to 200), defaulting to 100 (unchanged). */
function percentOr100(v: unknown): number {
  const n = num(v);
  return n === undefined ? 100 : Math.min(200, Math.max(0, Math.round(n)));
}

/** Writes a percentage, leaving out the default so stored configs stay small. */
function wirePercent(n: number): number | null {
  const v = Math.round(Math.min(200, Math.max(0, n)));
  return v === 100 ? null : v;
}

/** Ids the server assigned are integers; anything else was minted in the UI. */
export function toWireId(id: string | undefined): number | null {
  return id && /^\d+$/.test(id) ? Number(id) : null;
}

/** Turns a server id into a UI id, minting one for items without an id. */
function fromWireId(v: unknown, prefix: string, next: () => number): string {
  if (typeof v === "number" || (typeof v === "string" && v !== "")) return String(v);
  return `${prefix}-new${next()}`;
}

/** Reads a settings object from the server. */
export function settingsFromWire(v: unknown): Settings {
  const s = isObj(v) ? v : {};
  const accentRaw = str(s.accent)?.trim() ?? "";
  const accent = PALETTE[accentRaw.toLowerCase()] ?? (accentRaw || DEFAULT_ACCENT);
  let background: Background | null = null;
  if (isObj(s.background) && str(s.background.url)) {
    const op = num(s.background.opacity);
    background = {
      url: str(s.background.url)!,
      blur: blurToPx(s.background.blur),
      opacity: op === undefined ? 1 : Math.min(1, Math.max(0, op / 100)),
      saturate: percentOr100(s.background.saturate),
      brightness: percentOr100(s.background.brightness),
    };
  }
  let search: SearchSettings = { provider: "duckduckgo" };
  if (isObj(s.search)) {
    const p = str(s.search.provider);
    const url = str(s.search.url)?.replace("{q}", "%s");
    search =
      p === "google" || p === "duckduckgo" || p === "bing"
        ? { provider: p }
        : url
          ? { provider: "custom", url }
          : { provider: "duckduckgo" };
  }
  const theme = str(s.theme);
  return {
    title: str(s.title) ?? "",
    theme: theme === "light" || theme === "dark" ? theme : "system",
    accent,
    background,
    cardBlur: blurToPx(s.cardBlur ?? 0),
    customCss: str(s.customCss) ?? "",
    search,
  };
}

/** Writes settings in the server's shape. */
export function settingsToWire(s: Settings): Obj {
  return {
    title: s.title,
    theme: s.theme,
    accent: s.accent || null,
    background: s.background?.url
      ? {
          url: s.background.url,
          blur: s.background.blur > 0 ? `${Math.round(s.background.blur)}px` : null,
          opacity: Math.round(Math.min(1, Math.max(0, s.background.opacity)) * 100),
          saturate: wirePercent(s.background.saturate),
          brightness: wirePercent(s.background.brightness),
        }
      : null,
    cardBlur: s.cardBlur > 0 ? `${Math.round(s.cardBlur)}px` : null,
    customCss: s.customCss || null,
    search:
      s.search.provider === "custom"
        ? { provider: "custom", url: (s.search.url ?? "").replace("%s", "{q}") }
        : { provider: s.search.provider },
  };
}

/** Reads a whole document (`/api/dashboard`, `/api/config`, history versions). */
export function docFromWire(v: unknown): ConfigDoc {
  const d = isObj(v) ? v : {};
  let n = 0;
  const next = () => ++n;
  const service = (x: unknown): Service => {
    const o = isObj(x) ? x : {};
    const check = isObj(o.check) ? o.check : null;
    const widget = isObj(o.widget) ? o.widget : null;
    const kind = str(check?.kind);
    return {
      id: fromWireId(o.id, "s", next),
      name: str(o.name) ?? "",
      icon: str(o.icon),
      iconKind: o.iconKind === "mask" ? "mask" : o.iconKind === "image" ? "image" : undefined,
      href: str(o.href),
      description: str(o.description),
      check:
        check && (kind === "http" || kind === "tcp" || kind === "ping")
          ? { kind, target: str(check.target), expectStatus: num(check.expectStatus) }
          : null,
      widget: widget
        ? {
            type: str(widget.type) ?? "",
            fields: (isObj(widget.fields) ? widget.fields : {}) as Record<string, FieldValue>,
          }
        : null,
    };
  };
  const group = (x: unknown): Group => {
    const o = isObj(x) ? x : {};
    const layout = isObj(o.layout) ? o.layout : {};
    const style = str(layout.style);
    const sub = arr(o.groups).map(group);
    return {
      id: fromWireId(o.id, "g", next),
      name: str(o.name) ?? "",
      icon: str(o.icon),
      hidden: o.hidden === true,
      layout: {
        columns: num(layout.columns) ?? (style === "row" ? 4 : 3),
        style: style === "row" || style === "column" || style === "grid" ? style : "grid",
      },
      items: arr(o.items).map(service),
      ...(sub.length > 0 ? { groups: sub } : {}),
    };
  };
  const bookmarks = arr(d.bookmarks).map((x): BookmarkGroup => {
    const o = isObj(x) ? x : {};
    return {
      id: fromWireId(o.id, "b", next),
      name: str(o.name) ?? "",
      items: arr(o.items).map((y) => {
        const b = isObj(y) ? y : {};
        return {
          id: fromWireId(b.id, "bm", next),
          name: str(b.name) ?? "",
          abbr: str(b.abbr),
          icon: str(b.icon),
          iconKind: b.iconKind === "mask" ? ("mask" as const) : undefined,
          href: str(b.href) ?? "",
        };
      }),
    };
  });
  const infoWidgets = arr(d.infoWidgets).flatMap((x): InfoWidget[] => {
    const o = isObj(x) ? x : {};
    const type = str(o.type);
    if (type !== "clock" && type !== "weather" && type !== "resources" && type !== "docker")
      return [];
    return [
      {
        id: fromWireId(o.id, "iw", next),
        type,
        options: (isObj(o.options) ? o.options : {}) as InfoWidget["options"],
      },
    ];
  });
  return {
    settings: settingsFromWire(d.settings),
    groups: arr(d.groups).map(group),
    bookmarks,
    infoWidgets,
    version: d.version === undefined || d.version === null ? "" : String(d.version),
  };
}

/** Writes a document in the server's shape for `PUT /api/config`. */
export function docToWire(doc: ConfigDoc): Obj {
  const group = (g: Group): Obj => ({
    id: toWireId(g.id),
    name: g.name,
    icon: g.icon || undefined,
    hidden: g.hidden,
    layout: { columns: g.layout.columns, style: g.layout.style },
    items: g.items.map((s) => ({
      id: toWireId(s.id),
      name: s.name,
      icon: s.icon || undefined,
      href: s.href || undefined,
      description: s.description || undefined,
      check: s.check,
      widget: s.widget,
    })),
    groups: (g.groups ?? []).map(group),
  });
  return {
    settings: settingsToWire(doc.settings),
    groups: doc.groups.map(group),
    bookmarks: doc.bookmarks.map((b) => ({
      id: toWireId(b.id),
      name: b.name,
      items: b.items.map((x) => ({
        id: toWireId(x.id),
        name: x.name,
        abbr: x.abbr || undefined,
        icon: x.icon || undefined,
        href: x.href,
      })),
    })),
    infoWidgets: doc.infoWidgets.map((w) => ({
      id: toWireId(w.id),
      type: w.type,
      options: w.options,
    })),
    version: Number(doc.version) || 0,
  };
}

/** Reads `GET /api/history`. Timestamps may be Unix milliseconds or ISO strings. */
export function historyFromWire(v: unknown): HistoryEntry[] {
  return arr(v).map((x) => {
    const o = isObj(x) ? x : {};
    const t = o.createdAt;
    return {
      version: String(o.version ?? ""),
      createdAt:
        typeof t === "number" ? new Date(t).toISOString() : (str(t) ?? new Date(0).toISOString()),
      author: str(o.author) ?? "unknown",
      summary: str(o.summary) ?? "Saved",
    };
  });
}

/** Reads `GET /api/status`; keys are service ids. */
export function statusFromWire(v: unknown): StatusMap {
  const out: StatusMap = {};
  if (!isObj(v)) return out;
  for (const [k, raw] of Object.entries(v)) {
    if (!isObj(raw)) continue;
    out[k] = {
      up: raw.up === true,
      code: num(raw.code),
      latencyMs: num(raw.latencyMs),
      checkedAt: num(raw.checkedAt) ?? 0,
      history: arr(raw.history).flatMap((h) =>
        isObj(h) ? [{ t: num(h.t) ?? 0, up: h.up === true, ms: num(h.ms) }] : [],
      ),
    };
  }
  return out;
}

/** Reads one `status` server-sent event. */
export function statusEventFromWire(v: unknown): StatusEvent | null {
  if (!isObj(v) || v.serviceId === undefined) return null;
  return {
    serviceId: String(v.serviceId),
    up: v.up === true,
    code: num(v.code),
    latencyMs: num(v.latencyMs),
    checkedAt: num(v.checkedAt) ?? Date.now(),
  };
}
