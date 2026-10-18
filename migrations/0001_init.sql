-- Phase 1: every table from rencana-implementasi.md §3.
-- Timestamps are unix seconds (UTC); the UI converts to WIB.
-- Later phases add NEW migration files (0002_…), never edit this one.

CREATE TABLE admin_users (
    id            INTEGER PRIMARY KEY,
    username      TEXT    NOT NULL UNIQUE,
    password_hash TEXT    NOT NULL,                -- argon2id PHC string
    token_version INTEGER NOT NULL DEFAULT 0,      -- bump = revoke every JWT issued before
    created_at    INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at    INTEGER NOT NULL DEFAULT (unixepoch()),
    last_login_at INTEGER
);

-- One row per managed app. Layout on disk: <path>/{repo,releases/<n>,current,shared}.
CREATE TABLE apps (
    id                 INTEGER PRIMARY KEY,
    name               TEXT    NOT NULL UNIQUE,    -- ^[a-z0-9-]{1,40}$, also systemd unit app-<name>
    repo_url           TEXT    NOT NULL,
    branch             TEXT    NOT NULL DEFAULT 'main',
    path               TEXT    NOT NULL UNIQUE,    -- absolute, under the apps folder
    runtime            TEXT    NOT NULL DEFAULT 'other', -- rust | bun | node | static | other
    build_cmd          TEXT    NOT NULL DEFAULT '',
    before_restart_cmd TEXT    NOT NULL DEFAULT '',
    start_cmd          TEXT    NOT NULL DEFAULT '',
    port               INTEGER UNIQUE,
    health_path        TEXT    NOT NULL DEFAULT '/',
    shared_paths       TEXT    NOT NULL DEFAULT '[".env"]', -- JSON array of relative paths
    keep_releases      INTEGER NOT NULL DEFAULT 5,
    build_timeout_secs INTEGER NOT NULL DEFAULT 900,
    auto_deploy        INTEGER NOT NULL DEFAULT 0,  -- webhook on push to branch
    webhook_secret     TEXT,
    current_release    INTEGER,                     -- deploys.number that `current` points to
    created_at         INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at         INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE TABLE deploys (
    id             INTEGER PRIMARY KEY,
    app_id         INTEGER NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    number         INTEGER NOT NULL,               -- per app: #1, #2 … (= releases/<n>)
    commit_sha     TEXT,
    commit_message TEXT,
    branch         TEXT    NOT NULL,
    trigger        TEXT    NOT NULL,               -- manual | webhook | initial | rollback
    status         TEXT    NOT NULL DEFAULT 'queued', -- queued | running | success | failed | cancelled | rolled_back
    error          TEXT,
    log_path       TEXT,                           -- ~/.server-manager/logs/<app>/<n>.log
    actor          TEXT,
    created_at     INTEGER NOT NULL DEFAULT (unixepoch()),
    started_at     INTEGER,
    finished_at    INTEGER,
    UNIQUE (app_id, number)
);
CREATE INDEX deploys_app_created ON deploys (app_id, created_at DESC);

CREATE TABLE deploy_steps (
    id          INTEGER PRIMARY KEY,
    deploy_id   INTEGER NOT NULL REFERENCES deploys(id) ON DELETE CASCADE,
    idx         INTEGER NOT NULL,                  -- order in the pipeline
    name        TEXT    NOT NULL,                  -- Pull | Link shared | Build | Before restart | Switch | Restart | Health check | Clean up
    command     TEXT    NOT NULL DEFAULT '',
    status      TEXT    NOT NULL DEFAULT 'waiting', -- waiting | running | success | failed | skipped | cancelled
    exit_code   INTEGER,
    started_at  INTEGER,
    finished_at INTEGER,
    UNIQUE (deploy_id, idx)
);

-- nginx sites written to /etc/nginx/sites-manager/<domain>.conf
CREATE TABLE sites (
    id                INTEGER PRIMARY KEY,
    domain            TEXT    NOT NULL UNIQUE,
    aliases           TEXT    NOT NULL DEFAULT '[]', -- JSON array
    app_id            INTEGER REFERENCES apps(id) ON DELETE SET NULL,
    kind              TEXT    NOT NULL DEFAULT 'proxy', -- proxy | static
    upstream          TEXT,                           -- 127.0.0.1:<port> for proxy
    root_path         TEXT,                           -- for static
    max_upload_mb     INTEGER NOT NULL DEFAULT 10,
    custom_directives TEXT    NOT NULL DEFAULT '',
    enabled           INTEGER NOT NULL DEFAULT 1,
    ssl               INTEGER NOT NULL DEFAULT 0,
    ssl_email         TEXT,
    ssl_expires_at    INTEGER,
    created_at        INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at        INTEGER NOT NULL DEFAULT (unixepoch())
);

-- PostgreSQL databases created by the manager. Passwords are never stored.
CREATE TABLE databases (
    id             INTEGER PRIMARY KEY,
    name           TEXT    NOT NULL UNIQUE,
    owner          TEXT    NOT NULL,              -- role name
    app_id         INTEGER REFERENCES apps(id) ON DELETE SET NULL,
    extensions     TEXT    NOT NULL DEFAULT '[]', -- JSON array
    last_backup_at INTEGER,
    created_at     INTEGER NOT NULL DEFAULT (unixepoch())
);

-- Key/value: apps_dir, port_range, alert thresholds, https token (encrypted later), …
CREATE TABLE settings (
    key        TEXT    PRIMARY KEY,
    value      TEXT    NOT NULL,
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
);

-- Every write action (activity::log). Shown in Overview › Recent activity.
CREATE TABLE activity (
    id     INTEGER PRIMARY KEY,
    at     INTEGER NOT NULL DEFAULT (unixepoch()),
    actor  TEXT    NOT NULL,                       -- username, 'webhook' or 'system'
    action TEXT    NOT NULL,                       -- dotted: auth.login, app.create, deploy.start …
    target TEXT    NOT NULL DEFAULT '',            -- app name, domain, db name …
    detail TEXT    NOT NULL DEFAULT ''
);
CREATE INDEX activity_at ON activity (at DESC);

-- Server samples. 10 s rows kept 24 h, 5 min averages kept 30 days (K5).
-- Rates are per second over the sample interval. Per-mount / per-interface / top processes go in `extra` (JSON).
CREATE TABLE metrics_10s (
    ts         INTEGER PRIMARY KEY,
    cpu_pct    REAL    NOT NULL,
    load1      REAL    NOT NULL,
    load5      REAL    NOT NULL,
    load15     REAL    NOT NULL,
    mem_used   INTEGER NOT NULL,
    mem_total  INTEGER NOT NULL,
    swap_used  INTEGER NOT NULL,
    swap_total INTEGER NOT NULL,
    disk_used  INTEGER NOT NULL,
    disk_total INTEGER NOT NULL,
    disk_read  INTEGER NOT NULL,                   -- bytes/s
    disk_write INTEGER NOT NULL,
    net_rx     INTEGER NOT NULL,                   -- bytes/s
    net_tx     INTEGER NOT NULL,
    extra      TEXT    NOT NULL DEFAULT '{}'
);

CREATE TABLE metrics_5m (
    ts         INTEGER PRIMARY KEY,                -- bucket start
    cpu_pct    REAL    NOT NULL,
    cpu_max    REAL    NOT NULL,
    load1      REAL    NOT NULL,
    load5      REAL    NOT NULL,
    load15     REAL    NOT NULL,
    mem_used   INTEGER NOT NULL,
    mem_total  INTEGER NOT NULL,
    swap_used  INTEGER NOT NULL,
    swap_total INTEGER NOT NULL,
    disk_used  INTEGER NOT NULL,
    disk_total INTEGER NOT NULL,
    disk_read  INTEGER NOT NULL,
    disk_write INTEGER NOT NULL,
    net_rx     INTEGER NOT NULL,
    net_tx     INTEGER NOT NULL,
    samples    INTEGER NOT NULL,                   -- 10 s rows averaged; gaps = agent offline
    extra      TEXT    NOT NULL DEFAULT '{}'
);
