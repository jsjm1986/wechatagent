# roster MCP 限流归入「同步中」应急修复设计

**日期**：2026-07-09
**分支**：新分支基于最新 `origin/main`（已含合并的 #155 = `e2e4ec9`，syncing 态 + 自动重拉已在 main）。旧 worktree `fix/roster-fetch-cache-shape` 已合并过时，勿在其上叠加。
**状态**：设计已获批，待写实现计划

## 背景与症状

#155 修复上线到 117 后，前端通讯录页显示红色错误条 **`internal_error`**（不再是「暂无好友」空态）。

## 根因（2026-07-08/09 线上 117 亲验，非猜测）

- 前端红条 `internal_error` = 后端如实上抛了 MCP server 的 **HTTP 429「Too many MCP SSE connections」**。
- 链路（全部亲验）：
  - MCP server（`/opt/gewe-agent`，我们自己的 Node/TS 系统）`SseConnectionLimiter`（`src/mcp-performance.ts:821`）per-key 并发 SSE 连接数上限 = `MCP_SSE_MAX_CONNECTIONS_PER_KEY`（默认 **20**），超限抛 429。
  - wechatagent 侧 `McpClient::post_rpc`（`src/mcp.rs:144`）对任何非 2xx 一律返回 `AppError::External(format!("MCP HTTP {status}: ..."))`——**把状态码格式化进字符串，丢了类型信息**。
  - `fetch_roster_for_account`（`src/mcp.rs:564`）`logged_call_for_account(...).await?` 用 `?` 直接上抛。
  - `roster_endpoint`（`src/routes/contacts.rs:375`）`?` 上抛 → `error.rs:113-129` 把 `AppError::External` 映射成 `internal_error` + HTTP 502。
- 本次的 429 是**诊断副作用**（反复 MCP `initialize` 未优雅释放 SSE 会话，占满 20 名额），非生产 bug。但**把「上游瞬时限流」当 hard error 弹红条对用户不友好**——429/503 是「稍后重试」语义，应柔化为「同步中」并自动重拉，退避后自愈。

## 范围与非目标

**本设计只做应急柔化**：让 MCP 上游 429/503 归入既有 `syncing` 态，线上不再弹红条。

**非目标（后续独立工作，不在本设计）**：
- roster 数据源根治（改用 MCP server 新增的同步「拉全量+补昵称头像」具名工具）——**由用户在 MCP server 端实现该工具，wechatagent 端届时再做切换适配**，另立 spec。本设计不碰数据源。
- 昵称头像懒加载（模块 3/4）——数据源根治后由新工具一次带回，本设计不涉及。

## 设计（四节，均已获批）

### 第一节：新增错误变体 `AppError::UpstreamBusy`

`src/error.rs`：现有 `RateLimited{retry_after, account_id}` 是 webhook 限流专用（带 account_id / Retry-After 语义），`Http(reqwest::Error)` 是 reqwest 传输错误——都不贴合「MCP server 返回 429/503 body」这个语义。**新增独立变体**：

```rust
/// 上游 MCP server 返回 429/503(SSE 连接数满 / 瞬时不可用)。语义是「稍后重试」，
/// 调用方(如 roster)可捕获并柔化为「同步中」而非硬错误。
#[error("upstream busy: {0}")]
UpstreamBusy(String),
```

`IntoResponse` 映射：503 + `{"error":"upstream_busy"}`（**不**落入 `internal_error` 那组）：

```rust
AppError::UpstreamBusy(_) => (
    StatusCode::SERVICE_UNAVAILABLE,
    Json(json!({ "error": "upstream_busy" })),
)
    .into_response(),
```

（此映射是给「未捕获 UpstreamBusy 的其它调用方」的兜底；roster 路径会在 `fetch_roster_for_account` 内部捕获它转 syncing，不会走到这个 HTTP 映射。）

### 第二节：`post_rpc` 识别 429/503

`src/mcp.rs` `McpClient::post_rpc`（当前 :144）：

当前：
```rust
if !status.is_success() {
    return Err(AppError::External(format!(
        "MCP HTTP {status}: {}",
        truncate_for_error(&body)
    )));
}
```
改为先分类状态码：
```rust
if !status.is_success() {
    let code = status.as_u16();
    if code == 429 || code == 503 {
        return Err(AppError::UpstreamBusy(format!(
            "MCP HTTP {status}: {}",
            truncate_for_error(&body)
        )));
    }
    return Err(AppError::External(format!(
        "MCP HTTP {status}: {}",
        truncate_for_error(&body)
    )));
}
```

注意 404 会话失效分支（当前 :138 `status.as_u16() == 404 && !reinitialized`）**在此之前**，不受影响。

**只处理 HTTP 层 429/503**。MCP 标准可能用 JSON-RPC `error`（HTTP 200 + body 带限流码）表达限流，但**未亲验到此形态的真实样例**，按 YAGNI 不猜测、不处理；`call_tool_with_key`（:180）现有 `body.get("error")` 分支保持返回 `External` 不动。

### 第三节：`fetch_roster_for_account` 归 syncing

`src/mcp.rs` `fetch_roster_for_account`（当前 :551-587）循环体内当前：
```rust
last_result = logged_call_for_account(
    state,
    account_id,
    "contacts_fetch_cache",
    serde_json::json!({}),
)
.await?;
```
`?` 改为显式 match，遇 `UpstreamBusy` 柔化为空 cache（复用既有 syncing 重试路径）：
```rust
match logged_call_for_account(
    state,
    account_id,
    "contacts_fetch_cache",
    serde_json::json!({}),
)
.await
{
    Ok(v) => {
        last_result = v;
    }
    // 上游限流(429/503)：柔化为「同步中」而非硬错误。当作本次空 cache
    // 处理——若还有重试机会则退避重试(退避后 MCP 名额常已释放),用尽仍限流
    // 则返回 syncing:true 让前端提示「同步中」并自动重拉,退避后自愈。
    Err(AppError::UpstreamBusy(_)) => {
        last_result = serde_json::Value::Null;
    }
    // 真实错误(401/500/配置错等)照常上抛 → 前端红条,不掩盖真问题。
    Err(other) => return Err(other),
}
```
后续逻辑不变：`roster_outcome_from_result(&last_result)`（Null → `roster_result_is_empty_cache` 为 true → syncing 路径），循环重试，用尽返回 `Ok(roster_outcome_from_result(&last_result))`（syncing:true, friends 空）。

`roster_endpoint`（contacts.rs:375）`?` **不用改**——`fetch_roster_for_account` 内部已把 UpstreamBusy 转成 `Ok(RosterFetchOutcome{syncing:true})`，不再上抛。前端既有 syncing 态（#155 已实现「正在从微信同步好友…」+每 8s 自动重拉）直接生效。

### 第四节：测试

- `src/error.rs`：`UpstreamBusy` 映射断言——`into_response()` 状态码 503 + body `{"error":"upstream_busy"}`（对齐现有 error 映射测试风格，若无则新增一个最小测试）。
- `src/mcp.rs`：状态码分类。若 `post_rpc` 的分类逻辑能抽成纯函数（如 `fn classify_mcp_http_status(code: u16, body_snippet: String) -> AppError`），则纯函数单测 429→UpstreamBusy、503→UpstreamBusy、500→External、404 由既有会话失效分支处理不进此函数。若不便抽纯函数（`post_rpc` 是 async 方法且耦合 session），则在 `fetch_roster_for_account` 层用可注入的错误做测试，或退而在报告中说明该分支由集成测试覆盖，**不假绿**。
- roster 归 syncing：新增 `roster_outcome_tests` 已覆盖 Null→syncing:true（#155 加的 `empty_object_is_syncing` 同源）；本设计的增量是「UpstreamBusy → last_result=Null → syncing:true」这条路径。若 `fetch_roster_for_account` 依赖 `AppState`（真实 MCP/DB）难以单测，则抽出「错误分类 → 是否柔化」的纯判定做单测，`fetch_roster_for_account` 的集成留 CI。
- 基线门不回退（`cargo test --lib` ≥ 350 + 4 PBT ≥ 33）；no-human-takeover lint 新增行无禁词（`upstream_busy` / 「正在从微信同步好友」均合规）。

## 影响面

- `src/error.rs`：新增 `UpstreamBusy` 变体 + `IntoResponse` 映射。
- `src/mcp.rs`：`post_rpc` 状态码分类；`fetch_roster_for_account` 错误 match。
- 前端：**零改动**（复用 #155 已实现的 syncing 态 + 自动重拉）。

## 部署

wechatagent 单端部署（走既有 117 流程：`git pull` → 前端已无改动可跳过 build → `cargo build --release` → `systemctl restart wechatagent.service`）。不涉及 MCP server。

## 与后续数据源根治的关系

本应急修复上线后，线上通讯录在 MCP 限流/异步 cache 未就绪时显示「同步中」并自动重拉，不再弹红条。待用户在 MCP server 端实现同步「拉全量+补昵称头像」具名工具后，wechatagent 端再另立 spec 做数据源切换（届时 syncing 重试逻辑可能进一步简化为单次同步拉）。两步解耦，本设计独立可上线。
