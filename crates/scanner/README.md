# homedit-scanner

The port scanner and app finder behind homedit's "Find apps" page. It scans
the hosts and networks an admin lists, works out which self-hosted app answers
on each open port, and returns ready-to-use "Add to dashboard" values.

This crate is a library with no database or web framework dependency. The
server stores reports, enforces admin-only access and exposes the routes.

## What it does

- **Targets**: CIDR ranges (`192.168.1.0/24`, `fd00::/120`), addresses and
  hostnames. Hostnames go through the system resolver; nothing is ever run
  through a shell.
- **Safe by default**:
  - Only private (RFC 1918, IPv6 ULA), link-local and loopback addresses.
    Anything else needs `allow_public` or an `extra_allowed` range such as
    `100.64.0.0/10` for Tailscale. Multicast, broadcast and unspecified
    addresses are always refused, and IPv4-mapped IPv6 addresses are judged
    as IPv4.
  - Plain TCP `connect()` checks only: no raw sockets, no extra privileges.
  - A concurrency cap, an optional token-bucket limit on new connections
    per second (off by default; HTTP fingerprint requests count too), and
    limits on addresses and probes per scan.
  - Every target is checked before the first packet is sent. If one is
    refused, nothing is scanned.
  - An audit record for every scan start and finish on the
    `homedit_scanner::audit` tracing target.
- **Live hosts first**: addresses from a range get a quick connect to a few
  common ports (a refused connection proves the host is up too), a reverse
  DNS lookup and a look at the ARP table. Only hosts that answer are
  port-scanned, and each open port is identified as soon as it is found.
  Addresses and hostnames typed one by one are always scanned; set
  `discover: false` to port-scan every address of every range.
- **Default targets**: `local_networks()` lists this machine's private
  networks (each interface's /24) and its own addresses; give the addresses
  to `Scanner::with_own_addresses` so ranges skip them.
- **Ports**: `common` (80, 443, 3000–9999 and every catalogue port, the
  default), `catalogue` (fast, good for scheduled rescans), `full` (1–65535,
  opt-in) or a list such as `80,443,8000-8100`.
- **Fingerprints**: page title, favicon hash (Shodan-compatible
  MurmurHash3), `Server` header, redirects, and known unauthenticated API
  paths: `/identity` (Plex), `/System/Info/Public` (Jellyfin),
  `/api/v3/system/status` (Sonarr, Radarr), `/api/v1/system/status`
  (Prowlarr, Lidarr, Readarr), `/api/v2/app/version` (qBittorrent),
  `/api?mode=version` (SABnzbd), `/api/v1/status` (Seerr family) and more.
  Versions are read where the app reveals them. Unknown web apps are still
  listed with their title and favicon.
- **Catalogue**: 44 apps, with ids matching gethomepage widget types
  (`plex`, `jellyfin`, `emby`, `sonarr`, `radarr`, `lidarr`, `readarr`,
  `prowlarr`, `bazarr`, `qbittorrent`, `transmission`, `deluge`, `sabnzbd`,
  `nzbget`, `seerr`, `jellyseerr`, `overseerr`, `tautulli`, `proxmox`,
  `proxmoxbackupserver`, `portainer`, `immich`, `pihole`, `adguard`,
  `uptimekuma`, `homeassistant`, `grafana`, `nextcloud`, `gitea`, `traefik`,
  `npm`, `navidrome`, `audiobookshelf`, `paperlessngx`, `frigate`,
  `prometheus`, `mealie`, `glances`, `unifi`, `truenas`, plus `forgejo`, `syncthing`, `vaultwarden` and `codeserver`).
- **Docker**: reads published ports from the Docker API (Unix socket or a
  TCP socket proxy, `GET /containers/json` only), recognises apps from the
  image name, and can confirm them over HTTP. With `Scanner::with_docker`,
  every scan lists published ports first (they appear at once and are not
  scanned again) and names hosts on Docker networks after their container.
- **Rescans**: `diff()` compares two reports and words the changes, for
  example "Jellyfin found on 10.0.1.7:8096" or "Sonarr on 10.0.1.7:8989
  stopped answering". Ports or hosts outside the new scan's scope never count
  as gone.

## Using it

```rust
use scanner::{PortSet, ScanPolicy, ScanRequest, Scanner};

let scanner = Scanner::new(ScanPolicy::default())?;
let mut req = ScanRequest::new(vec!["192.168.1.0/24".parse()?]);
req.ports = PortSet::Common;
req.requested_by = Some("admin".into());

// Optional live progress for a progress bar or SSE stream.
let (tx, mut rx) = tokio::sync::mpsc::channel(256);
let mut report = scanner.scan(req, Some(tx)).await?;

// Mark what is already on the dashboard, then offer the rest.
report.mark_on_dashboard(existing_hrefs.iter().map(String::as_str));
for (host, service) in report.services() {
    let suggestion = service.suggestion(&host.host_label());
    // suggestion.name, .href, .icon ("jellyfin.png"), .widget
    // ({"type":"jellyfin","url":"http://..."}): the form asks only for the key.
}
```

Other entry points: `Scanner::fingerprint_service(ip, port)` for one port,
`Scanner::docker_services(endpoint, host_ip, hostname, fingerprint)` for a
Docker host, and `scanner::diff(&old, &new, scanner.catalogue())` for
scheduled rescans. Every type is `serde` serializable for storage and the
REST API. To cancel a scan, drop its future.

## Policy defaults

| Setting                   | Default    |
|---------------------------|------------|
| `allow_public`            | `false`    |
| `extra_allowed`           | `[]`       |
| `max_concurrency`         | 512        |
| `connects_per_second`     | 0 (unlimited) |
| `connect_timeout_ms`      | 400        |
| `http_timeout_ms`         | 2000       |
| `fingerprint_concurrency` | 32         |
| `max_addresses`           | 1024       |
| `max_probes`              | 10,000,000 |

## Tests

```sh
cargo test -p homedit-scanner
```

The integration tests in `tests/mock_network.rs` start fake app servers
(Jellyfin, Plex, Sonarr, qBittorrent, an HTTPS Proxmox, an unknown app with a
favicon, an SSH-style banner) on loopback addresses and a fake Docker socket,
then scan them end to end, including a `/29` range, rate limiting, progress
events and a rescan diff. `tests/fake_lan.rs` scans a simulated /24 (a fake
`Network` with live, firewalled and empty addresses on a paused clock) and
checks that finding live hosts first gives the same results as a full scan,
in under 30 seconds.

## Adding an app

Add an `AppDef` to `BUILTIN_APPS` in `src/catalogue.rs` with its default
ports, image names, title or header signals, and, if it has one, an
unauthenticated API path that confirms it. Only `GET` requests without
credentials are allowed. Write fingerprints from the app's public docs and
its own responses.
