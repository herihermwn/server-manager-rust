-- Phase 4: per-variable flags for shared/.env. The values themselves live only in the file.
-- A row exists once a key was saved through the manager; keys without a row fall back to
-- the name-based auto-detect (SECRET, KEY, TOKEN, DSN, PASSWORD, DATABASE_URL).
CREATE TABLE env_meta (
    app_id INTEGER NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    key    TEXT    NOT NULL,
    secret INTEGER NOT NULL DEFAULT 0,
    source TEXT,                                 -- e.g. 'database:radio_api' (written by the databases phase)
    PRIMARY KEY (app_id, key)
);
