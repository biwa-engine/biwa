use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use biwa_hub_usecase::UseCaseError;

use crate::dto::ErrorResponse;

pub struct ApiError(UseCaseError);

impl From<UseCaseError> for ApiError {
    fn from(err: UseCaseError) -> Self {
        Self(err)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match &self.0 {
            UseCaseError::NotFound => (StatusCode::NOT_FOUND, self.0.to_string()),
            UseCaseError::Conflict(_) => (StatusCode::CONFLICT, self.0.to_string()),
            UseCaseError::Invalid(_) => (StatusCode::BAD_REQUEST, self.0.to_string()),
            UseCaseError::Internal(msg) => {
                eprintln!("internal error: {msg}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal error".to_string(),
                )
            }
        };

        (status, Json(ErrorResponse { error: message })).into_response()
    }
}
