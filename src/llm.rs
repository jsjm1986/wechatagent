use std::collections::HashMap;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::mpsc::UnboundedSender;
use tokio::time::sleep;

use crate::error::{AppError, AppResult};

const LLM_USER_AGENT: &str = "Mozilla/5.0 wechatagent-llm/1.0";

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
            // 中性协议别名（前端/产品代码只用这些，不出现 LLM 品牌字面量）：
            // "chat" = Chat Completions 协议，"messages" = Messages 协议。
            // 历史品牌值同样兼容，旧库记录与既有测试不受影响。
            "chat" | "openai" | "" => Ok(Self::Openai),
            "messages" | "anthropic" | "claude" => Ok(Self::Anthropic),
            other => Err(AppError::BadRequest(format!(
                "unsupported llm format: {other}"
            ))),
        }
    }

    /// 对外暴露的中性协议名（API/前端用，与 LLM 品牌解耦）。
    /// `as_str()` 仍返回历史品牌值以保持存储/旧测试向后兼容。
    pub fn as_protocol(&self) -> &'static str {
        match self {
            LlmFormat::Openai => "chat",
            LlmFormat::Anthropic => "messages",
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
    /// 采样温度。生产默认 0.2（决策/审查要稳定）。测试侧 roleplayer 用
    /// [`Self::with_temperature`] 调高到 ~0.8 演客户（要有变化、像真人）。
    /// JSON 修复路径（fetch_raw_text）仍用 0.0 不受本字段影响——修复要确定性。
    temperature: f64,
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

/// OpenAI 兼容 `stream: true` 的单条 SSE chunk 形态：
/// `data: {"choices":[{"delta":{"content":"..."}}],"usage":{...}}`。
/// `usage` 一般仅在最后一条（`stream_options.include_usage=true` 时）出现。
#[derive(Debug, Deserialize)]
struct StreamChunkResponse {
    #[serde(default)]
    choices: Vec<StreamChoice>,
    #[serde(default)]
    usage: Option<ChatUsage>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    #[serde(default)]
    delta: StreamDelta,
}

#[derive(Debug, Deserialize, Default)]
struct StreamDelta {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicMessageResponse {
    #[serde(default)]
    content: Vec<AnthropicContentBlock>,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
    /// `end_turn` / `max_tokens` / `tool_use` / `stop_sequence`。长任务实测：claude 偶发
    /// 决定先调工具（`tool_use`）联网搜资料，真内容跑进 tool_use block，text block 只剩
    /// 开场白 → 需识别后给明确诊断，而非含糊的 json_decode。
    #[serde(default)]
    stop_reason: Option<String>,
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

impl From<AnthropicUsage> for ChatUsage {
    fn from(usage: AnthropicUsage) -> Self {
        Self {
            prompt_tokens: usage.input_tokens,
            completion_tokens: usage.output_tokens,
            total_tokens: usage.input_tokens.saturating_add(usage.output_tokens),
            usage_known: true,
            ..Default::default()
        }
    }
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
    /// Whether the upstream response actually contained a usage object.
    ///
    /// A missing usage object is not the same as a measured zero. Some relay
    /// providers omit usage entirely; keeping that distinction prevents cost
    /// and run-budget telemetry from presenting an unknown value as zero.
    #[serde(default)]
    pub usage_known: bool,
}

impl ChatUsage {
    fn reported(mut self) -> Self {
        self.usage_known = true;
        self
    }

    pub fn is_known(&self) -> bool {
        self.usage_known
            || self.prompt_tokens != 0
            || self.completion_tokens != 0
            || self.total_tokens != 0
            || self.prompt_cache_hit_tokens != 0
            || self.prompt_cache_miss_tokens != 0
    }
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

    /// 流式生成：把**上游模型原始输出**（对本系统而言即模型逐 token 吐出的 JSON
    /// 文本片段）作为 delta 通过 `token_tx` 增量推出；函数返回时给出与
    /// [`generate_json_with_usage`] 同形的最终结果（含完整解析后的 `value` 与 usage）。
    ///
    /// 约定 `token_tx` 承载的是**原始 content**（不是解码后的答案正文）。需要"干净
    /// 答案正文"的消费方（如知识库 agent）自行在通道下游跑增量 JSON `answer` 字段
    /// 抽取器，避免把 JSON 语法泄露到前端。
    ///
    /// 默认实现是**非流式兜底**：先 `generate_json_with_usage` 拿到整段结果，再把
    /// 解析后的 `value` 重新序列化成一段 JSON 文本，作为单个 delta 一次性推出 ——
    /// 与真流式路径口径一致（都喂"JSON 文本"给下游抽取器）。这样 `LlmRegistry` /
    /// `TestLlmGenerator` / mockall 生成的 mock 无需改动即可编译；真正的 token 级
    /// 上游 SSE 由 [`LlmClient`] 覆写实现。
    ///
    /// `token_tx` 在前端断开时 `send` 静默失败，不影响最终结果的返回与日志写入。
    async fn generate_json_streaming(
        &self,
        system: &str,
        user: &str,
        token_tx: UnboundedSender<String>,
    ) -> AppResult<LlmJsonResult> {
        let result = self.generate_json_with_usage(system, user).await?;
        if let Ok(raw) = serde_json::to_string(&result.value) {
            if !raw.is_empty() {
                let _ = token_tx.send(raw);
            }
        }
        Ok(result)
    }

    /// 多模态生成：把一张图片（base64 原始字节，无 data-uri 前缀）作为
    /// OpenAI 兼容的 `image_url` content block 与文本一起发给模型，返回解析后的 JSON。
    ///
    /// 默认实现**直接报错**（`vision_not_supported`）而非把 base64 当作文本塞进
    /// prompt —— 后者会让纯文字模型"看不到"图片却又不报错。真正支持视觉的
    /// provider（[`LlmClient`]，OpenAI 格式）覆写此方法发真正的 image content block。
    async fn generate_json_with_image(
        &self,
        _system: &str,
        _user: &str,
        _image_base64: &str,
        _mime: &str,
    ) -> AppResult<Value> {
        Err(AppError::External(
            "vision_not_supported: 当前 provider 未实现多模态图片输入".to_string(),
        ))
    }
}

/// 第三层「回喂 LLM 修复」的最大尝试次数（用户指定 2 次）。前两层（快路径 +
/// `repair_loose_json` + `extract_embedded_json`）全失败才触发。
const REPAIR_MAX_ATTEMPTS: u32 = 2;

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
                // Some OpenAI-compatible gateways reject the default reqwest
                // identity. Keep a stable user agent at the client boundary so
                // text, vision, streaming, Anthropic, and JSON-repair requests
                // all share the same accepted identity without changing their
                // content negotiation (especially SSE streaming).
                .user_agent(LLM_USER_AGENT)
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
                // Windows 上 reqwest 默认自动读 WinHTTP 系统代理（自动检测），
                // 绕过 VPN/浏览器插件的 198.18.x.x 内部地址，导致 LLM 请求 404。
                // 用 .no_proxy() 强制所有连接直连。
                .no_proxy()
                .build()?,
            max_retries: max_retries.max(1),
            retry_base_ms: retry_base_ms.max(100),
            temperature: 0.2,
        })
    }

    /// 链式覆盖采样温度（生产默认 0.2）。测试侧 roleplayer 演客户用 ~0.8。
    /// JSON 修复路径（fetch_raw_text）固定 0.0，不受此影响。
    pub fn with_temperature(mut self, temperature: f64) -> Self {
        self.temperature = temperature;
        self
    }

    /// 解析 `cleaned`（已聚合 SSE、已剥 reasoning 前缀的**干净模型文本**）为 JSON；
    /// 前两层（快路径 / `repair_loose_json` / `extract_embedded_json`）失败后进**第三层**：
    /// 把整段脏文本回喂 LLM 让其修成合法 JSON，最多 [`REPAIR_MAX_ATTEMPTS`] 次。
    ///
    /// 设计依据（本地实测 rsxermu claude-opus）：复杂结构化生成偶发"长度正常、开头合法
    /// `{`、未截断，却中间夹脏字符"——内容完整可读、但前两层的确定性修复救不回。回喂同族
    /// 模型让它重写成严格 JSON，成本不是问题（仅前两层全失败才触发，触发率低）。
    ///
    /// **红线**：回喂 N 次仍失败 → 抛 json_decode 错（测试该红就红 / 上游 skip），把原始脏文本
    /// 前缀与各次修复结果写进错误 detail，便于诊断"模型问题 vs 方法问题"。绝不把非 JSON 当数据吞下。
    async fn parse_or_repair(&self, cleaned: &str) -> AppResult<Value> {
        // 前三层（快路径 + repair_loose_json + extract_embedded_json）。
        if let Ok(value) = parse_json_content(cleaned) {
            return Ok(value);
        }
        // 部分中转网关会用 HTTP 200 + 普通文本返回账户/凭据错误。此时内容并不包含
        // 可修复的 JSON，继续回喂同一端点只会重复计费或重复失败。先转成稳定的非瞬时
        // 错误，让生产诊断和真实模型测试都不会把“欠费/过期”误当成端点抖动。
        if let Some(reason) = account_unavailable_reason(cleaned) {
            return Err(AppError::External(format!(
                "llm_account_unavailable: {reason}"
            )));
        }
        // 第三层：回喂 LLM 修复。每次修复响应只走 parse_json_content（不再回喂，断递归）。
        let mut attempts_diag: Vec<String> = Vec::new();
        for attempt in 1..=REPAIR_MAX_ATTEMPTS {
            match self.repair_via_llm(cleaned).await {
                Ok(value) => return Ok(value),
                Err(e) => attempts_diag.push(format!("repair#{attempt}={e}")),
            }
        }
        // 全部失败：抛严格错误，附原始脏文本前缀 + 各次修复诊断。
        let head: String = cleaned.chars().take(200).collect();
        let strict_err = parse_json_content(cleaned).unwrap_err();
        Err(AppError::External(format!(
            "json_decode after {REPAIR_MAX_ATTEMPTS} llm-repair attempts failed: {strict_err}; \
             raw_head={head:?}; {}",
            attempts_diag.join("; ")
        )))
    }

    /// 第三层修复的单次实现：把脏文本作为 user，配固定「JSON 修复器」system 发一次请求，
    /// 响应只用 `parse_json_content`（**不**再调 `parse_or_repair`，避免无限递归）。
    async fn repair_via_llm(&self, raw_dirty: &str) -> AppResult<Value> {
        const REPAIR_SYSTEM: &str = "你是一个 JSON 修复器。用户会给你一段文本，其中**包含**一个 JSON 对象，但格式可能有误（多余的解释、围栏、全角标点、缺引号、尾逗号、截断等）。请理解其语义，只输出**修正后的、严格合法的单个 JSON 对象**。第一个字符必须是 `{`，最后一个字符必须是 `}`，禁止任何前导/收尾说明、禁止代码块围栏。保持原始内容的字段与取值不变，只修复格式。";
        let cleaned = self.fetch_raw_text(REPAIR_SYSTEM, raw_dirty).await?;
        parse_json_content(&strip_reasoning_prefix(&cleaned))
    }

    /// 发一次请求、按当前 format 取出**纯文本 content**（聚合 SSE、解出信封），不做 JSON 解析。
    /// 专供 [`Self::repair_via_llm`] 复用 HTTP 链路，避免与 `generate_json_once_*` 的递归。
    async fn fetch_raw_text(&self, system: &str, user: &str) -> AppResult<String> {
        match self.format {
            LlmFormat::Openai => {
                let body = json!({
                    "model": self.model,
                    "temperature": 0.0,
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
                let text = String::from_utf8_lossy(&response.bytes().await?).into_owned();
                if !status.is_success() {
                    return Err(AppError::External(format!("LLM HTTP {status}: {text}")));
                }
                if is_openai_sse_body(&text) {
                    let (acc, _) = aggregate_openai_sse(&text);
                    if acc.trim().is_empty() {
                        return Err(AppError::External(
                            "LLM SSE body 聚合后内容为空".to_string(),
                        ));
                    }
                    Ok(acc)
                } else {
                    let parsed: ChatCompletionResponse = serde_json::from_str(&text)?;
                    parsed
                        .choices
                        .first()
                        .map(|choice| choice.message.content.clone())
                        .ok_or_else(|| AppError::External("LLM returned no choices".to_string()))
                }
            }
            LlmFormat::Anthropic => {
                let body = json!({
                    "model": self.model,
                    "max_tokens": 8192,
                    "temperature": 0.0,
                    "system": system,
                    "messages": [ {"role": "user", "content": user} ]
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
                let text = String::from_utf8_lossy(&response.bytes().await?).into_owned();
                if !status.is_success() {
                    return Err(AppError::External(format!("LLM HTTP {status}: {text}")));
                }
                let parsed: AnthropicMessageResponse = serde_json::from_str(&text)?;
                parsed
                    .content
                    .iter()
                    .find_map(|block| {
                        (block.kind.as_deref() == Some("text")).then(|| block.text.clone())
                    })
                    .ok_or_else(|| AppError::External("LLM returned no choices".to_string()))
            }
        }
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
            "temperature": self.temperature,
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

        // 兼容两种 OpenAI 响应：标准非流式 JSON（NVIDIA/deepseek 等）与强制
        // 流式 SSE（部分中转网关即使不传 stream 也只回 `data: {chunk}`）。
        // 检测到 SSE 帧则聚合 delta.content；否则按原 ChatCompletionResponse 解析。
        // 非 SSE 响应零行为变化。
        let (content, usage) = if is_openai_sse_body(&text) {
            let (acc, sse_usage) = aggregate_openai_sse(&text);
            if acc.trim().is_empty() {
                return Err(AppError::External(
                    "LLM SSE body 聚合后内容为空".to_string(),
                ));
            }
            (acc, sse_usage.map(ChatUsage::reported).unwrap_or_default())
        } else {
            let parsed: ChatCompletionResponse = serde_json::from_str(&text)?;
            let c = parsed
                .choices
                .first()
                .map(|choice| choice.message.content.clone())
                .ok_or_else(|| AppError::External("LLM returned no choices".to_string()))?;
            (c, parsed.usage.map(ChatUsage::reported).unwrap_or_default())
        };
        Ok((
            LlmJsonResult {
                value: self
                    .parse_or_repair(&strip_reasoning_prefix(&content))
                    .await?,
                usage,
                latency_ms: started_at.elapsed().as_millis() as i64,
                model: self.model.clone(),
                retry_count: 0,
            },
            None,
        ))
    }

    /// OpenAI 兼容多模态：user message 的 content 用数组形式
    /// `[{type:"text",...},{type:"image_url",image_url:{url:"data:<mime>;base64,<b64>"}}]`，
    /// 让 vision-capable 模型真正"看到"图片，而不是把 base64 当文本。
    async fn generate_json_once_openai_vision(
        &self,
        system: &str,
        user: &str,
        image_base64: &str,
        mime: &str,
    ) -> AppResult<LlmJsonResult> {
        let started_at = Instant::now();
        let data_uri = format!("data:{mime};base64,{image_base64}");
        let body = json!({
            "model": self.model,
            "temperature": self.temperature,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": [
                    { "type": "text", "text": user },
                    { "type": "image_url", "image_url": { "url": data_uri } }
                ]}
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
        let text = match response.bytes().await {
            Ok(buf) => String::from_utf8_lossy(&buf).into_owned(),
            Err(err) => {
                let elapsed_ms = started_at.elapsed().as_millis();
                return Err(AppError::External(format!(
                    "LLM HTTP body_decode_error status={status} elapsed_ms={elapsed_ms} cause={err}"
                )));
            }
        };
        if !status.is_success() {
            return Err(AppError::External(format!("LLM HTTP {status}: {text}")));
        }
        // 与文本路径 generate_json_once_openai 同口径：兼容标准非流式 JSON 与强制
        // 流式 SSE（部分中转网关即使不传 stream 也只回 `data: {chunk}`，如 rsxermu
        // gpt-5.4）。检测到 SSE 帧则聚合 delta.content；否则按原 ChatCompletionResponse
        // 解析。非 SSE 响应零行为变化。
        let (content, usage) = if is_openai_sse_body(&text) {
            let (acc, sse_usage) = aggregate_openai_sse(&text);
            if acc.trim().is_empty() {
                return Err(AppError::External(
                    "LLM SSE body 聚合后内容为空".to_string(),
                ));
            }
            (acc, sse_usage.map(ChatUsage::reported).unwrap_or_default())
        } else {
            let parsed: ChatCompletionResponse = serde_json::from_str(&text)?;
            let c = parsed
                .choices
                .first()
                .map(|choice| choice.message.content.clone())
                .ok_or_else(|| AppError::External("LLM returned no choices".to_string()))?;
            (c, parsed.usage.map(ChatUsage::reported).unwrap_or_default())
        };
        Ok(LlmJsonResult {
            value: self
                .parse_or_repair(&strip_reasoning_prefix(&content))
                .await?,
            usage,
            latency_ms: started_at.elapsed().as_millis() as i64,
            model: self.model.clone(),
            retry_count: 0,
        })
    }
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
        // claude（尤其 opus）对话遵从性强，遇到口语化/对话式 prompt（"你好，我需要你帮我…"）
        // 容易"入戏"先写共情散文再给 JSON，甚至 JSON 被 max_tokens 截断（rsxermu CI 实测
        // domain_profile/knowledge 大面积 json_decode：content 全是"我理解你的需求…让我生成"）。
        // 更严重的长任务坑（2026-06-17 本地实测）：复杂"极其详尽"prompt 会让 claude 触发
        // **工具调用**（stop_reason=tool_use，如 WebFetch 联网搜资料），真内容跑进 tool_use
        // block，text block 只剩一句开场白 → 我们只取 text 自然拿不到，json_decode。
        // 在 system 末尾追加**强制 JSON 输出约束 + 禁工具/对话模式声明**（通用、不改各调用方
        // prompt），逼 claude 第一字符即 `{`、禁任何前导/解释/收尾、禁 tool_use。实测加禁工具
        // 声明后 stop_reason 从 tool_use 回到 end_turn、内容回到 text block。下游 parse_json_content
        // + 第三层回喂修复仍兜底防漏网。
        const ANTHROPIC_JSON_GUARD: &str = "\n\n[OUTPUT FORMAT — STRICT] 当前是**对话生成模式**，不是 agent / 工具调用模式。禁止调用任何工具（不要 WebFetch、不要联网搜索、不要任何 tool_use），直接基于你已有的知识一次性生成完整内容。你必须只输出一个 JSON 对象，不要任何前导说明、寒暄、共情、思考过程或代码块围栏。第一个字符必须是 `{`，最后一个字符必须是 `}`。禁止在 JSON 前后写任何自然语言（包括「好的」「我理解」「让我」「希望有帮助」之类）。";
        let guarded_system = format!("{system}{ANTHROPIC_JSON_GUARD}");
        let body = json!({
            "model": self.model,
            "max_tokens": 8192,
            "temperature": self.temperature,
            "system": guarded_system,
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
        // 长任务防御：claude 偶发无视禁工具约束、决定先 tool_use（如 WebFetch 搜资料），
        // 真内容跑进 tool_use block，text block 只剩开场白。识别后给**明确诊断**，而非让
        // 下游对半句开场白做 json_decode/回喂修复（既浪费一轮修复、错误又含糊）。
        if let Some(diag) = detect_tool_use_hijack(&parsed) {
            return Err(AppError::External(diag));
        }
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
        let usage = parsed.usage.map(ChatUsage::from).unwrap_or_default();
        let cleaned = strip_reasoning_prefix(content);
        let value = self.parse_or_repair(&cleaned).await?;
        Ok((
            LlmJsonResult {
                value,
                usage,
                latency_ms: started_at.elapsed().as_millis() as i64,
                model: self.model.clone(),
                retry_count: 0,
            },
            None,
        ))
    }

    /// OpenAI 兼容 `stream: true` 的真流式实现：消费 `bytes_stream()`，按 SSE
    /// `\n\n` 事件边界增量解析 `choices[0].delta.content`，逐 token 通过
    /// `token_tx` 推出（**原始 content 片段**，由下游决定如何抽取答案正文），
    /// 累积成完整文本后 `parse_json_content` 得到最终 `value`。
    ///
    /// `stream_options.include_usage=true` 让上游在最后一条 chunk 带 usage；
    /// 若上游不支持则 usage fallback 到 `ChatUsage::default()`（token 计费按 0 计，
    /// 由 budget 的整体上限兜底，不影响功能）。
    ///
    /// 单次尝试、不走 `generate_json_with_usage` 的重试循环 —— 流式一旦开始
    /// 推 token，重试会导致前端重复/错乱；HTTP/1.1 + keepalive 已稳定链路，
    /// 失败直接上抛由调用方降级。
    async fn generate_json_streaming_openai(
        &self,
        system: &str,
        user: &str,
        token_tx: &UnboundedSender<String>,
    ) -> AppResult<LlmJsonResult> {
        let started_at = Instant::now();
        let body = json!({
            "model": self.model,
            "temperature": self.temperature,
            "stream": true,
            "stream_options": {"include_usage": true},
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
        if !status.is_success() {
            let retry_after = parse_retry_after(response.headers());
            let text = match response.bytes().await {
                Ok(buf) => String::from_utf8_lossy(&buf).into_owned(),
                Err(err) => format!("<body read failed: {err}>"),
            };
            let mut err = AppError::External(format!("LLM HTTP {status}: {text}"));
            if let Some(after) = retry_after {
                err = AppError::External(format!(
                    "LLM HTTP {status}: {text} [retry_after_secs={after}]"
                ));
            }
            return Err(err);
        }

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut accumulated = String::new();
        let mut usage: Option<ChatUsage> = None;

        'outer: while let Some(chunk) = stream.next().await {
            let bytes = chunk.map_err(|err| {
                AppError::External(format!(
                    "LLM HTTP body_decode_error status={} elapsed_ms={} cause={}",
                    status,
                    started_at.elapsed().as_millis(),
                    err
                ))
            })?;
            // 归一化 CRLF，确保后续以 `\n\n` 切分事件边界对 `\r\n\r\n` 服务也成立。
            buffer.push_str(&String::from_utf8_lossy(&bytes).replace("\r\n", "\n"));

            // SSE 事件以空行分隔；逐个抽出完整事件，残缺片段留在 buffer 等下一帧。
            while let Some(idx) = buffer.find("\n\n") {
                let event: String = buffer.drain(..idx + 2).collect();
                for line in event.lines() {
                    let data = match line.trim().strip_prefix("data:") {
                        Some(rest) => rest.trim(),
                        None => continue,
                    };
                    if data == "[DONE]" {
                        break 'outer;
                    }
                    if data.is_empty() {
                        continue;
                    }
                    // 容忍单条 chunk 解析失败（keepalive 注释 / 非标准心跳行）：跳过。
                    let parsed: StreamChunkResponse = match serde_json::from_str(data) {
                        Ok(p) => p,
                        Err(_) => continue,
                    };
                    if let Some(u) = parsed.usage {
                        usage = Some(u.reported());
                    }
                    if let Some(content) = parsed
                        .choices
                        .first()
                        .and_then(|choice| choice.delta.content.as_ref())
                    {
                        if !content.is_empty() {
                            accumulated.push_str(content);
                            let _ = token_tx.send(content.clone());
                        }
                    }
                }
            }
        }

        // M10：与非流式路径对齐——三层确定性解析全失败时再走第四层 LLM-repair
        // （对已累积完的文本发独立修复请求，不向 token_tx 再推 token、不 re-stream）。
        let value = self.parse_or_repair(&accumulated).await?;
        Ok(LlmJsonResult {
            value,
            usage: usage.unwrap_or_default(),
            latency_ms: started_at.elapsed().as_millis() as i64,
            model: self.model.clone(),
            retry_count: 0,
        })
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

    async fn generate_json_streaming(
        &self,
        system: &str,
        user: &str,
        token_tx: UnboundedSender<String>,
    ) -> AppResult<LlmJsonResult> {
        match self.format {
            // OpenAI 兼容端点走真 SSE 流式。
            LlmFormat::Openai => self
                .generate_json_streaming_openai(system, user, &token_tx)
                .await
                .map_err(|err| classify_llm_error_for_user(&err, 0)),
            // Anthropic 形态暂未实现 SSE 解析 —— 退回 trait 默认的非流式兜底
            // （整段一次性推），保持答案正确，仅失去 token 级增量。
            LlmFormat::Anthropic => {
                let result = self.generate_json_with_usage(system, user).await?;
                if let Ok(raw) = serde_json::to_string(&result.value) {
                    if !raw.is_empty() {
                        let _ = token_tx.send(raw);
                    }
                }
                Ok(result)
            }
        }
    }

    async fn generate_json_with_image(
        &self,
        system: &str,
        user: &str,
        image_base64: &str,
        mime: &str,
    ) -> AppResult<Value> {
        match self.format {
            // vision 走与文本同款的重试循环（此前单次调用、retry_count=0，端点偶发
            // 5xx/524 直接 skip 假绿——CI 实测 T3/K6/Q3 因此从不真跑）。复用
            // is_retryable_llm_error + compute_backoff，让 vision 也能熬过端点抖动。
            LlmFormat::Openai => {
                let mut last_error: Option<AppError> = None;
                let mut retry_count: u32 = 0;
                for attempt in 1..=self.max_retries {
                    match self
                        .generate_json_once_openai_vision(system, user, image_base64, mime)
                        .await
                    {
                        Ok(r) => return Ok(r.value),
                        Err(error)
                            if attempt < self.max_retries && is_retryable_llm_error(&error) =>
                        {
                            let retry_after_secs = parse_retry_after_from_error(&error);
                            let delay =
                                compute_backoff(attempt, self.retry_base_ms, retry_after_secs);
                            last_error = Some(error);
                            sleep(delay).await;
                            retry_count = retry_count.saturating_add(1);
                        }
                        Err(error) => {
                            return Err(classify_llm_error_for_user(&error, retry_count));
                        }
                    }
                }
                let final_err = last_error.unwrap_or_else(|| {
                    AppError::External("vision request failed after retries".to_string())
                });
                Err(classify_llm_error_for_user(&final_err, retry_count))
            }
            // Anthropic 的图片 block 形态（source.type=base64）与 OpenAI 不同，
            // 当前仅支持 OpenAI 兼容视觉端点；Anthropic 视觉留待后续。
            LlmFormat::Anthropic => Err(AppError::External(
                "vision_not_supported: Anthropic 格式的多模态输入尚未实现".to_string(),
            )),
        }
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
            if msg.contains("llm_account_unavailable") {
                (
                    "account_unavailable",
                    "上游 LLM 账户不可用（余额不足、欠费或凭据已过期）。请恢复额度或更新凭据后重试。",
                )
            } else if msg.contains("LLM HTTP 429") {
                (
                    "rate_limited",
                    "上游 LLM 触发限流（429），已多次重试。建议 30 秒后再试，或在 .env 中调高 LLM_RETRY_BASE_MS。",
                )
            } else if msg.contains("LLM HTTP 5") {
                (
                    "http_5xx",
                    "上游 LLM 返回 5xx 错误，已多次重试仍失败。这通常是 LLM 平台侧问题，请稍后再试。",
                )
            } else if msg.contains("LLM HTTP 404") {
                (
                    "endpoint_not_found",
                    "上游返回 404：baseUrl 路径不对。系统会在 baseUrl 后直接拼 /chat/completions，请填服务商的「OpenAI 兼容 base_url」原文，不要自行增删路径。阿里云百炼 Qwen 须为 https://dashscope.aliyuncs.com/compatible-mode/v1（注意 /compatible-mode）；DeepSeek 为 https://api.deepseek.com/v1。",
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

/// 真实模型测试只允许这些真正的瞬时基础设施错误跳过。未知、新增或账户/配置类
/// 错误默认返回 false（fail closed），避免测试框架显示绿色但业务链路根本没运行。
pub fn is_transient_llm_unavailable_kind(kind: &str) -> bool {
    matches!(
        kind,
        "rate_limited"
            | "http_5xx"
            | "timeout"
            | "connect_failed"
            | "network_error"
            | "body_decode_error"
    )
}

fn account_unavailable_reason(content: &str) -> Option<&'static str> {
    let normalized = content.trim().to_ascii_lowercase();
    let balance_unavailable = content.contains("余额不足")
        || content.contains("欠费")
        || normalized.contains("insufficient balance")
        || normalized.contains("insufficient credit")
        || normalized.contains("payment required")
        || normalized.contains("arrearage");
    if balance_unavailable {
        return Some("insufficient_balance");
    }

    let credential_unavailable = content.contains("密钥已过期")
        || content.contains("密钥过期")
        || content.contains("API Key 已过期")
        || normalized.contains("api key has expired")
        || normalized.contains("expired api key");
    credential_unavailable.then_some("credential_expired")
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
                // Cloudflare 源站层 5xx（520 unknown / 522 connection timed out /
                // 524 a timeout occurred）：经 CF 的端点（如 rsxermu）在源站慢/抖时回这些，
                // 属瞬时不可达应重试。此前漏列 524，与下方 classify_llm_error_for_user 把
                // 任何 "LLM HTTP 5*" 归 http_5xx 的口径不一致——一条 CF 524 不重试直接冒泡。
                || message.contains("LLM HTTP 520")
                || message.contains("LLM HTTP 522")
                || message.contains("LLM HTTP 524")
                || message.contains("LLM HTTP body_decode_error")
                // tool_use 劫持：claude 偶发无视 ANTHROPIC_JSON_GUARD 的禁工具约束,返回
                // tool_use block 而非 JSON(detect_tool_use_hijack 抛此诊断,~25% 长任务高发)。
                // 是"同输入重跑通常成功"的瞬态模型不遵从,与 HTTP 5xx 同属应熬过的抖动——此前
                // 漏列致一次冒泡(最该重试却没重试)。重试走指数退避,耗尽仍劫持才抛(行为不退化)。
                || message.contains("llm_tool_use_instead_of_json")
        }
        _ => false,
    }
}

/// 指数退避带 jitter，并尊重 Retry-After。
pub fn compute_backoff(attempt: u32, base_ms: u64, retry_after_secs: Option<u64>) -> Duration {
    // 单次退避封顶 60s：指数退避 base*2^(attempt-1) 在高 attempt 下会爆到几百秒
    // （2500*2^10≈42min），rsxermu 单点 503/超时需要"加大重试次数 + 每次封顶"才能在合理
    // 总时长内熬过端点抖动拿真分，而非单次 sleep 几十分钟撞 job 墙。Retry-After 头若更长则
    // 尊重它（端点明确要求等多久就等多久）。
    const MAX_BACKOFF_MS: u64 = 60_000;
    let shift = attempt.saturating_sub(1).min(10);
    let exp_ms = base_ms.saturating_mul(1u64 << shift);
    let jitter = fastrand_jitter(base_ms);
    let backoff_ms = exp_ms.saturating_add(jitter).min(MAX_BACKOFF_MS);
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

/// 检测 OpenAI 响应体是否是 SSE 流式（`data: {...}` 帧）而非标准 JSON。
/// 部分中转网关即使请求不带 stream 也强制回流式，需聚合后才能解析。
fn is_openai_sse_body(text: &str) -> bool {
    text.lines()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim_start().starts_with("data:"))
        .unwrap_or(false)
}

/// 聚合 OpenAI SSE 帧的 `choices[0].delta.content`，返回拼接内容 + usage。
/// 复用 [`StreamChunkResponse`]（与 token 级 streaming 同结构）。单帧解析失败跳过。
fn aggregate_openai_sse(text: &str) -> (String, Option<ChatUsage>) {
    let mut acc = String::new();
    let mut usage: Option<ChatUsage> = None;
    for line in text.lines() {
        let line = line.trim();
        let data = match line.strip_prefix("data:") {
            Some(rest) => rest.trim(),
            None => continue,
        };
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let parsed: StreamChunkResponse = match serde_json::from_str(data) {
            Ok(p) => p,
            Err(_) => continue,
        };
        if let Some(u) = parsed.usage {
            usage = Some(u.reported());
        }
        if let Some(c) = parsed
            .choices
            .first()
            .and_then(|ch| ch.delta.content.as_ref())
        {
            acc.push_str(c);
        }
    }
    (acc, usage)
}

/// 剥离部分模型在 JSON 前输出的 `<think>...</think>` 推理前缀（如此端点的 gpt 系）。
/// 无前缀时原样返回，对其它模型零影响。
fn strip_reasoning_prefix(content: &str) -> String {
    let trimmed = content.trim_start();
    if let Some(rest) = trimmed.strip_prefix("<think>") {
        if let Some(end) = rest.find("</think>") {
            return rest[end + "</think>".len()..].trim_start().to_string();
        }
    }
    content.to_string()
}

/// 检测 anthropic 响应是否被**工具调用劫持**：长复杂任务中 claude 偶发决定先 tool_use
/// （如 WebFetch 联网搜资料），把真内容放进 tool_use block，text block 只剩一句开场白。
/// 命中（stop_reason=tool_use 或存在 tool_use block）则返回明确诊断字符串，供上层抛错——
/// 比让下游对半句开场白做 json_decode/回喂修复更清晰，CI 日志一眼可辨。返回 None = 正常。
fn detect_tool_use_hijack(parsed: &AnthropicMessageResponse) -> Option<String> {
    let has_tool_use = parsed
        .content
        .iter()
        .any(|b| b.kind.as_deref() == Some("tool_use"));
    if parsed.stop_reason.as_deref() != Some("tool_use") && !has_tool_use {
        return None;
    }
    let head: String = parsed
        .content
        .iter()
        .find_map(|b| (b.kind.as_deref() == Some("text")).then(|| b.text.clone()))
        .unwrap_or_default()
        .chars()
        .take(120)
        .collect();
    Some(format!(
        "llm_tool_use_instead_of_json: 模型返回 tool_use 而非 JSON（长任务偶发；已在 system \
         禁工具，仍出现说明该上游/模型未遵从）。text_head={head:?}"
    ))
}

/// 把 claude 偶发的"单元素对象数组" `[{...}]` 拆成内部对象——generate_json 的契约是 JSON
/// 对象（下游 to_document 拒收顶层数组），claude-opus 经中转有时把唯一对象包进数组。仅拆
/// `len==1 且元素为对象` 的数组，其它（多元素数组 / 元素非对象 / 非数组）原样返回，避免误伤
/// 真正期望数组的场景。
fn normalize_singleton_array(value: Value) -> Value {
    if let Value::Array(arr) = &value {
        if arr.len() == 1 && arr[0].is_object() {
            if let Value::Array(mut arr) = value {
                return arr.remove(0);
            }
        }
    }
    value
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
    // 快路径：整体就是合法 JSON（绝大多数 provider 的纯 JSON 输出）。
    if let Ok(value) = serde_json::from_str::<Value>(json_text) {
        return Ok(normalize_singleton_array(value));
    }
    // 容错 1：trailing comma / 末尾未闭合（R15 / ISSUE-006，DeepSeek 偶发）。
    if let Some(repaired) = repair_loose_json(json_text) {
        if let Ok(value) = serde_json::from_str::<Value>(&repaired) {
            return Ok(normalize_singleton_array(value));
        }
    }
    // 容错 2：claude-opus 经中转常输出"自然语言推理 + JSON"混合体，JSON 可能夹在推理后
    // （rsxermu CI 实测：`我看到候选 catalog…\n\n{"action":"open_chunk",...}`）。遍历所有
    // `{`/`[` 起点，对每个配平块尝试严格解析 + repair，返回**首个解析成功**的块——推理里的
    // 伪括号截出来解析失败会被跳过，命中真正的 JSON。纯 JSON 场景走不到这里（已快路径返回）。
    if let Some(value) = extract_embedded_json(json_text) {
        return Ok(value);
    }
    // 全部失败：抛严格错误，不把非 JSON 文本当数据吞下。
    Err(AppError::from(
        serde_json::from_str::<Value>(json_text).unwrap_err(),
    ))
}

/// 从混合文本中提取**首个可解析**的 JSON 对象/数组：遍历每个 `{`/`[` 起点配平截块，
/// 逐个尝试严格解析（失败再试 repair）。**优先返回对象**——generate_json 的契约是 JSON
/// 对象，claude 偶发把对象包成单元素数组 `[{...}]` 或先输出推理里的伪数组，故对象命中即返；
/// 仅当全程没有对象、只有数组时才返数组（且单元素对象数组拆出内部对象，对齐 to_document）。
/// 用于 claude "推理 + JSON" 混合输出——推理里的伪括号块解析失败被跳过，命中真 JSON。
fn extract_embedded_json(text: &str) -> Option<Value> {
    let bytes = text.as_bytes();
    let mut array_fallback: Option<Value> = None;
    for start in 0..bytes.len() {
        if bytes[start] != b'{' && bytes[start] != b'[' {
            continue;
        }
        let Some(block) = balanced_block(text, start) else {
            continue;
        };
        let parsed = serde_json::from_str::<Value>(block).ok().or_else(|| {
            repair_loose_json(block).and_then(|r| serde_json::from_str::<Value>(&r).ok())
        });
        match parsed {
            // 对象：generate_json 期望形态，立即返回。
            Some(v @ Value::Object(_)) => return Some(v),
            // 数组：暂存作兜底；若是 `[{...}]` 单对象数组，拆出内部对象优先（claude 常见包法）。
            Some(Value::Array(arr)) if array_fallback.is_none() => {
                if arr.len() == 1 && arr[0].is_object() {
                    return Some(arr.into_iter().next().unwrap());
                }
                array_fallback = Some(Value::Array(arr));
            }
            _ => {}
        }
    }
    array_fallback
}

/// 从 `start`（须是 `{` 或 `[`）按括号配平扫描（跳过字符串字面量内括号与转义），返回到
/// 配对闭合符的子串。未闭合返回 None。
fn balanced_block(text: &str, start: usize) -> Option<&str> {
    let bytes = text.as_bytes();
    let open = bytes[start];
    let close = if open == b'{' { b'}' } else { b']' };
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut escape = false;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_string {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            x if x == open => depth += 1,
            x if x == close => {
                depth -= 1;
                if depth == 0 {
                    return text.get(start..=i);
                }
            }
            _ => {}
        }
    }
    None
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
            if escape {
                out.push(c);
                escape = false;
                continue;
            }
            match c {
                '\\' => {
                    out.push(c);
                    escape = true;
                }
                '"' => {
                    out.push(c);
                    in_string = false;
                }
                // 真模型偶发在字符串值里塞**裸控制字符**（未转义的换行/制表符等），
                // serde 严格模式直接拒收（"control character (U+0000-U+001F) found
                // while parsing a string"）。这里把它们转义成合法 JSON 转义序列——
                // 只改**表示形式**不改字符串语义，符合"只容错等价表达"红线。
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                '\u{0008}' => out.push_str("\\b"),
                '\u{000C}' => out.push_str("\\f"),
                other if (other as u32) < 0x20 => {
                    out.push_str(&format!("\\u{:04x}", other as u32));
                }
                other => out.push(other),
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '{' => {
                depth_obj += 1;
                out.push(c);
            }
            '}' => {
                depth_obj -= 1;
                out.push(c);
            }
            '[' => {
                depth_arr += 1;
                out.push(c);
            }
            ']' => {
                depth_arr -= 1;
                out.push(c);
            }
            ',' => {
                // peek non-whitespace next char
                let mut peek_iter = chars.clone();
                let mut next_significant = None;
                while let Some(&p) = peek_iter.peek() {
                    if p.is_whitespace() {
                        peek_iter.next();
                    } else {
                        next_significant = Some(p);
                        break;
                    }
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

/// Workspace-scoped hot-swappable LLM client registry.
///
/// 行为：
/// - 持有 `Arc<LlmClient>` + `LlmProviderMeta`，由 `tokio::sync::RwLock` 保护，
///   生产路径只读锁；前端「启用」一条 provider 时取写锁原子替换。
/// - 实现 [`LlmProvider`] 把 `generate_json` / `generate_json_with_usage`
///   转发给当前 client；调用前先 `read().await` 拿一次 `Arc` 克隆再放锁，
///   避免持锁期间发起 HTTP 阻塞 swap。
/// - 不缓存解析结果——只关心客户端实例本身的替换。
pub struct LlmRegistry {
    default_workspace_id: String,
    inner: tokio::sync::RwLock<HashMap<String, LlmRegistryInner>>,
}

struct LlmRegistryInner {
    client: std::sync::Arc<LlmClient>,
    meta: LlmProviderMeta,
    generation: u64,
}

/// Immutable view of one registry generation.
///
/// A request that obtains this snapshot keeps the matching client, metadata,
/// and generation even if an administrator hot-swaps the registry while the
/// request is in flight. This gives exact-cache lookup and the upstream call
/// the same linearization point.
#[derive(Clone)]
pub struct LlmRegistrySnapshot {
    client: std::sync::Arc<LlmClient>,
    pub meta: LlmProviderMeta,
    pub generation: u64,
}

impl LlmRegistrySnapshot {
    pub async fn generate_json(&self, system: &str, user: &str) -> AppResult<Value> {
        self.client.generate_json(system, user).await
    }

    pub async fn generate_json_with_usage(
        &self,
        system: &str,
        user: &str,
    ) -> AppResult<LlmJsonResult> {
        self.client.generate_json_with_usage(system, user).await
    }

    pub async fn generate_json_streaming(
        &self,
        system: &str,
        user: &str,
        token_tx: UnboundedSender<String>,
    ) -> AppResult<LlmJsonResult> {
        self.client
            .generate_json_streaming(system, user, token_tx)
            .await
    }

    pub async fn generate_json_with_image(
        &self,
        system: &str,
        user: &str,
        image_base64: &str,
        mime: &str,
    ) -> AppResult<Value> {
        self.client
            .generate_json_with_image(system, user, image_base64, mime)
            .await
    }
}

#[async_trait]
impl LlmProvider for LlmRegistrySnapshot {
    async fn generate_json(&self, system: &str, user: &str) -> AppResult<Value> {
        LlmRegistrySnapshot::generate_json(self, system, user).await
    }

    async fn generate_json_with_usage(&self, system: &str, user: &str) -> AppResult<LlmJsonResult> {
        LlmRegistrySnapshot::generate_json_with_usage(self, system, user).await
    }

    async fn generate_json_streaming(
        &self,
        system: &str,
        user: &str,
        token_tx: UnboundedSender<String>,
    ) -> AppResult<LlmJsonResult> {
        LlmRegistrySnapshot::generate_json_streaming(self, system, user, token_tx).await
    }

    async fn generate_json_with_image(
        &self,
        system: &str,
        user: &str,
        image_base64: &str,
        mime: &str,
    ) -> AppResult<Value> {
        LlmRegistrySnapshot::generate_json_with_image(self, system, user, image_base64, mime).await
    }
}

impl LlmRegistry {
    pub fn new(
        default_workspace_id: impl Into<String>,
        client: LlmClient,
        meta: LlmProviderMeta,
    ) -> Self {
        let default_workspace_id = default_workspace_id.into();
        let mut entries = HashMap::new();
        entries.insert(
            default_workspace_id.clone(),
            LlmRegistryInner {
                client: std::sync::Arc::new(client),
                meta,
                generation: 0,
            },
        );
        Self {
            default_workspace_id,
            inner: tokio::sync::RwLock::new(entries),
        }
    }

    pub async fn current_meta(&self, workspace_id: &str) -> Option<LlmProviderMeta> {
        self.inner
            .read()
            .await
            .get(workspace_id)
            .map(|entry| entry.meta.clone())
    }

    pub async fn snapshot(&self, workspace_id: &str) -> AppResult<LlmRegistrySnapshot> {
        let guard = self.inner.read().await;
        let entry = guard
            .get(workspace_id)
            .ok_or_else(|| AppError::LlmUnavailable {
                kind: "workspace_provider_missing".to_string(),
                detail: format!("no active LLM provider is loaded for workspace {workspace_id}"),
                hint: "configure and activate an LLM provider for this workspace".to_string(),
                retry_count: 0,
            })?;
        Ok(LlmRegistrySnapshot {
            client: entry.client.clone(),
            meta: entry.meta.clone(),
            generation: entry.generation,
        })
    }

    /// Atomically replace one workspace client. A newly loaded workspace starts
    /// at generation 0; subsequent replacements increment only that workspace.
    pub async fn swap(&self, workspace_id: &str, client: LlmClient, meta: LlmProviderMeta) -> u64 {
        let mut guard = self.inner.write().await;
        match guard.get_mut(workspace_id) {
            Some(entry) => {
                entry.client = std::sync::Arc::new(client);
                entry.meta = meta;
                entry.generation = entry
                    .generation
                    .checked_add(1)
                    .expect("LLM registry generation overflow");
                entry.generation
            }
            None => {
                guard.insert(
                    workspace_id.to_string(),
                    LlmRegistryInner {
                        client: std::sync::Arc::new(client),
                        meta,
                        generation: 0,
                    },
                );
                0
            }
        }
    }

    async fn current(&self) -> std::sync::Arc<LlmClient> {
        self.snapshot(&self.default_workspace_id)
            .await
            .expect("default workspace LLM provider must remain loaded")
            .client
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

    async fn generate_json_streaming(
        &self,
        system: &str,
        user: &str,
        token_tx: UnboundedSender<String>,
    ) -> AppResult<LlmJsonResult> {
        let client = self.current().await;
        client.generate_json_streaming(system, user, token_tx).await
    }

    async fn generate_json_with_image(
        &self,
        system: &str,
        user: &str,
        image_base64: &str,
        mime: &str,
    ) -> AppResult<Value> {
        let client = self.current().await;
        client
            .generate_json_with_image(system, user, image_base64, mime)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry_test_client(model: &str) -> LlmClient {
        LlmClient::new(
            "http://127.0.0.1:1".to_string(),
            "test-key".to_string(),
            model.to_string(),
            1,
            1,
            100,
        )
        .expect("build registry test client")
    }

    fn registry_test_meta(provider: &str, model: &str) -> LlmProviderMeta {
        LlmProviderMeta {
            provider_id: provider.to_string(),
            format: LlmFormat::Openai,
            model: model.to_string(),
            base_url: "http://127.0.0.1:1".to_string(),
        }
    }

    #[tokio::test]
    async fn registry_snapshots_pin_client_identity_and_swap_generation() {
        let registry = LlmRegistry::new(
            "ws-a",
            registry_test_client("model-a"),
            registry_test_meta("provider-a", "model-a"),
        );
        let before = registry.snapshot("ws-a").await.unwrap();
        assert_eq!(before.generation, 0);
        assert_eq!(before.meta.provider_id, "provider-a");
        assert_eq!(before.client.model, "model-a");

        let generation = registry
            .swap(
                "ws-a",
                registry_test_client("model-b"),
                registry_test_meta("provider-b", "model-b"),
            )
            .await;
        assert_eq!(generation, 1);
        let after = registry.snapshot("ws-a").await.unwrap();
        assert_eq!(after.generation, 1);
        assert_eq!(after.meta.provider_id, "provider-b");
        assert_eq!(after.client.model, "model-b");

        // An in-flight request keeps the exact client/meta generation it
        // obtained before the swap.
        assert_eq!(before.generation, 0);
        assert_eq!(before.meta.provider_id, "provider-a");
        assert_eq!(before.client.model, "model-a");

        // Re-activating or editing the same provider is still a new runtime
        // generation and must not reuse responses from the prior client.
        let generation = registry
            .swap(
                "ws-a",
                registry_test_client("model-b"),
                registry_test_meta("provider-b", "model-b"),
            )
            .await;
        assert_eq!(generation, 2);
        assert_eq!(registry.snapshot("ws-a").await.unwrap().generation, 2);
    }

    #[tokio::test]
    async fn registry_isolates_workspace_clients_and_generations() {
        let registry = LlmRegistry::new(
            "ws-a",
            registry_test_client("model-a"),
            registry_test_meta("provider-a", "model-a"),
        );
        assert!(registry.snapshot("ws-b").await.is_err());
        assert_eq!(
            registry
                .swap(
                    "ws-b",
                    registry_test_client("model-b"),
                    registry_test_meta("provider-b", "model-b"),
                )
                .await,
            0
        );
        registry
            .swap(
                "ws-a",
                registry_test_client("model-a2"),
                registry_test_meta("provider-a2", "model-a2"),
            )
            .await;
        let a = registry.snapshot("ws-a").await.unwrap();
        let b = registry.snapshot("ws-b").await.unwrap();
        assert_eq!(a.meta.provider_id, "provider-a2");
        assert_eq!(a.generation, 1);
        assert_eq!(b.meta.provider_id, "provider-b");
        assert_eq!(b.generation, 0);
    }

    #[test]
    fn detects_sse_body_vs_plain_json() {
        assert!(is_openai_sse_body("data: {\"choices\":[]}\n\ndata: [DONE]"));
        assert!(is_openai_sse_body("\n  data: {\"x\":1}"));
        assert!(!is_openai_sse_body(
            "{\"choices\":[{\"message\":{\"content\":\"hi\"}}]}"
        ));
        assert!(!is_openai_sse_body(""));
    }

    #[test]
    fn aggregates_sse_delta_content() {
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"{\\\"a\\\"\"}}]}\n\
                    data: {\"choices\":[{\"delta\":{\"content\":\":1}\"}}]}\n\
                    data: [DONE]";
        let (acc, _usage) = aggregate_openai_sse(body);
        assert_eq!(acc, "{\"a\":1}");
    }

    #[test]
    fn aggregates_sse_skips_unparseable_frames() {
        // keepalive 注释行 / 非法帧应被跳过，不污染聚合。
        let body = ": keepalive\n\
                    data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\
                    data: garbage-not-json\n";
        let (acc, _) = aggregate_openai_sse(body);
        assert_eq!(acc, "ok");
    }

    #[test]
    fn sse_usage_presence_distinguishes_reported_zero_from_unknown() {
        let reported_zero = "data: {\"choices\":[{\"delta\":{\"content\":\"{}\"}}]}\n\
                             data: {\"choices\":[],\"usage\":{\"prompt_tokens\":0,\"completion_tokens\":0,\"total_tokens\":0}}\n\
                             data: [DONE]";
        let (_, usage) = aggregate_openai_sse(reported_zero);
        let usage = usage.expect("explicit usage object must be retained");
        assert!(usage.is_known(), "reported zero is a measured value");
        assert_eq!(usage.total_tokens, 0);

        let omitted = "data: {\"choices\":[{\"delta\":{\"content\":\"{}\"}}]}\n\
                       data: [DONE]";
        let (_, usage) = aggregate_openai_sse(omitted);
        assert!(usage.is_none(), "omitted usage must remain unknown");
    }

    #[test]
    fn anthropic_usage_presence_distinguishes_reported_zero_from_unknown() {
        let reported: AnthropicMessageResponse = serde_json::from_str(
            r#"{"content":[{"type":"text","text":"{}"}],"usage":{"input_tokens":0,"output_tokens":0}}"#,
        )
        .unwrap();
        let usage = reported.usage.map(ChatUsage::from).unwrap_or_default();
        assert!(usage.is_known(), "reported zero is a measured value");
        assert_eq!(usage.total_tokens, 0);

        let omitted: AnthropicMessageResponse =
            serde_json::from_str(r#"{"content":[{"type":"text","text":"{}"}]}"#).unwrap();
        assert!(!omitted
            .usage
            .map(ChatUsage::from)
            .unwrap_or_default()
            .is_known());
    }

    #[test]
    fn strips_think_prefix_only_when_present() {
        assert_eq!(
            strip_reasoning_prefix("<think>reasoning here</think>{\"k\":1}"),
            "{\"k\":1}"
        );
        // 无 think 前缀原样返回。
        assert_eq!(strip_reasoning_prefix("{\"k\":1}"), "{\"k\":1}");
        // think 未闭合则不剥离（避免吞掉正文）。
        assert_eq!(strip_reasoning_prefix("<think>unclosed"), "<think>unclosed");
    }

    #[test]
    fn sse_with_think_prefix_parses_to_json() {
        // 端到端：gpt 风格 SSE（带 <think> 前缀）聚合后能被 parse_json_content 解析。
        let body =
            "data: {\"choices\":[{\"delta\":{\"content\":\"<think>hmm</think>{\\\"ok\\\"\"}}]}\n\
                    data: {\"choices\":[{\"delta\":{\"content\":\":true}\"}}]}\n\
                    data: [DONE]";
        let (acc, _) = aggregate_openai_sse(body);
        let value = parse_json_content(&strip_reasoning_prefix(&acc)).unwrap();
        assert_eq!(value["ok"], serde_json::json!(true));
    }

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
    fn cloudflare_5xx_is_retryable() {
        // 经 Cloudflare 的端点（rsxermu）源站慢/抖时回 520/522/524——属瞬时不可达应重试。
        // status 段是 reqwest 渲染的 "524 <unknown status code>"，故串里含 "LLM HTTP 524"。
        for code in ["520", "522", "524"] {
            let err = AppError::External(format!(
                "LLM HTTP {code} <unknown status code>: origin timeout"
            ));
            assert!(
                is_retryable_llm_error(&err),
                "Cloudflare {code} 应可重试（端点抖动，非配置错）"
            );
        }
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
    fn transient_unavailable_kind_is_fail_closed() {
        for kind in [
            "rate_limited",
            "http_5xx",
            "timeout",
            "connect_failed",
            "network_error",
            "body_decode_error",
        ] {
            assert!(is_transient_llm_unavailable_kind(kind), "{kind}");
        }
        for kind in [
            "account_unavailable",
            "endpoint_not_found",
            "http_4xx",
            "empty_response",
            "json_decode_error",
            "external_error",
            "unknown",
        ] {
            assert!(!is_transient_llm_unavailable_kind(kind), "{kind}");
        }
    }

    #[test]
    fn tool_use_hijack_is_retryable() {
        // claude 偶发无视禁工具约束返回 tool_use block 而非 JSON(~25%,长任务高发)。
        // detect_tool_use_hijack 抛 External("llm_tool_use_instead_of_json: ...")。
        // 这是"同输入重跑通常成功"的瞬态不遵从,本该重试——此前不在白名单致一次都不重试
        // 就冒泡(HTTP 5xx 反而能熬过抖动),是最该重试却没重试的错误。用真实诊断串锚定。
        let body = r#"{"content":[{"type":"text","text":"我将为您生成。"},{"type":"tool_use","name":"WebFetch","input":{}}],"stop_reason":"tool_use"}"#;
        let parsed: AnthropicMessageResponse = serde_json::from_str(body).unwrap();
        let diag = detect_tool_use_hijack(&parsed).expect("应识别 tool_use 劫持");
        let err = AppError::External(diag);
        assert!(
            is_retryable_llm_error(&err),
            "tool_use 劫持应可重试(同输入重跑通常成功),而非一次冒泡"
        );
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
    fn backoff_caps_at_60s_for_high_attempts() {
        // 单次退避封顶 60s：高 attempt 下指数退避（base*2^(attempt-1)）不再无界增长，
        // 防止 rsxermu 重试加码（10 次）时单次 sleep 几十分钟撞 job 墙。jitter=0（test-only）。
        // base=2500, attempt=10 → 2500*512=1_280_000ms，封顶到 60_000ms。
        assert_eq!(compute_backoff(10, 2500, None).as_millis(), 60_000);
        // attempt=6 → 2500*32=80_000ms 也已超顶 → 60_000ms。
        assert_eq!(compute_backoff(6, 2500, None).as_millis(), 60_000);
        // attempt=5 → 2500*16=40_000ms 未触顶，保持指数值。
        assert_eq!(compute_backoff(5, 2500, None).as_millis(), 40_000);
        // Retry-After 更长时仍尊重它（端点明确要求），不被 60s 顶压低。
        assert_eq!(compute_backoff(1, 2500, Some(120)).as_millis(), 120_000);
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

    #[test]
    fn parse_json_content_escapes_bare_control_chars_in_string() {
        // 真模型偶发在字符串值里塞**裸换行/制表符**（未转义），serde 严格模式拒收
        // （"control character (U+0000-U+001F) found while parsing a string"）。
        // parse_json_content SHALL 把裸控制字符转义成合法 JSON 转义序列后救回，
        // 只改表示形式不改字符串语义。
        let raw = "{\"reply\": \"第一行\n第二行\t制表\"}";
        let v = parse_json_content(raw).expect("裸控制字符必须被容错转义后解析成功");
        assert_eq!(
            v.get("reply").and_then(|x| x.as_str()),
            Some("第一行\n第二行\t制表"),
            "转义后字符串语义必须与原文一致（换行/制表保留）"
        );
    }

    #[test]
    fn repair_loose_json_escapes_low_control_char_as_unicode() {
        // 非常见的低位控制字符（如 U+0001）走 \uXXXX 兜底转义。
        let raw = "{\"x\":\"a\u{0001}b\"}";
        let repaired = repair_loose_json(raw).expect("含 \\u0001 应触发修复");
        let v: Value = serde_json::from_str(&repaired).expect("修复后必须是合法 JSON");
        assert_eq!(v.get("x").and_then(|x| x.as_str()), Some("a\u{0001}b"));
    }

    #[test]
    fn parse_json_content_extracts_json_with_natural_language_wrapper() {
        // rsxermu claude-opus-4-8 在复杂结构化 prompt 下偶发前后加自然语言包裹 JSON。
        // parse_json_content SHALL 截取首个平衡 JSON 对象后解析成功，不再 expected value at
        // line 1 column 1（domain_profile_e2e 真模型实测命中）。
        let raw = "好的，以下是生成的画像：\n{\"profile_id\": \"x\", \"display_name\": \"陪伴\"}\n希望对你有帮助。";
        let v = parse_json_content(raw).expect("前后自然语言包裹的 JSON 必须被截取解析");
        assert_eq!(v.get("profile_id").and_then(|x| x.as_str()), Some("x"));
        assert_eq!(v.get("display_name").and_then(|x| x.as_str()), Some("陪伴"));
    }

    #[test]
    fn parse_json_content_extracts_json_after_reasoning_prose() {
        // rsxermu claude-opus 真实形态：先长篇推理（可能含中文「」括号噪声），JSON 在末尾。
        // parse_json_content SHALL 遍历候选块命中真 JSON（domain_profile/knowledge 真模型实测）。
        let raw = "我看到候选 catalog 中有一条高度相关：「私有化部署」。需要先 open_chunk 展开正文。\n\n{\"action\":\"open_chunk\",\"ids\":[\"abc123\"]}";
        let v = parse_json_content(raw).expect("推理后的 JSON 必须被提取");
        assert_eq!(v.get("action").and_then(|x| x.as_str()), Some("open_chunk"));
        assert_eq!(
            v.get("ids").and_then(|x| x.as_array()).map(|a| a.len()),
            Some(1)
        );
    }

    #[test]
    fn parse_json_content_skips_unparseable_brace_block_picks_real_json() {
        // 推理里出现「伪 JSON」块（截出来解析失败），须跳过命中后面真正可解析的 JSON。
        let raw = "分析 {不是合法json的片段} 然后给出结果：{\"ok\":true,\"n\":2}";
        let v = parse_json_content(raw).expect("应跳过伪块命中真 JSON");
        assert_eq!(v.get("ok").and_then(|x| x.as_bool()), Some(true));
        assert_eq!(v.get("n").and_then(|x| x.as_i64()), Some(2));
    }

    #[test]
    fn extract_embedded_json_handles_braces_inside_strings() {
        // 字符串字面量内的 `}` 不应被当成对象闭合（配平扫描须跳过字符串内括号）。
        let raw = "前言 {\"text\": \"a } b { c\", \"n\": 1} 后缀";
        let v = extract_embedded_json(raw).expect("应提取出平衡对象");
        assert_eq!(v.get("text").and_then(|x| x.as_str()), Some("a } b { c"));
        assert_eq!(v.get("n").and_then(|x| x.as_i64()), Some(1));
    }

    #[test]
    fn extract_embedded_json_returns_none_without_json() {
        // 纯自然语言无 JSON → None（调用方回退原文走严格解析报错，不伪造数据）。
        assert!(extract_embedded_json("这里完全没有 JSON 对象").is_none());
    }

    #[test]
    fn parse_json_content_plain_object_unchanged() {
        // 非包裹的纯 JSON 对象：extract 分支不介入（首字符是 `{`），行为零变化。
        let v = parse_json_content(r#"{"a": 1}"#).expect("纯 JSON 必须直接解析");
        assert_eq!(v.get("a").and_then(|x| x.as_i64()), Some(1));
    }

    #[test]
    fn parse_json_content_unwraps_singleton_object_array() {
        // claude 偶发把唯一 profile 对象包成 `[{...}]`（domain_profile_e2e 真模型实测 got Array）。
        // 快路径解析成功是数组 → normalize 拆出内部对象，下游 to_document 不再报 got Array。
        let v = parse_json_content(r#"[{"displayName": "陪伴", "ok": true}]"#)
            .expect("单元素对象数组必须拆成对象");
        assert!(v.is_object(), "应拆成对象而非数组");
        assert_eq!(v.get("displayName").and_then(|x| x.as_str()), Some("陪伴"));
    }

    #[test]
    fn parse_json_content_keeps_multi_element_array() {
        // 多元素数组不拆（避免误伤真正期望数组的场景），原样返回。
        let v = parse_json_content(r#"[{"a":1},{"b":2}]"#).expect("多元素数组解析");
        assert!(v.is_array(), "多元素数组不应被拆");
        assert_eq!(v.as_array().map(|a| a.len()), Some(2));
    }

    #[test]
    fn extract_embedded_json_prefers_object_over_array() {
        // 推理里先出现伪数组、真 JSON 对象在后：应优先返回对象（generate_json 契约）。
        let raw = "分析候选 [1, 2, 3] 后得出结论：\n{\"action\":\"answer\",\"ok\":true}";
        let v = extract_embedded_json(raw).expect("应提取对象");
        assert!(v.is_object());
        assert_eq!(v.get("action").and_then(|x| x.as_str()), Some("answer"));
    }

    // 第三层「回喂 LLM 修复」由 parse_or_repair 驱动：前两层（快路径/repair/extract）能解的
    // **不触发**任何网络调用；前两层全失败时才发修复请求，2 次仍失败抛带诊断的 json_decode 错。
    // 这两条用一个指向不可达端点的 client 验证：可解析输入纯本地命中（即便端点死也成功），
    // 不可理解噪声最终抛错且不把噪声吞成 Ok（守 does_not_swallow_garbage 的第三层版本）。

    fn unreachable_client() -> LlmClient {
        // 127.0.0.1:1 必拒连：若 parse_or_repair 误对可解析输入发起 HTTP，测试会因连接失败而红。
        LlmClient::new(
            "http://127.0.0.1:1".to_string(),
            "test-key".to_string(),
            "test-model".to_string(),
            2,
            1,
            100,
        )
        .expect("build client")
    }

    #[tokio::test]
    async fn parse_or_repair_returns_first_two_layers_without_network() {
        // 可被前两层解析的脏输入（尾逗号）→ 不该触达不可达端点，直接成功。
        let client = unreachable_client();
        let v = client
            .parse_or_repair("{\"ok\":true,}")
            .await
            .expect("前两层应直接解析尾逗号，不发网络");
        assert_eq!(v.get("ok").and_then(|x| x.as_bool()), Some(true));
    }

    #[tokio::test]
    async fn parse_or_repair_surfaces_error_for_garbage_after_repair_exhausted() {
        // 纯噪声 + 不可达端点：前两层失败 → 第三层 2 次回喂均因连接失败 → 最终 Err。
        // 绝不把噪声吞成 Ok；错误信息须带 json_decode 诊断与原始前缀，便于排障。
        let client = unreachable_client();
        let err = client
            .parse_or_repair("这是一段没有任何 JSON 的纯自然语言噪声")
            .await
            .expect_err("噪声经修复仍失败，必须抛错而非吞下");
        let msg = err.to_string();
        assert!(
            msg.contains("json_decode") && msg.contains("raw_head"),
            "错误须含结构化诊断（json_decode + raw_head），实际: {msg}"
        );
    }

    #[tokio::test]
    async fn parse_or_repair_rejects_account_message_without_network_repair() {
        let client = unreachable_client();
        let err = client
            .parse_or_repair("您的余额不足以完成本次请求，功能受限，请立即充值。")
            .await
            .expect_err("账户错误不能进入 JSON 修复");
        assert_eq!(
            err.to_string(),
            "llm_account_unavailable: insufficient_balance"
        );

        let classified = classify_llm_error_for_user(&err, 0);
        match classified {
            AppError::LlmUnavailable { kind, .. } => {
                assert_eq!(kind, "account_unavailable");
                assert!(!is_transient_llm_unavailable_kind(&kind));
            }
            other => panic!("expected LlmUnavailable, got {other:?}"),
        }
    }

    // 长任务防御：claude 偶发 tool_use 劫持（真内容跑进 tool_use block，text 只剩开场白）。
    // detect_tool_use_hijack 应识别并给明确诊断；正常 end_turn 纯 text 不得误判。

    #[test]
    fn detect_tool_use_hijack_flags_tool_use_block() {
        let body = r#"{"content":[{"type":"text","text":"我将为您生成完整配置。"},{"type":"tool_use","name":"WebFetch","input":{"url":"https://x"}}],"stop_reason":"tool_use"}"#;
        let parsed: AnthropicMessageResponse = serde_json::from_str(body).unwrap();
        let diag = detect_tool_use_hijack(&parsed).expect("应识别 tool_use 劫持");
        assert!(
            diag.contains("llm_tool_use_instead_of_json"),
            "诊断: {diag}"
        );
        assert!(
            diag.contains("我将为您生成"),
            "诊断须含 text 开场白前缀: {diag}"
        );
    }

    #[test]
    fn detect_tool_use_hijack_flags_stop_reason_only() {
        // 即便 content 里暂无 tool_use block，stop_reason=tool_use 也要拦。
        let body =
            r#"{"content":[{"type":"text","text":"让我查一下。"}],"stop_reason":"tool_use"}"#;
        let parsed: AnthropicMessageResponse = serde_json::from_str(body).unwrap();
        assert!(detect_tool_use_hijack(&parsed).is_some());
    }

    #[test]
    fn detect_tool_use_hijack_passes_normal_text_response() {
        // 正常一次性 JSON 输出（end_turn，纯 text）不得误判。
        let body =
            r#"{"content":[{"type":"text","text":"{\"ok\":true}"}],"stop_reason":"end_turn"}"#;
        let parsed: AnthropicMessageResponse = serde_json::from_str(body).unwrap();
        assert!(detect_tool_use_hijack(&parsed).is_none());
    }

    #[test]
    fn temperature_defaults_to_02_and_setter_overrides() {
        // 生产构造默认 0.2（决策稳定）；with_temperature 链式覆盖（roleplayer 用 0.8）。
        let c = LlmClient::new("http://x".into(), "k".into(), "m".into(), 10, 1, 100).unwrap();
        assert!((c.temperature - 0.2).abs() < f64::EPSILON, "默认应 0.2");
        let hot = c.with_temperature(0.8);
        assert!(
            (hot.temperature - 0.8).abs() < f64::EPSILON,
            "setter 应覆盖到 0.8"
        );
    }

    #[tokio::test]
    async fn client_sends_gateway_compatible_default_headers() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("user-agent", LLM_USER_AGENT))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{ "message": { "content": "{\"ok\":true}" } }],
                "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = LlmClient::new(
            format!("{}/v1", server.uri()),
            "test-key".into(),
            "test-model".into(),
            10,
            1,
            100,
        )
        .expect("build client");
        let response = client
            .generate_json("Return JSON.", "Return {\"ok\":true}.")
            .await
            .expect("matching request headers must reach the mock");
        assert_eq!(response.get("ok").and_then(Value::as_bool), Some(true));
    }
}
