-- Core dashboard schema (owned by homedit-db).
--
-- Migration numbering by crate, so parallel work never collides:
--   0001-0099 core (this file and its follow-ups)
--   0100-0199 auth, users, sessions, secrets
--   0200-0299 widgets (cache, state)
--   0300-0399 scanner (scans, results, schedules, scan log)
--   0400-0499 discovery / import / ops

CREATE TABLE settings (
    id      INTEGER PRIMARY KEY CHECK (id = 1),
    doc     TEXT    NOT NULL           -- JSON: homedit_core::Settings
);

CREATE TABLE groups (
    id                  INTEGER PRIMARY KEY,
    parent_id           INTEGER REFERENCES groups(id) ON DELETE CASCADE,
    position            INTEGER NOT NULL,
    name                TEXT    NOT NULL,
    layout              TEXT    NOT NULL DEFAULT '{}',  -- JSON: GroupLayout
    hidden_from_viewers INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX groups_parent ON groups(parent_id, position);

CREATE TABLE services (
    id           INTEGER PRIMARY KEY,
    group_id     INTEGER NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    position     INTEGER NOT NULL,
    name         TEXT    NOT NULL,
    href         TEXT,
    description  TEXT,
    icon         TEXT,
    ping         TEXT,
    site_monitor TEXT,
    server       TEXT,
    container    TEXT,
    widget       TEXT,                         -- JSON or NULL
    extra        TEXT    NOT NULL DEFAULT '{}' -- JSON: unknown keys
);
CREATE INDEX services_group ON services(group_id, position);

CREATE TABLE bookmark_groups (
    id       INTEGER PRIMARY KEY,
    position INTEGER NOT NULL,
    name     TEXT    NOT NULL,
    layout   TEXT    NOT NULL DEFAULT '{}'
);

CREATE TABLE bookmarks (
    id          INTEGER PRIMARY KEY,
    group_id    INTEGER NOT NULL REFERENCES bookmark_groups(id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    name        TEXT    NOT NULL,
    href        TEXT    NOT NULL,
    abbr        TEXT,
    icon        TEXT,
    description TEXT
);
CREATE INDEX bookmarks_group ON bookmarks(group_id, position);

CREATE TABLE info_widgets (
    id       INTEGER PRIMARY KEY,
    position INTEGER NOT NULL,
    kind     TEXT    NOT NULL,
    options  TEXT    NOT NULL DEFAULT '{}'
);

CREATE TABLE docker_hosts (
    name   TEXT PRIMARY KEY,
    socket TEXT,
    host   TEXT,
    port   INTEGER,
    extra  TEXT NOT NULL DEFAULT '{}'
);

-- One row per probe. Pruned by age.
CREATE TABLE status_samples (
    service_id  INTEGER NOT NULL REFERENCES services(id) ON DELETE CASCADE,
    checked_at  INTEGER NOT NULL,       -- ms since epoch
    ok          INTEGER NOT NULL,
    status_code INTEGER,
    latency_ms  INTEGER,
    error       TEXT
);
CREATE INDEX status_samples_service ON status_samples(service_id, checked_at);

-- Dashboard version history. `version` increases by one on every save;
-- `snapshot` is the full Dashboard JSON (secrets as stored).
CREATE TABLE config_versions (
    version    INTEGER PRIMARY KEY,
    created_at INTEGER NOT NULL,
    author     TEXT,
    summary    TEXT,
    snapshot   TEXT    NOT NULL
);
