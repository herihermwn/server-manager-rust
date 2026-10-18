-- Phase 3: which releases/<n> a deploy put live. Equals `number` for a normal deploy; a rollback
-- gets its own number but points at an older release. apps.current_release refers to this.
ALTER TABLE deploys ADD COLUMN release INTEGER;

-- Phase 3 (manager runs as root): the system user an app's build/start commands and its systemd
-- unit run as. The real default comes from SM_DEFAULT_RUN_AS at insert time.
ALTER TABLE apps ADD COLUMN run_as TEXT NOT NULL DEFAULT 'ubuntu';
