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
    /// server / 多把 key 调用，会话按 pair 隔离）。值 = `initialize` 结果：
    /// `Some(id)` = 有状态 server 下发的 `mcp-session-id`（后续请求须带该头，
    /// `gewe-multi-tenant` 即此类，失效返 HTTP 404 `Unknown MCP session` → 丢缓存重连一次）；
    /// `None` = server 无状态（initialize 未回 session 头，如无状态 mock）→ 后续请求不带 session 头。
    /// MCP 规范里 `mcp-session-id` 是可选的，两类 server 都要兼容。
    sessions: std::sync::Arc<dashmap::DashMap<String, Option<String>>>,
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
        self.call_tool_with_key(&self.base_url, &self.api_key, tool_name, arguments, None)
            .await
    }

    /// MCP Streamable-HTTP 会话缓存键：同一进程可对不同 server / 不同 key 调用，
    /// 会话按 `(base_url, api_key)` pair 隔离。
    fn session_cache_key(base_url: &str, api_key: &str) -> String {
        format!("{}|{}", base_url.trim_end_matches('/'), api_key)
    }

    /// 取本 (base_url, api_key) 的缓存会话，缺失则跑一次 `initialize` 握手拿到并缓存。
    async fn ensure_session(&self, base_url: &str, api_key: &str) -> AppResult<Option<String>> {
        let cache_key = Self::session_cache_key(base_url, api_key);
        if let Some(existing) = self.sessions.get(&cache_key) {
            return Ok(existing.clone());
        }
        let session_id = self.initialize_session(base_url, api_key).await?;
        self.sessions.insert(cache_key, session_id.clone());
        Ok(session_id)
    }

    /// 跑 MCP `initialize` 握手，返回 server 下发的 `mcp-session-id`（有状态 server）
    /// 或 `None`（无状态 server）。
    async fn initialize_session(&self, base_url: &str, api_key: &str) -> AppResult<Option<String>> {
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
        // 有状态 server 返回 session-id；无状态 server 可不返回（兼容两类）。
        Ok(session_id)
    }

    /// 发一条 JSON-RPC 请求（带会话头），返回解析后的 JSON-RPC 消息体。
    /// 会话失效（server 重启 / 驱逐 → HTTP 404 `Unknown MCP session`）时丢缓存重握手一次。
    async fn post_rpc(&self, base_url: &str, api_key: &str, request: &Value) -> AppResult<Value> {
        let mut reinitialized = false;
        loop {
            let session_id = self.ensure_session(base_url, api_key).await?;
            let mut req = self
                .client
                .post(format!("{}/mcp", base_url.trim_end_matches('/')))
                .header(AUTHORIZATION, format!("Bearer {}", api_key))
                .header(CONTENT_TYPE, "application/json")
                .header(ACCEPT, "application/json, text/event-stream");
            // 有状态 server 带 mcp-session-id 头；无状态 server（session_id=None）不带。
            if let Some(ref sid) = session_id {
                req = req.header("mcp-session-id", sid);
            }
            let response = req.json(request).send().await?;
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
        account_alias: Option<&str>,
    ) -> AppResult<Value> {
        // Workspace Key 下调用账号类工具需传 account_alias；Account Key 下可省略。
        // 自动注入 account_alias（如果提供且 arguments 是对象且未包含该键）。
        let mut arguments_value = serde_json::to_value(arguments)?;
        if let (Some(alias), Some(obj)) = (account_alias, arguments_value.as_object_mut()) {
            if !obj.contains_key("account_alias") {
                obj.insert("account_alias".to_string(), json!(alias));
            }
        }
        let request = json!({
            "jsonrpc": "2.0",
            "id": uuid::Uuid::new_v4().to_string(),
            "method": "tools/call",
            "params": {
                "name": tool_name,
                "arguments": arguments_value
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

    pub async fn list_tools_with_key(
        &self,
        base_url: &str,
        api_key: &str,
        account_alias: Option<&str>,
    ) -> AppResult<Value> {
        // tools/list 是系统工具，通常不需要 account_alias，但为统一接口仍支持注入。
        let mut params = json!({});
        if let (Some(alias), Some(obj)) = (account_alias, params.as_object_mut()) {
            obj.insert("account_alias".to_string(), json!(alias));
        }
        let request = json!({
            "jsonrpc": "2.0",
            "id": uuid::Uuid::new_v4().to_string(),
            "method": "tools/list",
            "params": params
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
            credentials.account_alias.as_deref(),
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
        .list_tools_with_key(
            &credentials.base_url,
            &credentials.api_key,
            credentials.account_alias.as_deref(),
        )
        .await
}

struct McpCredentials {
    base_url: String,
    api_key: String,
    /// MCP server 的账号别名（Workspace Key 下调用账号类工具时必须传此参数，
    /// Account Key 下可省略）。来自 `wechat_accounts.alias`，对应 MCP server
    /// `auth_whoami` 响应的 `accounts[].alias`（如 "t-1"）。
    account_alias: Option<String>,
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
        .as_ref()
        .and_then(|item| item.mcp_api_key.clone())
        .unwrap_or_else(|| state.config.mcp_api_key.clone());
    let account_alias = account.as_ref().map(|item| item.alias.clone());
    Ok(McpCredentials {
        base_url,
        api_key,
        account_alias,
    })
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RosterFriend {
    pub wxid: String,
    pub nickname: Option<String>,
    pub remark: Option<String>,
    pub avatar_url: Option<String>,
}

fn first_str(obj: &serde_json::Map<String, serde_json::Value>, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(s) = obj.get(*k).and_then(|v| v.as_str()) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

/// 从对象里挑第一个「元素像联系人」的数组值（元素带 wxid/userName/username 键）。
/// contacts_fetch_cache 的数组 key 未线上核实（测试账号缓存为空），故命名候选之外
/// 再兜一层「按内容识别数组」——避免 server 用列表外的新 key 时整表解析成空。
fn contact_like_array(obj: &serde_json::Map<String, serde_json::Value>) -> Option<Vec<serde_json::Value>> {
    for value in obj.values() {
        if let Some(arr) = value.as_array() {
            let looks_like_contacts = arr.first().and_then(|first| first.as_object()).is_some_and(|o| {
                ["wxid", "userName", "UserName", "username"].iter().any(|k| o.contains_key(*k))
            });
            if looks_like_contacts {
                return Some(arr.clone());
            }
        }
    }
    None
}

fn parse_roster_items(result: &serde_json::Value) -> Vec<RosterFriend> {
    // 数组路径多候选。取第一个真正 **是数组** 的候选——不能先选中"存在的键"再
    // as_array，否则某高优先候选键存在但非数组（server 回 {} 或标量）会短路掉后面
    // 真正的数组候选，导致空列表。
    //
    // 关键事实（2026-07-07 线上亲验）：call_tool_with_key 只回 result.structuredContent
    // （mcp.rs:202-205 已剥掉 JSON-RPC 外壳与 content[0].text），所以生产态本函数收到的
    // 就是 structuredContent 本体——真正生效的是**顶层** /contacts 等候选。
    // /structuredContent/* 与 /content/0/text 仅作防御（万一某调用方传入完整外壳）。
    let first_array = |v: &serde_json::Value, keys: &[&str]| -> Option<Vec<serde_json::Value>> {
        for k in keys {
            if let Some(arr) = v.pointer(k).and_then(|x| x.as_array()) {
                return Some(arr.clone());
            }
        }
        None
    };
    let named = [
        // 生产态：structuredContent 本体的顶层数组。
        "/contacts",
        "/friends",
        "/list",
        "/items",
        "/data",
        // 防御：完整外壳形态（未剥壳的调用方）。
        "/structuredContent/contacts",
        "/structuredContent/friends",
        "/structuredContent/list",
    ];
    let arr = first_array(result, &named)
        .or_else(|| {
            // content[0].text 内嵌 JSON 字符串形态（防御）。
            let text = result.pointer("/content/0/text")?.as_str()?;
            let inner: serde_json::Value = serde_json::from_str(text).ok()?;
            first_array(&inner, &named).or_else(|| inner.as_object().and_then(contact_like_array))
        })
        // 末位兜底：命名候选全落空时，按内容识别顶层任一「联系人数组」。
        .or_else(|| result.as_object().and_then(contact_like_array))
        .unwrap_or_default();

    arr.iter()
        .filter_map(|item| {
            let obj = item.as_object()?;
            let wxid = first_str(obj, &["wxid", "userName", "UserName", "username"])?;
            Some(RosterFriend {
                wxid,
                nickname: first_str(obj, &["nickName", "nickname", "NickName"]),
                remark: first_str(obj, &["remark", "Remark", "conRemark"]),
                avatar_url: first_str(
                    obj,
                    &["bigHeadImg", "smallHeadImg", "headImgUrl", "avatarUrl", "headimgurl"],
                ),
            })
        })
        .collect()
}

pub async fn fetch_roster_for_account(
    state: &AppState,
    account_id: &str,
) -> AppResult<Vec<RosterFriend>> {
    // contacts_fetch_cache 是全量好友工具（gewe "Fetch the full remote contacts cache
    // from GeWe"，无参）；account_alias 由 logged_call_for_account 自动注入。
    // 注：早前指南页误载工具名为 contact_list，2026-07-07 线上 tools/list 亲验证伪
    // ——gewe-multi-tenant server 无 contact_list（返 "Forbidden tool"），im_sync 是
    // 企业微信同步（错域），全量个人好友唯一工具即 contacts_fetch_cache。
    let result =
        logged_call_for_account(state, account_id, "contacts_fetch_cache", serde_json::json!({}))
            .await?;
    Ok(parse_roster_items(&result))
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

#[cfg(test)]
mod roster_parse_tests {
    use super::parse_roster_items;

    #[test]
    fn parses_structured_contacts_with_big_head_img() {
        let v = serde_json::json!({
            "structuredContent": { "contacts": [
                { "wxid": "wxid_a", "nickName": "小明", "remark": "客户A", "bigHeadImg": "http://img/a" },
                { "userName": "wxid_b", "nickname": "小红", "smallHeadImg": "http://img/b" }
            ]}
        });
        let out = parse_roster_items(&v);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].wxid, "wxid_a");
        assert_eq!(out[0].nickname.as_deref(), Some("小明"));
        assert_eq!(out[0].remark.as_deref(), Some("客户A"));
        assert_eq!(out[0].avatar_url.as_deref(), Some("http://img/a"));
        // 第二条用 userName 作 wxid、smallHeadImg 作头像。
        assert_eq!(out[1].wxid, "wxid_b");
        assert_eq!(out[1].avatar_url.as_deref(), Some("http://img/b"));
    }

    #[test]
    fn skips_entries_without_wxid() {
        let v = serde_json::json!({ "contacts": [ { "nickName": "无id" } ] });
        assert_eq!(parse_roster_items(&v).len(), 0);
    }

    #[test]
    fn returns_empty_on_unknown_shape() {
        let v = serde_json::json!({ "unexpected": true });
        assert_eq!(parse_roster_items(&v).len(), 0);
    }

    #[test]
    fn parses_unwrapped_structured_content_top_level_contacts() {
        // 生产态真实形态：call_tool_with_key 已剥壳，本函数收到的就是 structuredContent
        // 本体——数组在**顶层** contacts，而非 /structuredContent/contacts。
        let v = serde_json::json!({ "contacts": [
            { "userName": "wxid_p", "nickName": "生产好友", "bigHeadImg": "http://img/p" }
        ]});
        let out = parse_roster_items(&v);
        assert_eq!(out.len(), 1, "顶层 contacts（剥壳后）应解析");
        assert_eq!(out[0].wxid, "wxid_p");
        assert_eq!(out[0].avatar_url.as_deref(), Some("http://img/p"));
    }

    #[test]
    fn falls_back_to_content_like_array_under_unknown_key() {
        // contacts_fetch_cache 的数组 key 未线上核实；若 server 用列表外的新 key
        // （如 friendList），命名候选全落空时应按内容识别到该数组，而非解析成空。
        let v = serde_json::json!({ "friendList": [
            { "wxid": "wx_unknown_key", "nickName": "新键好友" }
        ]});
        let out = parse_roster_items(&v);
        assert_eq!(out.len(), 1, "未知数组 key 应被内容识别兜底命中");
        assert_eq!(out[0].wxid, "wx_unknown_key");
    }

    #[test]
    fn higher_priority_non_array_key_does_not_shadow_valid_array() {
        // structuredContent.contacts 存在但非数组（server 回 {}），有效数组在顶层 contacts。
        // 不能因高优先键"存在"就短路掉后面真正的数组候选。
        let v = serde_json::json!({
            "structuredContent": { "contacts": {} },
            "contacts": [ { "wxid": "wx_top", "nickName": "顶层好友" } ]
        });
        let out = parse_roster_items(&v);
        assert_eq!(out.len(), 1, "应回落到顶层 contacts 数组，而非被非数组的高优先键短路成空");
        assert_eq!(out[0].wxid, "wx_top");
    }
}

