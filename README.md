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
| `HOMEDIT_DATA_DIR` | `/data`        | SQLite database, uploaded icons and images |
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
- Background images with blur, opacity, saturation and brightness, plus
  frosted-glass cards. Uploads go to `/data/images` and are served at
  `/images/`; gethomepage's `/images/…` and `/icons/…` paths work once those
  files are copied into `/data/images` and `/data/icons`.
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
| `crates/import`  | `homedit-import`  | gethomepage config folder import                |
| `crates/discovery` | `homedit-discovery` | Read-only Docker discovery from container labels |
| `crates/ops`     | `homedit-ops`     | `/healthz`, `/readyz`, Prometheus `/metrics`    |
| `crates/widgets` | `homedit-widgets` | Service widgets (Plex, *arr, qBittorrent, Proxmox, ...) |
| `web/`           | –                 | React + TypeScript + Vite frontend              |

Migration numbers are split by crate so parallel work never collides; see the
header of `crates/db/migrations/0001_core.sql`.

## Run with Docker

The image is a single static binary with the web UI embedded, built for
`linux/amd64` and `linux/arm64` on a `scratch` base (under 40 MB). It runs as
an unprivileged user (uid 65532) and keeps its data in `/data`.

```sh
docker run -d --name homedit -p 3000:3000 \
  -e HOMEDIT_SECRET_KEY="$(openssl rand -base64 32)" \
  -v homedit-data:/data \
  ghcr.io/krifmaz/homedit:latest
```

A fuller example with Docker discovery through a read-only socket proxy is in
[`deploy/docker-compose.yml`](deploy/docker-compose.yml).

| Variable                | Default         | Meaning                                              |
|-------------------------|-----------------|------------------------------------------------------|
| `HOMEDIT_LISTEN`        | `0.0.0.0:3000`  | Address and port to listen on                        |
| `HOMEDIT_DATA_DIR`      | `/data`         | Database, uploaded icons and backups                 |
| `HOMEDIT_SECRET_KEY`    | –               | Key for encrypting stored API keys and passwords     |
| `HOMEDIT_DOCKER_HOST`   | –               | `unix:///var/run/docker.sock` or `tcp://proxy:2375`  |
| `HOMEDIT_METRICS_TOKEN` | –               | If set, `/metrics` needs `Authorization: Bearer ...` |

The sign-in, single sign-on, reverse proxy, network, Docker and scan settings
(every `HOMEDIT_*` variable in [`crates/server/settings.toml`](crates/server/settings.toml))
can also be saved from Settings in the web app (`GET/PUT /api/settings`). An
environment variable always wins and locks its field; saved changes take
effect after a restart.

Build the image yourself:

```sh
docker build -t homedit .                                         # this machine
docker buildx build --platform linux/amd64,linux/arm64 -t homedit .  # both
scripts/smoke-test.sh homedit   # size, health, endpoints and idle memory
```

## Import from gethomepage

Point the importer at an existing gethomepage config folder. It reads
`services.yaml`, `bookmarks.yaml`, `widgets.yaml`, `settings.yaml`,
`docker.yaml` and `custom.css` (`.yml` works too), resolves
`{{HOMEPAGE_VAR_*}}` and `{{HOMEPAGE_FILE_*}}` placeholders from the
environment, and keeps unknown keys so nothing is lost. Entries it cannot read
are skipped and listed as warnings instead of failing the whole import.

## Export and backup

`GET /api/export` (admin) downloads the whole dashboard as `homedit.yaml`.
API keys and passwords are left out unless you add `?secrets=true`, which
writes them in plain text so the file restores on a server with a different
`HOMEDIT_SECRET_KEY`. Restore by uploading `homedit.yaml` to `POST /api/import`
(the Import button in the editor). Secrets are encrypted again on the way in,
and the previous dashboard stays in the version history.

## Scheduled rescans

An admin can have the App finder rescan on a schedule:
`PUT /api/scan/schedule` with `{"enabled": true, "intervalHours": 24,
"request": {"targets": ["192.168.1.0/24"], "ports": {"kind": "catalogue"}}}`
(1 to 720 hours). Scheduled scans follow the same policy, rate limit and
audit log as manual ones, are recorded as requested by `schedule`, and never
start while another scan runs. After every scan the server compares the
result with the previous one and logs and sends what appeared, disappeared
or changed (a `changes` scan event, and `diff` in `GET /api/scan`).

## API

Every endpoint is described in OpenAPI 3.1 at `/api/openapi.yaml`, and as a
readable page at `/api/docs` that works offline.

The shared routes of the [Krifmaz kit](https://github.com/Krifmaz/kit) (see its
`contracts/`) are served too: `/api/health`, What's new (`/api/changelog*`,
`/api/auth/prefs`), notices (`/api/notices*`), bug reports and crash logs
(`/api/bug-reports*`, `/api/crash-reports`), diagnostics (`/api/diagnostics*`)
and server settings (`/api/settings*`). The kit keeps its data in `kit.db` in
the data folder; back it up with `homedit.db`. Release notes are in
[`docs/releases`](docs/releases); non-obvious choices in
[`docs/DECISIONS.md`](docs/DECISIONS.md).

## Header widgets

The strip above the groups (gethomepage's `widgets.yaml`) is filled by
`GET /api/info/{id}`:

| Kind         | Shows                               | Options                          | Cache  |
|--------------|-------------------------------------|----------------------------------|--------|
| `datetime`   | Clock (rendered in the browser)     | gethomepage `format`             | –      |
| `resources`  | CPU, memory and disk of the server  | `disk` (path, default `/`)       | 5 s    |
| `openmeteo`  | Temperature and conditions          | `latitude`, `longitude`          | 10 min |
| `docker`     | Running and stopped containers      | uses the configured Docker hosts | 15 s   |

Weather comes from Open-Meteo (no API key), requested by the server.

## Docker discovery

homedit reads containers through the Docker API and adds a card for every
container with gethomepage-style labels. It only sends `GET` requests. Mount
the socket read-only, or better, use a socket proxy that allows only
`GET /containers`.

| Label                                   | Meaning                                         |
|-----------------------------------------|-------------------------------------------------|
| `homepage.group`, `homepage.name`       | Required. Group and card name                   |
| `homepage.icon`, `homepage.href`, `homepage.description` | Card icon, link and text       |
| `homepage.weight`                       | Sort order inside the group (lower first)       |
| `homepage.ping`, `homepage.siteMonitor` | TCP or HTTP status check target                 |
| `homepage.server`, `homepage.container` | Docker host and container (default: this one)   |
| `homepage.widget.type`, `homepage.widget.url`, `homepage.widget.*` | Service widget  |
| `homepage.widgets[N].*`                 | Extra widgets                                    |
| `homepage.instance.<name>.*`            | Overrides for one homedit instance               |

Values that look like JSON lists or objects (for example
`homepage.widget.fields=["queued","wanted"]`) are parsed as JSON.

## Health and metrics

* `GET /healthz`: liveness, used by the image's `HEALTHCHECK`
  (`homedit healthcheck`, so no shell or curl is needed).
* `GET /readyz`: readiness; checks the database and answers 503 if it fails.
* `GET /metrics`: Prometheus metrics: HTTP requests and latency by route,
  status checks, per-service up/down and response time, widget fetches,
  scans, logins, uptime and memory.

## Development

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check     # licences (no GPL) and advisories
```

CI runs all of the above, builds the web UI, builds the image for both
architectures, and checks the image size and idle memory. Pushes to `main`
publish `ghcr.io/krifmaz/homedit:edge`.

## Releases

Releases are automatic. When CI passes on `main`, the release workflow picks
the next version from the commits since the last tag, using
[Conventional Commits](https://www.conventionalcommits.org) prefixes:

| Commits since the last tag              | Next version (from 0.4.2)        |
|-----------------------------------------|----------------------------------|
| any `type!:` or `BREAKING CHANGE`       | 0.5.0 (on 1.x and later: 2.0.0)  |
| any `feat:` (and no breaking change)    | 0.5.0                            |
| anything else                           | 0.4.3                            |

It then bumps the workspace version, commits `chore(release): vX.Y.Z`, pushes
the `vX.Y.Z` tag and publishes:

* images `ghcr.io/krifmaz/homedit:X.Y.Z`, `:X.Y` and `:latest`,
* a GitHub Release with grouped notes, static `linux-amd64` and
  `linux-arm64` binaries, and `SHA256SUMS`.

The first release (no tag yet) uses the version already in `Cargo.toml`. To
release by hand, or to choose the version, run the **Release** workflow from
the Actions tab. `scripts/next-version.sh` prints the next version and
`scripts/release-notes.sh <version>` previews the notes.

## License

Apache-2.0. See [LICENSE](LICENSE).
