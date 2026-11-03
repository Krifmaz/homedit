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
| `web/`           | –                 | React + TypeScript + Vite frontend              |

Migration numbers are split by crate so parallel work never collides; see the
header of `crates/db/migrations/0001_core.sql`.

## License

Apache-2.0. See [LICENSE](LICENSE).
