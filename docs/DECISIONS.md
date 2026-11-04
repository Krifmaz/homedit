# Decisions

Non-obvious calls made while building homedit, newest first.

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
