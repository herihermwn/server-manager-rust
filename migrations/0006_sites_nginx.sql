-- Phase 6: what the nginx config generator needs on top of 0001's `sites`.
-- kind is proxy | static | redirect. ssl_expires_at doubles as "a certificate exists" (the
-- live/ folder is root-only, so the manager cannot stat it); NULL = config is generated without SSL.
ALTER TABLE sites ADD COLUMN redirect_url TEXT;
ALTER TABLE sites ADD COLUMN websocket    INTEGER NOT NULL DEFAULT 1;
ALTER TABLE sites ADD COLUMN gzip         INTEGER NOT NULL DEFAULT 1;
ALTER TABLE sites ADD COLUMN cache_assets INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sites ADD COLUMN force_https  INTEGER NOT NULL DEFAULT 1;
