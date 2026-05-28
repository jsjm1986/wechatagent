use std::time::{Duration, Instant};

use async_trait::async_trait;
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::time::sleep;

use crate::error::{AppError, AppResult};

/// 上游协议形态。
///
/// `Openai`：`POST {base_url}/chat/completions`，messages: [{system},{user}]，
/// 解析 `choices[0].message.content`。兼容 DeepSeek / 通义 / mimo 等大量
/// "OpenAI 兼容" endpoint。
///
/// `Anthropic`：`POST {base_url}/v1/messages`，header `x-api-key + anthropic-version`，
/// `system` 单独字段 + `messages: [{user}]`，解析 `content[0].text`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmFormat {
    Openai,
    Anthropic,
}

impl LlmFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            LlmFormat::Openai => "openai",
            LlmFormat::Anthropic => "anthropic",
        }
    }

    pub fn parse(value: &str) -> AppResult<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "openai" | "" => Ok(Self::Openai),
            "anthropic" | "claude" => Ok(Self::Anthropic),
            other => Err(AppError::BadRequest(format!(
                "unsupported llm format: {other}"
            ))),
        }
    }
}

#[derive(Clone)]
pub struct LlmClient {
    base_url: String,
    api_key: String,
    model: String,
    format: LlmFormat,
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

#[derive(Debug, Deserialize)]
struct AnthropicMessageResponse {
    #[serde(default)]
    content: Vec<AnthropicContentBlock>,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

#[derive(Debug, Deserialize)]
struct AnthropicContentBlock {
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    text: String,
}

#[derive(Debug, Deserialize, Default)]
struct AnthropicUsage {
    #[serde(default)]
    input_tokens: i64,
    #[serde(default)]
    output_tokens: i64,
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
///
/// 命名口径：与 `docs/agent-policy.md` Phase E2-T1 对齐为 `LlmProvider`，
/// reviewer 双脑（primary + cross_provider）通过 [`LlmRegistry`] 选择不同
/// provider 实现达成 epistemic diversity。
#[async_trait]
#[cfg_attr(test, mockall::automock)]
pub trait LlmProvider: Send + Sync {
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
        Self::with_format(
            base_url,
            api_key,
            model,
            LlmFormat::Openai,
            timeout_seconds,
            max_retries,
            retry_base_ms,
        )
    }

    pub fn with_format(
        base_url: String,
        api_key: String,
        model: String,
        format: LlmFormat,
        timeout_seconds: u64,
        max_retries: u32,
        retry_base_ms: u64,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            model,
            format,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(timeout_seconds))
                // 防 chunked body 中段被中间设备/CDN 静默掐断 ——
                // smoke 时观测到 DeepSeek HTTP/1.1 chunked stream 偶发在 60s
                // 时被中断（status=200 但 body 解码失败）。开 tcp_keepalive
                // 让 idle 连接周期性发包，避免 NAT/防火墙 idle 超时杀流。
                .tcp_keepalive(Duration::from_secs(15))
                // 关掉连接池：smoke 实测同一进程对 DeepSeek 复用 TCP
                // 时偶发 chunked body 在 60s 截断 —— 怀疑是 keep-alive
                // 池里的过期连接被复用。直接每请求新拨号，牺牲一点 RTT
                // 换稳定性（LLM 调用本身 >5s，TCP 握手成本可忽略）。
                .pool_max_idle_per_host(0)
                // 强制 HTTP/1.1：smoke 实测 reqwest 默认 HTTP/2 + rustls 通过
                // DeepSeek 时，对 chunked body 偶发在 ~60s 出现 stream stall
                // → "error decoding response body"。同样 prompt 通过 urllib
                // (HTTP/1.1) 17s 就能拿到完整 9980 bytes。改用 HTTP/1.1 后
                // 整条链路稳定。
                .http1_only()
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
        match self.format {
            LlmFormat::Openai => self.generate_json_once_openai(system, user).await,
            LlmFormat::Anthropic => self.generate_json_once_anthropic(system, user).await,
        }
    }

    async fn generate_json_once_openai(
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
        // 用 bytes() 而不是 text()。reqwest 的 text() 在底层 chunk 流中断时
        // 只丢一个 "error decoding response body" 出来，没有任何上下文 ——
        // smoke 时一个 502 会让所有 LLM 路径变盲。改用 bytes() + lossy UTF-8，
        // 失败时把 status / latency 一并报上来，并打成"LLM HTTP body_decode_error"
        // 标签让 is_retryable_llm_error 识别为可重试。
        let text = match response.bytes().await {
            Ok(buf) => String::from_utf8_lossy(&buf).into_owned(),
            Err(err) => {
                let elapsed_ms = started_at.elapsed().as_millis();
                return Err(AppError::External(format!(
                    "LLM HTTP body_decode_error status={} elapsed_ms={} cause={}",
                    status, elapsed_ms, err
                )));
            }
        };
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

    /// Anthropic Messages API 形态：
    /// `POST {base_url}/v1/messages` （base_url 通常 `https://api.anthropic.com`），
    /// header `x-api-key: <key>` + `anthropic-version: 2023-06-01`；
    /// body: `{ model, max_tokens, system, messages: [{role:"user", content}] }`；
    /// 响应：`{ content: [{type:"text", text:"..."}], usage: { input_tokens, output_tokens }, stop_reason }`。
    async fn generate_json_once_anthropic(
        &self,
        system: &str,
        user: &str,
    ) -> AppResult<(LlmJsonResult, Option<u64>)> {
        let started_at = Instant::now();
        let body = json!({
            "model": self.model,
            "max_tokens": 4096,
            "temperature": 0.2,
            "system": system,
            "messages": [
                {"role": "user", "content": user}
            ]
        });

        let response = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await?;
        let status = response.status();
        let retry_after = parse_retry_after(response.headers());
        let text = match response.bytes().await {
            Ok(buf) => String::from_utf8_lossy(&buf).into_owned(),
            Err(err) => {
                let elapsed_ms = started_at.elapsed().as_millis();
                return Err(AppError::External(format!(
                    "LLM HTTP body_decode_error status={} elapsed_ms={} cause={}",
                    status, elapsed_ms, err
                )));
            }
        };
        if !status.is_success() {
            let mut err = AppError::External(format!("LLM HTTP {status}: {text}"));
            if let Some(after) = retry_after {
                err = AppError::External(format!(
                    "LLM HTTP {status}: {text} [retry_after_secs={after}]"
                ));
            }
            return Err(err);
        }

        let parsed: AnthropicMessageResponse = serde_json::from_str(&text)?;
        let content = parsed
            .content
            .iter()
            .find_map(|block| {
                if block.kind.as_deref() == Some("text") {
                    Some(block.text.as_str())
                } else {
                    None
                }
            })
            .ok_or_else(|| AppError::External("LLM returned no choices".to_string()))?;
        let usage = parsed
            .usage
            .map(|u| ChatUsage {
                prompt_tokens: u.input_tokens,
                completion_tokens: u.output_tokens,
                total_tokens: u.input_tokens.saturating_add(u.output_tokens),
                ..Default::default()
            })
            .unwrap_or_default();
        Ok((
            LlmJsonResult {
                value: parse_json_content(content)?,
                usage,
                latency_ms: started_at.elapsed().as_millis() as i64,
                model: self.model.clone(),
                retry_count: 0,
            },
            None,
        ))
    }
}

#[async_trait]
impl LlmProvider for LlmClient {
    async fn generate_json(&self, system: &str, user: &str) -> AppResult<Value> {
        self.generate_json_with_usage(system, user)
            .await
            .map(|result| result.value)
    }

    async fn generate_json_with_usage(&self, system: &str, user: &str) -> AppResult<LlmJsonResult> {
        let mut last_error: Option<AppError> = None;
        let mut retry_count: u32 = 0;
        for attempt in 1..=self.max_retries {
            match self.generate_json_once(system, user).await {
                Ok((mut value, _)) => {
                    value.retry_count = retry_count;
                    return Ok(value);
                }
                Err(error) if attempt < self.max_retries && is_retryable_llm_error(&error) => {
                    let retry_after_secs = parse_retry_after_from_error(&error);
                    let delay = compute_backoff(attempt, self.retry_base_ms, retry_after_secs);
                    last_error = Some(error);
                    sleep(delay).await;
                    retry_count = retry_count.saturating_add(1);
                }
                Err(error) => {
                    // 重试耗尽（或不可重试的 LLM 错误）—— 把 raw 错误分类成
                    // [`AppError::LlmUnavailable`]，让前端按 `kind` 渲染中文文案，
                    // 而不是把 reqwest 原始 "error sending request for url ..."
                    // 直接糊到面板上。
                    return Err(classify_llm_error_for_user(&error, retry_count));
                }
            }
        }
        // for 循环正常退出（max_retries 用完且最后一次也走了可重试分支但 attempt
        // == max_retries 没机会再 sleep 重试）—— last_error 必有值。
        let final_err = last_error
            .unwrap_or_else(|| AppError::External("LLM request failed after retries".to_string()));
        Err(classify_llm_error_for_user(&final_err, retry_count))
    }
}

/// 把 LLM 调用最终失败的 raw 错误分类为 [`AppError::LlmUnavailable`]，附带
/// 中文 hint，供前端面板按 `kind` 渲染明确文案 + 「AI 重试」按钮。
///
/// 分类来源：
/// - `AppError::Http` → 看 reqwest::Error 的 `is_timeout / is_connect / is_request /
///   is_decode` 标志位；
/// - `AppError::External("LLM HTTP 4xx/5xx ...")` → 解析 status code 段；
/// - `AppError::External("LLM HTTP body_decode_error ...")` → `body_decode_error`；
/// - 其它 → `unknown`。
fn classify_llm_error_for_user(error: &AppError, retry_count: u32) -> AppError {
    let detail = error.to_string();
    let (kind, hint) = match error {
        AppError::Http(err) => {
            if err.is_timeout() {
                (
                    "timeout",
                    "上游 LLM 响应超时，已多次重试仍未收到结果。请稍后再试，或检查到上游服务商的网络链路。",
                )
            } else if err.is_connect() {
                (
                    "connect_failed",
                    "无法连接到上游 LLM 服务，请检查 baseUrl、网络、代理、DNS、TLS 证书是否正常。",
                )
            } else if err.is_decode() {
                (
                    "body_decode_error",
                    "上游 LLM 返回了不完整或非法的响应体，已多次重试。请稍后再试。",
                )
            } else {
                (
                    "network_error",
                    "请求 LLM 时网络出错，已多次重试。请稍后再试或检查网络连通性。",
                )
            }
        }
        AppError::External(msg) => {
            if msg.contains("LLM HTTP 429") {
                (
                    "rate_limited",
                    "上游 LLM 触发限流（429），已多次重试。建议 30 秒后再试，或在 .env 中调高 LLM_RETRY_BASE_MS。",
                )
            } else if msg.contains("LLM HTTP 5") {
                (
                    "http_5xx",
                    "上游 LLM 返回 5xx 错误，已多次重试仍失败。这通常是 LLM 平台侧问题，请稍后再试。",
                )
            } else if msg.contains("LLM HTTP 4") {
                (
                    "http_4xx",
                    "上游 LLM 拒绝了请求（4xx）。请检查 apiKey / model / baseUrl 是否正确、配额是否充足。",
                )
            } else if msg.contains("LLM HTTP body_decode_error") {
                (
                    "body_decode_error",
                    "上游 LLM 返回的响应体在传输中被截断（chunked stream 中断），已多次重试。请稍后再试。",
                )
            } else if msg.contains("LLM returned no choices") {
                (
                    "empty_response",
                    "上游 LLM 返回了空 choices。可能是 prompt 触发了平台过滤策略，请简化措辞后重试。",
                )
            } else {
                ("external_error", "调用 LLM 失败，请稍后再试。")
            }
        }
        AppError::Json(_) => (
            "json_decode_error",
            "上游 LLM 返回了非 JSON 文本，已尝试容错修复仍失败。请「AI 重试」一次。",
        ),
        AppError::BudgetExceeded { .. } => return error_clone_or_external(error),
        _ => ("unknown", "调用 LLM 时出现未知错误，请稍后再试。"),
    };
    AppError::LlmUnavailable {
        kind: kind.to_string(),
        retry_count,
        detail,
        hint: hint.to_string(),
    }
}

/// `AppError` 没实现 `Clone`，但 BudgetExceeded 是结构化 fields，需要原样转出。
/// 简单做法：取它的 Display 字符串包成 External，让上层不丢语义。
fn error_clone_or_external(error: &AppError) -> AppError {
    AppError::External(error.to_string())
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
                || message.contains("LLM HTTP body_decode_error")
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
    match serde_json::from_str::<Value>(json_text) {
        Ok(value) => Ok(value),
        Err(strict_err) => {
            // R15 / ISSUE-006：DeepSeek 偶发输出含 trailing comma 或末尾未闭合
            // 的 JSON（实测 user.reply.task / knowledge.import.preview 都中过）。
            // 严格解析失败时做一次有限容错：去掉 `,` 后跟空白/换行+`}` 或 `]`、
            // 自动补足末尾未闭合的 `]` / `}`。仍失败就把原始严格错误抛出去，
            // 不允许把非 JSON 文本当成 JSON。
            if let Some(repaired) = repair_loose_json(json_text) {
                if let Ok(value) = serde_json::from_str::<Value>(&repaired) {
                    return Ok(value);
                }
            }
            Err(AppError::from(strict_err))
        }
    }
}

/// 修复 LLM 偶发输出的非严格 JSON。只做两类局部修复：
/// 1. trailing comma（`,]` / `,}`）→ 删掉 `,`。
/// 2. 末尾少 `]` / `}` → 按 brackets 计数补足。
///
/// 不做以下"激进"修复：单引号→双引号、未引号 key、注释剥离 —— 这些会让本来
/// 真正非 JSON 的内容被误吞，反而让上游错误难以诊断。
pub(crate) fn repair_loose_json(input: &str) -> Option<String> {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escape = false;
    let mut depth_obj: i32 = 0;
    let mut depth_arr: i32 = 0;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => { in_string = true; out.push(c); }
            '{' => { depth_obj += 1; out.push(c); }
            '}' => { depth_obj -= 1; out.push(c); }
            '[' => { depth_arr += 1; out.push(c); }
            ']' => { depth_arr -= 1; out.push(c); }
            ',' => {
                // peek non-whitespace next char
                let mut peek_iter = chars.clone();
                let mut next_significant = None;
                while let Some(&p) = peek_iter.peek() {
                    if p.is_whitespace() { peek_iter.next(); } else { next_significant = Some(p); break; }
                }
                match next_significant {
                    Some('}') | Some(']') => {
                        // skip the trailing comma
                    }
                    _ => out.push(c),
                }
            }
            _ => out.push(c),
        }
    }
    // 末尾补足未闭合的 `]` `}`（按嵌套深度）。
    while depth_arr > 0 {
        out.push(']');
        depth_arr -= 1;
    }
    while depth_obj > 0 {
        out.push('}');
        depth_obj -= 1;
    }
    if out == input {
        None
    } else {
        Some(out)
    }
}

/// 当前激活的 LLM provider 元数据，便于排障日志写出真实使用的 provider。
#[derive(Debug, Clone)]
pub struct LlmProviderMeta {
    pub provider_id: String,
    pub format: LlmFormat,
    pub model: String,
    pub base_url: String,
}

/// 热替换 LLM 客户端 wrapper。
///
/// 行为：
/// - 持有 `Arc<LlmClient>` + `LlmProviderMeta`，由 `tokio::sync::RwLock` 保护，
///   生产路径只读锁；前端「启用」一条 provider 时取写锁原子替换。
/// - 实现 [`LlmProvider`] 把 `generate_json` / `generate_json_with_usage`
///   转发给当前 client；调用前先 `read().await` 拿一次 `Arc` 克隆再放锁，
///   避免持锁期间发起 HTTP 阻塞 swap。
/// - 不缓存解析结果——只关心客户端实例本身的替换。
pub struct LlmRegistry {
    inner: tokio::sync::RwLock<LlmRegistryInner>,
}

struct LlmRegistryInner {
    client: std::sync::Arc<LlmClient>,
    meta: LlmProviderMeta,
}

impl LlmRegistry {
    pub fn new(client: LlmClient, meta: LlmProviderMeta) -> Self {
        Self {
            inner: tokio::sync::RwLock::new(LlmRegistryInner {
                client: std::sync::Arc::new(client),
                meta,
            }),
        }
    }

    pub async fn current_meta(&self) -> LlmProviderMeta {
        self.inner.read().await.meta.clone()
    }

    /// 用新 client 原子替换当前实例。`active_provider_id` 等元数据由调用方透传。
    pub async fn swap(&self, client: LlmClient, meta: LlmProviderMeta) {
        let mut guard = self.inner.write().await;
        guard.client = std::sync::Arc::new(client);
        guard.meta = meta;
    }

    async fn current(&self) -> std::sync::Arc<LlmClient> {
        self.inner.read().await.client.clone()
    }
}

#[async_trait]
impl LlmProvider for LlmRegistry {
    async fn generate_json(&self, system: &str, user: &str) -> AppResult<Value> {
        let client = self.current().await;
        client.generate_json(system, user).await
    }

    async fn generate_json_with_usage(&self, system: &str, user: &str) -> AppResult<LlmJsonResult> {
        let client = self.current().await;
        client.generate_json_with_usage(system, user).await
    }
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
    fn body_decode_error_is_retryable() {
        // smoke 中观察到 DeepSeek chunked body 偶发中断 ——
        // reqwest 抛 "error decoding response body" 没有上下文。
        // 我们包装成 "LLM HTTP body_decode_error status=... elapsed_ms=... cause=..."
        // 后必须被分类为可重试，避免一条 TCP 抖动让整个 import-preview 直接 502。
        let err = AppError::External(
            "LLM HTTP body_decode_error status=200 elapsed_ms=1830 cause=error decoding response body".to_string(),
        );
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

    /// R15 / ISSUE-006：DeepSeek 偶发输出 trailing comma；parse_json_content
    /// SHALL 在严格解析失败后做一次 trailing-comma 修复，不让一个逗号毁掉整个
    /// run（user.reply.task 失败 → run 整体 502）。
    #[test]
    fn parse_json_content_repairs_trailing_comma_in_object() {
        let v = parse_json_content(r#"{"a": 1, "b": 2,}"#).unwrap();
        assert_eq!(v.get("a").and_then(|x| x.as_i64()), Some(1));
        assert_eq!(v.get("b").and_then(|x| x.as_i64()), Some(2));
    }

    #[test]
    fn parse_json_content_repairs_trailing_comma_in_array() {
        let v = parse_json_content(r#"{"items": [1, 2, 3,]}"#).unwrap();
        assert_eq!(v.get("items").and_then(|x| x.as_array()).unwrap().len(), 3);
    }

    #[test]
    fn parse_json_content_repairs_unclosed_object() {
        // LLM 偶发末尾被截断；尝试补 `}` 救回。
        let v = parse_json_content(r#"{"a": 1, "b": 2"#).unwrap();
        assert_eq!(v.get("b").and_then(|x| x.as_i64()), Some(2));
    }

    #[test]
    fn parse_json_content_does_not_swallow_garbage() {
        // 真的不是 JSON 时仍要报错，避免容错把噪声当数据吞下。
        assert!(parse_json_content("hello world").is_err());
    }

    #[test]
    fn repair_loose_json_keeps_strict_input_unchanged() {
        // 严格合法的 JSON 应直接走 strict 路径，repair 不应改写。
        assert_eq!(repair_loose_json(r#"{"a":1}"#), None);
    }

    #[test]
    fn repair_loose_json_does_not_remove_comma_inside_string() {
        // 字符串里的 `,` 后跟 `}` 是字面量，不能误删。
        let repaired = repair_loose_json(r#"{"x":"a,}b"}"#);
        assert!(repaired.is_none(), "字符串内的 , 不应触发修复");
    }
}
