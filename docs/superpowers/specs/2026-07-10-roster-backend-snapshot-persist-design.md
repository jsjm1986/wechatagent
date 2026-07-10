# 通讯录后端持久化快照（进频道秒回 + 过期后台自刷 + 传输失败兜底）

> 日期：2026-07-10 · 分支：feat/roster-backend-snapshot · 状态：设计（待写计划）

## 背景 / 根因（已亲验，2026-07-10）

通讯录切 `contacts_fetch_full`（#158）+ 性别/非真人/前端缓存（#160）上线后，用户反馈：**每次进通讯录频道都长时间加载、有时转圈**。

根因链（逐处亲验 file:line）：

1. `contacts_fetch_full` 就绪返回 4832 条好友的**大 body**（`{status:"ready", items:[4832…], count, fetchedAt, refreshing, lastError}`）。
2. reqwest 读该大 body：`response.text().await?`（`src/mcp.rs:104` initialize / `src/mcp.rs:136` post_rpc）。传输中断时抛 `reqwest::Error`，错误串是 `error decoding response body`。
3. `reqwest::Error` 经 `#[from]` 转成 **`AppError::Http`**（`src/error.rs:25`）。
4. `call_tool_with_key` → `post_rpc` 的 `?` 把它上抛；`fetch_roster_for_account`（`src/mcp.rs:607`）的匹配分支：
   - `Err(AppError::UpstreamBusy(_))`（429/503）→ 柔化为空 cache、继续重试（`src/mcp.rs:633`）。
   - **`Err(other) => return Err(other)`（`src/mcp.rs:637`）→ 解码失败走这条，直接上抛，不重试。**
5. `roster_endpoint`（`src/routes/contacts.rs:375`）拿到 `AppError::Http` → `IntoResponse` 映射 502 `upstream_error`（`src/error.rs:118-139`）。

**事实修正（澄清此前表述）**：解码失败是 502 红条（前端走 `catch` setError），**不是** `syncing:true` 的每 8s 无限重拉。所以"一直转圈"的真实成因是「首次 3 次重试各撞 60s 超时 + 大 body 慢/不稳偶发解码失败」，而非无限轮询。持久化快照能同时根治「慢」与「传输不稳」两种表现，故方向不变。

### 已亲验的实现事实（影响设计）

- `logged_call_for_account`（`src/mcp.rs:329`）写 `mcp_call_logs` 用的 workspace 是 `state.config.default_workspace_id`（`src/mcp.rs:358`），**不是**请求的 `admin.current_workspace`——MCP 层（凭证/日志）整体绑定 `default_workspace_id`（单租户实态）。
- `roster_endpoint` 的入口守卫 `validate_account` 与 contacts 查询用的是 `admin.current_workspace`（`src/routes/contacts.rs:373,383`）。单租户下两者相等。**快照键取 `current_workspace + account_id`**，与 contacts 查询同维度（多租户启用时不串）。
- `McpCallLog`（`src/models.rs:949`）`response: Option<Document>` 每次调用都写完整 body（含 4832 条），是**审计日志**：查「最近一次成功 ready 全量」很别扭、大 body 反复落库有存储压力。故**不复用 mcp_call_logs 当快照源**，另建专用集合。
- 后台异步范式（fire-and-forget）：`src/agent/decision_taxonomy.rs:135` —— `db.clone()` + owned params → `tokio::spawn`，失败仅 `tracing::warn`。
- `AppState` 是 `#[derive(Clone)]`（`src/routes/mod.rs:288`），可 move 进 `tokio::spawn`。
- `RosterFriend`（`src/mcp.rs:421`）当前仅 `Serialize`，从 DB 读回需补 `Deserialize`。
- db accessor 范式：`self.db.collection("name")`（`src/db/mod.rs`）；索引范式：`IndexModel::builder()`（`src/db/indexes.rs`），由 `Database::ensure_indexes` 统一建。
- 前端 `loadRoster`（`frontend/src/stores/userOpsStore.ts:490`）已有 `force` 语义，但 **force 未拼进后端 URL**（`:496` URL 无 force 参数）——即此前「刷新」按钮打的一直是无 force 的接口。本次一并修正。

## 设计决策（已与用户对齐）

1. **读快照优先**：进频道读 `roster_snapshots` DB 快照秒回，不打 MCP。
2. **过期阈值 24 小时**：快照龄 ≤ 24h 直接秒回、不刷；> 24h 才**后台静默自刷**（fire-and-forget，不阻塞本次响应，新数据下次进频道生效）。
3. **有快照永远兜底**：任何 MCP 失败（解码错误/超时/限流）下，只要有旧快照就返回旧快照，绝不让页面转圈/空白。
4. **首次无快照且失败**：返回 `syncing:true`（前端显「同步中，稍后重试」）+ 后台重试任务（**解码失败也重试**，不只限 429/503）。拉到就绪即写快照，之后永远秒回。
5. **后台重试上限**：最多 5 次、间隔递增 3→6→12→24→48s，用尽仍失败则放弃（等下次进频道再触发），避免大 body 不稳时后台无限打 MCP。
6. **force 刷新**：用户点「刷新」→ `?force=true` 透传后端 → 跳过快照同步拉 → 就绪则覆盖写快照 + 返回；失败则回退旧快照（有则用），刷新失败不空白。

## 数据模型

新增集合 `roster_snapshots`，typed model `RosterSnapshot`（`src/models.rs`）：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RosterSnapshot {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub friends: Vec<RosterFriend>,   // 复用 RosterFriend（需补 Deserialize）
    pub total: i64,
    pub fetched_at: DateTime,         // 快照写入时刻 → 算 24h 过期
}
```

- `RosterFriend`（`src/mcp.rs:421`）补 `#[derive(Deserialize)]`（现仅 Serialize）——从 DB 读回需要。这是唯一对既有类型的改动。
- `db.roster_snapshots()` accessor（`src/db/mod.rs`）：`self.db.collection("roster_snapshots")`。
- 唯一索引 `(workspace_id, account_id)`（`src/db/indexes.rs`，unique）→ `replace_one(filter, doc, upsert=true)` 覆盖写，每账号恒一条。

## 实现范围

### 后端 `src/mcp.rs`

- `RosterFriend` 加 `Deserialize`。
- 新增纯函数（便于确定性单测）：
  - `snapshot_is_stale(fetched_at: DateTime, now: DateTime) -> bool`：`now - fetched_at > 24h`。
  - `roster_refresh_backoff_secs(attempt: usize) -> u64`：attempt 0→3, 1→6, 2→12, 3→24, 4→48。
  - `const ROSTER_REFRESH_MAX_RETRIES: usize = 5;`
  - `const ROSTER_SNAPSHOT_STALE_HOURS: i64 = 24;`
- 新增后台自刷/重试函数 `spawn_roster_refresh(state: AppState, workspace_id: String, account_id: String)`：
  - `tokio::spawn` fire-and-forget（复用 decision_taxonomy 范式）。
  - 内部循环最多 `ROSTER_REFRESH_MAX_RETRIES` 次调 `fetch_roster_for_account`；**任何错误（含 `AppError::Http` 解码失败）都 catch、退避重试**（不像同步路径直接上抛）。
  - 拿到就绪结果（`!syncing`）→ `replace_one` upsert 写 `roster_snapshots` → 结束。
  - 用尽重试仍未就绪 → `tracing::warn` 放弃（下次进频道触发）。
  - 幂等/并发：best-effort，重复 spawn 只会多打几次 MCP + 覆盖写同一条，无害（YAGNI，不加分布式锁）。
- `fetch_roster_for_account` 本体**不改**（同步首次拉仍用现有 3 次重试逻辑；后台任务是独立的更健壮循环）。

### API 层 `src/routes/contacts.rs`

- `RosterQuery` 加 `pub force: Option<bool>`（camelCase wire `force`）。
- `roster_endpoint` 重写读写流程：
  ```
  1. validate_account（不变）
  2. 读 roster_snapshots[current_workspace + account_id]
  3. force==Some(true):
       同步 fetch_roster_for_account
         ├─ ready → replace_one 覆盖写快照 + 返回 items（syncing:false）
         └─ 失败/syncing → 若有旧快照则返回旧快照(syncing:false)；否则 syncing:true + spawn 后台重试
  4. 非 force:
       ├─ 有快照 → 秒回快照 items；若 stale(>24h) → spawn_roster_refresh 后台自刷
       └─ 无快照 → 同步 fetch_roster_for_account
             ├─ ready → 写快照 + 返回
             └─ 失败/syncing → 返回 syncing:true + spawn 后台重试
  ```
- agent_status 拼装逻辑（读 contacts、wxid→managed/normal/not_imported）**不变**，套在「拿到 friends（无论来自快照还是实时）之后」。
- 返回体不变：`{items, total, syncing}`。

### 前端 `frontend/src/stores/userOpsStore.ts`

唯一改动：`loadRoster` 把 force 透传到 URL（`:495-497`）：
```ts
const url = `/api/contacts/roster?accountId=${encodeURIComponent(accountId)}${opts?.force ? "&force=true" : ""}`;
const data = await api.get<{ items: RosterEntry[]; syncing?: boolean }>(url);
```

`RosterView.tsx` **不动**：8s 自动重拉、非真人折叠、分页、性别全保留。后端快照秒回后 `syncing:false`，前端轮询自然停。

前端 `rosterCache`（session 内存）与后端 DB 快照**分层共存**：前者管「本会话不重复打 API」，后者管「跨会话/重启不重打 MCP + 兜底传输失败」。

## 测试计划

后端（`cargo test --lib`，纯函数 + serde，不依赖 MCP/Docker）：
- `RosterSnapshot` BSON round-trip：含 sex/is_non_human 的 `RosterFriend` 序列化写盘再读回一致（验 `RosterFriend` 的 `Deserialize`）。
- `snapshot_is_stale`：23h→false、25h→true、边界 24h。
- `roster_refresh_backoff_secs`：0→3,1→6,2→12,3→24,4→48。
- 现有 `parse_roster_items` / `roster_outcome_from_result` / `is_non_human` 测试全保留不动（回归守卫）。

前端（`vitest`）：
- 现有 roster.test.tsx 11 用例全保留。
- 新增：`loadRoster(acc,{force:true})` → getMock 收到的 URL 含 `&force=true`；非 force → 不含。

集成层（`#[ignore]`，留 CI/Docker，testcontainers Mongo）：
- 有快照 + 非 force → 秒回快照、`fetch_roster_for_account` 不被调用（可用「无 MCP 凭证仍返回快照」间接验）。
- 无快照 → 走同步拉 → 写入 `roster_snapshots` 一条。
- force → 覆盖写快照。

基线门：`cargo test --lib` ≥ 350 passed / 0 failed；前端 `tsc --noEmit` 0 error + `vitest run` 全绿。

## YAGNI（明确不做）

- 快照 TTL 自动删除（好友列表长期有效，覆盖写即可，不设 TTL 索引）。
- 快照历史版本 / 审计（只存最新一份，覆盖写）。
- 多租户真隔离加固（沿用现状 `default_workspace_id` 绑定 MCP 层；快照键按 `current_workspace` 与 contacts 查询同维，单租户无害）。
- localStorage 前端持久化（后端快照已是跨会话真相源）。
- 后台自刷的分布式锁 / 去重（best-effort fire-and-forget，重复 spawn 覆盖写同一条无害）。

## 部署提醒

带前端改动，部署须 `cargo build --release` + `cd frontend && npm run build`（见 [[deploy-server-117]] 部署完整性三查：后端二进制 mtime + 前端 dist 内容双验）。新增集合首次启动由 `ensure_indexes` 自动建索引，无需手动迁移。
