//! Phase 4 · Environment: read/write shared/.env (mode 600), secret masking, reveal, paste + diff.
//! Routes are relative to /api and already behind auth. Log write actions with `activity::log`.

use crate::state::AppState;
use axum::Router;

pub fn routes() -> Router<AppState> {
    Router::new()
}

pub fn spawn(_st: &AppState) {}
