# homedit

A self-hosted homelab dashboard written in Rust, with a built-in visual editor
and a service finder. It reads existing [gethomepage](https://gethomepage.dev)
config folders, so current setups can move over without rewriting YAML.

Status: early development (Phase 1). See the crates below for what exists.

## Workspace layout

| Crate            | Package           | Purpose                                         |
|------------------|-------------------|-------------------------------------------------|
| `crates/core`    | `homedit-core`    | Shared data model, icon names, status checks    |
| `crates/db`      | `homedit-db`      | SQLite storage (sqlx), migrations, history      |
| `crates/server`  | `homedit`         | The axum server and binary, embeds `web/dist`   |
| `crates/import`  | `homedit-import`  | gethomepage config folder import                |
| `crates/discovery` | `homedit-discovery` | Read-only Docker discovery from container labels |
| `crates/ops`     | `homedit-ops`     | `/healthz`, `/readyz`, Prometheus `/metrics`    |
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
publish `ghcr.io/krifmaz/homedit:edge`; tags like `v0.1.0` publish `0.1.0`,
`0.1` and `latest`.

## License

Apache-2.0. See [LICENSE](LICENSE).
