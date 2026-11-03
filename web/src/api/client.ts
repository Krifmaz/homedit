/**
 * The API surface the UI uses, with an HTTP implementation.
 *
 * Components never call `fetch` directly; they go through an {@link Api}
 * instance so tests and the mock mode can substitute their own.
 */
import type {
  ConfigDoc,
  DashboardData,
  HistoryEntry,
  IconResult,
  Me,
  StatusEvent,
  StatusMap,
  TestResult,
  WidgetConfig,
  WidgetData,
  WidgetSchema,
} from "./types";

/** Thrown when a save is rejected because the config changed on the server. */
export class ConflictError extends Error {
  constructor() {
    super("The configuration was changed elsewhere. Reload to get the latest version.");
    this.name = "ConflictError";
  }
}

/** Thrown for any other non-2xx response. */
export class ApiError extends Error {
  /** HTTP status code. */
  readonly status: number;
  constructor(status: number, message: string) {
    super(message);
    this.name = "ApiError";
    this.status = status;
  }
}

/** Everything the UI needs from the backend. */
export interface Api {
  /** Current session and role. */
  me(): Promise<Me>;
  /** Dashboard data as the current viewer may see it. */
  dashboard(): Promise<DashboardData>;
  /** Latest status of every checked service. */
  status(): Promise<StatusMap>;
  /**
   * Subscribe to live status updates.
   * @returns a function that closes the subscription.
   */
  subscribeStatus(onEvent: (e: StatusEvent) => void): () => void;
  /** Live stats for the widget of a service. */
  widgetData(serviceId: string): Promise<WidgetData>;
  /** Payload of an info widget (weather, resources, docker). */
  infoData(widgetId: string): Promise<unknown>;
  /** The widget catalogue with field schemas. */
  catalog(): Promise<WidgetSchema[]>;
  /** Try a widget configuration against the upstream service. */
  testWidget(widget: WidgetConfig, serviceId?: string): Promise<TestResult>;
  /** Full editable config (admin only). */
  config(): Promise<ConfigDoc>;
  /**
   * Save the config.
   * @throws {ConflictError} when `doc.version` is stale.
   * @returns the new version.
   */
  saveConfig(doc: ConfigDoc): Promise<string>;
  /** Saved versions, newest first. */
  history(): Promise<HistoryEntry[]>;
  /** A past version of the config. */
  historyVersion(version: string): Promise<ConfigDoc>;
  /** Restore a past version; returns the new current version. */
  restore(version: string): Promise<string>;
  /** Search built-in and uploaded icons. */
  searchIcons(q: string): Promise<IconResult[]>;
  /** Upload a custom icon. */
  uploadIcon(file: File): Promise<IconResult>;
}

/** Maps a non-URL icon reference (e.g. `di:plex`) to a URL. */
export type IconResolver = (ref: string) => string;

let resolveIcon: IconResolver = (ref) => `/icons/${encodeURIComponent(ref)}`;

/** Replaces how icon references are resolved (used by the mock mode). */
export function setIconResolver(fn: IconResolver): void {
  resolveIcon = fn;
}

/** Turns an icon reference from the config into a URL the browser can load. */
export function iconUrl(ref: string | undefined): string | undefined {
  if (!ref) return undefined;
  if (/^(https?:)?\/\//.test(ref) || ref.startsWith("/") || ref.startsWith("data:") || ref.startsWith("blob:")) {
    return ref;
  }
  return resolveIcon(ref);
}

/** {@link Api} implementation over `fetch` against the homedit backend. */
export class HttpApi implements Api {
  private csrf = "";
  private readonly base: string;

  /** @param base URL prefix of the API, `/api` by default. */
  constructor(base = "/api") {
    this.base = base;
  }

  private async request<T>(path: string, init: RequestInit = {}): Promise<T> {
    const headers = new Headers(init.headers);
    if (init.method && init.method !== "GET") headers.set("X-CSRF-Token", this.csrf);
    if (init.body && typeof init.body === "string") headers.set("Content-Type", "application/json");
    const res = await fetch(this.base + path, { credentials: "same-origin", ...init, headers });
    if (res.status === 409) throw new ConflictError();
    if (!res.ok) {
      const text = await res.text().catch(() => "");
      throw new ApiError(res.status, text || res.statusText);
    }
    if (res.status === 204) return undefined as T;
    return (await res.json()) as T;
  }

  async me(): Promise<Me> {
    const me = await this.request<Me>("/me");
    this.csrf = me.csrfToken;
    return me;
  }
  dashboard() {
    return this.request<DashboardData>("/dashboard");
  }
  status() {
    return this.request<StatusMap>("/status");
  }
  subscribeStatus(onEvent: (e: StatusEvent) => void) {
    if (typeof EventSource === "undefined") return () => {};
    const es = new EventSource(`${this.base}/events`);
    es.addEventListener("status", (msg) => {
      try {
        onEvent(JSON.parse((msg as MessageEvent<string>).data) as StatusEvent);
      } catch {
        /* ignore malformed events */
      }
    });
    return () => es.close();
  }
  widgetData(serviceId: string) {
    return this.request<WidgetData>(`/widgets/${encodeURIComponent(serviceId)}/data`);
  }
  infoData(widgetId: string) {
    return this.request<unknown>(`/info/${encodeURIComponent(widgetId)}`);
  }
  catalog() {
    return this.request<WidgetSchema[]>("/widgets/catalog");
  }
  testWidget(widget: WidgetConfig, serviceId?: string) {
    return this.request<TestResult>("/widgets/test", {
      method: "POST",
      body: JSON.stringify({ ...widget, serviceId }),
    });
  }
  config() {
    return this.request<ConfigDoc>("/config");
  }
  async saveConfig(doc: ConfigDoc) {
    const res = await this.request<{ version: string }>("/config", {
      method: "PUT",
      headers: { "If-Match": doc.version },
      body: JSON.stringify(doc),
    });
    return res.version;
  }
  history() {
    return this.request<HistoryEntry[]>("/history");
  }
  historyVersion(version: string) {
    return this.request<ConfigDoc>(`/history/${encodeURIComponent(version)}`);
  }
  async restore(version: string) {
    const res = await this.request<{ version: string }>(
      `/history/${encodeURIComponent(version)}/restore`,
      { method: "POST" },
    );
    return res.version;
  }
  searchIcons(q: string) {
    return this.request<IconResult[]>(`/icons?q=${encodeURIComponent(q)}`);
  }
  uploadIcon(file: File) {
    const body = new FormData();
    body.append("file", file);
    return this.request<IconResult>("/icons", { method: "POST", body });
  }
}

/** Reads the dashboard data the server inlined into `index.html`, if any. */
export function readInlineData(): DashboardData | null {
  const el = document.getElementById("homedit-data");
  if (!el?.textContent) return null;
  try {
    return JSON.parse(el.textContent) as DashboardData;
  } catch {
    return null;
  }
}
