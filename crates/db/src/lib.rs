//! SQLite storage for homedit.
//!
//! [`Db`] wraps a connection pool and owns the core schema
//! (`migrations/0001_core.sql`): the dashboard tables, status samples and
//! the version history. Other crates add their own tables with migrations
//! in their numbering range (see the header of `0001_core.sql`) and can use
//! [`Db::pool`] directly.
//!
//! The dashboard is saved as a whole with [`Db::save_dashboard`]. Group and
//! service ids are kept across saves so that status history stays attached
//! to the right card. Every save also writes a full snapshot to
//! `config_versions`, which is the basis for undo and one-click restore.

use std::collections::HashSet;
use std::path::Path;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use homedit_core::{
    Bookmark, BookmarkGroup, Dashboard, DockerHost, Group, InfoWidget, Service, Settings,
    StatusSample,
};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Row, SqliteConnection, SqlitePool};

/// Errors from the storage layer.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// The database driver failed.
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),
    /// A migration failed.
    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    /// A stored JSON column did not parse.
    #[error("stored JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    /// The caller's version did not match the current one.
    #[error("version conflict: expected {expected}, current is {current}")]
    Conflict {
        /// Version the caller based its edit on.
        expected: i64,
        /// Version currently stored.
        current: i64,
    },
    /// A requested row does not exist.
    #[error("not found")]
    NotFound,
}

/// Shorthand result type.
pub type Result<T, E = DbError> = std::result::Result<T, E>;

/// One entry of the version history.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionInfo {
    /// Version number; increases by one on every save.
    pub version: i64,
    /// Milliseconds since the Unix epoch.
    pub created_at: i64,
    /// Who saved it, when known.
    pub author: Option<String>,
    /// Short description of the change.
    pub summary: Option<String>,
}

/// Handle to the database. Cheap to clone.
#[derive(Debug, Clone)]
pub struct Db {
    pool: SqlitePool,
}

/// Current time in milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

impl Db {
    /// Opens (creating if needed) the database file at `path` and runs all
    /// pending migrations.
    pub async fn open(path: &Path) -> Result<Self> {
        let opts = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true);
        Self::connect(opts, 8).await
    }

    /// Opens a private in-memory database, for tests.
    pub async fn open_in_memory() -> Result<Self> {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")?.foreign_keys(true);
        // One connection: every in-memory connection is its own database.
        Self::connect(opts, 1).await
    }

    async fn connect(opts: SqliteConnectOptions, max: u32) -> Result<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(max)
            .connect_with(opts)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    /// The underlying pool, for crates that own their own tables.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// The current dashboard version, or 0 when nothing was saved yet.
    pub async fn current_version(&self) -> Result<i64> {
        let mut conn = self.pool.acquire().await?;
        current_version(&mut conn).await
    }

    /// Loads the whole dashboard. An empty database gives
    /// [`Dashboard::default`].
    pub async fn load_dashboard(&self) -> Result<Dashboard> {
        let mut conn = self.pool.acquire().await?;
        load(&mut conn).await
    }

    /// Loads the dashboard together with its version, consistently.
    pub async fn load_versioned(&self) -> Result<(i64, Dashboard)> {
        let mut tx = self.pool.begin().await?;
        let v = current_version(&mut tx).await?;
        let d = load(&mut tx).await?;
        tx.commit().await?;
        Ok((v, d))
    }

    /// Replaces the stored dashboard with `dash` and records a new version.
    ///
    /// When `expected_version` is given and does not match the current
    /// version, nothing is written and [`DbError::Conflict`] is returned.
    /// Returns the new version and the dashboard as stored (with ids filled
    /// in for new rows).
    pub async fn save_dashboard(
        &self,
        dash: &Dashboard,
        expected_version: Option<i64>,
        author: Option<&str>,
        summary: Option<&str>,
    ) -> Result<(i64, Dashboard)> {
        let mut tx = self.pool.begin().await?;
        let current = current_version(&mut tx).await?;
        if let Some(expected) = expected_version
            && expected != current
        {
            return Err(DbError::Conflict { expected, current });
        }
        write(&mut tx, dash).await?;
        let stored = load(&mut tx).await?;
        let version = current + 1;
        sqlx::query(
            "INSERT INTO config_versions (version, created_at, author, summary, snapshot)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(version)
        .bind(now_ms())
        .bind(author)
        .bind(summary)
        .bind(serde_json::to_string(&stored)?)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok((version, stored))
    }

    /// Lists saved versions, newest first.
    pub async fn history(&self, limit: i64) -> Result<Vec<VersionInfo>> {
        let rows = sqlx::query(
            "SELECT version, created_at, author, summary FROM config_versions
             ORDER BY version DESC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| VersionInfo {
                version: r.get(0),
                created_at: r.get(1),
                author: r.get(2),
                summary: r.get(3),
            })
            .collect())
    }

    /// Loads the snapshot saved as `version`.
    pub async fn version_snapshot(&self, version: i64) -> Result<Dashboard> {
        let row = sqlx::query("SELECT snapshot FROM config_versions WHERE version = ?")
            .bind(version)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(DbError::NotFound)?;
        Ok(serde_json::from_str(row.get::<&str, _>(0))?)
    }

    /// Stores one status sample. Samples for services that no longer exist
    /// are dropped silently.
    pub async fn insert_sample(&self, s: &StatusSample) -> Result<()> {
        sqlx::query(
            "INSERT INTO status_samples (service_id, checked_at, ok, status_code, latency_ms, error)
             SELECT ?, ?, ?, ?, ?, ? WHERE EXISTS (SELECT 1 FROM services WHERE id = ?)",
        )
        .bind(s.service_id)
        .bind(s.checked_at)
        .bind(s.ok)
        .bind(s.status_code.map(i64::from))
        .bind(s.latency_ms.map(i64::from))
        .bind(&s.error)
        .bind(s.service_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Returns the newest `per_service` samples of every service, oldest
    /// first within each service.
    pub async fn recent_samples(&self, per_service: i64) -> Result<Vec<StatusSample>> {
        let rows = sqlx::query(
            "SELECT service_id, checked_at, ok, status_code, latency_ms, error FROM (
                SELECT *, ROW_NUMBER() OVER (PARTITION BY service_id ORDER BY checked_at DESC) AS rn
                FROM status_samples)
             WHERE rn <= ? ORDER BY service_id, checked_at",
        )
        .bind(per_service)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| StatusSample {
                service_id: r.get(0),
                checked_at: r.get(1),
                ok: r.get(2),
                status_code: r
                    .get::<Option<i64>, _>(3)
                    .and_then(|v| u16::try_from(v).ok()),
                latency_ms: r
                    .get::<Option<i64>, _>(4)
                    .and_then(|v| u32::try_from(v).ok()),
                error: r.get(5),
            })
            .collect())
    }

    /// Deletes samples older than `cutoff_ms`. Returns how many were removed.
    pub async fn prune_samples(&self, cutoff_ms: i64) -> Result<u64> {
        let r = sqlx::query("DELETE FROM status_samples WHERE checked_at < ?")
            .bind(cutoff_ms)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected())
    }
}

async fn current_version(conn: &mut SqliteConnection) -> Result<i64> {
    let v: Option<i64> = sqlx::query_scalar("SELECT MAX(version) FROM config_versions")
        .fetch_one(&mut *conn)
        .await?;
    Ok(v.unwrap_or(0))
}

fn json_opt(v: &Option<serde_json::Value>) -> Result<Option<String>> {
    Ok(v.as_ref().map(serde_json::to_string).transpose()?)
}

async fn group_ids(conn: &mut SqliteConnection) -> Result<HashSet<i64>> {
    let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM groups")
        .fetch_all(&mut *conn)
        .await?;
    Ok(ids.into_iter().collect())
}

async fn service_ids(conn: &mut SqliteConnection) -> Result<HashSet<i64>> {
    let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM services")
        .fetch_all(&mut *conn)
        .await?;
    Ok(ids.into_iter().collect())
}

/// Writes `dash` over the current contents. Must run inside a transaction.
async fn write(conn: &mut SqliteConnection, dash: &Dashboard) -> Result<()> {
    sqlx::query(
        "INSERT INTO settings (id, doc) VALUES (1, ?)
         ON CONFLICT(id) DO UPDATE SET doc = excluded.doc",
    )
    .bind(serde_json::to_string(&dash.settings)?)
    .execute(&mut *conn)
    .await?;

    // Groups and services are upserted so their ids survive.
    let old_groups = group_ids(conn).await?;
    let old_services = service_ids(conn).await?;
    let mut kept_groups = HashSet::new();
    let mut kept_services = HashSet::new();
    let mut stack: Vec<(Option<i64>, usize, &Group)> = dash
        .groups
        .iter()
        .enumerate()
        .rev()
        .map(|(i, g)| (None, i, g))
        .collect();
    while let Some((parent, pos, g)) = stack.pop() {
        let reuse =
            g.id.filter(|id| old_groups.contains(id) && !kept_groups.contains(id));
        let layout = serde_json::to_string(&g.layout)?;
        let gid = if let Some(id) = reuse {
            sqlx::query(
                "UPDATE groups SET parent_id = ?, position = ?, name = ?, layout = ?,
                 hidden_from_viewers = ? WHERE id = ?",
            )
            .bind(parent)
            .bind(pos as i64)
            .bind(&g.name)
            .bind(&layout)
            .bind(g.hidden_from_viewers)
            .bind(id)
            .execute(&mut *conn)
            .await?;
            id
        } else {
            sqlx::query(
                "INSERT INTO groups (parent_id, position, name, layout, hidden_from_viewers)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(parent)
            .bind(pos as i64)
            .bind(&g.name)
            .bind(&layout)
            .bind(g.hidden_from_viewers)
            .execute(&mut *conn)
            .await?
            .last_insert_rowid()
        };
        kept_groups.insert(gid);
        for (spos, s) in g.services.iter().enumerate() {
            let sid =
                s.id.filter(|id| old_services.contains(id) && !kept_services.contains(id));
            let id = upsert_service(conn, gid, spos as i64, s, sid).await?;
            kept_services.insert(id);
        }
        stack.extend(
            g.groups
                .iter()
                .enumerate()
                .rev()
                .map(|(i, c)| (Some(gid), i, c)),
        );
    }
    for id in old_services.difference(&kept_services) {
        sqlx::query("DELETE FROM services WHERE id = ?")
            .bind(id)
            .execute(&mut *conn)
            .await?;
    }
    for id in old_groups.difference(&kept_groups) {
        sqlx::query("DELETE FROM groups WHERE id = ?")
            .bind(id)
            .execute(&mut *conn)
            .await?;
    }

    // Bookmarks, info widgets and Docker hosts carry no history: rewrite.
    sqlx::query("DELETE FROM bookmark_groups")
        .execute(&mut *conn)
        .await?;
    for (pos, bg) in dash.bookmark_groups.iter().enumerate() {
        let bgid =
            sqlx::query("INSERT INTO bookmark_groups (position, name, layout) VALUES (?, ?, ?)")
                .bind(pos as i64)
                .bind(&bg.name)
                .bind(serde_json::to_string(&bg.layout)?)
                .execute(&mut *conn)
                .await?
                .last_insert_rowid();
        for (bpos, b) in bg.bookmarks.iter().enumerate() {
            sqlx::query(
                "INSERT INTO bookmarks (group_id, position, name, href, abbr, icon, description)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(bgid)
            .bind(bpos as i64)
            .bind(&b.name)
            .bind(&b.href)
            .bind(&b.abbr)
            .bind(&b.icon)
            .bind(&b.description)
            .execute(&mut *conn)
            .await?;
        }
    }
    sqlx::query("DELETE FROM info_widgets")
        .execute(&mut *conn)
        .await?;
    for (pos, w) in dash.info_widgets.iter().enumerate() {
        sqlx::query("INSERT INTO info_widgets (position, kind, options) VALUES (?, ?, ?)")
            .bind(pos as i64)
            .bind(&w.kind)
            .bind(serde_json::to_string(&w.options)?)
            .execute(&mut *conn)
            .await?;
    }
    sqlx::query("DELETE FROM docker_hosts")
        .execute(&mut *conn)
        .await?;
    for h in &dash.docker_hosts {
        sqlx::query(
            "INSERT OR REPLACE INTO docker_hosts (name, socket, host, port, extra)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&h.name)
        .bind(&h.socket)
        .bind(&h.host)
        .bind(h.port.map(i64::from))
        .bind(serde_json::to_string(&h.extra)?)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

async fn upsert_service(
    conn: &mut SqliteConnection,
    group_id: i64,
    pos: i64,
    s: &Service,
    reuse: Option<i64>,
) -> Result<i64> {
    let widget = json_opt(&s.widget)?;
    let extra = serde_json::to_string(&s.extra)?;
    if let Some(id) = reuse {
        sqlx::query(
            "UPDATE services SET group_id = ?, position = ?, name = ?, href = ?, description = ?,
             icon = ?, ping = ?, site_monitor = ?, server = ?, container = ?, widget = ?, extra = ?
             WHERE id = ?",
        )
        .bind(group_id)
        .bind(pos)
        .bind(&s.name)
        .bind(&s.href)
        .bind(&s.description)
        .bind(&s.icon)
        .bind(&s.ping)
        .bind(&s.site_monitor)
        .bind(&s.server)
        .bind(&s.container)
        .bind(&widget)
        .bind(&extra)
        .bind(id)
        .execute(&mut *conn)
        .await?;
        Ok(id)
    } else {
        Ok(sqlx::query(
            "INSERT INTO services (group_id, position, name, href, description, icon, ping,
             site_monitor, server, container, widget, extra)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(group_id)
        .bind(pos)
        .bind(&s.name)
        .bind(&s.href)
        .bind(&s.description)
        .bind(&s.icon)
        .bind(&s.ping)
        .bind(&s.site_monitor)
        .bind(&s.server)
        .bind(&s.container)
        .bind(&widget)
        .bind(&extra)
        .execute(&mut *conn)
        .await?
        .last_insert_rowid())
    }
}

/// Reads the whole dashboard.
async fn load(conn: &mut SqliteConnection) -> Result<Dashboard> {
    let settings: Settings =
        match sqlx::query_scalar::<_, String>("SELECT doc FROM settings WHERE id = 1")
            .fetch_optional(&mut *conn)
            .await?
        {
            Some(doc) => serde_json::from_str(&doc)?,
            None => Settings::default(),
        };

    // Flat lists first, then assemble the tree.
    let group_rows = sqlx::query(
        "SELECT id, parent_id, name, layout, hidden_from_viewers FROM groups
         ORDER BY position, id",
    )
    .fetch_all(&mut *conn)
    .await?;
    let service_rows = sqlx::query(
        "SELECT id, group_id, name, href, description, icon, ping, site_monitor, server, container,
         widget, extra FROM services ORDER BY position, id",
    )
    .fetch_all(&mut *conn)
    .await?;

    let mut services: Vec<(i64, Service)> = Vec::with_capacity(service_rows.len());
    for r in service_rows {
        let widget: Option<String> = r.get(10);
        services.push((
            r.get(1),
            Service {
                id: Some(r.get(0)),
                name: r.get(2),
                href: r.get(3),
                description: r.get(4),
                icon: r.get(5),
                ping: r.get(6),
                site_monitor: r.get(7),
                server: r.get(8),
                container: r.get(9),
                widget: widget.map(|w| serde_json::from_str(&w)).transpose()?,
                extra: serde_json::from_str(r.get::<&str, _>(11))?,
            },
        ));
    }
    let mut flat: Vec<(Option<i64>, Group)> = Vec::with_capacity(group_rows.len());
    for r in group_rows {
        let id: i64 = r.get(0);
        flat.push((
            r.get(1),
            Group {
                id: Some(id),
                name: r.get(2),
                layout: serde_json::from_str(r.get::<&str, _>(3))?,
                hidden_from_viewers: r.get(4),
                services: services
                    .iter()
                    .filter(|(gid, _)| *gid == id)
                    .map(|(_, s)| s.clone())
                    .collect(),
                groups: Vec::new(),
            },
        ));
    }
    fn build(parent: Option<i64>, flat: &[(Option<i64>, Group)]) -> Vec<Group> {
        flat.iter()
            .filter(|(p, _)| *p == parent)
            .map(|(_, g)| Group {
                groups: build(g.id, flat),
                ..g.clone()
            })
            .collect()
    }
    let groups = build(None, &flat);

    let bg_rows = sqlx::query("SELECT id, name, layout FROM bookmark_groups ORDER BY position, id")
        .fetch_all(&mut *conn)
        .await?;
    let b_rows = sqlx::query(
        "SELECT id, group_id, name, href, abbr, icon, description FROM bookmarks
         ORDER BY position, id",
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut bookmark_groups = Vec::with_capacity(bg_rows.len());
    for r in bg_rows {
        let id: i64 = r.get(0);
        bookmark_groups.push(BookmarkGroup {
            id: Some(id),
            name: r.get(1),
            layout: serde_json::from_str(r.get::<&str, _>(2))?,
            bookmarks: b_rows
                .iter()
                .filter(|b| b.get::<i64, _>(1) == id)
                .map(|b| Bookmark {
                    id: Some(b.get(0)),
                    name: b.get(2),
                    href: b.get(3),
                    abbr: b.get(4),
                    icon: b.get(5),
                    description: b.get(6),
                })
                .collect(),
        });
    }

    let mut info_widgets = Vec::new();
    for r in sqlx::query("SELECT id, kind, options FROM info_widgets ORDER BY position, id")
        .fetch_all(&mut *conn)
        .await?
    {
        info_widgets.push(InfoWidget {
            id: Some(r.get(0)),
            kind: r.get(1),
            options: serde_json::from_str(r.get::<&str, _>(2))?,
        });
    }

    let mut docker_hosts = Vec::new();
    for r in sqlx::query("SELECT name, socket, host, port, extra FROM docker_hosts ORDER BY name")
        .fetch_all(&mut *conn)
        .await?
    {
        docker_hosts.push(DockerHost {
            name: r.get(0),
            socket: r.get(1),
            host: r.get(2),
            port: r
                .get::<Option<i64>, _>(3)
                .and_then(|p| u16::try_from(p).ok()),
            extra: serde_json::from_str(r.get::<&str, _>(4))?,
        });
    }

    Ok(Dashboard {
        settings,
        groups,
        bookmark_groups,
        info_widgets,
        docker_hosts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Dashboard {
        let mut media = Group {
            name: "Media".into(),
            ..Group::default()
        };
        media.services.push(Service {
            ping: Some("10.0.0.5:8096".into()),
            widget: Some(json!({"type":"jellyfin","key":"k"})),
            ..Service::new("Jellyfin", Some("http://10.0.0.5:8096"))
        });
        media.groups.push(Group {
            name: "Arr".into(),
            services: vec![Service::new("Sonarr", Some("http://s"))],
            ..Group::default()
        });
        Dashboard {
            groups: vec![
                media,
                Group {
                    name: "Infra".into(),
                    ..Group::default()
                },
            ],
            bookmark_groups: vec![BookmarkGroup {
                name: "Dev".into(),
                bookmarks: vec![Bookmark {
                    name: "GitHub".into(),
                    href: "https://github.com".into(),
                    ..Bookmark::default()
                }],
                ..BookmarkGroup::default()
            }],
            info_widgets: vec![InfoWidget {
                kind: "datetime".into(),
                ..InfoWidget::default()
            }],
            docker_hosts: vec![DockerHost {
                name: "local".into(),
                socket: Some("/var/run/docker.sock".into()),
                ..DockerHost::default()
            }],
            ..Dashboard::default()
        }
    }

    /// Clears ids so documents can be compared by content.
    fn strip(mut d: Dashboard) -> Dashboard {
        fn g(gs: &mut [Group]) {
            for x in gs {
                x.id = None;
                x.services.iter_mut().for_each(|s| s.id = None);
                g(&mut x.groups);
            }
        }
        g(&mut d.groups);
        for bg in &mut d.bookmark_groups {
            bg.id = None;
            bg.bookmarks.iter_mut().for_each(|b| b.id = None);
        }
        d.info_widgets.iter_mut().for_each(|w| w.id = None);
        d
    }

    fn sample_at(service_id: i64, t: i64, ok: bool) -> StatusSample {
        StatusSample {
            service_id,
            checked_at: t,
            ok,
            status_code: Some(200),
            latency_ms: Some(3),
            error: None,
        }
    }

    #[tokio::test]
    async fn empty_db_gives_default() {
        let db = Db::open_in_memory().await.unwrap();
        assert_eq!(db.load_dashboard().await.unwrap(), Dashboard::default());
        assert_eq!(db.current_version().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn round_trip() {
        let db = Db::open_in_memory().await.unwrap();
        let (v, stored) = db
            .save_dashboard(&sample(), Some(0), Some("me"), None)
            .await
            .unwrap();
        assert_eq!(v, 1);
        assert_eq!(strip(stored.clone()), sample());
        assert_eq!(db.load_dashboard().await.unwrap(), stored);
    }

    #[tokio::test]
    async fn ids_survive_reorder_and_move() {
        let db = Db::open_in_memory().await.unwrap();
        let (_, mut d) = db
            .save_dashboard(&sample(), None, None, None)
            .await
            .unwrap();
        let jelly_id = d.groups[0].services[0].id;
        // Move Jellyfin into Infra and swap the top-level groups.
        let jelly = d.groups[0].services.remove(0);
        d.groups[1].services.push(jelly);
        d.groups.swap(0, 1);
        let (_, after) = db.save_dashboard(&d, None, None, None).await.unwrap();
        assert_eq!(after.groups[0].name, "Infra");
        assert_eq!(after.groups[0].services[0].id, jelly_id);
        assert_eq!(after.groups[1].groups[0].services[0].name, "Sonarr");
    }

    #[tokio::test]
    async fn conflict_and_history() {
        let db = Db::open_in_memory().await.unwrap();
        db.save_dashboard(&sample(), Some(0), None, Some("first"))
            .await
            .unwrap();
        let err = db
            .save_dashboard(&Dashboard::default(), Some(0), None, None)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            DbError::Conflict {
                expected: 0,
                current: 1
            }
        ));
        db.save_dashboard(&Dashboard::default(), Some(1), None, Some("wipe"))
            .await
            .unwrap();
        let h = db.history(10).await.unwrap();
        assert_eq!(h.iter().map(|v| v.version).collect::<Vec<_>>(), [2, 1]);
        assert_eq!(strip(db.version_snapshot(1).await.unwrap()), sample());
        assert!(matches!(
            db.version_snapshot(9).await,
            Err(DbError::NotFound)
        ));
    }

    #[tokio::test]
    async fn samples() {
        let db = Db::open_in_memory().await.unwrap();
        let (_, d) = db
            .save_dashboard(&sample(), None, None, None)
            .await
            .unwrap();
        let id = d.groups[0].services[0].id.unwrap();
        for t in 0..5 {
            db.insert_sample(&sample_at(id, t, t % 2 == 0))
                .await
                .unwrap();
        }
        // Unknown service: dropped.
        db.insert_sample(&sample_at(999, 0, true)).await.unwrap();
        let s = db.recent_samples(3).await.unwrap();
        assert_eq!(
            s.iter().map(|x| x.checked_at).collect::<Vec<_>>(),
            [2, 3, 4]
        );
        assert_eq!(db.prune_samples(3).await.unwrap(), 3);
        // Deleting the service drops its history.
        db.save_dashboard(&Dashboard::default(), None, None, None)
            .await
            .unwrap();
        assert!(db.recent_samples(10).await.unwrap().is_empty());
    }
}
