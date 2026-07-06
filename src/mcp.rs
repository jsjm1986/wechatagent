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
}

impl McpClient {
    pub fn new(base_url: String, api_key: String) -> anyhow::Result<Self> {
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(MCP_CLIENT_TIMEOUT_SECONDS))
                .build()?,
        })
    }

    pub async fn call_tool<A: Serialize>(&self, tool_name: &str, arguments: A) -> AppResult<Value> {
        self.call_tool_with_key(&self.base_url, &self.api_key, tool_name, arguments)
            .await
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
        let body: Value = response.json().await?;
        if !status.is_success() {
            return Err(AppError::External(format!("MCP HTTP {status}: {body}")));
        }
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
        let body: Value = response.json().await?;
        if !status.is_success() {
            return Err(AppError::External(format!("MCP HTTP {status}: {body}")));
        }
        if let Some(error) = body.get("error") {
            return Err(AppError::External(format!(
                "MCP tools/list failed: {error}"
            )));
        }
        Ok(body.get("result").cloned().unwrap_or(Value::Null))
    }
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

