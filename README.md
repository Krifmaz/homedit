# homedit

A self-hosted homelab dashboard written in Rust, with a built-in visual editor
and a service finder. It reads existing [gethomepage](https://gethomepage.dev)
config folders, so current setups can move over without rewriting YAML.

Status: early development (Phase 1).

## Running from source

```sh
(cd web && npm ci && npm run build)   # optional: without it a placeholder page is served
HOMEDIT_DATA_DIR=./data HOMEDIT_LISTEN=127.0.0.1:3000 cargo run -p homedit
```

| Variable           | Default        | Meaning                           |
|--------------------|----------------|-----------------------------------|
| `HOMEDIT_LISTEN`   | `0.0.0.0:3000` | Address to listen on              |
| `HOMEDIT_DATA_DIR` | `/data`        | SQLite database and uploaded icons |
| `RUST_LOG`         | `info`         | Log filter                        |

## What works now

- Dashboard data model compatible with gethomepage keys (groups, nested groups,
  services, bookmarks, info widgets, Docker hosts); unknown keys are kept.
- SQLite storage with stable card ids, a version on every save, and history
  with one-click restore.
- Status checks: `siteMonitor` (HTTP, up when status < 500) and `ping`
  (TCP connect, no raw sockets), every 30 s by default, with the last 30
  samples per card for sparklines and 7 days kept in the database.
- REST API (see `crates/server/src/api.rs`) and live updates over SSE at
  `/api/events` (`status` and `config` events).
- The first page load carries the dashboard and status inline, so the UI
  paints without waiting for the API.
- Icons by name: dashboard-icons (`sonarr`, `sonarr.svg`), selfh.st
  (`sh-sonarr`), Material Design (`mdi-server`), Simple Icons (`si-github`),
  URLs, and uploads (`custom/nas.png`, served with a sandboxing CSP).
- Widget secrets are masked in every response (`••••` on the dashboard,
  `{"$secret":"set"}` in the editor document, kept on save).

Login is not wired in yet: until the auth crate lands, every visitor is an
admin, so only run this on a trusted network.

## Workspace layout

| Crate            | Package           | Purpose                                         |
|------------------|-------------------|-------------------------------------------------|
| `crates/core`    | `homedit-core`    | Shared data model, icon names, status checks    |
| `crates/db`      | `homedit-db`      | SQLite storage (sqlx), migrations, history      |
| `crates/auth`    | `homedit-auth`    | Login (local, OIDC, proxy), roles, CSRF, encrypted secrets, SSRF guard |
| `crates/server`  | `homedit`         | The axum server and binary, embeds `web/dist`   |
| `crates/scanner` | `homedit-scanner` | Port scanner and app fingerprinting (Phase 4)   |
| `web/`           | –                 | React + TypeScript + Vite frontend              |

Migration numbers are split by crate so parallel work never collides; see the
header of `crates/db/migrations/0001_core.sql`.

## License

Apache-2.0. See [LICENSE](LICENSE).
