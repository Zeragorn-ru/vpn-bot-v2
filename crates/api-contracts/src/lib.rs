use axum::{Json, response::IntoResponse};
use http::StatusCode;
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Serialize)]
pub struct ApiError {
    pub code: &'static str,
    pub message: &'static str,
    pub request_id: Uuid,
}

impl ApiError {
    #[must_use]
    pub fn internal(request_id: Uuid) -> Self {
        Self {
            code: "internal_error",
            message: "An unexpected error occurred.",
            request_id,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(self)).into_response()
    }
}
