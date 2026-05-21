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
    /// MP-5 / Task 15：单 run LLM 预算超额。调用方应捕获并走降级路径
    /// （如使用 `local_decision_review`、跳过 rewrite、跳过二次 router 等），
    /// 不应原样返回给 webhook 调用者。
    #[error("run budget exceeded: {reason}")]
    BudgetExceeded { run_id: String, reason: String },
    /// LP-14 / Task 20：webhook 限流命中。返回 HTTP 429 + Retry-After。
    #[error("rate limited for account {account_id}, retry after {retry_after}s")]
    RateLimited {
        retry_after: u64,
        account_id: String,
    },
}

pub type AppResult<T> = Result<T, AppError>;

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            AppError::BadRequest(msg) => {
                (StatusCode::BAD_REQUEST, Json(json!({ "error": msg }))).into_response()
            }
            AppError::NotFound(msg) => {
                (StatusCode::NOT_FOUND, Json(json!({ "error": msg }))).into_response()
            }
            AppError::BudgetExceeded { run_id, reason } => (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "budget_exceeded", "run_id": run_id, "reason": reason })),
            )
                .into_response(),
            AppError::RateLimited {
                retry_after,
                account_id,
            } => {
                let body = Json(json!({
                    "error": "rate_limited",
                    "account_id": account_id
                }));
                let mut response = (StatusCode::TOO_MANY_REQUESTS, body).into_response();
                if let Ok(value) = retry_after.to_string().parse() {
                    response.headers_mut().insert("Retry-After", value);
                }
                response
            }
            AppError::Db(_)
            | AppError::Http(_)
            | AppError::Json(_)
            | AppError::BsonSer(_)
            | AppError::External(_) => {
                let msg = self.to_string();
                (StatusCode::BAD_GATEWAY, Json(json!({ "error": msg }))).into_response()
            }
        }
    }
}
