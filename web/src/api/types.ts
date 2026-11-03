/**
 * Types shared with the homedit REST API.
 *
 * These mirror the contract in `/api` exactly (JSON, camelCase). Keep them in
 * sync with the Rust backend's serde models.
 */

/** A role a signed-in (or anonymous) viewer has. Only admins may edit. */
export type Role = "viewer" | "admin";

/** Sign-in methods the server offers. */
export interface AuthMethods {
  /** Username and password login. */
  local: boolean;
  /** Label of the OIDC button, or `null` when OIDC is off. */
  oidc: string | null;
  /** Trusted reverse-proxy headers. */
  proxy: boolean;
  /** Where to send the browser to log out of the proxy, if any. */
  proxyLogoutUrl?: string | null;
}

/** Response of `GET /api/me`. */
export interface Me {
  /** The signed-in user, or `null` when anonymous or auth is disabled. */
  user: { name: string; id?: string; source?: string } | null;
  /**
   * Effective role. `null` with `authEnabled` means the visitor must sign in
   * before seeing anything.
   */
  role: Role | null;
  /** Whether any login method is configured on the server. */
  authEnabled: boolean;
  /** Token to send back in the `X-CSRF-Token` header on mutations. */
  csrfToken: string;
  /** No users exist yet: show the first-run admin setup. */
  setupRequired?: boolean;
  /** Available sign-in methods. */
  methods?: AuthMethods;
}

/** A user account as returned by the auth API. */
export interface UserAccount {
  id: string;
  username: string;
  role: Role;
  /** `local`, `oidc` or `proxy`. */
  source: string;
  disabled: boolean;
  /** ISO 8601 timestamp. */
  createdAt: string;
  /** ISO 8601 timestamp, or `null` when never signed in. */
  lastLoginAt: string | null;
}

/** Response of login, setup and password change. */
export interface LoginResult {
  user: UserAccount;
  csrfToken: string;
}

/** Theme preference. `system` follows `prefers-color-scheme`. */
export type ThemeMode = "light" | "dark" | "system";

/** Optional full-page background image. */
export interface Background {
  /** Image URL (absolute or an icon/asset path served by the backend). */
  url: string;
  /** Blur radius in pixels applied to the image. */
  blur: number;
  /** Opacity of the image from 0 to 1. */
  opacity: number;
}

/** Web search configuration for the search bar. */
export interface SearchSettings {
  /** Which engine to send queries to. */
  provider: "google" | "duckduckgo" | "bing" | "custom";
  /** Query URL for `custom`; `%s` is replaced by the URL-encoded query. */
  url?: string;
}

/** Dashboard-wide settings. */
export interface Settings {
  /** Page title shown in the header and the browser tab. */
  title: string;
  /** Light, dark or follow the system. */
  theme: ThemeMode;
  /** Accent colour as a CSS colour string, e.g. `#3b6fd8`. */
  accent: string;
  /** Background image, if any. */
  background: Background | null;
  /** Extra CSS appended after the built-in styles. */
  customCss: string;
  /** Search bar configuration. */
  search: SearchSettings;
}

/** How the cards of a group are laid out. */
export interface GroupLayout {
  /** Number of columns on wide screens (1 to 8). */
  columns: number;
  /** `grid` fills columns, `row` places cards on one scrolling row, `column` stacks them. */
  style: "grid" | "row" | "column";
}

/** Kinds of status checks a service can have. */
export type CheckKind = "http" | "tcp" | "ping";

/** Status check configuration of a service. */
export interface Check {
  /** HTTP request, TCP connect, or ICMP-less ping (TCP fallback on the server). */
  kind: CheckKind;
  /** Target override; defaults to the service `href` (http) or its host (tcp/ping). */
  target?: string;
  /** Expected HTTP status for `http` checks; any 2xx/3xx when omitted. */
  expectStatus?: number;
}

/**
 * Placeholder the server sends instead of a stored secret value.
 *
 * On save, send the same object back to keep the secret, a string to replace
 * it, or `null` to clear it.
 */
export interface SecretPlaceholder {
  $secret: "set";
}

/** A value of a widget field. */
export type FieldValue = string | number | boolean | null | SecretPlaceholder;

/** A service widget instance attached to a card. */
export interface WidgetConfig {
  /** Widget type from the catalogue, e.g. `sonarr`. */
  type: string;
  /** Field values keyed by the catalogue field `key`. */
  fields: Record<string, FieldValue>;
}

/** How an icon is drawn: as an image, or as a CSS mask tinted with the text colour. */
export type IconKind = "image" | "mask";

/** A service card. */
export interface Service {
  /** Stable identifier (server- or client-generated). */
  id: string;
  /** Display name. */
  name: string;
  /**
   * Icon. In `/api/dashboard` this is a resolved URL; in `/api/config` it is
   * a gethomepage-style name (`sonarr.png`, `sh-sonarr`, `mdi-server`,
   * `custom/x.png`) or a URL.
   */
  icon?: string;
  /** `mask` icons are monochrome glyphs tinted with the text colour. */
  iconKind?: IconKind;
  /** Link opened when the card is clicked. */
  href?: string;
  /** One-line description under the name. */
  description?: string;
  /** Status check, or `null` for none. */
  check: Check | null;
  /** Service widget with live stats, or `null` for none. */
  widget: WidgetConfig | null;
}

/** A group of service cards. */
export interface Group {
  /** Stable identifier. */
  id: string;
  /** Heading shown above the cards. */
  name: string;
  /** Optional icon reference for the heading. */
  icon?: string;
  /** Hidden from viewers (admins still see it, dimmed). */
  hidden: boolean;
  /** Card layout for this group. */
  layout: GroupLayout;
  /** Cards in display order. */
  items: Service[];
  /** Nested sub-groups, rendered inside this group. */
  groups?: Group[];
}

/** A single bookmark link. */
export interface Bookmark {
  /** Stable identifier. */
  id: string;
  /** Display name. */
  name: string;
  /** Short abbreviation shown when there is no icon (up to 2 characters). */
  abbr?: string;
  /** Icon name or URL (resolved URL in `/api/dashboard`). */
  icon?: string;
  /** How the icon is drawn. */
  iconKind?: IconKind;
  /** Target URL. */
  href: string;
}

/** A named list of bookmarks. */
export interface BookmarkGroup {
  /** Stable identifier. */
  id: string;
  /** Heading. */
  name: string;
  /** Bookmarks in display order. */
  items: Bookmark[];
}

/** Types of info widgets shown in the header strip. */
export type InfoWidgetType = "clock" | "weather" | "resources" | "docker";

/** An info widget in the header strip. */
export interface InfoWidget {
  /** Stable identifier. */
  id: string;
  /** Widget type. */
  type: InfoWidgetType;
  /** Type-specific options (e.g. `{ format: "24h" }` for the clock). */
  options: Record<string, string | number | boolean>;
}

/** The full editable configuration document (`GET/PUT /api/config`). */
export interface ConfigDoc {
  settings: Settings;
  groups: Group[];
  bookmarks: BookmarkGroup[];
  infoWidgets: InfoWidget[];
  /** Opaque version string; sent as `If-Match` on save. */
  version: string;
}

/** Response of `GET /api/dashboard`; same shape as the config document. */
export type DashboardData = ConfigDoc;

/** One status sample in a service's history. */
export interface StatusSample {
  /** Unix time in milliseconds. */
  t: number;
  /** Whether the check succeeded. */
  up: boolean;
  /** Response time in milliseconds, when measured. */
  ms?: number;
}

/** Current status of a service (`GET /api/status`). */
export interface ServiceStatus {
  up: boolean;
  /** HTTP status code for `http` checks. */
  code?: number;
  /** Last response time in milliseconds. */
  latencyMs?: number;
  /** Unix time in milliseconds of the last check. */
  checkedAt: number;
  /** Recent samples, oldest first. */
  history: StatusSample[];
}

/** Map of service id to status. */
export type StatusMap = Record<string, ServiceStatus>;

/** A status update pushed over `GET /api/events`. */
export interface StatusEvent {
  serviceId: string;
  up: boolean;
  code?: number;
  latencyMs?: number;
  checkedAt: number;
}

/** Input kinds a widget field can have. */
export type FieldKind = "string" | "url" | "secret" | "number" | "bool" | "select";

/** Schema of one widget field, used to generate the edit form. */
export interface FieldSchema {
  /** Key in `WidgetConfig.fields`. */
  key: string;
  /** Form label. */
  label: string;
  /** Input kind. */
  kind: FieldKind;
  /** Whether a value must be provided. */
  required: boolean;
  /** Help text shown under the input. */
  help?: string;
  /** Default value pre-filled for new widgets. */
  default?: string | number | boolean;
  /** Choices for `select` fields. */
  options?: string[];
}

/** A widget type in the catalogue (`GET /api/widgets/catalog`). */
export interface WidgetSchema {
  /** Type key, e.g. `sonarr`. */
  type: string;
  /** Human name, e.g. `Sonarr`. */
  name: string;
  /** Grouping in the picker, e.g. `Media`. */
  category: string;
  /** Fields of the widget form. */
  fields: FieldSchema[];
}

/** A single stat shown by a service widget. */
export interface WidgetStat {
  label: string;
  value: string | number;
  unit?: string;
}

/** Response of `GET /api/widgets/{serviceId}/data`. */
export interface WidgetData {
  fields: WidgetStat[];
  /** Error message when the upstream call failed. */
  error?: string;
}

/** Response of `POST /api/widgets/test`. */
export interface TestResult {
  ok: boolean;
  message: string;
}

/** An entry in the version history (`GET /api/history`). */
export interface HistoryEntry {
  version: string;
  /** ISO 8601 timestamp. */
  createdAt: string;
  /** Name of the user who saved it. */
  author: string;
  /** Short description of the change. */
  summary: string;
}

/** An icon search result (`GET /api/icons`). */
export interface IconResult {
  /** Icon reference to store in config, e.g. `di:plex`. */
  name: string;
  source: "dashboard-icons" | "selfhst" | "custom";
  /** URL the browser can load. */
  url: string;
}

/** Info widget payloads (`GET /api/info/{widgetId}`). */
export interface WeatherData {
  tempC: number;
  condition: string;
  icon?: string;
}

/** Host resource usage. Sizes are in bytes, `cpu` in percent. */
export interface ResourcesData {
  cpu: number;
  memUsed: number;
  memTotal: number;
  diskUsed: number;
  diskTotal: number;
}

/** Docker container counts. */
export interface DockerData {
  running: number;
  stopped: number;
}
