use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

pub struct ApiError(pub StatusCode, pub String);

impl ApiError {
    pub fn new(code: StatusCode, msg: impl Into<String>) -> Self {
        Self(code, msg.into())
    }
    pub fn bad(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, msg)
    }
    pub fn conflict(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, msg)
    }
    pub fn forbidden(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, msg)
    }
    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "We couldn't find that case.")
    }
    pub fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "Log in to continue.")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        eprintln!("database error: {e}");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Something broke on our side. Try again in a moment.",
        )
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
