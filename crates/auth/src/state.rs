//! Shared state for the auth middleware, extractors and routes.

use std::sync::Arc;

use axum::http::request::Parts;
use sqlx::SqlitePool;

use crate::client_ip::ClientInfo;
use crate::config::{AuthConfig, CookieSecure};
use crate::csrf::{CSRF_COOKIE, plausible_token};
use crate::error::AuthError;
use crate::extract::{AuthUser, CurrentUser, SessionRef};
use crate::oidc::OidcProvider;
use crate::password::{hash_password_async, validate_new_password};
use crate::ratelimit::LoginLimiter;
use crate::role::{Role, UserSource};
use crate::secrets::{SecretBox, SecretStore};
use crate::sessions::SessionStore;
use crate::users::UserStore;
use crate::util::{get_cookie, random_token};

/// Everything the auth layer needs. Cheap to clone; implement
/// `FromRef<YourAppState>` for it.
#[derive(Clone, Debug)]
pub struct AuthState {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    config: AuthConfig,
    pool: SqlitePool,
    users: UserStore,
    sessions: SessionStore,
    limiter: LoginLimiter,
    oidc: Option<OidcProvider>,
    secrets: SecretBox,
    secret_store: SecretStore,
    setup_lock: tokio::sync::Mutex<()>,
}

/// The result of resolving a request, before CSRF checks.
#[derive(Debug, Clone)]
pub(crate) struct Resolved {
    pub(crate) user: CurrentUser,
    /// A fresh CSRF cookie to set on the response.
    pub(crate) set_csrf_cookie: bool,
    /// The request carried a session cookie that is no longer valid.
    pub(crate) clear_session_cookie: bool,
}

impl AuthState {
    /// Builds the state. The `auth_*` tables must exist (call
    /// [`crate::schema::init`] first).
    ///
    /// When `HOMEDIT_ADMIN_USERNAME` and `HOMEDIT_ADMIN_PASSWORD` are set and
    /// no users exist yet, that admin is created.
    pub async fn new(
        pool: SqlitePool,
        config: AuthConfig,
        secrets: SecretBox,
    ) -> Result<Self, AuthError> {
        let oidc = match (&config.oidc, config.enabled) {
            (Some(c), true) => Some(OidcProvider::new(c.clone())?),
            _ => None,
        };
        if !config.enabled {
            tracing::warn!(
                "authentication is disabled: every visitor can edit the dashboard and run scans. Set HOMEDIT_AUTH_ENABLED=true to require login."
            );
        }
        if config.proxy.is_some() && config.trusted_proxies.is_empty() {
            tracing::warn!(
                "proxy header auth is on but HOMEDIT_TRUSTED_PROXIES is empty, so the headers will be ignored"
            );
        }
        let state = Self {
            inner: Arc::new(Inner {
                users: UserStore::new(pool.clone()),
                sessions: SessionStore::new(pool.clone(), config.session.ttl_secs),
                limiter: LoginLimiter::new(config.login_rate_limit.clone()),
                secret_store: SecretStore::new(pool.clone(), secrets.clone()),
                secrets,
                oidc,
                pool,
                config,
                setup_lock: tokio::sync::Mutex::new(()),
            }),
        };
        state
            .bootstrap_admin(
                std::env::var("HOMEDIT_ADMIN_USERNAME").ok(),
                std::env::var("HOMEDIT_ADMIN_PASSWORD").ok(),
            )
            .await?;
        Ok(state)
    }

    /// Creates the first admin from the given credentials if no users exist.
    pub async fn bootstrap_admin(
        &self,
        username: Option<String>,
        password: Option<String>,
    ) -> Result<(), AuthError> {
        let (Some(username), Some(password)) = (username, password) else {
            return Ok(());
        };
        let _guard = self.inner.setup_lock.lock().await;
        if self.users().count().await? > 0 {
            return Ok(());
        }
        validate_new_password(&password, self.config().local.min_password_length)?;
        let hash = hash_password_async(password).await?;
        self.users()
            .create_local(username.trim(), &hash, Role::Admin)
            .await?;
        tracing::info!(
            username = username.trim(),
            "created initial admin from environment"
        );
        Ok(())
    }

    /// The configuration.
    pub fn config(&self) -> &AuthConfig {
        &self.inner.config
    }
    /// The database pool.
    pub fn pool(&self) -> &SqlitePool {
        &self.inner.pool
    }
    /// User accounts.
    pub fn users(&self) -> &UserStore {
        &self.inner.users
    }
    /// Sessions.
    pub fn sessions(&self) -> &SessionStore {
        &self.inner.sessions
    }
    /// Login limiter.
    pub fn limiter(&self) -> &LoginLimiter {
        &self.inner.limiter
    }
    /// OIDC provider, when configured and auth is enabled.
    pub fn oidc(&self) -> Option<&OidcProvider> {
        self.inner.oidc.as_ref()
    }
    /// Encryption for secrets.
    pub fn secrets(&self) -> &SecretBox {
        &self.inner.secrets
    }
    /// Named secrets in `auth_secrets`.
    pub fn secret_store(&self) -> &SecretStore {
        &self.inner.secret_store
    }
    pub(crate) fn setup_lock(&self) -> &tokio::sync::Mutex<()> {
        &self.inner.setup_lock
    }

    /// Whether cookies for this client should carry `Secure`.
    pub fn cookie_secure(&self, client: &ClientInfo) -> bool {
        match self.config().session.secure {
            CookieSecure::Always => true,
            CookieSecure::Never => false,
            CookieSecure::Auto => client.https,
        }
    }

    /// Works out who is calling. Never fails on bad credentials: those just
    /// make the caller anonymous.
    pub(crate) async fn resolve(&self, parts: &Parts) -> Result<Resolved, AuthError> {
        let cfg = self.config();
        let client = ClientInfo::from_parts(parts, &cfg.trusted_proxies);
        let cookie_csrf = get_cookie(&parts.headers, CSRF_COOKIE).filter(|t| plausible_token(t));
        let set_csrf_cookie = cookie_csrf.is_none();
        let fallback_csrf = cookie_csrf.unwrap_or_else(|| random_token(32));

        let mk = |user: Option<AuthUser>,
                  role: Option<Role>,
                  csrf: String,
                  session: Option<SessionRef>| CurrentUser {
            user,
            role,
            auth_enabled: cfg.enabled,
            csrf_token: csrf,
            client: client.clone(),
            session,
        };

        if !cfg.enabled {
            return Ok(Resolved {
                user: mk(None, Some(Role::Admin), fallback_csrf, None),
                set_csrf_cookie,
                clear_session_cookie: false,
            });
        }

        if let Some(proxy_cfg) = &cfg.proxy {
            if let Some(user) = crate::proxy::authenticate(proxy_cfg, &client, &parts.headers) {
                let role = Some(user.role);
                return Ok(Resolved {
                    user: mk(Some(user), role, fallback_csrf, None),
                    set_csrf_cookie,
                    clear_session_cookie: false,
                });
            }
        }

        let mut clear_session_cookie = false;
        if let Some(token) = get_cookie(&parts.headers, &cfg.session.cookie_name) {
            match self.sessions().lookup(&token).await? {
                Some(s) => {
                    let user = AuthUser {
                        id: Some(s.user.id),
                        username: s.user.username.clone(),
                        role: s.user.role,
                        source: s.user.source,
                    };
                    let role = Some(user.role);
                    return Ok(Resolved {
                        user: mk(
                            Some(user),
                            role,
                            s.csrf_token,
                            Some(SessionRef {
                                token_hash: s.token_hash,
                            }),
                        ),
                        set_csrf_cookie: false,
                        clear_session_cookie: false,
                    });
                }
                None => clear_session_cookie = true,
            }
        }

        Ok(Resolved {
            user: mk(None, cfg.anonymous_role, fallback_csrf, None),
            set_csrf_cookie,
            clear_session_cookie,
        })
    }
}

impl CurrentUser {
    /// Whether the caller signed in with a homedit session (local or OIDC).
    pub fn has_session(&self) -> bool {
        self.session.is_some()
    }

    /// Whether the caller is a local account that can change its password.
    pub fn is_local(&self) -> bool {
        self.user
            .as_ref()
            .is_some_and(|u| u.source == UserSource::Local)
    }
}
