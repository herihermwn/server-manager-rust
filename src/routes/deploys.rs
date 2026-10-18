//! Phase 3 · Deploy pipeline (rencana §4), history, rollback, cancel, live log SSE, GitHub webhook.
//! Routes are relative to /api and already behind auth. Log write actions with `activity::log`.

use crate::state::AppState;
use axum::Router;

pub fn routes() -> Router<AppState> {
    Router::new()
}

pub fn spawn(_st: &AppState) {}

/// Outside /api and without auth: `/hooks/{app}` verifies X-Hub-Signature-256 itself.
pub fn public_routes() -> Router<AppState> {
    Router::new()
}
