//! Phase 3 · Apps: CRUD, wizard checks (git ls-remote, folder, port), start/stop/restart via systemctl --user.
//! Routes are relative to /api and already behind auth. Log write actions with `activity::log`.

use crate::state::AppState;
use axum::Router;

pub fn routes() -> Router<AppState> {
    Router::new()
}

pub fn spawn(_st: &AppState) {}
