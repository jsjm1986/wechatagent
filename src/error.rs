use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    NotFound(String),
    #[error("database error: {0}")]
    Db(#[from] mongodb::error::Error),
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("bson serialization error: {0}")]
    BsonSer(#[from] mongodb::bson::ser::Error),
    #[error("{0}")]
    External(String),
}

pub type AppResult<T> = Result<T, AppError>;

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match self {
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::NotFound(_) => StatusCode::NOT_FOUND,
            AppError::Db(_)
            | AppError::Http(_)
            | AppError::Json(_)
            | AppError::BsonSer(_)
            | AppError::External(_) => StatusCode::BAD_GATEWAY,
        };
        let body = Json(json!({
            "error": self.to_string()
        }));
        (status, body).into_response()
    }
}
