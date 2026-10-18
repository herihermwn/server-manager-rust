-- Phase 5: result of the latest pg_dump per database (daily 02:00 WIB task or the Back up button).
-- last_backup_at (from 0001) is the time of the latest attempt, ok or not. Passwords are never stored.
ALTER TABLE databases ADD COLUMN last_backup_status TEXT;     -- ok | failed
ALTER TABLE databases ADD COLUMN last_backup_size   INTEGER;  -- bytes of the dump file (ok only)
ALTER TABLE databases ADD COLUMN last_backup_error  TEXT;     -- short reason (failed only)
