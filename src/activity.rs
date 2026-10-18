//! Audit trail of write actions. Call `activity::log` from every handler that changes something.

use crate::{http::ApiResult, state::AppState};
use axum::{
    Json, Router,
    extract::{Query, State},
    response::IntoResponse,
    routing::get,
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

/// Never fails the caller: a lost audit row is logged to stderr, the action itself already happened.
/// `action` is dotted (`auth.login`, `app.create`, `deploy.start`); `target` names the record.
pub async fn log(db: &SqlitePool, actor: &str, action: &str, target: &str, detail: &str) {
    let r = sqlx::query("INSERT INTO activity (actor, action, target, detail) VALUES (?, ?, ?, ?)")
        .bind(actor)
        .bind(action)
        .bind(target)
        .bind(detail)
        .execute(db)
        .await;
    if let Err(e) = r {
        eprintln!("[activity] could not log {action} {target}: {e}");
    }
}

#[derive(Serialize, sqlx::FromRow)]
struct Row {
    id: i64,
    at: i64,
    actor: String,
    action: String,
    target: String,
    detail: String,
}

#[derive(Deserialize)]
struct ListQuery {
    limit: Option<i64>,
}

/// GET /api/activity?limit=50 (newest first, max 500).
async fn list(State(st): State<AppState>, Query(q): Query<ListQuery>) -> ApiResult<impl IntoResponse> {
    let rows: Vec<Row> =
        sqlx::query_as("SELECT id, at, actor, action, target, detail FROM activity ORDER BY id DESC LIMIT ?")
            .bind(q.limit.unwrap_or(50).clamp(1, 500))
            .fetch_all(&st.db)
            .await?;
    Ok(Json(rows))
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/activity", get(list))
}
