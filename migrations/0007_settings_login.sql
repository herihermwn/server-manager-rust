-- Phase 7: per-IP sign-in failures survive a restart (auth::Lockout writes through to this).
CREATE TABLE login_fails (
    ip           TEXT    PRIMARY KEY,
    fails        INTEGER NOT NULL,
    last         INTEGER NOT NULL,
    locked_until INTEGER NOT NULL DEFAULT 0
);
