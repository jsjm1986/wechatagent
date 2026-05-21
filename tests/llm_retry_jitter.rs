//! HP-4 / Task 11 / Task 24：LLM 重试退避与 JSON 错误不重试回归。
//!
//! 性质：
//! 1. `compute_backoff(attempt, base, None)` 严格指数增长（attempt=1 → base，
//!    attempt=2 → 2·base，attempt=3 → 4·base）；
//! 2. `Retry-After` 时长大于退避基线时使用 Retry-After（5s 必然 ≥ 5000ms）；
//! 3. `is_retryable_llm_error` 对 429/5xx 返回 true，对 JSON parse 错误返回
//!    false（即非 JSON 内容只调一次）；
//!
//! 测试运行在测试环境下 `fastrand_jitter` 编译路径中（`#[cfg(test)]` 关 jitter），
//! 所以退避值确定性可断言。
//!
//! 不依赖 testcontainers / mongodb / mock LLM，默认参与 `cargo test`。

use std::time::Duration;
use wechatagent::error::AppError;
use wechatagent::llm::{compute_backoff, is_retryable_llm_error};

#[test]
fn retry_after_dominates_short_baseline() {
    // base=1000ms, attempt=1 baseline 1000ms; Retry-After=5s → 至少 5000ms。
    let delay = compute_backoff(1, 1000, Some(5));
    assert!(
        delay >= Duration::from_secs(5),
        "Retry-After=5 应至少 5000ms，实际 {:?}",
        delay
    );
}

#[test]
fn exponential_backoff_when_no_retry_after() {
    // 在外部测试中 jitter 走 prod 路径，会加上 [0, base) 的随机值。
    // 性质：退避 ≥ base * 2^(attempt-1)，且 ≤ base * 2^(attempt-1) + base。
    for attempt in 1u32..=4 {
        let base = 500u64;
        let lower = base * (1u64 << (attempt - 1));
        let upper = lower + base;
        let delay = compute_backoff(attempt, base, None).as_millis() as u64;
        assert!(
            delay >= lower && delay < upper + 1, // upper 含 jitter 边界
            "attempt={} delay={}ms 应在 [{}, {}] 范围内",
            attempt,
            delay,
            lower,
            upper
        );
    }
}

#[test]
fn long_exponential_overrides_short_retry_after() {
    // attempt=4 + base=1000 → 8000ms 退避基线（含 jitter）；Retry-After=2 (=2000ms) 不足。
    // 性质：实际等待 ≥ 8000ms。
    let delay = compute_backoff(4, 1000, Some(2)).as_millis() as u64;
    assert!(
        delay >= 8000,
        "退避指数(8000) 大于 Retry-After(2000) 时应使用退避，实际 {}ms",
        delay
    );
}

#[test]
fn http_429_and_5xx_are_retryable() {
    let cases = [
        "LLM HTTP 429: rate limited",
        "LLM HTTP 500: internal",
        "LLM HTTP 502: bad gateway",
        "LLM HTTP 503: maintenance",
        "LLM HTTP 504: gateway timeout",
    ];
    for msg in cases {
        let err = AppError::External(msg.to_string());
        assert!(is_retryable_llm_error(&err), "{} 应被判定为可重试", msg);
    }
}

#[test]
fn http_400_and_401_are_not_retryable() {
    for msg in ["LLM HTTP 400: bad request", "LLM HTTP 401: unauthorized"] {
        let err = AppError::External(msg.to_string());
        assert!(!is_retryable_llm_error(&err), "{} 不应重试", msg);
    }
}

#[test]
fn json_parse_error_is_not_retryable() {
    // 模型返回非 JSON 内容时，is_retryable_llm_error 必须返回 false。
    let parse_err = serde_json::from_str::<serde_json::Value>("not a json").unwrap_err();
    let err = AppError::Json(parse_err);
    assert!(!is_retryable_llm_error(&err), "JSON 解析失败不应触发重试");
}
