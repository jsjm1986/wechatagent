use std::time::{Duration, Instant};

use async_trait::async_trait;
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::time::sleep;

use crate::error::{AppError, AppResult};

#[derive(Clone)]
pub struct LlmClient {
    base_url: String,
    api_key: String,
    model: String,
    client: reqwest::Client,
    max_retries: u32,
    retry_base_ms: u64,
}

#[derive(Debug, Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
    usage: Option<ChatUsage>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatChoiceMessage,
}

#[derive(Debug, Deserialize)]
struct ChatChoiceMessage {
    content: String,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct ChatUsage {
    #[serde(default)]
    pub prompt_tokens: i64,
    #[serde(default)]
    pub completion_tokens: i64,
    #[serde(default)]
    pub total_tokens: i64,
    #[serde(default)]
    pub prompt_cache_hit_tokens: i64,
    #[serde(default)]
    pub prompt_cache_miss_tokens: i64,
}

#[derive(Debug, Clone, Default)]
pub struct LlmJsonResult {
    pub value: Value,
    pub usage: ChatUsage,
    pub latency_ms: i64,
    pub model: String,
    /// HP-4 / Task 11：本次成功之前发生的重试次数（0 表示一次成功）。
    pub retry_count: u32,
}

/// LLM 生成接口抽象。
///
/// 用 trait 隔离运行时 LLM 客户端与测试中的 mock，便于通过 mockall 或手写
/// fake 实现注入预期响应；运行时仍使用 [`LlmClient`] 走 HTTP。
#[async_trait]
#[cfg_attr(test, mockall::automock)]
pub trait LlmGenerator: Send + Sync {
    async fn generate_json(&self, system: &str, user: &str) -> AppResult<Value>;
    async fn generate_json_with_usage(&self, system: &str, user: &str) -> AppResult<LlmJsonResult>;
}

impl LlmClient {
    pub fn new(
        base_url: String,
        api_key: String,
        model: String,
        timeout_seconds: u64,
        max_retries: u32,
        retry_base_ms: u64,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            model,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(timeout_seconds))
                .build()?,
            max_retries: max_retries.max(1),
            retry_base_ms: retry_base_ms.max(100),
        })
    }

    /// 执行一次实际 HTTP 请求；返回 (result, retry_after_seconds)。
    /// `retry_after_seconds` 仅在请求失败时可能 Some，由调用方决定如何与
    /// 指数退避取 max。
    async fn generate_json_once(
        &self,
        system: &str,
        user: &str,
    ) -> AppResult<(LlmJsonResult, Option<u64>)> {
        let started_at = Instant::now();
        let body = json!({
            "model": self.model,
            "temperature": 0.2,
            "messages": [
                ChatMessage { role: "system", content: system },
                ChatMessage { role: "user", content: user }
            ]
        });

        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;
        let status = response.status();
        let retry_after = parse_retry_after(response.headers());
        let text = response.text().await?;
        if !status.is_success() {
            // 把 retry_after 透传给上层，由 retry 循环决定如何用。
            let mut err = AppError::External(format!("LLM HTTP {status}: {text}"));
            if let Some(after) = retry_after {
                // 编码到 message 末尾让 retry 循环也能解析（或后续重构为带元数据的错误）。
                err = AppError::External(format!(
                    "LLM HTTP {status}: {text} [retry_after_secs={after}]"
                ));
            }
            return Err(err);
        }

        let parsed: ChatCompletionResponse = serde_json::from_str(&text)?;
        let content = parsed
            .choices
            .first()
            .map(|choice| choice.message.content.as_str())
            .ok_or_else(|| AppError::External("LLM returned no choices".to_string()))?;
        Ok((
            LlmJsonResult {
                value: parse_json_content(content)?,
                usage: parsed.usage.unwrap_or_default(),
                latency_ms: started_at.elapsed().as_millis() as i64,
                model: self.model.clone(),
                retry_count: 0,
            },
            None,
        ))
    }
}

#[async_trait]
impl LlmGenerator for LlmClient {
    async fn generate_json(&self, system: &str, user: &str) -> AppResult<Value> {
        self.generate_json_with_usage(system, user)
            .await
            .map(|result| result.value)
    }

    async fn generate_json_with_usage(&self, system: &str, user: &str) -> AppResult<LlmJsonResult> {
        let mut last_error = None;
        let mut retry_count: u32 = 0;
        for attempt in 1..=self.max_retries {
            match self.generate_json_once(system, user).await {
                Ok((mut value, _)) => {
                    value.retry_count = retry_count;
                    return Ok(value);
                }
                Err(error) if attempt < self.max_retries && is_retryable_llm_error(&error) => {
                    let retry_after_secs = parse_retry_after_from_error(&error);
                    last_error = Some(error.to_string());
                    let delay = compute_backoff(attempt, self.retry_base_ms, retry_after_secs);
                    sleep(delay).await;
                    retry_count = retry_count.saturating_add(1);
                }
                Err(error) => {
                    if retry_count > 0 {
                        return Err(AppError::External(format!(
                            "{}; retry_count={}",
                            error, retry_count
                        )));
                    }
                    return Err(error);
                }
            }
        }
        Err(AppError::External(format!(
            "LLM request failed after retries: {}",
            last_error.unwrap_or_else(|| "unknown error".to_string())
        )))
    }
}

/// HP-4：可重试错误判定。
///
/// **不**把 `AppError::Json(_)` 当可重试 —— 模型确定性吐出非 JSON 时，重试
/// 几乎一定继续失败，只会浪费 token。让上层 fail-fast 走降级路径。
pub fn is_retryable_llm_error(error: &AppError) -> bool {
    match error {
        AppError::Http(err) => err.is_timeout() || err.is_connect(),
        AppError::External(message) => {
            message.contains("LLM HTTP 429")
                || message.contains("LLM HTTP 500")
                || message.contains("LLM HTTP 502")
                || message.contains("LLM HTTP 503")
                || message.contains("LLM HTTP 504")
        }
        _ => false,
    }
}

/// 指数退避带 jitter，并尊重 Retry-After。
pub fn compute_backoff(attempt: u32, base_ms: u64, retry_after_secs: Option<u64>) -> Duration {
    let shift = attempt.saturating_sub(1).min(10);
    let exp_ms = base_ms.saturating_mul(1u64 << shift);
    let jitter = fastrand_jitter(base_ms);
    let backoff_ms = exp_ms.saturating_add(jitter);
    let final_ms = match retry_after_secs {
        Some(s) => backoff_ms.max(s.saturating_mul(1000)),
        None => backoff_ms,
    };
    Duration::from_millis(final_ms)
}

#[cfg(not(test))]
fn fastrand_jitter(base_ms: u64) -> u64 {
    if base_ms == 0 {
        0
    } else {
        fastrand::u64(0..base_ms)
    }
}

#[cfg(test)]
fn fastrand_jitter(_base_ms: u64) -> u64 {
    // 测试中关掉 jitter，便于断言确定性退避值。
    0
}

fn parse_retry_after(headers: &HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
}

fn parse_retry_after_from_error(error: &AppError) -> Option<u64> {
    if let AppError::External(message) = error {
        if let Some(idx) = message.find("[retry_after_secs=") {
            let rest = &message[idx + "[retry_after_secs=".len()..];
            if let Some(end) = rest.find(']') {
                return rest[..end].trim().parse::<u64>().ok();
            }
        }
    }
    None
}

fn parse_json_content(content: &str) -> AppResult<Value> {
    let trimmed = content.trim();
    let json_text = if trimmed.starts_with("```") {
        trimmed
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim()
    } else {
        trimmed
    };
    serde_json::from_str(json_text).map_err(AppError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_error_is_not_retryable() {
        let err = AppError::Json(serde_json::from_str::<Value>("not json").unwrap_err());
        assert!(!is_retryable_llm_error(&err));
    }

    #[test]
    fn http_429_is_retryable() {
        let err = AppError::External("LLM HTTP 429: rate limited".to_string());
        assert!(is_retryable_llm_error(&err));
    }

    #[test]
    fn http_5xx_is_retryable() {
        let err = AppError::External("LLM HTTP 502: bad gateway".to_string());
        assert!(is_retryable_llm_error(&err));
    }

    #[test]
    fn http_400_is_not_retryable() {
        let err = AppError::External("LLM HTTP 400: bad request".to_string());
        assert!(!is_retryable_llm_error(&err));
    }

    #[test]
    fn backoff_grows_exponentially() {
        // base = 1000ms, jitter=0 (test-only), so attempt 1 => 1000, 2 => 2000, 3 => 4000.
        assert_eq!(compute_backoff(1, 1000, None).as_millis(), 1000);
        assert_eq!(compute_backoff(2, 1000, None).as_millis(), 2000);
        assert_eq!(compute_backoff(3, 1000, None).as_millis(), 4000);
    }

    #[test]
    fn backoff_respects_retry_after() {
        // base=1000, attempt=1 → 1000ms baseline; Retry-After=5s → 5000ms wins.
        assert_eq!(compute_backoff(1, 1000, Some(5)).as_millis(), 5000);
        // 当指数退避更长时使用指数退避。
        assert_eq!(compute_backoff(4, 1000, Some(2)).as_millis(), 8000);
    }

    #[test]
    fn parse_retry_after_extracts_marker() {
        let err =
            AppError::External("LLM HTTP 429: please slow down [retry_after_secs=7]".to_string());
        assert_eq!(parse_retry_after_from_error(&err), Some(7));
    }
}
