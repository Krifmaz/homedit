# Decisions

Non-obvious calls made while building homedit, newest first.

## 2026-11-05: A widget's failed upstream is a 200

- `GET /api/info/{id}` answers `200 {"error": "<sentence>"}` when its upstream fails (weather, Docker, host resources), like `/api/widgets/{id}/data` already did, not 502 (which read as homedit itself being down) and not 424 (an error status still lands in the client's error path and in proxies' and browsers' logs as a failure). Errors are not cached, so the next poll retries.
- The card shows the sentence with the kit's Copy button (`details::copy_button_html`, handled by the kit's page listener) rather than the `CopyButton` view: that one stops the click's propagation, so the card's link around it could not cancel the navigation.
- The kit fix for the "server isn't answering" banner (a marked 502–504 is one call's error, not the server down) went into the kit (Krifmaz/kit#28) since no kit release had it.

## 2026-11-05: Fast and accurate network scan (after Caddymium's)

- Live hosts first: addresses from a range get a quick connect to 16 common ports (a refused connection counts as an answer), a reverse DNS lookup and a look at the ARP table, and only hosts that answer are port-scanned. Addresses and hostnames typed one by one skip the check and are always scanned, since the admin named them. `discover: false` in the request brings back the old scan-everything behaviour.
- A reverse DNS name counts as "up", as in Caddymium: Docker's DNS only answers for running containers. A router that keeps names for stale leases costs one extra port scan (about 6 s at the defaults), not a wrong result.
- The probes of the live-host check that answered are not repeated in the port scan; the rest of a host's 16 are cancelled as soon as one answers.
- New defaults: 512 connections at once, 400 ms connect timeout, no per-second cap (still a setting: `HOMEDIT_SCAN_RATE`, 0 = none), 32 fingerprints at a time with a 2 s timeout. Saved settings keep their values; only unset ones move to the new defaults.
- Unidentified ports are named after the catalogue app usually on that port (`guess`, "Sonarr" on 8989). It is a hint only: `app` stays empty, so no widget is suggested from a port number alone.
- Reverse DNS and Docker container names are a label (`name` on the host), not the link host: Docker names such as `jellyfin.media_default` don't resolve in a browser, so links keep the address. Docker names lose the network suffix (names with `_` are cut to the first label; LAN hostnames never contain `_`).
- Default targets come from `GET /api/scan/networks`: each private IPv4 interface's /24 (smaller prefixes kept), LAN first, Docker networks marked from the interface name or the Docker API's container addresses. The usual 192.168.1.0/24 only stands in when the server can't tell. Ranges skip the server's own addresses.
- Docker first: with a Docker address set, every scan lists published ports before anything else (named by container and image, fingerprinted in the background) and the LAN scan doesn't probe them again. Hosts on Docker networks are named after their container. Published ports are shown at the server's LAN address; in a bridge-networked container, which can't see the LAN address, at the private address the admin opened homedit at (`Host` / `X-Forwarded-Host`), which saved schedules keep. With neither, published ports are skipped and only container names are used.
- Network detection and live-host discovery stay in homedit's scanner crate rather than the kit: the kit holds app-shell pieces (look, settings, updates, reports), and the scanner crate is already a standalone library Caddymium-style apps could use.
- Tests use a fake `Network` (probe, reverse DNS, ARP table) on a paused clock: a /24 with seven live hosts, two of them firewalled, gives the same results as a full scan with about a sixth of the time and probes, and the default ports finish in 14.5 s of simulated time.

## 2026-11-04: Release on the kit's shared workflows

- CI's Rust and web jobs are the kit's `kit-ci.yml`; homedit's own checks (the kit's stylesheets, `cargo doc`, the trunk build) go in its `check` input. The licence check and the image job with its smoke test stay homedit's, since the kit has neither.
- Releases are the kit's `kit-release.yml` and `kit-docker.yml`. The version rule changes with them: PATCH by default, MINOR or MAJOR only when the change adds that version's `docs/releases/<version>.md`, instead of reading `feat:` from commit messages. The notes file already had to exist for What's new, so it now also picks the bump.
- The kit tags through the release API and makes no version-bump commit, so the version is built in from `HOMEDIT_VERSION` (as the kit's template does with `APP_VERSION`, named for homedit so it can't clash). `Cargo.toml` keeps the version of the last release made the old way.
- The static binaries are still the musl ones from the image (pulled anonymously, the package is public) through `kit-release`'s `files` input, not the kit's own glibc server tarballs, which would be built before the web app is embedded.
- The `X.Y` image tag is kept (as one of `kit-docker`'s `extra-tags`), so anyone following `:0.3` still gets patch releases.

## 2026-11-04: Move the web app to Leptos on the kit

- The web app moved from React to Leptos (the owner's call), so the kit's Leptos components are used as they are instead of copied into TypeScript. It is a Cargo workspace of its own in `web/`, so the server's `cargo test --workspace` never builds WebAssembly.
- The kit's `tokens.css` and `kit.css` and the template's `app.css` are copied into `web/static/css` the way the kit's template does it, and CI (`scripts/kit-css-check.sh`) fails when they drift from the pinned kit tag.
- Crash logs for the web app went into the kit (`kit_leptos::crash`), not homedit, since every kit web app needs them and the server side was already there.
- The self-update waits while edit mode is open (not only with unsaved changes), so a new build never closes an open form.
- Keyboard reordering is Alt+arrow keys on a focused card, group or bookmark (the React app used Space then arrows); touch drags start only from the ⠿ handle so the page still scrolls.
- The web app's version follows the server's: the release workflow bumps `web/Cargo.toml` too.
- The /24 question before a scan of a bare network address (from #23) is now the kit's confirm dialog, like the "all 65,535 ports" one: OK scans the /24, "Scan as typed" scans the single address.

## 2026-11-04: Adopt the Krifmaz kit on the server

- The kit's tables live in their own `kit.db` in the data folder, not in `homedit.db`: the kit opens its file with rusqlite and runs its own migrations, homedit runs sqlx's, and two migrators on one file would fight over locks for nothing.
- The kit needed rusqlite 0.39 (libsqlite3-sys 0.37) so it links the same sqlite3 as homedit's sqlx 0.9; that went into the kit (Krifmaz/kit#25), not a workaround here.
- The account preferences route (What's new's switch) was added to the kit (`kit_server_changelog::prefs_router`) instead of homedit, since every kit app needs one and the template had its own copy.
- The kit sees homedit's caller through the auth middleware's `CurrentUser`: users by their row id; with sign-in off (or visitors made admins) the one admin, id 0; reverse-proxy users, who have no row, by a stable negative id from their name. Admins are also the kit's "owner", since homedit has no role above admin.
- Server settings: every `HOMEDIT_*` variable that is read after start-up is in `crates/server/settings.toml`, with today's defaults, so a server that sets nothing behaves exactly as before and an environment variable still wins (and locks the field). LISTEN, DATA_DIR, SECRET_KEY, IMPORT_DIR and the ADMIN bootstrap pair stay environment-only: they are needed before the settings can be read, or are one-time.
- Saved settings take effect after a restart (the cards and the save's warning say so) instead of rebuilding auth, OIDC and the scanner live; a restart is what changing the variables needed before.
- A settings save that would not start (for example an OIDC issuer without a client ID) is refused with the reason, and if `kit.db` is edited by hand into such a state the server logs it and starts from the environment alone, so a bad save can never lock the owner out.
- Bool settings can't drive `when` in the kit's schema (only selects can), so the sign-in and proxy fields are always shown; their help text says what they need.
- Release notes now live in `docs/releases/<version>.md` (What's new reads them); notes for 0.1.0–0.1.5 were written from the GitHub releases.
- No kit-server-updates: homedit ships only a Docker image (no Android or desktop app to update in place), and the web app updates itself from `/api/health`.

## 2026-11-04: Offer a /24 scan for bare network addresses at start and on empty results

Entering `10.0.1.0` scans that one (usually unused) address and finds nothing.
A hint under the targets box already suggested `/24` since v0.1.3, but it was
easy to miss. Now Start scan asks "Scan 10.0.1.0/24 instead?" (yes widens the
box and scans the network, no scans the single address as typed), and a
finished scan of a bare address that found nothing offers a one-click
"Scan 10.0.1.0/24".

- The server still treats `10.0.1.0` as a single address. Rewriting it
  server-side would silently change what an admin asked for (and break
  saved rescan schedules that target exactly that address), so the
  widening stays an explicit choice in the UI.
- The question uses `window.confirm` like the existing "all 65,535 ports"
  prompt. When the kit's confirm dialog is adopted, both switch together.
