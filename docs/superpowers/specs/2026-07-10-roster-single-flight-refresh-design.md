# 通讯录后台刷新 single-flight 去重（根治 8s force 轮询叠加打爆 SSE 并发致卡死）

> 日期：2026-07-10 · 状态：设计（待写计划）
> 关联既有设计：[[2026-07-10-roster-backend-snapshot-persist-design]]（PR#162 引入快照层）、
> `2026-07-09-roster-fetch-full-design.md`（切 contacts_fetch_full）、
> `2026-07-09-roster-mcp-ratelimit-syncing-design.md`（限流柔化 syncing）
> 关联代码：`src/mcp.rs`、`src/routes/contacts.rs`、`frontend/src/features/user-ops/RosterView.tsx`、`frontend/src/stores/userOpsStore.ts`

## 背景 / 根因（2026-07-10 117 生产逐处亲验）

PR#162（后端持久化快照）部署后，用户反馈通讯录进频道**长时间显示"正在从微信同步好友…"，等 10-20 分钟仍无好友列表**；而"之前没保存到数据（用 `contacts_fetch_cache` 旧工具）时反而很快能看到"。

### 亲验证据链（file:line + 生产实测）

1. **前端卡在 syncing 态无限轮询**（非 PR#162 文档所述的 502 红条）：
   `RosterView.tsx:212` 显示"正在从微信同步好友…"的条件是 `humanRows.length===0 && syncing===true`；
   `RosterView.tsx:91-97` 每 8s 用 `{force:true}` 自动重拉，直到 `syncing` 变 false。
   `syncing` 值直接来自后端 `GET /api/contacts/roster` 响应的 `data.syncing`（`userOpsStore.ts:498-499`）。
   **前端持续轮询 = 后端持续返回 `syncing:true, items:[]`。**

2. **`contacts_fetch_full` 是异步工具**（本机 curl 直连 `127.0.0.1:3001` 亲测）：
   首次调用立即返回 `{status:"pending", items:[], count:0, refreshing:true}`（0.09s / 360B），
   触发 GeWe 后台拉全量好友；约 **2-9s 波动**后再调才返回 `{status:"ready", items:[4832…], count:4832}`。
   `roster_outcome_from_result`（`mcp.rs:599-600`）据 `status=="ready"` 判就绪，`pending` → `syncing:true`。

3. **同步重试窗口太短**：`fetch_roster_for_account`（`mcp.rs:631-632`）只重试
   `MAX_RETRIES=3` × 间隔 `RETRY_INTERVAL_SECS=2` ≈ 4-6s（最后一次不等待），
   常在 server ready 之前用尽 → 返回 `syncing:true`。

4. **高频 force 轮询叠加 spawn 打爆 SSE 并发（致命循环，PR#162 低估的后果）**：
   前端每 8s `force:true` → `roster_endpoint` force 分支（`contacts.rs:380-399`）每次
   同步调 `fetch_roster_for_account`（3 请求）+ 无快照时 `spawn_roster_refresh`
   （`mcp.rs:721`，**fire-and-forget、无去重锁**，PR#162 第 77/137 行刻意 YAGNI）。
   多个后台任务 + 前端轮询**并发**打同一 MCP server，撞 SSE per-key 并发上限
   （见 [[reference-mcp-server-gewe-agent-internals]] SseConnectionLimiter 默认 20），
   连接排队 → `response.text().await?`（`mcp.rs:136`）读 **4832 好友 1.37MB 大 body** `TimedOut`
   → `reqwest::Error{kind:Decode, source:Body, TimedOut}` → 日志刷屏 `error decoding response body`。
   **每次 TimedOut 又中断该次拉取**，后台任务全部 attempt 用尽"放弃" → 快照永远写不进
   → `roster_snapshots` 恒空（生产亲验 `count=0`）→ 前端永远 syncing → 死循环。

5. **手工焐热验证根因**：单路轮询 `contacts_fetch_full` 到 ready 后，后端某次重试命中，
   快照成功写入（`total:4832, 1.37MB, 读取 49ms`），后端 roster 报错立即停止刷屏，
   前端可出好友。证明 server 侧健康、数据真实存在，卡死纯由"重试窗口 + 并发自我限流"造成。

### 为什么"之前很快"

旧代码用 `contacts_fetch_cache`（只读小缓存、body 小、不触发全量同步拉取），返回快。
切 `contacts_fetch_full`（全量异步、大 body）后暴露"异步 ready 时间 > 同步窗口"+"高频并发自噬"两缺陷。

### PR#162 被低估的假设（本设计修正点）

PR#162 设计文档明确选择"后台自刷 best-effort fire-and-forget、**不加去重/分布式锁**"
（第 37/77/137 行），理由是"重复 spawn 覆盖写同一条**无害**"。
该判断只看到"写快照无害"，**漏了"多任务并发抢 SSE 连接名额 → 大 body 读取 TimedOut → 相互中断"这一真实后果**。
本设计不推翻快照层（方向正确），仅补上被低估的并发控制。

## 交互模型（已与用户对齐）：异步单飞 + 前端只读轮询

首次无快照（或快照过期）时：后端**立即返回 `syncing:true`**（不再同步阻塞拉全量），
后台**单飞（single-flight）**任务拉取写快照；前端**只读轮询**（不再高频 force 触发新拉取），
后台写好快照后下一轮普通轮询秒出。

## 设计（三支柱）

### 支柱 1 · 后台刷新 single-flight 去重锁（核心）

- `McpClient`（`mcp.rs:28`）新增字段 `roster_refreshing: Arc<DashMap<String, ()>>`
  （键 = `account_id`），与现有 `sessions: Arc<DashMap>`（`mcp.rs:38`）同款模式，
  `#[derive(Clone)]` 下 clone 共享同一份、进程内全局唯一。`McpClient::new`（`mcp.rs:42`）初始化空 map。
- `spawn_roster_refresh`（`mcp.rs:721`）进入时抢锁：
  ```rust
  if state.mcp.roster_refreshing.insert(account_id.clone(), ()).is_some() {
      return; // 已有同账号任务在拉 → 放弃（DashMap::insert 返回旧值，原子、无 TOCTOU）
  }
  let _guard = RosterRefreshGuard { map: state.mcp.roster_refreshing.clone(), key: account_id.clone() };
  // …现有 5 轮退避重试循环不变…
  ```
- `RosterRefreshGuard` 实现 `Drop`（RAII）：drop 时 `map.remove(&key)`，
  保证任务正常结束 / 提前 return / **panic** 时锁都释放（`tokio::spawn` 内 panic 不传播，guard drop 仍执行）。
- 效果：全局同一账号同时只有一个后台拉取任务；前端多次轮询、多标签页、force 连点都不再叠加 spawn
  → 消除并发自我限流 → server ~10s ready 能在退避第 2-3 轮（间隔 3/6/12s）命中写入快照。

### 支柱 2 · 前端改「只读轮询」，不再高频 force 全量

- `RosterView.tsx:91-97` 的 8s 自动重拉：**去掉 `{force:true}`**，改普通读。
  普通读走 `read_roster_snapshot` 秒回、不触发新的同步拉取；后台单飞写好快照后普通轮询自然读到。
- 轮询间隔 `8000` → `10000`（适度拉长，降低无谓请求；覆盖 server ready 时间有余量）。
- `force:true` 仅保留给用户**手动点「刷新」按钮**（`RosterView.tsx:188`），频率极低，不再叠加。

### 支柱 3 · 同步端点不再阻塞拉全量

- `roster_endpoint` **无快照分支**（`contacts.rs:409-422，非 force`）：
  去掉同步 `fetch_roster_for_account` 调用（6s 阻塞 + 常拿 pending + 占一个连接名额，纯浪费），改为：
  ```rust
  None => {
      spawn_roster_refresh(state.clone(), ws.clone(), acc.clone()); // 后台单飞
      (Vec::new(), true) // 立即返回 syncing:true，前端进只读轮询
  }
  ```
- `roster_endpoint` **force 分支**（`contacts.rs:380-399`）：也改为"触发单飞后台刷新 + 立即返回当前快照"
  （有旧快照返回旧的、后台刷新覆盖；无旧快照 → `syncing:true` + spawn）。不再同步阻塞 6s。
- **有快照分支**（`contacts.rs:402-408`）不变：秒回快照；stale(>24h) 时 `spawn_roster_refresh`（现由单飞锁保护）。
- `fetch_roster_for_account` 本体（3 次同步重试）**不删**——仍是 `spawn_roster_refresh` 后台循环每轮调用的单位。
- agent_status 拼装（`contacts.rs:427-460`）、返回体 `{items,total,syncing}`（`contacts.rs:462`）**不变**。

## 测试计划（守 `cargo test --lib ≥ 350 passed / 0 failed` 基线，只增不减）

lib 单测（纯并发结构，不依赖 MCP/Docker）：

1. **锁去重**：同一 key 连续两次 `roster_refreshing.insert` → 第二次返回 `Some`（应放弃）。
2. **guard 释放**：`RosterRefreshGuard` drop 后再 insert 同 key → 返回 `None`（锁已释放可重抢）。
3. **panic 释放**：`std::panic::catch_unwind` 包住持有 guard 的作用域触发 panic，
   验证 unwind 后 map 该键已被移除（Drop 在 panic 时仍执行）。
4. **多账号独立**：不同 account_id 各持有键、互不阻塞。
5. 现有 `roster_outcome_from_result` / `parse_roster_items` / `snapshot_is_stale` /
   `roster_refresh_backoff_secs` 等单测（`mcp.rs:1056-1134` 等）**全部保留不动**（回归守卫）。

前端（`vitest`）：
- `RosterView.test.tsx` / `roster.test.tsx` 现有用例保留；若有断言 8s 自动重拉带 force 的，
  更新为"自动轮询不带 force、仅手动刷新带 force"。

验证（本地 → 117）：
1. 本地 `cargo test --lib` 过 350 基线 + `cargo check`。
2. 前端 `npm run build` 过（`tsc --noEmit` 0 error + `vitest run` 全绿）。
3. `scripts/check-no-human-takeover.{sh,ps1}` lint 自检（roster 不涉禁词，过场）。
4. 部署 117（git bundle + paramiko 一条龙脚本，见 [[deploy-server-117]] 三查）。
5. **117 真实验证**：删现有快照 → 进通讯录频道 → 观察：
   首次 `syncing:true` 立即返回、后台单飞任务**无刷屏**、约 10s 后快照写入、
   前端只读轮询秒出 4832 好友；`mcp_call_logs` 不再刷 `error decoding response body`；
   多次快速刷新 / 多标签页不产生并发叠加（journalctl 无多任务交错 attempt 日志）。

## YAGNI（明确不做）

- 不修「昵称/头像 null 需 getBriefInfo 补」——独立数据质量问题（另有 `2026-07-10-roster-sex-parse-nonhuman-design.md`），非卡死根因。
- 不改 MCP server（另一 Node/TS 系统）、不改 SSE 并发上限。
- 不引入分布式锁——进程内 `DashMap` 够用（单副本部署；横向扩多副本才需 DB 原子 claim，届时再议，与 webhooks.rs PENDING 同 caveat）。
- 不改快照 24h 过期阈值、不改 `fetch_roster_for_account` 的 3 次同步重试常量、不动限流柔化逻辑。
- 不推翻 PR#162 快照层（方向正确，仅补并发控制）。

## 部署提醒

带前端改动，部署须 `cargo build --release` + `cd frontend && npm run build`
（见 [[deploy-server-117]] 完整性三查：后端二进制 mtime + 前端 dist 内容双验）。
无新增集合 / 索引（复用 PR#162 的 `roster_snapshots`），无需迁移。
