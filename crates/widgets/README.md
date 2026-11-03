# homedit-widgets

Service widgets for homedit: live stats from self-hosted apps, fetched by the
server so API keys never reach the browser.

Each widget is a small module behind the `Widget` trait:

- `schema()` describes its settings (types, required fields, help text). The
  editor generates forms from it and the server encrypts and masks every
  field of type `secret`.
- `fetch()` calls the app's API and returns `WidgetData` (stats plus detail
  rows) that the frontend renders generically.
- `test_connection()` backs the editor's "test connection" button.

`WidgetRegistry::builtin()` lists every widget; `fetch_service(ctx, service_id,
&widget_json)` takes a gethomepage-shaped block (`{"type": "sonarr", "url":
.., "key": ..}`), validates it and caches the result per service, sharing one
upstream request between concurrent callers.

## Safety

All requests go through `WidgetContext`:

- 10 s total and 5 s connect timeouts, 8 MiB body cap.
- Redirects are never followed.
- Only the scheme, host and port of the widget's configured `url` may be
  requested, plus hosts the admin adds to `ContextOptions::extra_allowed_hosts`
  (server-side request forgery guard).
- TLS verification can be relaxed per widget (`insecure_tls`) and only for
  that widget's own host.
- Error messages never contain URLs with query strings or credentials.

## Widgets

| `type` | App | Settings |
| --- | --- | --- |
| `plex` | Plex | `key` (X-Plex-Token) |
| `jellyfin`, `emby` | Jellyfin, Emby | `key` |
| `audiobookshelf` | Audiobookshelf | `key` (API token) |
| `immich` | Immich | `key` (admin), `version` |
| `sonarr`, `radarr`, `prowlarr` | *arr apps | `key` |
| `jellystat` | Jellystat | `key`, `days` (default 30) |
| `lidarr` | Lidarr | `key` |
| `qbittorrent` | qBittorrent | `username`, `password` (optional) |
| `slskd` | slskd (Soulseek) | `key` |
| `sabnzbd` | SABnzbd | `key` |
| `proxmox` | Proxmox VE | `username` (token id, `user@realm!id`), `password` (token secret), `node`, `insecure_tls` |
| `portainer` | Portainer | `key`, `env` |
| `docker` | Docker Engine API over HTTP (e.g. a read-only socket proxy) | `container` (optional) |
| `pihole` | Pi-hole v6 and v5 | `key`, `version` |
| `adguard` | AdGuard Home | `username`, `password` |
| `homeassistant` | Home Assistant | `key` (long-lived token), `custom` (optional `state`/`template` entries) |
| `uptimekuma` | Uptime Kuma status page | `slug` |
| `overseerr`, `jellyseerr`, `seerr` | Seerr family | `key` |
| `customapi` | Any JSON API | `method`, `headers`, `body`, `key`, `key_header`, `mappings` |

`customapi` mappings are `[{"field": "a.b.0.c", "label": "Name", "format":
"number|text|bytes|rate|percent|duration|millis"}]`; gethomepage's nested
`{"a": {"b": "c"}}` field form is accepted too.

## Tests

Parsers are tested against recorded-style API responses in
`tests/fixtures/<type>/`, and every widget has a wiremock end-to-end test
that checks its authentication. Run `cargo test -p homedit-widgets`.

## Adding a widget

1. Add `src/widgets/<type>.rs` with serde structs, pure `parse_*` functions
   and a `Widget` impl that uses only `WidgetContext` for HTTP.
2. Add fixtures and tests.
3. Register it in `widgets::all()`.
