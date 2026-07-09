use mongodb::bson::{doc, to_document, DateTime};
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
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
                return Err(classify_mcp_http_error(
                    status.as_u16(),
                    format!("MCP HTTP {status}: {}", truncate_for_error(&body)),
                ));
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

/// 分类 MCP server 的非 2xx HTTP 响应:429/503(SSE 连接数满/瞬时不可用)→UpstreamBusy
/// (调用方可柔化为「同步中」);其余(401/500 等)→External(→internal_error,不掩盖真错误)。
fn classify_mcp_http_error(code: u16, detail: String) -> AppError {
    if code == 429 || code == 503 {
        AppError::UpstreamBusy(detail)
    } else {
        AppError::External(detail)
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RosterFriend {
    pub wxid: String,
    pub nickname: Option<String>,
    pub remark: Option<String>,
    pub avatar_url: Option<String>,
    pub sex: Option<i32>,
    pub is_non_human: bool,
}

/// roster 拉取结果：友列表 + 是否仍在同步（cache 空 {} 未就绪）。
#[derive(Debug)]
pub struct RosterFetchOutcome {
    pub friends: Vec<RosterFriend>,
    pub syncing: bool,
}

/// 判定 contacts_fetch_cache 返回是否为「空 cache（异步未就绪）」——区别于
/// 「真 0 好友」（result.friends 是空数组）。空对象 {} / Null → 空 cache（可重试）；
/// 任何含非空数组候选的形态 → 已就绪。空数组 → 已就绪（真 0 好友，不重试）。
fn roster_result_is_empty_cache(result: &serde_json::Value) -> bool {
    match result {
        serde_json::Value::Null => true,
        serde_json::Value::Object(map) if map.is_empty() => true,
        // result.friends / result.contacts 等存在且是数组（哪怕空）→ 已就绪。
        _ => {
            // 若解析能拿到任何数组候选（含空数组），视为已就绪；完全无数组候选 → 空 cache。
            let has_any_array = ["/result/friends", "/result/contacts", "/result/list",
                "/contacts", "/friends", "/list", "/items", "/data"]
                .iter()
                .any(|k| result.pointer(k).and_then(|v| v.as_array()).is_some());
            !has_any_array
        }
    }
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

/// 从对象里挑第一个「元素像联系人」的数组值：元素带 wxid/userName/username 键，
/// 或元素是纯字符串（contacts_fetch_cache 的 wxid 字符串数组）。
/// 命名候选之外再兜一层「按内容识别数组」——避免 server 用列表外的新 key 时整表解析成空。
fn contact_like_array(obj: &serde_json::Map<String, serde_json::Value>) -> Option<Vec<serde_json::Value>> {
    for value in obj.values() {
        if let Some(arr) = value.as_array() {
            let looks_like_contacts = arr.first().is_some_and(|first| {
                first.as_str().is_some()
                    || first.as_object().is_some_and(|o| {
                        ["wxid", "userName", "UserName", "username"].iter().any(|k| o.contains_key(*k))
                    })
            });
            if looks_like_contacts {
                return Some(arr.clone());
            }
        }
    }
    None
}

/// 微信官方保留系统账号 wxid（业界通用白名单）——这些不是真人好友，
/// 通讯录里标记为非真人（前端默认折叠）。公众号无可靠字段识别，不在此列。
const WECHAT_SYSTEM_ACCOUNTS: &[&str] = &[
    "fmessage", "qqmail", "weixin", "mphelper", "medianote",
    "qmessage", "floatbottle", "tmessage", "qqsync", "newsapp",
    "filehelper", "weibo", "brandsessionholder",
];

/// 判定是否非真人账号：type=="system" 或 wxid 命中微信保留白名单。
fn is_non_human_account(user_name: &str, item_type: Option<&str>) -> bool {
    item_type == Some("system") || WECHAT_SYSTEM_ACCOUNTS.contains(&user_name)
}

fn parse_roster_items(result: &serde_json::Value) -> Vec<RosterFriend> {
    // 数组路径多候选。取第一个真正 **是数组** 的候选——不能先选中"存在的键"再
    // as_array，否则某高优先候选键存在但非数组（server 回 {} 或标量）会短路掉后面
    // 真正的数组候选，导致空列表。
    //
    // 关键事实（2026-07-08 线上亲验）：contacts_fetch_cache 就绪返回
    // structuredContent = {result:{friends:[wxid字符串]}}，故真正生效的是嵌套
    // /result/friends。call_tool_with_key 已剥掉 JSON-RPC 外壳与 content[0].text，
    // 生产态本函数收到的就是 structuredContent 本体。顶层 /contacts 等 + /content
    // 兜底仅作防御（万一 server 换形态或某调用方传入完整外壳）。
    let first_array = |v: &serde_json::Value, keys: &[&str]| -> Option<Vec<serde_json::Value>> {
        for k in keys {
            if let Some(arr) = v.pointer(k).and_then(|x| x.as_array()) {
                return Some(arr.clone());
            }
        }
        None
    };
    let named = [
        // 生产态：contacts_fetch_cache 的嵌套 result.friends。
        "/result/friends",
        "/result/contacts",
        "/result/list",
        // 顶层数组（其它工具/形态）。
        "/contacts",
        "/friends",
        "/list",
        "/items",
        "/data",
        // 防御：完整外壳形态（未剥壳的调用方）。
        "/structuredContent/result/friends",
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
            // 纯字符串元素：直接当 wxid（contacts_fetch_cache 的生产形态）。
            if let Some(s) = item.as_str() {
                if s.is_empty() {
                    return None;
                }
                return Some(RosterFriend {
                    wxid: s.to_string(),
                    nickname: None,
                    remark: None,
                    avatar_url: None,
                    sex: None,
                    is_non_human: is_non_human_account(s, None),
                });
            }
            // 对象元素：从命名键提取（防御其它形态）。
            let obj = item.as_object()?;
            let wxid = first_str(obj, &["wxid", "userName", "UserName", "username"])?;
            Some(RosterFriend {
                wxid: wxid.clone(),
                nickname: first_str(obj, &["nickName", "nickname", "NickName"]),
                remark: first_str(obj, &["remark", "Remark", "conRemark"]),
                avatar_url: first_str(
                    obj,
                    &["bigHeadImgUrl", "smallHeadImgUrl", "bigHeadImg", "smallHeadImg", "headImgUrl", "avatarUrl", "headimgurl"],
                ),
                sex: obj
                    .get("sex")
                    .and_then(|v| v.as_i64().or_else(|| v.get("low").and_then(|l| l.as_i64())))
                    .map(|n| n as i32),
                is_non_human: is_non_human_account(&wxid, obj.get("type").and_then(|v| v.as_str())),
            })
        })
        .collect()
}

/// 从单次 contacts_fetch_cache 返回体推导 roster 结果 + 是否仍在同步。
/// 不变式：**解析出任何好友一定 syncing=false（就绪）**。仅当解析为空且返回体判为空
/// cache（{}/Null/无命名数组候选）才 syncing=true。这样即便 parse_roster_items 识别的
/// 形态多于 roster_result_is_empty_cache 的命名路径集（/structuredContent/*、/content/0/text
/// 内嵌 JSON、contact_like_array 内容兜底），也绝不会「列表有人却仍报同步中」——
/// 否则前端会无限 8s 重拉且每次清空运营勾选（RosterView refresh 重置草稿）。
fn roster_outcome_from_result(result: &serde_json::Value) -> RosterFetchOutcome {
    let friends = parse_roster_items(result);
    if !friends.is_empty() {
        // 铁律：解析出任何好友一定就绪（否则前端无限重拉且清空运营勾选）。
        return RosterFetchOutcome { friends, syncing: false };
    }
    // 空列表：区分「真 0 好友（就绪）」vs「未就绪（同步中）」。
    // contacts_fetch_full 有权威 status：ready → 就绪；其它 → 同步中。refreshing:true 带全量
    // 数据也算就绪，故不参与判据。旧 contacts_fetch_cache 形态无 status → 回落空 cache 判据。
    let syncing = match result.pointer("/status").and_then(|v| v.as_str()) {
        Some("ready") => false,
        Some(_) => true,
        None => roster_result_is_empty_cache(result),
    };
    RosterFetchOutcome { friends, syncing }
}

pub async fn fetch_roster_for_account(
    state: &AppState,
    account_id: &str,
) -> AppResult<RosterFetchOutcome> {
    // contacts_fetch_full 是全量好友工具（返回昵称/头像/性别等富化字段，无参）；
    // account_alias 由 logged_call_for_account 自动注入。就绪信号是返回体 status=="ready"
    // （亲验：ready 时带全量 items，refreshing:true 是后台刷新标志、非未就绪）。未就绪时
    // 同一请求内短重试（间隔 2s、最多 3 次），仍未就绪则 syncing=true 让前端提示「同步中」。
    const MAX_RETRIES: usize = 3;
    const RETRY_INTERVAL_SECS: u64 = 2;
    let mut last_result = serde_json::Value::Null;
    for attempt in 0..MAX_RETRIES {
        match logged_call_for_account(
            state,
            account_id,
            "contacts_fetch_full",
            serde_json::json!({}),
        )
        .await
        {
            Ok(v) => {
                last_result = v;
            }
            // 上游限流(429/503):柔化为「同步中」而非硬错误。当作本次空 cache——
            // 还有重试机会则退避重试(退避后 MCP SSE 名额常已释放),用尽仍限流则
            // 返回 syncing:true 让前端提示「同步中」并自动重拉,退避后自愈。
            Err(AppError::UpstreamBusy(_)) => {
                last_result = serde_json::Value::Null;
            }
            // 真实错误(401/500/配置错等)照常上抛 → 前端红条,不掩盖真问题。
            Err(other) => return Err(other),
        }
        // 解析器是「是否就绪」的唯一真相源：解析出好友即就绪返回。
        let outcome = roster_outcome_from_result(&last_result);
        if !outcome.syncing {
            return Ok(outcome);
        }
        // 未就绪：还有重试机会则等待后重试（最后一次不等）。
        if attempt + 1 < MAX_RETRIES {
            tokio::time::sleep(std::time::Duration::from_secs(RETRY_INTERVAL_SECS)).await;
        }
    }
    // 重试用尽仍未就绪 → syncing:true（friends 为空）。
    Ok(roster_outcome_from_result(&last_result))
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

    #[test]
    fn parses_nested_result_friends_string_array() {
        // 生产真实形态（2026-07-08 线上亲验）：structuredContent.result.friends
        // 是纯 wxid 字符串数组。
        let v = serde_json::json!({
            "result": { "friends": ["medianote", "wxid_2o93p4cc9n4x22", "wxid_ax8y68dxucvm22"] }
        });
        let out = parse_roster_items(&v);
        assert_eq!(out.len(), 3, "纯字符串数组应逐条解析为 wxid-only");
        assert_eq!(out[0].wxid, "medianote");
        assert_eq!(out[1].wxid, "wxid_2o93p4cc9n4x22");
        assert_eq!(out[0].nickname, None, "字符串元素无昵称");
        assert_eq!(out[0].avatar_url, None, "字符串元素无头像");
    }

    #[test]
    fn parses_nested_result_friends_object_array() {
        // 防御：万一 GeWe 换成 result.friends 里带对象详情，也要能解析。
        let v = serde_json::json!({
            "result": { "friends": [
                { "wxid": "wxid_a", "nickName": "小明", "bigHeadImg": "http://img/a" }
            ]}
        });
        let out = parse_roster_items(&v);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].wxid, "wxid_a");
        assert_eq!(out[0].nickname.as_deref(), Some("小明"));
        assert_eq!(out[0].avatar_url.as_deref(), Some("http://img/a"));
    }

    #[test]
    fn empty_object_yields_empty_roster() {
        // 空 cache 返回 {} → 空列表（不 panic、不误命中）。
        let v = serde_json::json!({});
        assert_eq!(parse_roster_items(&v).len(), 0);
    }

    #[test]
    fn mixed_string_and_object_array_all_parsed() {
        // 混合数组：字符串 + 对象都应解析（不因首元素类型短路）。
        let v = serde_json::json!({
            "result": { "friends": [
                "wxid_str",
                { "userName": "wxid_obj", "nickName": "对象好友" }
            ]}
        });
        let out = parse_roster_items(&v);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].wxid, "wxid_str");
        assert_eq!(out[1].wxid, "wxid_obj");
        assert_eq!(out[1].nickname.as_deref(), Some("对象好友"));
    }

    #[test]
    fn parses_contacts_fetch_full_envelope_with_rich_fields() {
        // contacts_fetch_full 真实形态（2026-07-09 117 亲验）：顶层 items 数组，
        // 单条带 userName(=wxid)/nickName/bigHeadImgUrl/sex。
        let v = serde_json::json!({
            "status": "ready",
            "count": 2,
            "refreshing": true,
            "items": [
                { "userName": "wxid_full1", "nickName": "富化好友", "remark": "客户", "bigHeadImgUrl": "http://img/big", "sex": 1 },
                { "userName": "wxid_full2", "nickName": "无头像", "smallHeadImgUrl": "http://img/small", "sex": 2 }
            ]
        });
        let out = parse_roster_items(&v);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].wxid, "wxid_full1");
        assert_eq!(out[0].nickname.as_deref(), Some("富化好友"));
        assert_eq!(out[0].avatar_url.as_deref(), Some("http://img/big"), "bigHeadImgUrl 必须命中");
        assert_eq!(out[0].sex, Some(1));
        assert_eq!(out[1].avatar_url.as_deref(), Some("http://img/small"), "smallHeadImgUrl 回退命中");
        assert_eq!(out[1].sex, Some(2));
    }

    #[test]
    fn parses_sex_int64_object_form() {
        // MCP contacts_fetch_full 真实形态：sex 是 int64 序列化对象 {high,low}，真值在 .low。
        let v = serde_json::json!({
            "status": "ready",
            "items": [
                { "userName": "wx_m", "nickName": "男", "sex": { "high": 0, "low": 1, "unsigned": false } },
                { "userName": "wx_f", "nickName": "女", "sex": { "high": 0, "low": 2, "unsigned": false } },
                { "userName": "wx_bare", "nickName": "裸整数", "sex": 1 }
            ]
        });
        let out = parse_roster_items(&v);
        assert_eq!(out[0].sex, Some(1), "对象 {{low:1}} → 男");
        assert_eq!(out[1].sex, Some(2), "对象 {{low:2}} → 女");
        assert_eq!(out[2].sex, Some(1), "裸整数 1 仍兼容");
    }
}

#[cfg(test)]
mod roster_empty_cache_tests {
    use super::roster_result_is_empty_cache;

    #[test]
    fn empty_object_is_empty_cache() {
        // contacts_fetch_cache 未就绪返回 {} → 判定为空 cache（syncing）。
        assert!(roster_result_is_empty_cache(&serde_json::json!({})));
    }

    #[test]
    fn null_is_empty_cache() {
        // call_tool_with_key 无 structuredContent 时返回 Null → 也视为空 cache。
        assert!(roster_result_is_empty_cache(&serde_json::Value::Null));
    }

    #[test]
    fn populated_result_is_not_empty_cache() {
        // 有 friends 数据 → 不是空 cache（已就绪）。
        let v = serde_json::json!({ "result": { "friends": ["wxid_a"] } });
        assert!(!roster_result_is_empty_cache(&v));
    }

    #[test]
    fn empty_friends_array_is_not_empty_cache() {
        // result.friends 是空数组（真 0 好友，已就绪）→ 不是空 cache，不该无限重试。
        let v = serde_json::json!({ "result": { "friends": [] } });
        assert!(!roster_result_is_empty_cache(&v));
    }
}

#[cfg(test)]
mod roster_outcome_tests {
    use super::roster_outcome_from_result;

    #[test]
    fn production_string_array_is_ready() {
        // 线上亲验形态：result.friends 纯 wxid 字符串数组 → 就绪、非同步中。
        let v = serde_json::json!({ "result": { "friends": ["wxid_a", "wxid_b"] } });
        let out = roster_outcome_from_result(&v);
        assert_eq!(out.friends.len(), 2);
        assert!(!out.syncing);
    }

    #[test]
    fn empty_object_is_syncing() {
        // 空 cache {} → 同步中、空列表。
        let out = roster_outcome_from_result(&serde_json::json!({}));
        assert!(out.friends.is_empty());
        assert!(out.syncing);
    }

    #[test]
    fn real_zero_friends_is_ready_not_syncing() {
        // result.friends 空数组 = 真 0 好友，已就绪，不该同步中（否则无限重拉）。
        let v = serde_json::json!({ "result": { "friends": [] } });
        let out = roster_outcome_from_result(&v);
        assert!(out.friends.is_empty());
        assert!(!out.syncing, "真 0 好友必须 syncing=false");
    }

    #[test]
    fn parseable_but_not_in_empty_cache_pathset_is_ready() {
        // 回归守卫：此形态 parse_roster_items 能解析（/structuredContent/contacts），
        // 但 roster_result_is_empty_cache 的 8 条命名路径**不含**它 → 旧逻辑会误判空
        // cache 而返回 syncing:true+非空列表。新不变式：解析出好友即 syncing=false。
        let v = serde_json::json!({
            "structuredContent": { "contacts": [
                { "userName": "wxid_p", "nickName": "生产好友" }
            ]}
        });
        let out = roster_outcome_from_result(&v);
        assert_eq!(out.friends.len(), 1, "解析器识别的形态必须算就绪");
        assert!(!out.syncing, "列表有人时绝不能报同步中");
    }

    #[test]
    fn content_fallback_array_is_ready() {
        // 回归守卫：未知 key 经 contact_like_array 内容兜底解析出好友，
        // empty-cache 命名路径同样不含 → 必须 syncing=false。
        let v = serde_json::json!({ "friendList": [ { "wxid": "wx_unknown" } ] });
        let out = roster_outcome_from_result(&v);
        assert_eq!(out.friends.len(), 1);
        assert!(!out.syncing);
    }

    #[test]
    fn full_ready_with_items_is_ready() {
        // contacts_fetch_full：status=ready + items 非空 → 就绪。
        let v = serde_json::json!({ "status": "ready", "items": [ { "userName": "wxid_a", "sex": 1 } ] });
        let out = roster_outcome_from_result(&v);
        assert_eq!(out.friends.len(), 1);
        assert!(!out.syncing);
    }

    #[test]
    fn full_ready_zero_items_is_ready_not_syncing() {
        // status=ready + 空 items = 真 0 好友，就绪不重试。
        let v = serde_json::json!({ "status": "ready", "items": [] });
        let out = roster_outcome_from_result(&v);
        assert!(out.friends.is_empty());
        assert!(!out.syncing, "ready 且 0 好友必须 syncing=false");
    }

    #[test]
    fn full_pending_empty_is_syncing() {
        // status!=ready + 空 items → 未就绪，同步中（refreshing 是干扰项，不参与判据）。
        let v = serde_json::json!({ "status": "pending", "items": [], "refreshing": true });
        let out = roster_outcome_from_result(&v);
        assert!(out.friends.is_empty());
        assert!(out.syncing, "非 ready 空列表必须 syncing=true");
    }

    #[test]
    fn null_result_is_syncing() {
        // fetch_roster_for_account 遇 UpstreamBusy(限流)会把 last_result 置 Null,
        // 应判为空 cache→syncing:true(而非当作真 0 好友或报错)。
        let out = roster_outcome_from_result(&serde_json::Value::Null);
        assert!(out.friends.is_empty());
        assert!(out.syncing, "Null(限流柔化)必须 syncing=true");
    }
}

#[cfg(test)]
mod mcp_http_classify_tests {
    use super::classify_mcp_http_error;
    use crate::error::AppError;

    #[test]
    fn http_429_is_upstream_busy() {
        assert!(matches!(
            classify_mcp_http_error(429, "MCP HTTP 429: too many".into()),
            AppError::UpstreamBusy(_)
        ));
    }

    #[test]
    fn http_503_is_upstream_busy() {
        assert!(matches!(
            classify_mcp_http_error(503, "MCP HTTP 503".into()),
            AppError::UpstreamBusy(_)
        ));
    }

    #[test]
    fn http_500_is_external() {
        // 非限流错误仍是 External(→internal_error),不被柔化,不掩盖真问题。
        assert!(matches!(
            classify_mcp_http_error(500, "MCP HTTP 500".into()),
            AppError::External(_)
        ));
    }

    #[test]
    fn http_401_is_external() {
        assert!(matches!(
            classify_mcp_http_error(401, "MCP HTTP 401".into()),
            AppError::External(_)
        ));
    }
}

#[cfg(test)]
mod is_non_human_tests {
    use super::is_non_human_account;

    #[test]
    fn system_type_is_non_human() {
        assert!(is_non_human_account("weixin", Some("system")));
    }

    #[test]
    fn whitelisted_wxid_is_non_human() {
        assert!(is_non_human_account("fmessage", Some("friend")));
        assert!(is_non_human_account("qqmail", None));
        assert!(is_non_human_account("mphelper", Some("friend")));
    }

    #[test]
    fn real_person_is_not_non_human() {
        // 真人：新号 wxid_ / 老号自定义短 id —— 都不是非真人。
        assert!(!is_non_human_account("wxid_42jvcxc49rbf12", Some("friend")));
        assert!(!is_non_human_account("songboyu1993", Some("friend")));
    }

    #[test]
    fn public_account_not_misjudged() {
        // 公众号(福州晚报 wxid_8874178741811)无可靠字段识别 → 不误判为非真人。
        assert!(!is_non_human_account("wxid_8874178741811", Some("friend")));
    }
}

