//! Authentication configuration.
//!
//! [`AuthConfig`] deserializes from YAML/JSON (so the server can keep it in
//! `settings.yaml`) and can also be built from environment variables with
//! [`AuthConfig::from_env`].

use ipnet::IpNet;
use serde::{Deserialize, Serialize};

use crate::role::Role;

/// Top-level authentication settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AuthConfig {
    /// Master switch. When `false` (the default, like gethomepage) nobody
    /// logs in and every visitor is treated as an admin. A warning is logged
    /// at startup so this is never silent.
    pub enabled: bool,
    /// Role given to visitors who are not signed in. `None` means they get
    /// nothing and must log in. Ignored when `enabled` is `false`.
    pub anonymous_role: Option<Role>,
    /// Local username/password accounts.
    pub local: LocalConfig,
    /// OpenID Connect single sign-on. Off when absent.
    pub oidc: Option<OidcConfig>,
    /// Trusted reverse-proxy header authentication. Off when absent.
    pub proxy: Option<ProxyConfig>,
    /// Session cookie settings.
    pub session: SessionConfig,
    /// Failed-login throttling.
    pub login_rate_limit: RateLimitConfig,
    /// Reverse proxies whose `X-Forwarded-*` headers are believed, as
    /// addresses or CIDR ranges. Empty means none are.
    pub trusted_proxies: Vec<IpNet>,
    /// Extra origins (e.g. `https://dash.example.com`) allowed to send
    /// state-changing requests, in addition to the request's own host.
    pub allowed_origins: Vec<String>,
}

/// Local accounts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LocalConfig {
    /// Allow username/password login.
    pub enabled: bool,
    /// Allow `POST /api/auth/setup` to create the first admin while no
    /// users exist. Turn off if you bootstrap with environment variables.
    pub allow_setup: bool,
    /// Minimum password length for new and changed passwords.
    pub min_password_length: usize,
}

impl Default for LocalConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_setup: true,
            min_password_length: 10,
        }
    }
}

/// OpenID Connect provider settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OidcConfig {
    /// Issuer URL; `<issuer>/.well-known/openid-configuration` must exist.
    pub issuer_url: String,
    /// OAuth client id.
    pub client_id: String,
    /// OAuth client secret. Leave empty for public clients (PKCE only).
    #[serde(default)]
    pub client_secret: Option<String>,
    /// Absolute callback URL registered with the provider, ending in
    /// `/api/auth/oidc/callback`.
    pub redirect_url: String,
    /// Extra scopes besides `openid` (default: `profile email groups`).
    #[serde(default = "default_scopes")]
    pub scopes: Vec<String>,
    /// ID-token claim holding the user's groups.
    #[serde(default = "default_groups_claim")]
    pub groups_claim: String,
    /// Members of this group become admins. Without it everyone gets
    /// `default_role`.
    #[serde(default)]
    pub admin_group: Option<String>,
    /// If set, only members of this group may sign in at all.
    #[serde(default)]
    pub allowed_group: Option<String>,
    /// Role for users who are not in `admin_group`.
    #[serde(default = "default_viewer")]
    pub default_role: Role,
    /// Text on the login button.
    #[serde(default = "default_oidc_label")]
    pub button_label: String,
}

fn default_scopes() -> Vec<String> {
    vec!["profile".into(), "email".into(), "groups".into()]
}
fn default_groups_claim() -> String {
    "groups".into()
}
fn default_viewer() -> Role {
    Role::Viewer
}
fn default_oidc_label() -> String {
    "Sign in with SSO".into()
}

/// Trusted reverse-proxy header authentication (Authelia, Authentik,
/// oauth2-proxy, Cloudflare Access, ...).
///
/// Headers are only believed when the TCP peer is in
/// [`AuthConfig::trusted_proxies`]; otherwise they are ignored.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyConfig {
    /// Header carrying the username.
    #[serde(default = "default_user_header")]
    pub user_header: String,
    /// Header carrying the user's groups (comma or `|` separated).
    #[serde(default = "default_groups_header")]
    pub groups_header: String,
    /// Members of this group become admins.
    #[serde(default)]
    pub admin_group: Option<String>,
    /// Usernames that are admins regardless of groups.
    #[serde(default)]
    pub admin_users: Vec<String>,
    /// Role for everyone else.
    #[serde(default = "default_viewer")]
    pub default_role: Role,
    /// URL to send the browser to for logging out at the proxy.
    #[serde(default)]
    pub logout_url: Option<String>,
}

fn default_user_header() -> String {
    "Remote-User".into()
}
fn default_groups_header() -> String {
    "Remote-Groups".into()
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            user_header: default_user_header(),
            groups_header: default_groups_header(),
            admin_group: None,
            admin_users: Vec::new(),
            default_role: Role::Viewer,
            logout_url: None,
        }
    }
}

/// When to mark cookies `Secure`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CookieSecure {
    /// Secure when the request arrived over HTTPS (directly, or according to
    /// `X-Forwarded-Proto` from a trusted proxy). Lets plain-HTTP LAN
    /// installs work while HTTPS installs get secure cookies.
    #[default]
    Auto,
    /// Always set `Secure`.
    Always,
    /// Never set `Secure` (not recommended).
    Never,
}

/// Session settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SessionConfig {
    /// Name of the session cookie.
    pub cookie_name: String,
    /// Session lifetime in seconds (default 7 days).
    pub ttl_secs: u64,
    /// When to set the `Secure` cookie flag.
    pub secure: CookieSecure,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            cookie_name: "homedit_session".into(),
            ttl_secs: 7 * 24 * 3600,
            secure: CookieSecure::Auto,
        }
    }
}

/// Failed-login throttling.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RateLimitConfig {
    /// Failures allowed per key (client IP, and separately username) within
    /// `window_secs` before the key is locked.
    pub max_failures: u32,
    /// Window in which failures are counted.
    pub window_secs: u64,
    /// How long a key stays locked.
    pub lockout_secs: u64,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_failures: 5,
            window_secs: 15 * 60,
            lockout_secs: 15 * 60,
        }
    }
}

/// An invalid environment variable.
#[derive(Debug, thiserror::Error)]
#[error("invalid value for {var}: {reason}")]
pub struct ConfigError {
    /// The variable name.
    pub var: &'static str,
    /// Why it was rejected.
    pub reason: String,
}

impl AuthConfig {
    /// Whether any login method is active. `false` means everyone is admin.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Builds a config from `HOMEDIT_*` environment variables.
    ///
    /// | Variable | Meaning |
    /// |---|---|
    /// | `HOMEDIT_AUTH_ENABLED` | `true` to require login |
    /// | `HOMEDIT_AUTH_ANONYMOUS_ROLE` | `viewer`, `admin` or `none` |
    /// | `HOMEDIT_AUTH_LOCAL` | `false` to disable password login |
    /// | `HOMEDIT_AUTH_ALLOW_SETUP` | `false` to disable first-run setup |
    /// | `HOMEDIT_COOKIE_SECURE` | `auto`, `always` or `never` |
    /// | `HOMEDIT_SESSION_TTL` | session lifetime in seconds |
    /// | `HOMEDIT_TRUSTED_PROXIES` | comma separated IPs/CIDRs |
    /// | `HOMEDIT_ALLOWED_ORIGINS` | comma separated origins |
    /// | `HOMEDIT_PROXY_AUTH` | `true` to enable header auth |
    /// | `HOMEDIT_PROXY_USER_HEADER` / `_GROUPS_HEADER` / `_ADMIN_GROUP` / `_ADMIN_USERS` / `_LOGOUT_URL` | header auth settings |
    /// | `HOMEDIT_OIDC_ISSUER` | enables OIDC |
    /// | `HOMEDIT_OIDC_CLIENT_ID` / `_CLIENT_SECRET` / `_REDIRECT_URL` | OIDC client |
    /// | `HOMEDIT_OIDC_SCOPES` / `_GROUPS_CLAIM` / `_ADMIN_GROUP` / `_ALLOWED_GROUP` / `_DEFAULT_ROLE` / `_BUTTON_LABEL` | OIDC options |
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|k| std::env::var(k).ok())
    }

    /// Like [`from_env`](Self::from_env) but reads variables through `get`,
    /// which makes it testable.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let get = |k: &str| {
            get(k)
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
        };
        let mut c = AuthConfig::default();

        if let Some(v) = get("HOMEDIT_AUTH_ENABLED") {
            c.enabled = parse_bool("HOMEDIT_AUTH_ENABLED", &v)?;
        }
        if let Some(v) = get("HOMEDIT_AUTH_ANONYMOUS_ROLE") {
            c.anonymous_role = match v.to_ascii_lowercase().as_str() {
                "none" => None,
                other => Some(other.parse().map_err(|reason| ConfigError {
                    var: "HOMEDIT_AUTH_ANONYMOUS_ROLE",
                    reason,
                })?),
            };
        }
        if let Some(v) = get("HOMEDIT_AUTH_LOCAL") {
            c.local.enabled = parse_bool("HOMEDIT_AUTH_LOCAL", &v)?;
        }
        if let Some(v) = get("HOMEDIT_AUTH_ALLOW_SETUP") {
            c.local.allow_setup = parse_bool("HOMEDIT_AUTH_ALLOW_SETUP", &v)?;
        }
        if let Some(v) = get("HOMEDIT_COOKIE_SECURE") {
            c.session.secure = match v.to_ascii_lowercase().as_str() {
                "auto" => CookieSecure::Auto,
                "always" | "true" => CookieSecure::Always,
                "never" | "false" => CookieSecure::Never,
                _ => {
                    return Err(ConfigError {
                        var: "HOMEDIT_COOKIE_SECURE",
                        reason: "expected auto, always or never".into(),
                    });
                }
            };
        }
        if let Some(v) = get("HOMEDIT_SESSION_TTL") {
            c.session.ttl_secs = v.parse().map_err(|_| ConfigError {
                var: "HOMEDIT_SESSION_TTL",
                reason: "expected seconds".into(),
            })?;
        }
        if let Some(v) = get("HOMEDIT_TRUSTED_PROXIES") {
            c.trusted_proxies = parse_nets(&v)?;
        }
        if let Some(v) = get("HOMEDIT_ALLOWED_ORIGINS") {
            c.allowed_origins = split_list(&v);
        }

        if get("HOMEDIT_PROXY_AUTH")
            .map(|v| parse_bool("HOMEDIT_PROXY_AUTH", &v))
            .transpose()?
            == Some(true)
        {
            let mut p = ProxyConfig::default();
            if let Some(v) = get("HOMEDIT_PROXY_USER_HEADER") {
                p.user_header = v;
            }
            if let Some(v) = get("HOMEDIT_PROXY_GROUPS_HEADER") {
                p.groups_header = v;
            }
            p.admin_group = get("HOMEDIT_PROXY_ADMIN_GROUP");
            p.admin_users = get("HOMEDIT_PROXY_ADMIN_USERS")
                .map(|v| split_list(&v))
                .unwrap_or_default();
            p.logout_url = get("HOMEDIT_PROXY_LOGOUT_URL");
            c.proxy = Some(p);
        }

        if let Some(issuer_url) = get("HOMEDIT_OIDC_ISSUER") {
            let need = |k: &'static str| {
                get(k).ok_or(ConfigError {
                    var: k,
                    reason: "required when HOMEDIT_OIDC_ISSUER is set".into(),
                })
            };
            c.oidc = Some(OidcConfig {
                issuer_url,
                client_id: need("HOMEDIT_OIDC_CLIENT_ID")?,
                client_secret: get("HOMEDIT_OIDC_CLIENT_SECRET"),
                redirect_url: need("HOMEDIT_OIDC_REDIRECT_URL")?,
                scopes: get("HOMEDIT_OIDC_SCOPES")
                    .map(|v| split_list(&v))
                    .unwrap_or_else(default_scopes),
                groups_claim: get("HOMEDIT_OIDC_GROUPS_CLAIM").unwrap_or_else(default_groups_claim),
                admin_group: get("HOMEDIT_OIDC_ADMIN_GROUP"),
                allowed_group: get("HOMEDIT_OIDC_ALLOWED_GROUP"),
                default_role: match get("HOMEDIT_OIDC_DEFAULT_ROLE") {
                    Some(v) => v.parse().map_err(|reason| ConfigError {
                        var: "HOMEDIT_OIDC_DEFAULT_ROLE",
                        reason,
                    })?,
                    None => Role::Viewer,
                },
                button_label: get("HOMEDIT_OIDC_BUTTON_LABEL").unwrap_or_else(default_oidc_label),
            });
        }
        Ok(c)
    }
}

fn parse_bool(var: &'static str, v: &str) -> Result<bool, ConfigError> {
    match v.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(ConfigError {
            var,
            reason: "expected true or false".into(),
        }),
    }
}

fn split_list(v: &str) -> Vec<String> {
    v.split([',', ' '])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

fn parse_nets(v: &str) -> Result<Vec<IpNet>, ConfigError> {
    split_list(v)
        .into_iter()
        .map(|s| {
            s.parse::<IpNet>()
                .or_else(|_| s.parse::<std::net::IpAddr>().map(IpNet::from))
                .map_err(|_| ConfigError {
                    var: "HOMEDIT_TRUSTED_PROXIES",
                    reason: format!("`{s}` is not an IP or CIDR"),
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| m.get(k).cloned()
    }

    #[test]
    fn defaults_disable_auth() {
        let c = AuthConfig::from_lookup(lookup(&[])).unwrap();
        assert!(!c.enabled);
        assert!(c.oidc.is_none() && c.proxy.is_none());
    }

    #[test]
    fn env_parses_everything() {
        let c = AuthConfig::from_lookup(lookup(&[
            ("HOMEDIT_AUTH_ENABLED", "true"),
            ("HOMEDIT_AUTH_ANONYMOUS_ROLE", "viewer"),
            ("HOMEDIT_TRUSTED_PROXIES", "10.0.0.0/8, 172.17.0.1"),
            ("HOMEDIT_PROXY_AUTH", "yes"),
            ("HOMEDIT_PROXY_ADMIN_GROUP", "admins"),
            ("HOMEDIT_OIDC_ISSUER", "https://id.example.com"),
            ("HOMEDIT_OIDC_CLIENT_ID", "homedit"),
            (
                "HOMEDIT_OIDC_REDIRECT_URL",
                "https://dash.example.com/api/auth/oidc/callback",
            ),
        ]))
        .unwrap();
        assert!(c.enabled);
        assert_eq!(c.anonymous_role, Some(Role::Viewer));
        assert_eq!(c.trusted_proxies.len(), 2);
        assert_eq!(c.proxy.unwrap().admin_group.as_deref(), Some("admins"));
        assert_eq!(c.oidc.unwrap().groups_claim, "groups");
    }

    #[test]
    fn oidc_requires_client_id() {
        let e =
            AuthConfig::from_lookup(lookup(&[("HOMEDIT_OIDC_ISSUER", "https://id")])).unwrap_err();
        assert_eq!(e.var, "HOMEDIT_OIDC_CLIENT_ID");
    }

    #[test]
    fn yaml_round_trip() {
        let y = "enabled: true\nanonymous_role: viewer\nproxy:\n  admin_group: admins\n";
        let c: AuthConfig = serde_json::from_value(serde_json::json!({
            "enabled": true, "anonymous_role": "viewer", "proxy": {"admin_group": "admins"}
        }))
        .unwrap();
        assert!(c.enabled && c.proxy.is_some(), "{y}");
    }
}
