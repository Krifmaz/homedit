/**
 * In-memory mock of the homedit API.
 *
 * Used in development with `VITE_MOCK=1` (or `?mock` in the URL) and in tests.
 * It behaves like the real server for versioning, secrets and history, so the
 * editor can be exercised end to end without a backend.
 */
import { ConflictError, type Api } from "./client";
import type {
  ConfigDoc,
  FieldValue,
  HistoryEntry,
  IconResult,
  Me,
  StatusEvent,
  StatusMap,
  WidgetConfig,
  WidgetSchema,
} from "./types";

/** Field schema list shared by the *arr apps. */
const arrFields = [
  {
    key: "url",
    label: "URL",
    kind: "url",
    required: true,
    help: "Base URL, e.g. http://sonarr:8989",
  },
  {
    key: "key",
    label: "API key",
    kind: "secret",
    required: true,
    help: "Settings → General → API Key",
  },
] as const;

/** A representative slice of the widget catalogue. */
export const mockCatalog: WidgetSchema[] = [
  { type: "sonarr", name: "Sonarr", category: "Media management", fields: [...arrFields] },
  { type: "radarr", name: "Radarr", category: "Media management", fields: [...arrFields] },
  { type: "prowlarr", name: "Prowlarr", category: "Media management", fields: [...arrFields] },
  {
    type: "plex",
    name: "Plex",
    category: "Media servers",
    fields: [
      { key: "url", label: "URL", kind: "url", required: true },
      { key: "key", label: "Plex token", kind: "secret", required: true, help: "X-Plex-Token" },
    ],
  },
  {
    type: "jellyfin",
    name: "Jellyfin",
    category: "Media servers",
    fields: [
      { key: "url", label: "URL", kind: "url", required: true },
      { key: "key", label: "API key", kind: "secret", required: true },
      {
        key: "enableNowPlaying",
        label: "Show now playing",
        kind: "bool",
        required: false,
        default: true,
      },
    ],
  },
  {
    type: "qbittorrent",
    name: "qBittorrent",
    category: "Downloads",
    fields: [
      { key: "url", label: "URL", kind: "url", required: true },
      { key: "username", label: "Username", kind: "string", required: true },
      { key: "password", label: "Password", kind: "secret", required: true },
    ],
  },
  {
    type: "proxmox",
    name: "Proxmox",
    category: "Infrastructure",
    fields: [
      { key: "url", label: "URL", kind: "url", required: true, help: "https://pve:8006" },
      {
        key: "username",
        label: "API token ID",
        kind: "string",
        required: true,
        help: "user@pam!token",
      },
      { key: "password", label: "API token secret", kind: "secret", required: true },
      {
        key: "node",
        label: "Node",
        kind: "string",
        required: false,
        help: "Leave empty for all nodes",
      },
    ],
  },
  {
    type: "customapi",
    name: "Custom API",
    category: "Generic",
    fields: [
      { key: "url", label: "URL", kind: "url", required: true },
      {
        key: "method",
        label: "Method",
        kind: "select",
        required: true,
        default: "GET",
        options: ["GET", "POST"],
      },
      {
        key: "refreshInterval",
        label: "Refresh (seconds)",
        kind: "number",
        required: false,
        default: 30,
      },
      {
        key: "mappings",
        label: "Field mapping",
        kind: "string",
        required: false,
        help: "label=json.path, one per comma",
      },
    ],
  },
];

/** Sample configuration the mock starts with. */
export function sampleConfig(): ConfigDoc {
  return {
    version: "v1",
    settings: {
      title: "Homelab",
      theme: "system",
      accent: "#3b6fd8",
      background: null,
      customCss: "",
      search: { provider: "duckduckgo" },
    },
    infoWidgets: [
      { id: "iw-clock", type: "clock", options: { format: "24h" } },
      { id: "iw-res", type: "resources", options: {} },
      { id: "iw-weather", type: "weather", options: { location: "Home" } },
      { id: "iw-docker", type: "docker", options: {} },
    ],
    groups: [
      {
        id: "g-media",
        name: "Media",
        hidden: false,
        layout: { columns: 3, style: "grid" },
        items: [
          {
            id: "s-plex",
            name: "Plex",
            icon: "plex.png",
            href: "http://plex.lan:32400",
            description: "Movies and shows",
            check: { kind: "http" },
            widget: {
              type: "plex",
              fields: { url: "http://plex.lan:32400", key: { $secret: "set" } },
            },
          },
          {
            id: "s-jellyfin",
            name: "Jellyfin",
            icon: "jellyfin.png",
            href: "http://jellyfin.lan:8096",
            description: "Open media server",
            check: { kind: "http" },
            widget: null,
          },
          {
            id: "s-sonarr",
            name: "Sonarr",
            icon: "sonarr.png",
            href: "http://sonarr.lan:8989",
            description: "TV series",
            check: { kind: "http" },
            widget: {
              type: "sonarr",
              fields: { url: "http://sonarr.lan:8989", key: { $secret: "set" } },
            },
          },
          {
            id: "s-radarr",
            name: "Radarr",
            icon: "radarr.png",
            href: "http://radarr.lan:7878",
            description: "Movies",
            check: { kind: "http" },
            widget: {
              type: "radarr",
              fields: { url: "http://radarr.lan:7878", key: { $secret: "set" } },
            },
          },
        ],
      },
      {
        id: "g-infra",
        name: "Infrastructure",
        hidden: false,
        layout: { columns: 2, style: "grid" },
        items: [
          {
            id: "s-proxmox",
            name: "Proxmox",
            icon: "proxmox.png",
            href: "https://pve.lan:8006",
            description: "Hypervisor",
            check: { kind: "tcp", target: "pve.lan:8006" },
            widget: null,
          },
          {
            id: "s-pihole",
            name: "Pi-hole",
            icon: "pi-hole.png",
            href: "http://pihole.lan/admin",
            description: "DNS sinkhole",
            check: { kind: "ping", target: "pihole.lan" },
            widget: null,
          },
        ],
      },
      {
        id: "g-admin",
        name: "Admin only",
        hidden: true,
        layout: { columns: 2, style: "grid" },
        items: [
          {
            id: "s-router",
            name: "Router",
            href: "http://192.168.1.1",
            description: "Gateway",
            check: { kind: "ping", target: "192.168.1.1" },
            widget: null,
          },
        ],
      },
    ],
    bookmarks: [
      {
        id: "b-dev",
        name: "Developer",
        items: [
          { id: "b-gh", name: "GitHub", abbr: "GH", href: "https://github.com" },
          { id: "b-docs", name: "Rust docs", abbr: "RS", href: "https://doc.rust-lang.org" },
        ],
      },
      {
        id: "b-social",
        name: "Social",
        items: [
          {
            id: "b-reddit",
            name: "r/selfhosted",
            abbr: "SH",
            href: "https://reddit.com/r/selfhosted",
          },
        ],
      },
    ],
  };
}

/**
 * Resolves gethomepage-style icon names to public CDN URLs, standing in for
 * the server's `/icons` route in mock mode.
 */
export function mockIconUrl(ref: string): string {
  if (ref.startsWith("sh-"))
    return `https://cdn.jsdelivr.net/gh/selfhst/icons/png/${ref.slice(3)}.png`;
  if (ref.startsWith("mdi-"))
    return `https://cdn.jsdelivr.net/npm/@mdi/svg@7.4.47/svg/${ref.slice(4)}.svg`;
  const name = ref.replace(/\.(png|svg|webp)$/, "");
  return `https://cdn.jsdelivr.net/gh/homarr-labs/dashboard-icons/png/${name}.png`;
}

const clone = <T>(v: T): T => structuredClone(v);

/** Options for {@link MockApi}. */
export interface MockOptions {
  /** Role the mock session has; `admin` by default. */
  role?: "viewer" | "admin";
  /** Artificial latency in milliseconds; 0 by default. */
  latency?: number;
  /** Whether to emit random status events; true by default outside tests. */
  liveEvents?: boolean;
}

/** In-memory {@link Api} implementation. */
export class MockApi implements Api {
  private doc: ConfigDoc;
  private readonly versions = new Map<string, ConfigDoc>();
  private readonly log: HistoryEntry[] = [];
  private counter = 1;
  /** Real secret values, keyed by `serviceId.fieldKey`, never returned. */
  private readonly secrets = new Map<string, string>();
  private readonly opts: Required<MockOptions>;
  private readonly statusMap: StatusMap = {};
  private readonly customIcons: IconResult[] = [];

  constructor(opts: MockOptions = {}, initial: ConfigDoc = sampleConfig()) {
    this.opts = { role: "admin", latency: 0, liveEvents: true, ...opts };
    this.doc = clone(initial);
    this.versions.set(this.doc.version, clone(this.doc));
    this.log.push({
      version: this.doc.version,
      createdAt: new Date(Date.now() - 86_400_000).toISOString(),
      author: "import",
      summary: "Imported from gethomepage",
    });
    const now = Date.now();
    for (const g of this.doc.groups) {
      for (const s of g.items) {
        if (!s.check) continue;
        const history = Array.from({ length: 30 }, (_, i) => {
          const up = !(s.id === "s-jellyfin" && i > 25);
          return {
            t: now - (30 - i) * 60_000,
            up,
            ms: up ? 20 + Math.round(Math.random() * 80) : undefined,
          };
        });
        const last = history[history.length - 1]!;
        this.statusMap[s.id] = {
          up: last.up,
          code: s.check.kind === "http" ? (last.up ? 200 : 502) : undefined,
          latencyMs: last.ms,
          checkedAt: last.t,
          history,
        };
      }
    }
  }

  private async wait() {
    if (this.opts.latency > 0) await new Promise((r) => setTimeout(r, this.opts.latency));
  }

  private visible(doc: ConfigDoc): ConfigDoc {
    const out = clone(doc);
    if (this.opts.role !== "admin") out.groups = out.groups.filter((g) => !g.hidden);
    return out;
  }

  async me(): Promise<Me> {
    await this.wait();
    return {
      user: this.opts.role === "admin" ? { name: "admin" } : null,
      role: this.opts.role,
      authEnabled: true,
      csrfToken: "mock",
    };
  }
  async dashboard() {
    await this.wait();
    return this.visible(this.doc);
  }
  async status() {
    await this.wait();
    return clone(this.statusMap);
  }
  subscribeStatus(onEvent: (e: StatusEvent) => void) {
    if (!this.opts.liveEvents) return () => {};
    const timer = setInterval(() => {
      const ids = Object.keys(this.statusMap);
      const id = ids[Math.floor(Math.random() * ids.length)];
      if (!id) return;
      const ms = 15 + Math.round(Math.random() * 90);
      const up = id === "s-jellyfin" ? Math.random() > 0.5 : true;
      const e: StatusEvent = {
        serviceId: id,
        up,
        latencyMs: up ? ms : undefined,
        checkedAt: Date.now(),
        code: up ? 200 : 502,
      };
      const st = this.statusMap[id]!;
      st.history = [...st.history.slice(-29), { t: e.checkedAt, up, ms: e.latencyMs }];
      Object.assign(st, { up, latencyMs: e.latencyMs, checkedAt: e.checkedAt, code: e.code });
      onEvent(e);
    }, 3000);
    return () => clearInterval(timer);
  }
  async widgetData(serviceId: string) {
    await this.wait();
    const svc = this.doc.groups.flatMap((g) => g.items).find((s) => s.id === serviceId);
    switch (svc?.widget?.type) {
      case "sonarr":
        return {
          fields: [
            { label: "Wanted", value: 4 },
            { label: "Queued", value: 2 },
            { label: "Series", value: 118 },
          ],
        };
      case "radarr":
        return {
          fields: [
            { label: "Wanted", value: 7 },
            { label: "Missing", value: 3 },
            { label: "Movies", value: 642 },
          ],
        };
      case "plex":
        return {
          fields: [
            { label: "Streams", value: 1 },
            { label: "Movies", value: 642 },
            { label: "Shows", value: 118 },
          ],
        };
      default:
        return { fields: [], error: svc?.widget ? "No mock data for this widget" : undefined };
    }
  }
  async infoData(widgetId: string) {
    await this.wait();
    const w = this.doc.infoWidgets.find((x) => x.id === widgetId);
    switch (w?.type) {
      case "weather":
        return { tempC: 14, condition: "Partly cloudy" };
      case "resources":
        return {
          cpu: 12 + Math.random() * 20,
          memUsed: 6.2e9,
          memTotal: 16e9,
          diskUsed: 1.1e12,
          diskTotal: 4e12,
        };
      case "docker":
        return { running: 23, stopped: 2 };
      default:
        return {};
    }
  }
  async catalog() {
    await this.wait();
    return clone(mockCatalog);
  }
  async testWidget(widget: WidgetConfig) {
    await this.wait();
    const url = widget.fields.url;
    if (typeof url !== "string" || !/^https?:\/\//.test(url)) {
      return { ok: false, message: "URL must start with http:// or https://" };
    }
    return { ok: true, message: `Connected to ${widget.type} at ${url}` };
  }
  async config() {
    await this.wait();
    if (this.opts.role !== "admin") throw new Error("forbidden");
    return clone(this.doc);
  }
  async saveConfig(doc: ConfigDoc) {
    await this.wait();
    if (doc.version !== this.doc.version) throw new ConflictError();
    const next = clone(doc);
    for (const g of next.groups) {
      for (const s of g.items) {
        if (!s.widget) continue;
        for (const [k, v] of Object.entries(s.widget.fields)) {
          if (typeof v === "string" && this.isSecretField(s.widget.type, k)) {
            this.secrets.set(`${s.id}.${k}`, v);
            s.widget.fields[k] = { $secret: "set" } satisfies FieldValue;
          }
        }
      }
    }
    next.version = `v${++this.counter}`;
    this.doc = next;
    this.versions.set(next.version, clone(next));
    this.log.unshift({
      version: next.version,
      createdAt: new Date().toISOString(),
      author: "admin",
      summary: "Edited in dashboard",
    });
    return next.version;
  }
  private isSecretField(type: string, key: string) {
    return (
      mockCatalog
        .find((w) => w.type === type)
        ?.fields.some((f) => f.key === key && f.kind === "secret") ?? false
    );
  }
  async history() {
    await this.wait();
    return clone(this.log);
  }
  async historyVersion(version: string) {
    await this.wait();
    const v = this.versions.get(version);
    if (!v) throw new Error("not found");
    return clone(v);
  }
  async restore(version: string) {
    const v = await this.historyVersion(version);
    v.version = this.doc.version;
    const nv = await this.saveConfig(v);
    this.log[0]!.summary = `Restored ${version}`;
    return nv;
  }
  async searchIcons(q: string) {
    await this.wait();
    const names = [
      "plex",
      "jellyfin",
      "sonarr",
      "radarr",
      "prowlarr",
      "proxmox",
      "pi-hole",
      "adguard-home",
      "portainer",
      "immich",
      "qbittorrent",
      "sabnzbd",
      "uptime-kuma",
      "home-assistant",
      "grafana",
      "nextcloud",
    ];
    const ql = q.toLowerCase();
    const builtIn: IconResult[] = names
      .filter((n) => n.includes(ql))
      .flatMap((n) => [
        { name: `${n}.png`, source: "dashboard-icons" as const, url: mockIconUrl(`${n}.png`) },
        { name: `sh-${n}`, source: "selfhst" as const, url: mockIconUrl(`sh-${n}`) },
      ]);
    return [...this.customIcons.filter((i) => i.name.includes(ql)), ...builtIn].slice(0, 40);
  }
  async uploadIcon(file: File) {
    await this.wait();
    const url = URL.createObjectURL(file);
    const icon: IconResult = { name: `custom:${file.name}`, source: "custom", url };
    this.customIcons.push(icon);
    return icon;
  }
}
