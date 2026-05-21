//! 演化器错误类型。与 `crate::error::AppError` 解耦，避免演化器异常污染主链路
//! HTTP 响应。所有 mongo / serde 错误统一透传到 [`EvolutionError`]。
//!
//! 不引入 `BudgetExceeded` 字面量映射到 webhook 5xx 的路径——演化器与 webhook
//! 完全解耦，预算耗尽只影响 worker 自己。

use thiserror::Error;

#[derive(Debug, Error)]
pub enum EvolutionError {
    #[error("evolution budget exceeded: tokens_used={tokens_used}, calls_used={calls_used}")]
    BudgetExceeded {
        tokens_used: i64,
        calls_used: i32,
    },
    #[error("envelope state invalid: {0}")]
    InvalidStatus(String),
    #[error("mongo error: {0}")]
    Mongo(#[from] mongodb::error::Error),
    #[error("serde bson error: {0}")]
    Bson(#[from] mongodb::bson::de::Error),
    #[error("internal: {0}")]
    Internal(String),
}
