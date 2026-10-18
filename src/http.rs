//! Error type for handlers. Success bodies are plain `Json(...)`; errors are `{ "error": "..." }`.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

pub type ApiResult<T = Response> = Result<T, AppError>;

#[derive(Debug)]
pub enum AppError {
    /// Shown to the client as-is.
    Status(StatusCode, String),
    /// Logged; the client gets a generic 500.
    Internal(String),
}

impl AppError {
    pub fn bad_request(msg: impl Into<String>) -> Self {
        AppError::Status(StatusCode::BAD_REQUEST, msg.into())
    }
    pub fn not_found(msg: impl Into<String>) -> Self {
        AppError::Status(StatusCode::NOT_FOUND, msg.into())
    }
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        AppError::Internal(format!("db: {e}"))
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Internal(format!("io: {e}"))
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            AppError::Status(status, msg) => (status, Json(json!({ "error": msg }))).into_response(),
            AppError::Internal(msg) => {
                eprintln!("[error] {msg}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": "Internal server error" })),
                )
                    .into_response()
            }
        }
    }
}

pub async fn api_not_found() -> AppError {
    AppError::not_found("Endpoint not found")
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
