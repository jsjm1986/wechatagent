use mongodb::bson::{doc, to_document, DateTime};
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use serde::Serialize;
use serde_json::{json, Value};

use crate::{
    error::{AppError, AppResult},
    models::McpCallLog,
    routes::AppState,
};

/// reqwest 客户端级硬超时——**每一次** MCP HTTP 调用的上界。
///
/// 关键约束（finding ①）：本值必须 **严格小于** dispatcher 对整条 send 的外层
/// `tokio::time::timeout`（`outbox_dispatcher::MCP_SEND_TIMEOUT_SECONDS`，且后者需
/// 覆盖单条 send 内最多顺序调用次数 × 本值）。这样「MCP 已送达但回包慢」时是
/// reqwest 自己超时返回 `Err`、`logged_call_for_account` 随后照常写 mcp_logs，而
/// **不是**被外层 timeout 取消整个 future——取消会丢掉 mcp_logs 写入，令 post-hoc
/// 守卫查不到成功记录 → 误重试 → 客户收重复消息。
///
/// 取值需 ≥ 真实「已送达但回包慢」的最大延迟：过小会把已送达的发送经 `Ok(Err)`
/// 分支（无 post-hoc 守卫）当失败重试而重复；60s 匹配 47.108.57.147 + 微信协议
/// 出栈的真实链路 RTT。无外层 timeout 的直接调用路径（推 principal 卡 / relay
/// 转述）也以本值为唯一阻塞上界。
pub(crate) const MCP_CLIENT_TIMEOUT_SECONDS: u64 = 60;

#[derive(Clone)]
pub struct McpClient {
    base_url: String,
    api_key: String,
    client: reqwest::Client,
    /// MCP Streamable-HTTP 会话缓存，键 = `base_url|api_key`（同一进程可对多个
    /// server / 多把 key 调用，会话按 pair 隔离），值 = server 在 `initialize`
    /// 时下发的 `mcp-session-id`。`gewe-multi-tenant` server 要求所有非 initialize
    /// 请求携带该头，缺失/失效（HTTP 404 `Unknown MCP session`）时本模块丢缓存重连一次。
    sessions: std::sync::Arc<dashmap::DashMap<String, String>>,
}

impl McpClient {
    pub fn new(base_url: String, api_key: String) -> anyhow::Result<Self> {
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(MCP_CLIENT_TIMEOUT_SECONDS))
                .build()?,
            sessions: std::sync::Arc::new(dashmap::DashMap::new()),
        })
    }

    pub async fn call_tool<A: Serialize>(&self, tool_name: &str, arguments: A) -> AppResult<Value> {
        self.call_tool_with_key(&self.base_url, &self.api_key, tool_name, arguments)
            .await
    }

    /// MCP Streamable-HTTP 会话缓存键：同一进程可对不同 server / 不同 key 调用，
    /// 会话按 `(base_url, api_key)` pair 隔离。
    fn session_cache_key(base_url: &str, api_key: &str) -> String {
        format!("{}|{}", base_url.trim_end_matches('/'), api_key)
    }

    /// 取本 (base_url, api_key) 的缓存会话，缺失则跑一次 `initialize` 握手拿到并缓存。
    async fn ensure_session(&self, base_url: &str, api_key: &str) -> AppResult<String> {
        let cache_key = Self::session_cache_key(base_url, api_key);
        if let Some(existing) = self.sessions.get(&cache_key) {
            return Ok(existing.clone());
        }
        let session_id = self.initialize_session(base_url, api_key).await?;
        self.sessions.insert(cache_key, session_id.clone());
        Ok(session_id)
    }

    /// 跑 MCP `initialize` 握手，返回 server 下发的 `mcp-session-id`。
    async fn initialize_session(&self, base_url: &str, api_key: &str) -> AppResult<String> {
        let request = json!({
            "jsonrpc": "2.0",
            "id": uuid::Uuid::new_v4().to_string(),
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "wechatagent", "version": "0.1" }
            }
        });
        let response = self
            .client
            .post(format!("{}/mcp", base_url.trim_end_matches('/')))
            .header(AUTHORIZATION, format!("Bearer {}", api_key))
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "application/json, text/event-stream")
            .json(&request)
            .send()
            .await?;
        let status = response.status();
        // session id 在响应头，body 可能是 SSE，必须先取头再消费 body。
        let session_id = response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let body = response.text().await?;
        if !status.is_success() {
            return Err(AppError::External(format!(
                "MCP initialize HTTP {status}: {}",
                truncate_for_error(&body)
            )));
        }
        if let Some(error) = parse_mcp_response_body(&body)?.get("error") {
            return Err(AppError::External(format!("MCP initialize failed: {error}")));
        }
        session_id
            .ok_or_else(|| AppError::External("MCP initialize 未返回 mcp-session-id 头".to_string()))
    }

    /// 发一条 JSON-RPC 请求（带会话头），返回解析后的 JSON-RPC 消息体。
    /// 会话失效（server 重启 / 驱逐 → HTTP 404 `Unknown MCP session`）时丢缓存重握手一次。
    async fn post_rpc(&self, base_url: &str, api_key: &str, request: &Value) -> AppResult<Value> {
        let mut reinitialized = false;
        loop {
            let session_id = self.ensure_session(base_url, api_key).await?;
            let response = self
                .client
                .post(format!("{}/mcp", base_url.trim_end_matches('/')))
                .header(AUTHORIZATION, format!("Bearer {}", api_key))
                .header(CONTENT_TYPE, "application/json")
                .header(ACCEPT, "application/json, text/event-stream")
                .header("mcp-session-id", &session_id)
                .json(request)
                .send()
                .await?;
            let status = response.status();
            let body = response.text().await?;
            // 会话失效：丢缓存重握手一次（server 重启常见），仍失败则如实报错。
            if status.as_u16() == 404 && !reinitialized {
                self.sessions
                    .remove(&Self::session_cache_key(base_url, api_key));
                reinitialized = true;
                continue;
            }
            if !status.is_success() {
                return Err(AppError::External(format!(
                    "MCP HTTP {status}: {}",
                    truncate_for_error(&body)
                )));
            }
            return parse_mcp_response_body(&body);
        }
    }

    pub async fn call_tool_with_key<A: Serialize>(
        &self,
        base_url: &str,
        api_key: &str,
        tool_name: &str,
        arguments: A,
    ) -> AppResult<Value> {
        let request = json!({
            "jsonrpc": "2.0",
            "id": uuid::Uuid::new_v4().to_string(),
            "method": "tools/call",
            "params": {
                "name": tool_name,
                "arguments": arguments
            }
        });
        let body = self.post_rpc(base_url, api_key, &request).await?;
        if let Some(error) = body.get("error") {
            return Err(AppError::External(format!(
                "MCP tool {tool_name} failed: {error}"
            )));
        }
        let result = body.get("result");
        // finding ③：MCP 标准用 result.isError=true + HTTP200 表示「工具执行了但失败」
        // （如联系人拒收）。仅查 HTTP 状态 + 顶层 JSON-RPC error 会把这类失败读成成功，
        // 令发送链路误判送达。server 不发 isError 时此分支不触发（标准兼容 no-op）。
        if result
            .and_then(|r| r.get("isError"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            let detail = result
                .and_then(|r| r.get("content"))
                .map(|c| c.to_string())
                .unwrap_or_default();
            return Err(AppError::External(format!(
                "MCP tool {tool_name} returned isError: {detail}"
            )));
        }
        Ok(result
            .and_then(|result| result.get("structuredContent"))
            .cloned()
            .unwrap_or(Value::Null))
    }

    pub async fn list_tools_with_key(&self, base_url: &str, api_key: &str) -> AppResult<Value> {
        let request = json!({
            "jsonrpc": "2.0",
            "id": uuid::Uuid::new_v4().to_string(),
            "method": "tools/list",
            "params": {}
        });
        let body = self.post_rpc(base_url, api_key, &request).await?;
        if let Some(error) = body.get("error") {
            return Err(AppError::External(format!(
                "MCP tools/list failed: {error}"
            )));
        }
        Ok(body.get("result").cloned().unwrap_or(Value::Null))
    }
}

/// 截断 body 供错误信息，避免超长 SSE / HTML 灌满日志。
fn truncate_for_error(body: &str) -> String {
    body.chars().take(300).collect()
}

/// 解析 MCP Streamable-HTTP 响应体：可能是 SSE（`event: message\ndata: {json}`）
/// 或纯 JSON。取 `data:` 行拼成的 JSON-RPC 消息；纯 JSON 则直接解析。
fn parse_mcp_response_body(body: &str) -> AppResult<Value> {
    let looks_like_sse = body
        .lines()
        .any(|line| line.starts_with("data:") || line.starts_with("event:"));
    if looks_like_sse {
        // SSE 单事件的多条 data 行按 \n 拼接（SSE 规范）；MCP 通常单行整段 JSON。
        let data = body
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(str::trim_start)
            .collect::<Vec<_>>()
            .join("\n");
        if !data.is_empty() {
            return serde_json::from_str(&data).map_err(|e| {
                AppError::External(format!(
                    "MCP SSE data 解析失败: {e}; data={}",
                    truncate_for_error(&data)
                ))
            });
        }
    }
    serde_json::from_str(body.trim()).map_err(|e| {
        AppError::External(format!(
            "MCP 响应解析失败: {e}; body={}",
            truncate_for_error(body)
        ))
    })
}

/// M16：落库 mcp_logs 前脱敏超大二进制字段。`media_upload_base64` 的 `base64`
/// 参数可达 ~67MB（50MB 文件上限 ×4/3），原样落库会超 16MB BSON 上限致 insert
/// 静默失败、或撑爆崩溃恢复热路径集合 `mcp_logs`。base64 对审计/恢复零价值
/// （`mcp_already_succeeded` 只读 recipient/mediaId/content），故仅把它替换成
/// 占位符（保留字节数供审计），其它字段一字不动——尤其不碰 `content`，否则
/// `mcp_already_succeeded` 的 `request.content` 精确匹配会失败 → 重复发送。
fn redact_request_for_log(request: &mongodb::bson::Document) -> mongodb::bson::Document {
    let mut doc = request.clone();
    if let Ok(b64) = request.get_str("base64") {
        doc.insert(
            "base64",
            format!("<redacted base64: {} chars>", b64.len()),
        );
    }
    doc
}

pub async fn logged_call<A: Serialize>(
    state: &AppState,
    tool_name: &str,
    arguments: A,
) -> AppResult<Value> {
    let request_doc = to_document(&serde_json::to_value(&arguments)?)?;
    let result = state.mcp.call_tool(tool_name, arguments).await;
    let (response, error) = match &result {
        Ok(value) => (to_document(value).ok(), None),
        Err(err) => (None, Some(err.to_string())),
    };
    let _ = state
        .db
        .mcp_logs()
        .insert_one(
            McpCallLog {
                id: None,
                workspace_id: state.config.default_workspace_id.clone(),
                account_id: state.config.default_account_id.clone(),
                tool_name: tool_name.to_string(),
                request: redact_request_for_log(&request_doc),
                response,
                error,
                created_at: DateTime::now(),
            },
            None,
        )
        .await;
    result
}

pub async fn logged_call_for_account<A: Serialize>(
    state: &AppState,
    account_id: &str,
    tool_name: &str,
    arguments: A,
) -> AppResult<Value> {
    let credentials = credentials_for_account(state, account_id).await?;
    let arguments_value = serde_json::to_value(arguments)?;
    let request_doc = to_document(&arguments_value)?;
    let result = state
        .mcp
        .call_tool_with_key(
            &credentials.base_url,
            &credentials.api_key,
            tool_name,
            arguments_value,
        )
        .await;
    let (response, error) = match &result {
        Ok(value) => (to_document(value).ok(), None),
        Err(err) => (None, Some(err.to_string())),
    };
    let _ = state
        .db
        .mcp_logs()
        .insert_one(
            McpCallLog {
                id: None,
                workspace_id: state.config.default_workspace_id.clone(),
                account_id: account_id.to_string(),
                tool_name: tool_name.to_string(),
                request: redact_request_for_log(&request_doc),
                response,
                error,
                created_at: DateTime::now(),
            },
            None,
        )
        .await;
    result
}

pub async fn list_tools_for_account(state: &AppState, account_id: &str) -> AppResult<Value> {
    let credentials = credentials_for_account(state, account_id).await?;
    state
        .mcp
        .list_tools_with_key(&credentials.base_url, &credentials.api_key)
        .await
}

struct McpCredentials {
    base_url: String,
    api_key: String,
}

async fn credentials_for_account(state: &AppState, account_id: &str) -> AppResult<McpCredentials> {
    let account = state
        .db
        .accounts()
        .find_one(
            doc! {
                "workspace_id": &state.config.default_workspace_id,
                "account_id": account_id
            },
            None,
        )
        .await?;
    let base_url = account
        .as_ref()
        .and_then(|item| item.mcp_base_url.clone())
        .unwrap_or_else(|| state.config.mcp_base_url.clone());
    let api_key = account
        .and_then(|item| item.mcp_api_key)
        .unwrap_or_else(|| state.config.mcp_api_key.clone());
    Ok(McpCredentials { base_url, api_key })
}

#[cfg(test)]
mod sse_parse_tests {
    use super::{parse_mcp_response_body, McpClient};

    #[test]
    fn parses_sse_single_data_line() {
        // gewe-multi-tenant server 的真实回包形态：event: message + 单行 data JSON。
        let body = "event: message\nid: abc\ndata: {\"jsonrpc\":\"2.0\",\"id\":\"c\",\"result\":{\"structuredContent\":{\"ok\":true}}}\n\n";
        let v = parse_mcp_response_body(body).expect("SSE data 应解析成 JSON-RPC");
        assert_eq!(
            v.pointer("/result/structuredContent/ok"),
            Some(&serde_json::Value::Bool(true))
        );
    }

    #[test]
    fn parses_plain_json_body() {
        // initialize 用 Accept 协商到纯 JSON 时（或 server 直接回 JSON）也要能解析。
        let body = "{\"jsonrpc\":\"2.0\",\"id\":\"x\",\"result\":{\"a\":1}}";
        let v = parse_mcp_response_body(body).expect("纯 JSON 应解析");
        assert_eq!(v.pointer("/result/a"), Some(&serde_json::json!(1)));
    }

    #[test]
    fn parses_sse_multiline_data_concatenated() {
        // SSE 规范：单事件多条 data 行按 \n 拼接后才是完整负载。
        let body = "event: message\ndata: {\"jsonrpc\":\"2.0\",\ndata: \"id\":\"m\",\"result\":{}}\n\n";
        let v = parse_mcp_response_body(body).expect("多行 data 拼接后应解析");
        assert_eq!(v.pointer("/id"), Some(&serde_json::json!("m")));
    }

    #[test]
    fn errors_on_garbage_body() {
        // 非 JSON、非 SSE → 如实报错，不静默吞成空（防误判送达）。
        assert!(parse_mcp_response_body("<html>Bad Gateway</html>").is_err());
    }

    #[test]
    fn session_cache_key_isolates_by_base_url_and_key() {
        // 同一进程对不同 server / 不同 key 调用，会话必须按 pair 隔离，绝不串用。
        let a = McpClient::session_cache_key("http://h1:3001", "keyA");
        let b = McpClient::session_cache_key("http://h1:3001", "keyB");
        let c = McpClient::session_cache_key("http://h2:3001", "keyA");
        assert_ne!(a, b);
        assert_ne!(a, c);
        // 尾部斜杠归一化，避免同一 server 因写法不同分裂出两份会话。
        assert_eq!(
            McpClient::session_cache_key("http://h1:3001/", "keyA"),
            McpClient::session_cache_key("http://h1:3001", "keyA")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::redact_request_for_log;
    use mongodb::bson::doc;

    #[test]
    fn redact_removes_base64_keeps_other_fields() {
        let req = doc! {
            "fileName": "报价单.pdf",
            "mediaType": "file",
            "base64": "AAAABBBBCCCC",
        };
        let out = redact_request_for_log(&req);
        assert_eq!(
            out.get_str("base64").unwrap(),
            "<redacted base64: 12 chars>",
            "base64 应被替换为占位符(保留字节数)"
        );
        assert_eq!(out.get_str("fileName").unwrap(), "报价单.pdf");
        assert_eq!(out.get_str("mediaType").unwrap(), "file");
    }

    #[test]
    fn redact_preserves_content_and_recipient() {
        // 红线:content/recipient 是崩溃恢复 mcp_already_succeeded 的精确匹配字段,
        // 绝不能被脱敏改动,否则匹配失败→重复发送。
        let long_content = "很长的AI回复".repeat(1000);
        let req = doc! {
            "recipient": "wxid_customer_a",
            "content": &long_content,
        };
        let out = redact_request_for_log(&req);
        assert_eq!(out.get_str("content").unwrap(), long_content, "content 一字不动");
        assert_eq!(out.get_str("recipient").unwrap(), "wxid_customer_a");
    }

    #[test]
    fn redact_noop_without_base64() {
        let req = doc! { "recipient": "wxid_x", "mediaId": "mid_123" };
        let out = redact_request_for_log(&req);
        assert_eq!(out, req, "无 base64 key 时脱敏应与原 doc 相等");
    }
}

