//! The Krifmaz kit's server crates, wired to homedit.
//!
//! | Route | From |
//! | --- | --- |
//! | `GET /api/health` | kit-server-health: version and the web app's build |
//! | `/api/changelog`, `/api/changelog/new`, `/api/changelog/seen` | kit-server-changelog: What's new |
//! | `GET/PUT /api/auth/prefs` | kit-server-changelog: the account's preferences (the What's new switch) |
//! | `/api/notices`, `/api/notices/check`, `/api/notices/{id}`, `/api/notices/{id}/check` | kit-server-notices |
//! | `/api/bug-reports/*`, `/api/crash-reports` | kit-server-bugreports |
//! | `/api/diagnostics`, `/api/diagnostics/{id}/test` | kit-server-diagnostics |
//! | `/api/settings`, `/api/settings/schema`, `/api/settings/secret/{key}` | kit-server-settings |
//!
//! The kit keeps its tables (`kit_*`) in `kit.db` in the data folder, next to `homedit.db`, so its
//! migrations never meet homedit's. Who is asking comes from homedit's auth middleware ([`CurrentUser`]),
//! which also checks CSRF on every change, so the kit's routes follow the same sign-in and CSRF rules as
//! the rest of the API. The server settings ([`SETTINGS`], from `settings.toml`) are read at start through
//! [`lookup`], so an environment variable still wins over a saved value.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::http::request::Parts;
use homedit_auth::AuthConfig;
use homedit_auth::extract::CurrentUser;
use kit_server_changelog::{Changelog, ChangelogHost};
use kit_server_core::{Account, Host, PrefsStore, SqliteDb, SqlitePrefs, err};
use kit_server_diagnostics::{Check, Kind, Registry};
use kit_server_notices::{Notice, NoticeCheck, Notices, Severity, Toasts};
use kit_server_settings::{
    DefaultValue, Field, Group, KeyDef, Section, SettingsHost, SqliteSettings, Store, Table, Values,
};
use serde_json::{Map, Value, json};

use crate::AppState;

/// The settings table generated from `settings.toml` by build.rs: `Keys`, `TABLE` and `SCHEMA`.
#[allow(missing_docs, dead_code)]
mod settings_gen {
    use super::{DefaultValue, Field, Group, KeyDef, Section};
    include!(concat!(env!("OUT_DIR"), "/settings_gen.rs"));
}

/// The release notes in `docs/releases`, built in by build.rs: `NOTES`, (version, Markdown) pairs.
mod changelog_gen {
    include!(concat!(env!("OUT_DIR"), "/changelog_gen.rs"));
}

/// The server settings (`crates/server/settings.toml`).
pub static SETTINGS: Table = Table {
    keys: settings_gen::TABLE,
    sections: settings_gen::SCHEMA,
};

/// The name people see in What's new titles and bug report exports.
pub const APP_NAME: &str = "homedit";

/// The server's version (Cargo.toml's, which the release workflow sets).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The id of the data folder notice.
pub const DATA_NOTICE: &str = "data";
/// The id of the Docker notice.
pub const DOCKER_NOTICE: &str = "docker";

/// What the kit keeps for homedit, shared by every clone of [`AppState`].
pub struct Kit {
    /// `kit.db`: the kit's own tables.
    pub db: SqliteDb,
    /// Each account's preferences (`kit_account_prefs`): What's new's seen version and switch.
    pub prefs: SqlitePrefs,
    /// Bug reports and crash logs (`kit_bug_report*`, `kit_client_crashes`).
    pub reports: kit_server_bugreports::SqliteReports,
    /// The server settings in effect: environment, then saved (`kit_settings`), then the defaults.
    pub settings: Store,
    /// What's new: the release notes built into this server.
    pub changelog: Changelog,
    /// The notices (the data folder, Docker), each keeping its last probe.
    pub notices: Notices<AppState>,
    /// Bug report limits per account.
    pub limits: Limits,
}

impl Kit {
    /// The kit's state on `kit.db` in `data_dir` (made when missing), with the environment variables `env`
    /// over the saved settings.
    pub async fn open(
        data_dir: &Path,
        env: impl Fn(&str) -> Option<String> + Send + Sync + 'static,
    ) -> anyhow::Result<Kit> {
        let db = SqliteDb::open(&data_dir.join("kit.db"))
            .map_err(|e| anyhow::anyhow!("cannot open kit.db: {}", e.detail))?;
        Kit::on(db, env).await
    }

    /// The kit's state on a database in memory and no environment (tests).
    pub async fn memory() -> anyhow::Result<Kit> {
        Kit::on(SqliteDb::memory(), |_| None).await
    }

    async fn on(
        db: SqliteDb,
        env: impl Fn(&str) -> Option<String> + Send + Sync + 'static,
    ) -> anyhow::Result<Kit> {
        let kit = |e: kit_server_core::Error| {
            anyhow::anyhow!("cannot create the kit tables: {}", e.detail)
        };
        let prefs = SqlitePrefs::new(db.clone()).map_err(kit)?;
        let reports = kit_server_bugreports::SqliteReports::new(db.clone()).map_err(kit)?;
        let saved = SqliteSettings::new(db.clone()).map_err(kit)?;
        let kept = kit_server_notices::SqliteNotices::new(db.clone()).map_err(kit)?;
        let notices = notices().keep_in(Arc::new(kept));
        notices.restore().await.map_err(kit)?;
        let settings = Store::load_with(saved, SETTINGS, env)
            .await
            .map_err(|e| anyhow::anyhow!("cannot read the settings: {}", e.detail))?;
        Ok(Kit {
            db,
            prefs,
            reports,
            settings,
            changelog: Changelog::new(APP_NAME, VERSION, changelog_gen::NOTES),
            notices,
            limits: Limits::default(),
        })
    }

    /// The sign-in settings in effect. A saved combination that does not parse (which a save refuses, so
    /// only a database edited by hand) is logged and the environment alone is used, so a bad save never
    /// keeps the server from starting.
    pub fn auth_config(&self) -> anyhow::Result<AuthConfig> {
        match AuthConfig::from_lookup(lookup(&self.settings)) {
            Ok(c) => Ok(c),
            Err(e) => {
                tracing::error!(error = %e, "the saved sign-in settings are invalid; using the environment only");
                Ok(AuthConfig::from_env()?)
            }
        }
    }
}

/// Reads a `HOMEDIT_*` variable through the settings: the setting whose `env` it is (environment, then
/// saved, then default) as text, `None` when empty; any variable the table doesn't have from the
/// environment.
pub fn lookup(store: &Store) -> impl Fn(&str) -> Option<String> + '_ {
    move |name| match SETTINGS.keys.iter().find(|k| k.env == Some(name)) {
        Some(def) => text(&store.get(def.key)),
        None => std::env::var(name).ok(),
    }
}

/// Like [`lookup`], with `changes` (a save about to happen) over the store.
fn lookup_with<'a>(
    store: &'a Store,
    changes: &'a Map<String, Value>,
) -> impl Fn(&str) -> Option<String> + 'a {
    move |name| match SETTINGS.keys.iter().find(|k| k.env == Some(name)) {
        Some(def) => match changes.get(def.key) {
            Some(v) => text(v),
            None => text(&store.get(def.key)),
        },
        None => std::env::var(name).ok(),
    }
}

/// A setting's value as the text an environment variable would hold; `None` for empty or null.
fn text(v: &Value) -> Option<String> {
    let s = match v {
        Value::Null => return None,
        Value::String(s) => s.trim().to_owned(),
        other => other.to_string(),
    };
    (!s.is_empty()).then_some(s)
}

/// A fixed-window limiter: at most `limit` hits per key within `window_secs`.
#[derive(Default)]
pub struct Limits {
    hits: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl Limits {
    /// Whether `key` is over `limit` hits in the last `window_secs`; when not and `record`, counts this one.
    pub fn limited(&self, key: &str, limit: usize, window_secs: f64, record: bool) -> bool {
        let now = Instant::now();
        let window = Duration::from_secs_f64(window_secs.max(0.0));
        let mut all = self.hits.lock().unwrap_or_else(|p| p.into_inner());
        let q = all.entry(key.to_owned()).or_default();
        while q.front().is_some_and(|t| now.duration_since(*t) > window) {
            q.pop_front();
        }
        if q.len() >= limit {
            return true;
        }
        if record {
            q.push_back(now);
        }
        false
    }
}

/// The account the kit sees for a caller: signed-in users by their id; with sign-in off (or visitors made
/// admins), the one admin (id 0); reverse-proxy users, who have no row, by a stable id from their name (negative, so it never
/// meets a row id). Admins are owners too: homedit has no role above admin.
pub fn account(user: &CurrentUser) -> Option<Account> {
    let admin = user.is_admin();
    match &user.user {
        Some(u) => Some(Account {
            id: u.id.unwrap_or_else(|| name_id(&u.username)),
            username: u.username.clone(),
            admin,
            owner: admin,
        }),
        None if !user.auth_enabled || admin => Some(Account {
            id: 0,
            username: "admin".into(),
            admin: true,
            owner: true,
        }),
        None => None,
    }
}

/// A stable negative id for a user without a row (FNV-1a of the name).
fn name_id(name: &str) -> i64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in name.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    -((h >> 1) as i64) - 1
}

impl Host for AppState {
    async fn signed_in(&self, parts: &mut Parts) -> kit_server_core::Result<Account> {
        let user = parts
            .extensions
            .get::<CurrentUser>()
            .ok_or_else(|| err(500, "The auth middleware is not installed"))?;
        if user.role.is_none() {
            return Err(err(401, "Not authenticated"));
        }
        account(user).ok_or_else(|| err(401, "Not authenticated"))
    }
}

impl ChangelogHost for AppState {
    fn changelog(&self) -> &Changelog {
        &self.kit.changelog
    }

    fn prefs(&self) -> &dyn PrefsStore {
        &self.kit.prefs
    }
}

impl SettingsHost for AppState {
    fn settings(&self) -> &dyn Values {
        &self.kit.settings
    }

    fn before_save(&self, values: &Map<String, Value>) -> kit_server_core::Result<Vec<Value>> {
        let get = lookup_with(&self.kit.settings, values);
        if let Err(e) = AuthConfig::from_lookup(&get) {
            let key = SETTINGS
                .keys
                .iter()
                .find(|k| k.env == Some(e.var))
                .and_then(|k| k.label)
                .unwrap_or(e.var);
            return Err(err(400, format!("{key}: {}", e.reason)));
        }
        if let Some(v) = get("HOMEDIT_DOCKER_HOST")
            && crate::discovery::env_host(&v).is_none()
        {
            return Err(err(
                400,
                "Docker address: use unix:///path/to/docker.sock or tcp://host:port",
            ));
        }
        Ok(vec![
            json!({ "key": "", "message": "Restart homedit for the new settings to take effect" }),
        ])
    }

    fn log(&self, message: &str) {
        tracing::info!(target: "homedit::settings", "{message}");
    }
}

impl kit_server_bugreports::BugReportsHost for AppState {
    fn reports(&self) -> &dyn kit_server_bugreports::BugReportStore {
        &self.kit.reports
    }

    fn data_dir(&self) -> PathBuf {
        self.data_dir.clone()
    }

    fn app_name(&self) -> String {
        APP_NAME.into()
    }

    fn app_version(&self) -> String {
        VERSION.into()
    }

    fn limited(&self, key: &str, limit: usize, window_secs: f64) -> bool {
        self.kit.limits.limited(key, limit, window_secs, true)
    }

    fn log(&self, level: &str, message: &str) {
        match level {
            "error" => tracing::error!(target: "homedit::bug_reports", "{message}"),
            "warning" | "warn" => tracing::warn!(target: "homedit::bug_reports", "{message}"),
            _ => tracing::info!(target: "homedit::bug_reports", "{message}"),
        }
    }
}

impl kit_server_bugreports::crash::CrashHost for AppState {
    fn crashes(&self) -> &dyn kit_server_bugreports::CrashStore {
        &self.kit.reports
    }
}

impl kit_server_notices::NoticesHost for AppState {
    fn notices(&self) -> Notices<AppState> {
        self.kit.notices.clone()
    }
}

impl kit_server_diagnostics::Diagnostics for AppState {
    fn diagnostics(&self) -> Registry<AppState> {
        diagnostics()
    }
}

/// Whether a file can be made in `dir`: `Ok`, or the reason in words.
pub fn writable(dir: &Path) -> Result<(), String> {
    let probe = dir.join(format!(".write-test-{}", std::process::id()));
    match std::fs::write(&probe, b"ok") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err("missing".into()),
        Err(e) if e.raw_os_error() == Some(30) => Err("read-only".into()),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            Err("not writable by the server's user (uid 65532 in the image)".into())
        }
        Err(e) => Err(e.to_string()),
    }
}

/// The Docker hosts discovery reads: the dashboard's (`docker.yaml` servers) and the Docker address
/// setting (`local`).
async fn docker_hosts(st: &AppState) -> Vec<homedit_core::DockerHost> {
    let mut hosts = st
        .db
        .load_dashboard()
        .await
        .map(|d| d.docker_hosts)
        .unwrap_or_default();
    if let Some(h) = crate::discovery::configured_host(st)
        && !hosts.iter().any(|x| x.name == h.name)
    {
        hosts.push(h);
    }
    hosts
}

/// Every Docker host's answer to a ping: (name, address, error).
async fn ping_docker(st: &AppState) -> Vec<(String, String, Option<String>)> {
    let mut out = Vec::new();
    for h in docker_hosts(st).await {
        let Some(ep) = crate::discovery::endpoint(&h) else {
            continue;
        };
        let addr = crate::discovery::describe(&ep);
        let client = homedit_discovery::DockerClient::new(ep).with_timeout(Duration::from_secs(5));
        let e = client.ping().await.err().map(|e| e.to_string());
        out.push((h.name, addr, e));
    }
    out
}

/// homedit's notices: the data folder can't be written; a Docker host doesn't answer. Both are probed
/// every minute and on Check again.
pub fn notices() -> Notices<AppState> {
    Notices::new()
        .register(
            NoticeCheck::kept(DATA_NOTICE, Severity::Err, |st: AppState| async move {
                let dir = st.data_dir.clone();
                match tokio::task::spawn_blocking(move || writable(&dir).map_err(|why| (dir, why))).await {
                    Ok(Ok(())) => Notice::fine(),
                    Ok(Err((dir, why))) => Notice::failing(format!(
                        "The data folder {} is {why}, so changes can't be saved. Check the disk and the volume mount.",
                        dir.display()
                    )),
                    Err(e) => Notice::failing(e.to_string()),
                }
            })
            .every(Duration::from_secs(60))
            .toasts(Toasts::new(
                "The data folder still can't be written",
                "Still checking the data folder",
                "The data folder can be written again",
            )),
        )
        .register(
            NoticeCheck::kept(DOCKER_NOTICE, Severity::Warn, |st: AppState| async move {
                let failing: Vec<String> = ping_docker(&st)
                    .await
                    .into_iter()
                    .filter_map(|(name, addr, e)| e.map(|e| format!("{name} ({addr}): {e}")))
                    .collect();
                if failing.is_empty() {
                    Notice::fine()
                } else {
                    Notice::failing(format!(
                        "Docker isn't answering, so new containers aren't added: {}. Check the socket mount or the socket proxy.",
                        failing.join("; ")
                    ))
                }
            })
            .every(Duration::from_secs(60))
            .toasts(Toasts::new(
                "Docker still isn't answering",
                "Still checking Docker",
                "Docker answers again",
            )),
        )
}

/// The Diagnostics page: data folder, databases, secret key, sign-in, Docker; and the Test buttons of the
/// single sign-on and Docker cards.
pub fn diagnostics() -> Registry<AppState> {
    Registry::new()
        .check("data", |st: AppState| async move {
            let dir = st.data_dir.clone();
            let shown = dir.display().to_string();
            match tokio::task::spawn_blocking(move || writable(&dir))
                .await
                .unwrap_or_else(|e| Err(e.to_string()))
            {
                Ok(()) => Check::new("Data folder", Kind::Path, true, format!("{shown} · writable")).request(shown),
                Err(why) => Check::new("Data folder", Kind::Path, false, format!("{shown} · {why}"))
                    .request(shown)
                    .fix("Check the disk and the volume mount, and that the server's user may write to the folder"),
            }
        })
        .check("database", |st: AppState| async move {
            match sqlx::query("SELECT COUNT(*) FROM groups").fetch_one(st.db.pool()).await {
                Ok(_) => Check::new("Database", Kind::Path, true, "homedit.db · OK"),
                Err(e) => Check::new("Database", Kind::Path, false, format!("homedit.db · {e}"))
                    .fix("Check the data folder; restore homedit.db from a backup if it is damaged"),
            }
        })
        .check("kit-database", |st: AppState| async move {
            use kit_server_core::Db;
            match st.kit.db.one("SELECT COUNT(*) AS n FROM kit_migrations", vec![]) {
                Ok(_) => Check::new("Settings and reports database", Kind::Path, true, "kit.db · OK"),
                Err(e) => Check::new("Settings and reports database", Kind::Path, false, format!("kit.db · {}", e.detail))
                    .fix("Check the data folder; restore kit.db from a backup if it is damaged"),
            }
        })
        .check("secret-key", |st: AppState| async move {
            let from_env = std::env::var_os("HOMEDIT_SECRET_KEY").is_some()
                || std::env::var_os("HOMEDIT_SECRET_KEY_FILE").is_some();
            if from_env {
                Check::new("Secret key", Kind::Path, true, "From HOMEDIT_SECRET_KEY")
            } else {
                let file = st.data_dir.join("secret.key");
                Check::new("Secret key", Kind::Path, true, format!("{} · kept with the data", file.display()))
                    .request(file.display().to_string())
                    .status(kit_server_diagnostics::Status::Warn)
                    .fix("Back up secret.key with the data, or set HOMEDIT_SECRET_KEY: stored API keys can't be read without it")
            }
        })
        .check("signin", |st: AppState| async move {
            let c = st.auth.config();
            let mut ways = Vec::new();
            if c.local.enabled {
                ways.push("passwords");
            }
            if c.oidc.is_some() {
                ways.push("single sign-on");
            }
            if c.proxy.is_some() {
                ways.push("reverse proxy");
            }
            if !c.enabled {
                Check::new("Sign-in", Kind::Service, true, "Off: every visitor can edit the dashboard")
                    .status(kit_server_diagnostics::Status::Warn)
                    .fix("Turn on Require sign-in in Settings › Sign-in if others can reach this server")
            } else if ways.is_empty() {
                Check::new("Sign-in", Kind::Service, false, "On, but no way to sign in is turned on")
                    .fix("Turn on passwords, single sign-on or reverse proxy sign-in in Settings › Sign-in")
            } else {
                Check::new("Sign-in", Kind::Service, true, format!("On: {}", ways.join(", ")))
            }
        })
        .group("docker", |st: AppState| async move {
            Ok(ping_docker(&st)
                .await
                .into_iter()
                .map(|(name, addr, e)| match e {
                    None => Check::new(format!("Docker {name}"), Kind::Service, true, "Answers").request(addr),
                    Some(e) => Check::new(format!("Docker {name}"), Kind::Service, false, e)
                        .request(addr)
                        .fix("Check that the socket is mounted (read-only is enough) or that the socket proxy runs and allows GET /containers"),
                })
                .collect())
        })
        .test("docker", |st: AppState, _who: Account| async move {
            let all = ping_docker(&st).await;
            if all.is_empty() {
                return Ok(json!({ "ok": false, "message": "No Docker address is set" }));
            }
            let bad: Vec<String> = all
                .iter()
                .filter_map(|(n, a, e)| e.as_ref().map(|e| format!("{n} ({a}): {e}")))
                .collect();
            Ok(if bad.is_empty() {
                json!({ "ok": true, "message": format!("Docker answers ({} host{})", all.len(), if all.len() == 1 { "" } else { "s" }) })
            } else {
                json!({ "ok": false, "message": bad.join("; ") })
            })
        })
        .test("oidc", |st: AppState, _who: Account| async move {
            let issuer = st.kit.settings.str("oidc_issuer");
            if issuer.trim().is_empty() {
                return Ok(json!({ "ok": false, "message": "No issuer URL is saved" }));
            }
            let url = format!("{}/.well-known/openid-configuration", issuer.trim().trim_end_matches('/'));
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .map_err(|e| e.to_string())?;
            match client.get(&url).send().await {
                Ok(r) if r.status().is_success() => match r.json::<Value>().await {
                    Ok(v) if v.get("authorization_endpoint").is_some() => {
                        Ok(json!({ "ok": true, "message": format!("The provider answers at {url}") }))
                    }
                    _ => Ok(json!({ "ok": false, "message": format!("{url} is not an OpenID Connect configuration") })),
                },
                Ok(r) => Ok(json!({ "ok": false, "message": format!("{url} answered {}", r.status()) })),
                Err(e) => Ok(json!({ "ok": false, "message": format!("{url}: {e}") })),
            }
        })
}

/// The build of the web app built into this binary: the hash in its script's name
/// (`homedit-web-<hash>.js`), which `/api/health` reports so open tabs reload into a new build.
pub fn web_build() -> Option<String> {
    crate::assets::file_names().find_map(|name| {
        let hash = name.strip_prefix("homedit-web-")?.strip_suffix(".js")?;
        (hash.len() >= 12 && hash.bytes().all(|b| b.is_ascii_hexdigit())).then(|| hash.to_owned())
    })
}

/// The kit's routes. Merge them inside the auth middleware layer, so every route sees the caller; start
/// [`kit_server_notices::spawn_rechecks`] once the state exists.
pub fn router() -> Router<AppState> {
    Router::new()
        .merge(kit_server_health::router(kit_server_health::Health::new(
            VERSION,
            web_build(),
        )))
        .merge(kit_server_changelog::router())
        .merge(kit_server_changelog::prefs_router("/api/auth/prefs"))
        .merge(kit_server_notices::router())
        .merge(kit_server_bugreports::router())
        .merge(kit_server_bugreports::crash::router())
        .merge(kit_server_diagnostics::router())
        .merge(kit_server_settings::router())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_users_get_stable_negative_ids() {
        assert_eq!(name_id("ana"), name_id("ana"));
        assert_ne!(name_id("ana"), name_id("bo"));
        assert!(name_id("ana") < 0 && name_id("") < 0);
    }

    #[test]
    fn limits_count_per_key() {
        let l = Limits::default();
        assert!(!l.limited("a", 2, 60.0, true));
        assert!(!l.limited("a", 2, 60.0, true));
        assert!(l.limited("a", 2, 60.0, true));
        assert!(!l.limited("b", 2, 60.0, true));
    }

    #[tokio::test]
    async fn defaults_read_like_no_environment() {
        let kit = Kit::memory().await.unwrap();
        let from_settings = AuthConfig::from_lookup(lookup(&kit.settings)).unwrap();
        let from_nothing = AuthConfig::from_lookup(|_| None).unwrap();
        assert_eq!(format!("{from_settings:?}"), format!("{from_nothing:?}"));
    }

    #[test]
    fn values_read_as_environment_text() {
        assert_eq!(text(&json!(true)), Some("true".into()));
        assert_eq!(text(&json!(64)), Some("64".into()));
        assert_eq!(text(&json!("  ")), None);
        assert_eq!(text(&Value::Null), None);
    }
}
