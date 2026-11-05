# homedit web UI

The browser side of homedit: [Leptos](https://leptos.dev) 0.8 (client-side, WebAssembly) on the
[Krifmaz kit](https://github.com/Krifmaz/kit)'s components and Encore look, built by
[trunk](https://trunkrs.dev). The build in `web/dist` is embedded into the `homedit` binary with rust-embed, so
the Docker image ships a single executable.

It is a Cargo workspace of its own, so the server's workspace never builds WebAssembly.

## Develop

```sh
rustup target add wasm32-unknown-unknown
cargo install trunk --locked     # or a release binary from github.com/trunk-rs/trunk
cd web
trunk watch --release            # rebuilds web/dist; a debug server reads it from disk
```

Run the server next to it (`cargo run -p homedit` in the repository root) and open http://localhost:3000/.

## Check

```sh
cd web
cargo fmt -- --check
cargo clippy --target wasm32-unknown-unknown -- -D warnings
cargo test                       # the pure parts (model, edits, undo, drag and drop, words) run natively
trunk build --release            # writes web/dist
../scripts/kit-css-check.sh      # the kit's stylesheets match the pinned kit (--fix copies them)
```

## App icon

`icon.svg` is homedit's glyph (24 × 24, `currentColor` and `{accent-2}`). The kit's `kit-icons` puts it on the kit's
tile and writes the whole set (favicon, PNGs, Apple touch and maskable icons, web manifest) into
`public/app-icons/`, served at `/app-icons/` (`/icons/` is the dashboard's own icon folder). After changing the
glyph, run this in the repository root and commit the result:

```sh
cargo install --git https://github.com/Krifmaz/kit --tag v0.3.0 kit-icons --locked
kit-icons web/icon.svg web/public/app-icons --name homedit --base /app-icons/
```

## How it fits together

| Path | What it does |
| --- | --- |
| `src/main.rs` | Start-up: the kit's config, API client, crash logs, toasts, notices, offline banner, self-update |
| `src/client.rs` | homedit's API calls on the kit's client (CSRF and `If-Match` headers), the live events |
| `src/model.rs` | The API's shapes (session, dashboard, config document, statuses) |
| `src/state.rs` | App state: session, document, live status, editor and undo/redo, the open drawer |
| `src/shell.rs` | The page: header, search, info widgets, dashboard, toolbar, drawer, account menu, routes |
| `src/dash.rs`, `src/widgets.rs` | Groups, cards, status sparklines, bookmarks; card widgets and the info strip |
| `src/ops.rs`, `src/undo.rs` | Pure edits on the config document; undo/redo (one coalesce key, one step) |
| `src/dnd.rs` | Drag and drop of cards, groups and bookmarks (mouse, touch from the handle, Alt+arrows) |
| `src/forms.rs`, `src/appearance.rs` | Drawer forms (widget fields from the catalogue's schemas), appearance, info widgets |
| `src/panels.rs`, `src/scan.rs`, `src/backup.rs` | History, accounts, password; Find services; import and export |
| `src/kitpages.rs` | The kit's pages: Settings (from the server's schema), Diagnostics, What's new, Bug reports |
| `static/css/` | The kit's `tokens.css` and `kit.css`, the template's `app.css` (copies), then `homedit.css` |

### First paint

The server replaces `<!--HOMEDIT_DATA-->` in `index.html` with
`<script id="homedit-data" type="application/json">…</script>` holding the
`/api/dashboard` payload. The app renders from it immediately, then fetches
status and widget data live (status updates stream over `/api/events`).

### Edit mode

Admins get an **Edit** button. Edit mode loads `/api/config` (which includes
groups hidden from viewers) and the widget catalogue. Every change is applied
to a working copy shown live on the dashboard:

- Drag cards within and between groups, reorder groups and bookmarks (mouse;
  touch from the ⠿ handle; keyboard: focus a card and press Alt+arrow keys).
- Forms open in a side drawer; the dashboard previews each keystroke. A whole
  form session is one undo step, and **Cancel** reverts it.
- Undo and redo with the toolbar or Ctrl/Cmd+Z, Ctrl/Cmd+Shift+Z, Ctrl+Y.
  Ctrl/Cmd+S saves.
- Saving sends `PUT /api/config` with `If-Match`; if someone else saved
  first, the UI offers to reload.
- Secrets come back from the server as `{"$secret":"set"}` and are shown as
  `••••••••`. They are only sent when you replace them.
- **History** lists saved versions with what changed and restores any of
  them in one click (the restore is itself a new version).
