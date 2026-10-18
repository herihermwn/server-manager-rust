//! Phase 6 · Web server: site config generator, nginx -t before reload, certbot, enable/disable.
//! Routes are relative to /api and already behind auth. Log write actions with `activity::log`.

use crate::state::AppState;
use axum::Router;

pub fn routes() -> Router<AppState> {
    Router::new()
}

pub fn spawn(_st: &AppState) {}
