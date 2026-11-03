# homedit web UI

The browser side of homedit: React 19, TypeScript and Vite. The production
build in `web/dist` is embedded into the `homedit` binary with rust-embed, so
the Docker image ships a single executable.

## Develop

```sh
cd web
npm ci
npm run dev:mock      # in-browser mock API, no backend needed: http://localhost:5173/
npm run dev           # proxies /api and /icons to a backend on 127.0.0.1:3000
```

In mock mode, add `?role=viewer` to the URL to see the dashboard as a viewer.
Any build also switches to the mock when the URL has `?mock`.

## Check

```sh
npm run typecheck
npm run lint
npm run format:check
npm test
npm run build         # writes web/dist
```

## How it fits together

| Path                          | What it does                                                       |
| ----------------------------- | ------------------------------------------------------------------ |
| `src/api/types.ts`            | Types of the REST API (the contract shared with the Rust server)   |
| `src/api/client.ts`           | `Api` interface and the `fetch` implementation (CSRF, `If-Match`)  |
| `src/api/mock.ts`             | In-memory `Api` with sample data, used by `dev:mock` and the tests |
| `src/app/context.tsx`         | App state: session, document, live status, editor and undo/redo    |
| `src/app/App.tsx`             | Shell: header, search, info widgets, dashboard, toolbar, drawer    |
| `src/components/`             | Dashboard, cards, status sparkline, bookmarks, search, widgets     |
| `src/editor/ops.ts`           | Pure edit operations on the config document                        |
| `src/editor/history.ts`       | Undo/redo reducer (edits with one coalesce key form one step)      |
| `src/editor/forms/`           | Drawer forms, generated from widget schemas where possible         |
| `src/editor/HistoryPanel.tsx` | Saved versions with a diff summary and one-click restore           |

### First paint

The server replaces `<!--HOMEDIT_DATA-->` in `index.html` with
`<script id="homedit-data" type="application/json">…</script>` holding the
`/api/dashboard` payload. The app renders from it immediately, then fetches
status and widget data live (status updates stream over `/api/events`).

### Edit mode

Admins get an **Edit** button. Edit mode loads `/api/config` (which includes
groups hidden from viewers) and the widget catalogue. Every change is applied
to a working copy shown live on the dashboard:

- Drag cards within and between groups, reorder groups and bookmarks
  (mouse, touch, or keyboard: focus a card, press Space, use the arrow keys).
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
