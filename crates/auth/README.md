# homedit-auth

Users, login, roles, CSRF, encrypted secrets and SSRF protection for homedit.

## What it does

- **Login methods** (any combination): local users with argon2id passwords,
  OpenID Connect (authorization code + PKCE, groups claim or userinfo), and
  trusted reverse-proxy headers (`Remote-User` / `Remote-Groups`, only from
  addresses in `HOMEDIT_TRUSTED_PROXIES`).
- **Roles**: `viewer` and `admin`. Only admins edit, manage users or scan.
  Groups with `hidden_from_viewers` are visible to admins only
  (`CurrentUser::can_view_group`).
- **Sessions**: random 256-bit token in an `HttpOnly`, `SameSite=Lax` cookie,
  stored hashed (SHA-256). `Secure` is set automatically on HTTPS (or always,
  with `HOMEDIT_COOKIE_SECURE=always`). Changing a password, role or
  disabling a user ends their sessions.
- **CSRF**: Origin / `Sec-Fetch-Site` check plus an `X-CSRF-Token` header on
  every state-changing request, also when login is disabled.
- **Login rate limit**: per client IP and per username (default 5 failures in
  15 minutes locks for 15 minutes), `429` with `Retry-After`.
- **Secrets**: AES-256-GCM with a key from `HOMEDIT_SECRET_KEY`. Values are
  bound to their field, never serialized in clear (`{"$secret":"set"}`), and
  zeroed in memory on drop.
- **SSRF guard**: outbound widget requests only reach hosts the admin
  configured; every redirect and DNS answer is checked; cloud metadata
  addresses are always blocked.

## Wiring (server crate)

```rust,ignore
homedit_auth::schema::init(&pool).await?;
let auth = AuthState::new(pool, AuthConfig::from_env()?, SecretBox::from_env_or_file(&data_dir.join("secret.key"))?).await?;
let app = Router::new()
    .route("/api/me", get(homedit_auth::routes::me))
    .nest("/api/auth", homedit_auth::routes::router())
    // ... your routes, using CurrentUser / RequireViewer / RequireAdmin ...
    .layer(axum::middleware::from_fn_with_state(auth.clone(), homedit_auth::middleware::auth_middleware))
    .with_state(app_state); // AppState: FromRef for AuthState
axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await?;
```

Widgets build their HTTP client with
`GuardedClient::new(SsrfGuard::from_urls(configured_urls), ClientOptions::default())`
and call `guard.replace(...)` when the dashboard changes.

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `HOMEDIT_AUTH_ENABLED` | `false` | Require login. When off, every visitor is admin (a warning is logged). |
| `HOMEDIT_AUTH_ANONYMOUS_ROLE` | `none` | Role for visitors who are not signed in: `none`, `viewer`, `admin`. |
| `HOMEDIT_ADMIN_USERNAME`, `HOMEDIT_ADMIN_PASSWORD` | | Create the first admin at startup if no users exist. |
| `HOMEDIT_AUTH_ALLOW_SETUP` | `true` | Allow `POST /api/auth/setup` while no users exist. |
| `HOMEDIT_AUTH_LOCAL` | `true` | Allow password login. |
| `HOMEDIT_SESSION_TTL` | `604800` | Session lifetime in seconds. |
| `HOMEDIT_COOKIE_SECURE` | `auto` | `auto`, `always` or `never`. |
| `HOMEDIT_TRUSTED_PROXIES` | | IPs / CIDRs whose `X-Forwarded-*` and auth headers are believed. |
| `HOMEDIT_ALLOWED_ORIGINS` | | Extra origins allowed to send changes. |
| `HOMEDIT_PROXY_AUTH` | `false` | Enable reverse-proxy header login. |
| `HOMEDIT_PROXY_USER_HEADER` / `_GROUPS_HEADER` | `Remote-User` / `Remote-Groups` | Header names. |
| `HOMEDIT_PROXY_ADMIN_GROUP` / `_ADMIN_USERS` / `_LOGOUT_URL` | | Admin mapping and logout link. |
| `HOMEDIT_OIDC_ISSUER`, `_CLIENT_ID`, `_CLIENT_SECRET`, `_REDIRECT_URL` | | Enable OIDC. Redirect URL ends in `/api/auth/oidc/callback`. |
| `HOMEDIT_OIDC_SCOPES` / `_GROUPS_CLAIM` / `_ADMIN_GROUP` / `_ALLOWED_GROUP` / `_DEFAULT_ROLE` / `_BUTTON_LABEL` | `profile email groups` / `groups` / | OIDC options. Dotted claims like `realm_access.roles` work. |
| `HOMEDIT_SECRET_KEY` | | 32 bytes, base64 (`openssl rand -base64 32`). |
| `HOMEDIT_SECRET_KEY_FILE` | | File holding the key (Docker secrets). |

## Endpoints

See the module docs of `routes`: `/api/me`, `/api/auth/{me,login,logout,setup,password}`,
`/api/auth/oidc/{login,callback}` and admin-only `/api/auth/users`.

## Tests

`cargo test -p homedit-auth` runs unit tests plus end-to-end tests through a
real axum router and in-memory SQLite (setup, login, roles, rate limit, CSRF,
proxy headers, sessions) and the SSRF client against local HTTP servers.
