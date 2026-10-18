//! Phase 5 · Databases: PostgreSQL list/create/link/drop, credentials shown once, pg_dump backups.
//! Routes are relative to /api and already behind auth. Log write actions with `activity::log`.

use crate::state::AppState;
use axum::Router;

pub fn routes() -> Router<AppState> {
    Router::new()
}

pub fn spawn(_st: &AppState) {}
