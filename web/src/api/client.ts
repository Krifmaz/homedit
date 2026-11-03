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
  ImportResult,
  ImportWarning,
  LoginResult,
  Me,
  IconKind,
  Role,
  ScanJob,
  ScanRequest,
  ScanSchedule,
  StatusEvent,
  StatusMap,
  TestResult,
  UserAccount,
  WidgetConfig,
  WidgetData,
  WidgetSchema,
} from "./types";
import {
  docFromWire,
  docToWire,
  historyFromWire,
  statusEventFromWire,
  statusFromWire,
  toWireId,
} from "./wire";

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
  /** Machine-readable error code from `{error, message}` bodies, if any. */
  readonly code?: string;
  /** Seconds to wait before retrying, from `Retry-After` (rate limits). */
  readonly retryAfter?: number;
  constructor(status: number, message: string, code?: string, retryAfter?: number) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.code = code;
    this.retryAfter = retryAfter;
  }
}

/** Builds an {@link ApiError} from a failed response. */
async function toApiError(res: Response): Promise<ApiError> {
  const text = await res.text().catch(() => "");
  let code: string | undefined;
  let message = text || res.statusText;
  try {
    const body = JSON.parse(text) as { error?: string; message?: string };
    code = body.error;
    message = body.message || body.error || message;
  } catch {
    /* not JSON */
  }
  const ra = Number(res.headers.get("Retry-After"));
  return new ApiError(res.status, message, code, Number.isFinite(ra) && ra > 0 ? ra : undefined);
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
   * Subscribe to live updates: status changes, and config changes (a save or
   * restore elsewhere) with the new version.
   * @returns a function that closes the subscription.
   */
  subscribeStatus(
    onEvent: (e: StatusEvent) => void,
    onConfig?: (version: string) => void,
    onScanChanges?: (messages: string[]) => void,
  ): () => void;
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
  /** Sign in with a username and password. */
  login(username: string, password: string): Promise<LoginResult>;
  /** Create the first admin (only while no users exist) and sign in. */
  setup(username: string, password: string): Promise<LoginResult>;
  /** Sign out; returns a URL to navigate to (proxy logout), if any. */
  logout(): Promise<string | null>;
  /** Change the signed-in local user's password. */
  changePassword(currentPassword: string, newPassword: string): Promise<void>;
  /** List user accounts (admin). */
  users(): Promise<UserAccount[]>;
  /** Create a local user (admin). */
  createUser(u: { username: string; password: string; role: Role }): Promise<UserAccount>;
  /** Change a user's role, password or disabled flag (admin). */
  updateUser(
    id: string,
    patch: { role?: Role; password?: string; disabled?: boolean },
  ): Promise<UserAccount>;
  /** Delete a user (admin); fails with 409 for the last admin. */
  deleteUser(id: string): Promise<void>;
  /** Start a network scan (admin); fails with 409 while one runs. */
  startScan(req: ScanRequest): Promise<number>;
  /** The running or last scan with its results (admin). */
  scanJob(): Promise<ScanJob>;
  /**
   * Import gethomepage config files (`services.yaml`, `bookmarks.yaml`, ...)
   * or a `homedit.yaml` backup. The file name decides how each is read.
   */
  importFiles(files: File[]): Promise<ImportResult>;
  /** The rescan schedule (admin). */
  scanSchedule(): Promise<ScanSchedule>;
  /** Save the rescan schedule (admin); returns it with the next run time. */
  saveScanSchedule(s: ScanSchedule): Promise<ScanSchedule>;
  /** URL that downloads the config as YAML, optionally with secrets. */
  exportUrl(withSecrets: boolean): string;
  /** Search built-in and uploaded icons. */
  searchIcons(q: string): Promise<IconResult[]>;
  /** Upload a custom icon. */
  uploadIcon(file: File): Promise<IconResult>;
}

/** Maps a non-URL icon reference (e.g. `di:plex`) to a URL. */
export type IconResolver = (ref: string) => string;

/** Icon CDNs, the same ones the server uses (`homedit_core::icons`). */
const DASHBOARD_ICONS_CDN = "https://cdn.jsdelivr.net/gh/homarr-labs/dashboard-icons";
const SELFHST_ICONS_CDN = "https://cdn.jsdelivr.net/gh/selfhst/icons";
const MDI_CDN = "https://cdn.jsdelivr.net/npm/@mdi/svg@7.4.47/svg";
const SIMPLE_ICONS_CDN = "https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons";

/** Splits `name.ext` into stem and a supported extension (png by default). */
function splitExt(name: string): [string, string] {
  const m = /^(.*)\.(png|svg|webp)$/.exec(name);
  return m ? [m[1]!, m[2]!] : [name, "png"];
}

/**
 * Resolves a gethomepage-style icon name the way the server does, for the
 * editor, which works on names rather than the dashboard's resolved URLs:
 * `sonarr.png` (dashboard-icons), `sh-sonarr` (selfh.st), `mdi-x` and `si-x`
 * (monochrome glyphs), `custom/x.png` (uploads). Unknown shapes yield `""`.
 */
export function resolveIconName(name: string): { url: string; kind: IconKind } {
  const n = name.trim();
  if (n.startsWith("custom/")) return { url: `/icons/custom/${n.slice(7)}`, kind: "image" };
  if (!/^[A-Za-z0-9_-][A-Za-z0-9_.-]*$/.test(n)) return { url: "", kind: "image" };
  if (n.startsWith("mdi-"))
    return { url: `${MDI_CDN}/${n.slice(4).replace(/\.svg$/, "")}.svg`, kind: "mask" };
  if (n.startsWith("si-")) {
    return { url: `${SIMPLE_ICONS_CDN}/${n.slice(3).replace(/\.svg$/, "")}.svg`, kind: "mask" };
  }
  if (n.startsWith("sh-")) {
    const [stem, ext] = splitExt(n.slice(3));
    return { url: `${SELFHST_ICONS_CDN}/${ext}/${stem}.${ext}`, kind: "image" };
  }
  const [stem, ext] = splitExt(n);
  return { url: `${DASHBOARD_ICONS_CDN}/${ext}/${stem}.${ext}`, kind: "image" };
}

let resolveIcon: IconResolver = (ref) => resolveIconName(ref).url;

/** Replaces how icon references are resolved (used by the mock mode). */
export function setIconResolver(fn: IconResolver): void {
  resolveIcon = fn;
}

/** Turns an icon reference from the config into a URL the browser can load. */
export function iconUrl(ref: string | undefined): string | undefined {
  if (!ref) return undefined;
  if (
    /^(https?:)?\/\//.test(ref) ||
    ref.startsWith("/") ||
    ref.startsWith("data:") ||
    ref.startsWith("blob:")
  ) {
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
    if (!res.ok) throw await toApiError(res);
    const text = await res.text();
    return (text ? JSON.parse(text) : undefined) as T;
  }

  async me(): Promise<Me> {
    const me = await this.request<Me>("/me");
    this.csrf = me.csrfToken;
    return me;
  }
  async dashboard() {
    return docFromWire(await this.request<unknown>("/dashboard"));
  }
  async status() {
    return statusFromWire(await this.request<unknown>("/status"));
  }
  subscribeStatus(
    onEvent: (e: StatusEvent) => void,
    onConfig?: (version: string) => void,
    onScanChanges?: (messages: string[]) => void,
  ) {
    if (typeof EventSource === "undefined") return () => {};
    const es = new EventSource(`${this.base}/events`);
    es.addEventListener("status", (msg) => {
      try {
        const e = statusEventFromWire(JSON.parse((msg as MessageEvent<string>).data));
        if (e) onEvent(e);
      } catch {
        /* ignore malformed events */
      }
    });
    es.addEventListener("config", (msg) => {
      try {
        const { version } = JSON.parse((msg as MessageEvent<string>).data) as { version: unknown };
        onConfig?.(String(version));
      } catch {
        /* ignore malformed events */
      }
    });
    es.addEventListener("scan", (msg) => {
      try {
        const { data } = JSON.parse((msg as MessageEvent<string>).data) as {
          data?: { event?: string; messages?: unknown };
        };
        if (data?.event === "changes" && Array.isArray(data.messages)) {
          onScanChanges?.(data.messages.filter((m): m is string => typeof m === "string"));
        }
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
      body: JSON.stringify({ ...widget, serviceId: toWireId(serviceId) }),
    });
  }
  async config() {
    return docFromWire(await this.request<unknown>("/config"));
  }
  async saveConfig(doc: ConfigDoc) {
    const res = await this.request<{ version: string | number }>("/config", {
      method: "PUT",
      headers: { "If-Match": doc.version },
      body: JSON.stringify(docToWire(doc)),
    });
    return String(res.version);
  }
  async history() {
    return historyFromWire(await this.request<unknown>("/history"));
  }
  async historyVersion(version: string) {
    return docFromWire(await this.request<unknown>(`/history/${encodeURIComponent(version)}`));
  }
  async restore(version: string) {
    const res = await this.request<{ version: string | number }>(
      `/history/${encodeURIComponent(version)}/restore`,
      { method: "POST" },
    );
    return String(res.version);
  }
  private async session(path: string, body: unknown): Promise<LoginResult> {
    const res = await this.request<LoginResult>(path, {
      method: "POST",
      body: JSON.stringify(body),
    });
    this.csrf = res.csrfToken;
    return res;
  }
  login(username: string, password: string) {
    return this.session("/auth/login", { username, password });
  }
  setup(username: string, password: string) {
    return this.session("/auth/setup", { username, password });
  }
  async logout() {
    const res = await this.request<{ ok: boolean; logoutUrl: string | null }>("/auth/logout", {
      method: "POST",
    });
    return res.logoutUrl ?? null;
  }
  async changePassword(currentPassword: string, newPassword: string) {
    await this.session("/auth/password", { currentPassword, newPassword });
  }
  users() {
    return this.request<UserAccount[]>("/auth/users");
  }
  createUser(u: { username: string; password: string; role: Role }) {
    return this.request<UserAccount>("/auth/users", { method: "POST", body: JSON.stringify(u) });
  }
  updateUser(id: string, patch: { role?: Role; password?: string; disabled?: boolean }) {
    return this.request<UserAccount>(`/auth/users/${encodeURIComponent(id)}`, {
      method: "PATCH",
      body: JSON.stringify(patch),
    });
  }
  async deleteUser(id: string) {
    await this.request<unknown>(`/auth/users/${encodeURIComponent(id)}`, { method: "DELETE" });
  }
  async startScan(req: ScanRequest) {
    const res = await this.request<{ id: number }>("/scan", {
      method: "POST",
      body: JSON.stringify(req),
    });
    return res.id;
  }
  scanJob() {
    return this.request<ScanJob>("/scan");
  }
  async importFiles(files: File[]) {
    const body = new FormData();
    for (const f of files) body.append("file", f, f.name);
    const res = await this.request<{ version: unknown; warnings?: ImportWarning[] }>("/import", {
      method: "POST",
      body,
    });
    return { version: String(res.version), warnings: res.warnings ?? [] };
  }
  scanSchedule() {
    return this.request<ScanSchedule>("/scan/schedule");
  }
  saveScanSchedule(sch: ScanSchedule) {
    const { enabled, intervalHours, request } = sch;
    return this.request<ScanSchedule>("/scan/schedule", {
      method: "PUT",
      body: JSON.stringify({ enabled, intervalHours, request }),
    });
  }
  exportUrl(withSecrets: boolean) {
    return `${this.base}/export?secrets=${withSecrets}`;
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

/** Data the server inlines into `index.html` for the first paint. */
export interface InlineData {
  dashboard: DashboardData;
  status: StatusMap;
}

/**
 * Reads the data the server inlined into `index.html`, if any. Accepts both
 * `{dashboard, status}` and a bare dashboard object.
 */
export function readInlineData(): InlineData | null {
  const el = document.getElementById("homedit-data");
  if (!el?.textContent) return null;
  try {
    const raw = JSON.parse(el.textContent) as Partial<InlineData> & Partial<DashboardData>;
    if (!raw || typeof raw !== "object") return null;
    if (raw.dashboard)
      return { dashboard: docFromWire(raw.dashboard), status: statusFromWire(raw.status) };
    if (raw.groups) return { dashboard: docFromWire(raw), status: {} };
    return null;
  } catch {
    return null;
  }
}
