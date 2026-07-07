# MCP Server 正式接入兼容性审查（2026-07-07）

## 环境
- **MCP Server**: `http://117.72.54.28:3001`
- **API Key**: `gwa_ba60a98aada58c10b77f6f20841c77c6c3c0506d9431871f`（Workspace Key）
- **当前实现**: `src/mcp.rs`（commit `b640ce4`）

## 活体测试结果（✅ 全绿）

### 1. Initialize 握手
- HTTP 200，返回 `mcp-session-id: 4e940798-79f2-4c82-8158-c1d38c2494b4` ✅
- SSE 响应正确解析 ✅
- 协议版本 `2024-11-05` 匹配 ✅

### 2. auth_whoami（Key 身份核实）
- **Key 类型**: `workspace_account_key_v1`（**Workspace Key**，非 Account Key）
- **scope**: `workspace`（可管理多个微信账号）
- **workspace**: `weagent / 官方内测`（id=203）
- **member**: `weagent官方`（role=tenant_user）
- **可用账号**: 1 个账号 `alias="t-1" / display_name="测试1"`（id=102，当前 offline）
- **allowed_tools**: 136 个工具（包含 `message_send_text`/`contacts_search`/`account_list` 等核心工具）
- **bound_account_id**: `null`（确认是 Workspace Key，非绑定单账号的 Account Key）

### 3. tools/list
- HTTP 200，返回 136 个工具 ✅
- 前5个: `auth_whoami`, `account_list`, `account_get_status`, `account_reconnect`, `account_logout`

## 兼容性逐项核对

| 要求（来自 mcp-guide.html） | 当前实现状态 | 审查结论 |
|---|---|---|
| **1. 优先使用 POST /mcp** | ✅ `src/mcp.rs:89,125` 均用 `/mcp` 端点 | GREEN |
| **2. 鉴权 `Authorization: Bearer`** | ✅ `src/mcp.rs:90,126` | GREEN |
| **3. 会话管理（initialize + mcp-session-id 头）** | ✅ `ensure_session`/`initialize_session`/会话缓存/失效重连 | GREEN |
| **4. SSE 响应解析** | ✅ `parse_mcp_response_body` 兼容 SSE + 纯 JSON | GREEN |
| **5. 不传 tenantId/userId/workspaceId** | ✅ 未传任何内部 ID，server 从 Key 自动识别 | GREEN |
| **6. Workspace Key 调用账号类工具需传 `account_alias`** | ⚠️ **关键缺口**（见下） | **需修复** |
| **7. tools/list 可用** | ✅ `list_tools_with_key` 已实现 | GREEN |
| **8. 协议版本 `2024-11-05`** | ✅ `src/mcp.rs:82` | GREEN |

## 🔴 关键缺口：Workspace Key 下 account_alias 缺失

### 问题定界

**当前架构假设**: 每个微信账号配置独立的 Account Key（存在 `wechat_accounts.mcp_api_key`），`credentials_for_account` 按 `account_id` 查库拿到该账号专属的 `base_url` 和 `api_key`。

**实际情况**: 提供的 Key `gwa_ba60a98a...` 是 **Workspace Key**（`scope=workspace`，`bound_account_id=null`，管理多个账号）。按 MCP guide 和 `auth_whoami` 响应：

> Workspace Key 调用 **账号类工具**（如 `message_send_text`/`contacts_search`/`account_list`）时，**必须在 `arguments` 里传 `account_alias`**，否则 server 不知道操作哪个微信号。

当前 `call_tool_with_key` 只接受 `tool_name` 和 `arguments`，**不会自动注入 `account_alias`**。如果 DB 里配的是 Workspace Key 而非 Account Key，调用账号类工具会失败（server 报 `Missing account_alias` 或类似错误）。

### 两种修复路径

#### 路径 A：改代码支持 Workspace Key + account_alias（推荐，灵活性高）

1. **`wechat_accounts` 表加 `account_alias` 字段**（可选，与 `account_id` 解耦）：
   ```rust
   // models.rs WechatAccount
   pub account_alias: Option<String>,  // 新增，对应 MCP server 的 alias（如 "t-1"）
   ```

2. **`credentials_for_account` 返回 account_alias**：
   ```rust
   struct McpCredentials {
       base_url: String,
       api_key: String,
       account_alias: Option<String>,  // 新增
   }
   ```

3. **`call_tool_with_key` / `logged_call_for_account` 自动注入 `account_alias`**（仅当 Key 是 Workspace Key 且工具是账号类工具时）：
   ```rust
   pub async fn call_tool_with_key<A: Serialize>(
       &self,
       base_url: &str,
       api_key: &str,
       tool_name: &str,
       arguments: A,
       account_alias: Option<&str>,  // 新增参数
   ) -> AppResult<Value> {
       let mut args_value = serde_json::to_value(arguments)?;
       // 如果传了 account_alias 且 arguments 是对象，自动注入（不覆盖已有值）
       if let (Some(alias), Some(obj)) = (account_alias, args_value.as_object_mut()) {
           if !obj.contains_key("account_alias") {
               obj.insert("account_alias".to_string(), json!(alias));
           }
       }
       // ... 后续 JSON-RPC 调用不变
   }
   ```

4. **migration 补 `account_alias`**：读 `auth_whoami` 的 `accounts[]` 填充到 DB。

#### 路径 B：继续用 Account Key 架构（简单但受限）

为每个微信账号单独申请绑定的 Account Key（`bound_account_id` 非空），DB 里每个 `wechat_accounts` 行配自己的 Account Key。Account Key 下 server 自动定位账号，不需要 `account_alias`。

**缺点**：Key 管理成本高（N 个账号 = N 把 Key），且当前提供的 Workspace Key 用不上（需重新申请）。

### 推荐方案

**路径 A**（支持 Workspace Key）更通用，既能用 Workspace Key 统一管理（`account_alias` 路由），又向后兼容 Account Key（`account_alias=None` 时不注入）。

## ⚠️ 次要问题：initialize 对无状态 server 的容错

**现象**: `src/mcp.rs:113-114` 在 `mcp-session-id` 缺失时直接报错：
```rust
session_id.ok_or_else(|| AppError::External("MCP initialize 未返回 mcp-session-id 头".to_string()))
```

**注释承诺**（line 36-37）但未兑现："值 = ... `None` = server 无状态（initialize 未回 session 头，如无状态 mock）→ 后续请求不带 session 头"。

**影响范围**: 仅影响无状态 server（如本地 mock）；`gewe-multi-tenant` 是有状态 server（**必返回** session-id），对当前生产接入**无影响**。

**修复**（可选，增强测试鲁棒性）:
```rust
// initialize_session 返回 Option<String>，None 表示无状态
let session_id = response.headers().get("mcp-session-id")
    .and_then(|v| v.to_str().ok())
    .map(str::to_string);
// 有状态 server 必须返回 session-id；无状态 server 可返 None
Ok(session_id)

// ensure_session 返回 Option<String>
async fn ensure_session(&self, base_url: &str, api_key: &str) -> AppResult<Option<String>> { ... }

// post_rpc 有条件带 session 头
if let Some(ref sid) = session_id {
    request = request.header("mcp-session-id", sid);
}
```

## 其他观察（非阻断）

1. **auth_whoami 未调用**: guide 推荐接入第一步先跑 `auth_whoami` 验证 Key 权限。当前代码未封装此调用，但不影响工具调用（属"最佳实践"非硬性要求）。可加一个 `whoami_for_account` 辅助方法供管理面板展示 Key 身份。

2. **消息回调配置**: guide 说"每个微信账号槽位都可以配置自己的 `messageWebhookUrl`"——这对应当前 `wechat_accounts` 表可能缺的字段，但 webhook 路由由 server 自动处理（按 `appId` 找账号转发），客户端无需改动，只需确保 DB 里 `account_id` 与 server 的 `alias` 对齐。

3. **media_upload_base64 脱敏**: 已正确实现（`redact_request_for_log`，M16 修复），防 67MB base64 撑爆 mcp_logs。

4. **finding ③ isError 检查**: 已实现（`src/mcp.rs:178-189`），兼容 MCP 标准的工具级失败信号。

## 汇总

| 维度 | 状态 | 说明 |
|---|---|---|
| **核心协议兼容** | ✅ GREEN | Streamable-HTTP / 会话管理 / SSE 解析 / 鉴权全正确 |
| **活体连接测试** | ✅ GREEN | initialize / auth_whoami / tools/list 全通过 |
| **Workspace Key 支持** | 🔴 **缺失** | 账号类工具缺 `account_alias` 注入，当前用不了 Workspace Key |
| **无状态 server 容错** | ⚠️ 未兑现注释 | 对 gewe-multi-tenant 无影响，仅影响 mock 测试 |

**下一步建议**：
1. **立即修**: 路径 A（支持 Workspace Key + account_alias 自动注入）—— 这是用上当前 Key 的唯一路径。
2. **可选修**: 无状态 server 容错（增强测试，非生产阻断）。
3. **最佳实践**: 封装 `auth_whoami` 辅助方法，供管理面板展示 Key 身份和权限。

当前 `src/mcp.rs` 与 `gewe-multi-tenant` 的**协议层 100% 兼容**，唯一阻断是**业务层 account_alias 缺失**——修复后即可正式接入。
