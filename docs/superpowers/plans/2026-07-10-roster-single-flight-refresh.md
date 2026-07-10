# 通讯录后台刷新 single-flight 去重 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给 roster 后台刷新加进程内 single-flight 去重锁，并让前端只读轮询、同步端点不阻塞拉全量，根治"每 8s force 轮询叠加 spawn 不去重后台任务 → 并发打爆 MCP SSE 并发上限 → 1.37MB 大 body 读取 TimedOut → 快照永远写不进 → 前端无限卡'正在从微信同步好友'"。

**Architecture:** 后端 `McpClient` 加 `roster_refreshing: Arc<DashMap<String,()>>` 做 per-account in-flight 标记；`spawn_roster_refresh` 进入即抢锁（`insert` 返回 `Some` 则放弃），RAII guard 保证 panic 也释放。`roster_endpoint` 无快照/force 分支不再同步阻塞 `fetch_roster_for_account`，改为立即 `spawn_roster_refresh` + 返回 `syncing:true`（或旧快照）。前端自动轮询去掉 `force`、间隔 8s→10s，只有手动"刷新"按钮才 force。

**Tech Stack:** Rust 2021 / Axum / dashmap / tokio；前端 React 19 + TypeScript + Vite + vitest。

## Global Constraints

- `cargo test --lib` **≥ 350 passed, 0 failed**（`scripts/check-baseline.{sh,ps1}` 合并门）——新工作只加测试不降基线。
- 累计 4 个 PBT 文件 **≥ 33 passed, 0 failed**（`state_transition_pbt`/`memory_card_invariants`/`wiki_chunk_revision_pbt`/`llm_retry_jitter`）——本计划不碰这些，保持即可。
- 不改 MCP server（外部 Node/TS 系统）、不改 SSE 并发上限、不引入分布式锁（进程内 DashMap 单副本足够）。
- 不推翻 PR#162 快照层：复用现有 `roster_snapshots` 集合、`RosterSnapshot`、`snapshot_is_stale`、`fetch_roster_for_account`、`read_roster_snapshot`、`write_roster_snapshot`——仅补并发控制与端点/前端调用方式。
- `scripts/check-no-human-takeover.{sh,ps1}` lint：`src/**` / `frontend/src/**` 新增行禁 `人工接管|takeover|hand-off|人工` 等词——roster 改动不涉，保持即可。
- 分支：`fix/roster-single-flight-refresh`（已建，基于 827f6de / #162，已含设计文档 commit eab2827）。
- 提交：仅 `git add` 具名文件，绝不 `git add -A`；每 Task 末尾一次 commit。

---

### Task 1: McpClient 加 `roster_refreshing` in-flight map 字段

**Files:**
- Modify: `src/mcp.rs:28-39`（`struct McpClient` 加字段）
- Modify: `src/mcp.rs:42-51`（`McpClient::new` 初始化字段）

**Interfaces:**
- Consumes: 现有 `sessions: Arc<DashMap<String, Option<String>>>` 模式（`mcp.rs:38`）。
- Produces: `McpClient.roster_refreshing: std::sync::Arc<dashmap::DashMap<String, ()>>`（键 = `account_id`），供 Task 2 的 `spawn_roster_refresh` 抢锁。字段随 `#[derive(Clone)]` clone 共享同一份 `Arc`。

- [ ] **Step 1: 加字段**

在 `src/mcp.rs` 的 `struct McpClient`（`:28`）内，`sessions` 字段（`:38`）之后加：

```rust
    /// roster 后台刷新的 per-account in-flight 去重标记（键 = account_id）。
    /// `spawn_roster_refresh` 抢锁：键已存在 → 放弃本次 spawn（全局同一账号同时只有
    /// 一个后台拉取任务），消除"前端 8s force 轮询叠加 spawn → 并发打爆 MCP SSE
    /// 并发上限 → 大 body 读取 TimedOut → 相互中断"的自我限流循环。
    /// 与 `sessions` 同款 Arc<DashMap>：#[derive(Clone)] 下 clone 共享、进程内全局唯一。
    roster_refreshing: std::sync::Arc<dashmap::DashMap<String, ()>>,
```

- [ ] **Step 2: 初始化字段**

在 `McpClient::new`（`:42-51`）的 `Ok(Self { ... })` 内，`sessions:` 行（`:49`）之后加：

```rust
            roster_refreshing: std::sync::Arc::new(dashmap::DashMap::new()),
```

- [ ] **Step 3: 编译验证**

Run: `cargo check`
Expected: 通过（新字段暂未被读，可能有 `dead_code` warning，Task 2 接入后消除；若 CI 视 warning 为错则本步先接受 warning，Task 2 一并消除）。

- [ ] **Step 4: Commit**

```bash
git add src/mcp.rs
git commit -m "feat(roster): McpClient 加 roster_refreshing in-flight map 字段"
```

---

### Task 2: RosterRefreshGuard + spawn_roster_refresh 抢锁去重

**Files:**
- Modify: `src/mcp.rs:721-749`（`spawn_roster_refresh` 加抢锁 + guard）
- Create: `src/mcp.rs` 内新增 `struct RosterRefreshGuard`（放在 `spawn_roster_refresh` 之前）
- Test: `src/mcp.rs` 内 `#[cfg(test)]` 模块新增单测

**Interfaces:**
- Consumes: `McpClient.roster_refreshing`（Task 1）；现有 `spawn_roster_refresh(state: AppState, workspace_id: String, account_id: String)`（`mcp.rs:721`）签名不变；现有 `fetch_roster_for_account` / `write_roster_snapshot` / `ROSTER_REFRESH_MAX_RETRIES` / `roster_refresh_backoff_secs` 均不变。
- Produces: `RosterRefreshGuard { map: Arc<DashMap<String,()>>, key: String }`，`impl Drop` 移除键。测试通过 `McpClient` 无法直接访问私有字段，故 guard 逻辑用**独立可测函数/结构**承载（见下，测试直接构造 `Arc<DashMap>` + guard 验证，不经 McpClient）。

- [ ] **Step 1: 写失败测试**

在 `src/mcp.rs` 末尾的测试区新增（若已有 `#[cfg(test)] mod tests` 就并入，否则新建 `mod roster_refresh_lock_tests`）：

```rust
#[cfg(test)]
mod roster_refresh_lock_tests {
    use super::RosterRefreshGuard;
    use std::sync::Arc;

    #[test]
    fn second_insert_same_key_is_rejected() {
        let map: Arc<dashmap::DashMap<String, ()>> = Arc::new(dashmap::DashMap::new());
        // 首次抢锁成功（insert 返回 None）。
        assert!(map.insert("acc1".to_string(), ()).is_none());
        // 同 key 二次抢锁：insert 返回 Some(旧值) → 调用方应放弃。
        assert!(map.insert("acc1".to_string(), ()).is_some());
    }

    #[test]
    fn guard_drop_releases_key() {
        let map: Arc<dashmap::DashMap<String, ()>> = Arc::new(dashmap::DashMap::new());
        {
            map.insert("acc1".to_string(), ());
            let _guard = RosterRefreshGuard { map: map.clone(), key: "acc1".to_string() };
            assert_eq!(map.len(), 1);
        } // guard drop 此处释放
        assert_eq!(map.len(), 0, "guard drop 后键应被移除");
        // 释放后可重新抢锁。
        assert!(map.insert("acc1".to_string(), ()).is_none());
    }

    #[test]
    fn guard_releases_on_panic() {
        let map: Arc<dashmap::DashMap<String, ()>> = Arc::new(dashmap::DashMap::new());
        let map_for_closure = map.clone();
        let result = std::panic::catch_unwind(move || {
            map_for_closure.insert("acc1".to_string(), ());
            let _guard = RosterRefreshGuard {
                map: map_for_closure.clone(),
                key: "acc1".to_string(),
            };
            panic!("模拟后台任务体 panic");
        });
        assert!(result.is_err(), "闭包应 panic");
        assert_eq!(map.len(), 0, "panic unwind 后 guard Drop 仍应移除键");
    }

    #[test]
    fn distinct_accounts_are_independent() {
        let map: Arc<dashmap::DashMap<String, ()>> = Arc::new(dashmap::DashMap::new());
        assert!(map.insert("acc1".to_string(), ()).is_none());
        assert!(map.insert("acc2".to_string(), ()).is_none(), "不同账号各自独立键，互不阻塞");
        assert_eq!(map.len(), 2);
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib roster_refresh_lock`
Expected: 编译失败（`RosterRefreshGuard` 未定义 / 未 `pub`）。

- [ ] **Step 3: 定义 RosterRefreshGuard + 接入 spawn_roster_refresh**

在 `src/mcp.rs` 的 `pub fn spawn_roster_refresh`（`:721`）**之前**新增（`pub(crate)` 使同文件测试模块可见其字段）：

```rust
/// roster 后台刷新的 RAII 去重锁 guard：drop 时移除 in-flight 键，保证任务
/// 正常结束 / 提前 return / **panic**（tokio::spawn 内 panic 不传播，但 Drop 仍执行）
/// 时锁都释放，避免键泄漏后该账号永远无法再刷新。
pub(crate) struct RosterRefreshGuard {
    pub(crate) map: std::sync::Arc<dashmap::DashMap<String, ()>>,
    pub(crate) key: String,
}

impl Drop for RosterRefreshGuard {
    fn drop(&mut self) {
        self.map.remove(&self.key);
    }
}
```

把 `spawn_roster_refresh`（`:721-749`）改为进入 spawn 后**先抢锁**：

```rust
pub fn spawn_roster_refresh(state: AppState, workspace_id: String, account_id: String) {
    tokio::spawn(async move {
        // single-flight 抢锁：键已存在 → 已有同账号后台任务在拉，直接放弃（去重）。
        // insert 返回旧值：Some(_)=已占用→放弃；None=抢到→继续。原子，无 TOCTOU。
        if state
            .mcp
            .roster_refreshing
            .insert(account_id.clone(), ())
            .is_some()
        {
            return;
        }
        // RAII：本作用域结束（含 return / panic）自动 remove 键释放锁。
        let _guard = RosterRefreshGuard {
            map: state.mcp.roster_refreshing.clone(),
            key: account_id.clone(),
        };
        for attempt in 0..ROSTER_REFRESH_MAX_RETRIES {
            match fetch_roster_for_account(&state, &account_id).await {
                Ok(outcome) if !outcome.syncing => {
                    if let Err(err) =
                        write_roster_snapshot(&state, &workspace_id, &account_id, &outcome.friends)
                            .await
                    {
                        tracing::warn!(?err, account_id = %account_id, "roster 快照写入失败");
                    }
                    return;
                }
                Ok(_) => {}
                Err(err) => {
                    tracing::warn!(?err, account_id = %account_id, attempt, "roster 后台刷新单次失败,退避重试");
                }
            }
            if attempt + 1 < ROSTER_REFRESH_MAX_RETRIES {
                tokio::time::sleep(std::time::Duration::from_secs(
                    roster_refresh_backoff_secs(attempt),
                ))
                .await;
            }
        }
        tracing::warn!(account_id = %account_id, "roster 后台刷新用尽重试仍未就绪,放弃(下次进频道再触发)");
    });
}
```

`roster_refreshing` 字段的 `#[allow(dead_code)]`（若 Task 1 加过）此时移除——字段已被读。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --lib roster_refresh_lock`
Expected: 4 个测试全 PASS。

- [ ] **Step 5: 全量 lib 基线**

Run: `cargo test --lib`
Expected: ≥ 350 passed, 0 failed（新增 4 个 → 应比基线多 4）。

- [ ] **Step 6: Commit**

```bash
git add src/mcp.rs
git commit -m "feat(roster): spawn_roster_refresh single-flight 抢锁去重 + RAII guard"
```

---

### Task 3: roster_endpoint 无快照/force 分支不再同步阻塞拉全量

**Files:**
- Modify: `src/routes/contacts.rs:380-425`（`roster_endpoint` 的 force 分支 + 无快照分支）

**Interfaces:**
- Consumes: `mcp::spawn_roster_refresh`（Task 2）、`mcp::read_roster_snapshot`、`mcp::snapshot_is_stale`（均已存在）。
- Produces: `roster_endpoint` 行为变更——首次无快照 / force 时立即返回 `syncing:true`（或旧快照），后台单飞拉取。返回体 `{items,total,syncing}` 形状不变，前端契约不变。

- [ ] **Step 1: 改 force 分支**

`src/routes/contacts.rs` 的 `if query.force { ... }`（`:380-399`）整块替换为：

```rust
    let (friends, syncing): (Vec<mcp::RosterFriend>, bool) = if query.force {
        // 强制刷新（仅用户手动点「刷新」触发，前端自动轮询不带 force）：
        // 不再同步阻塞拉全量（异步工具首返 pending、大 body 易 TimedOut、占连接名额）。
        // 触发后台单飞刷新；有旧快照先返回旧快照（后台写好下次秒回），无则 syncing:true。
        mcp::spawn_roster_refresh(state.clone(), ws.clone(), acc.clone());
        match mcp::read_roster_snapshot(&state, ws, acc).await? {
            Some(snap) => (snap.friends, false),
            None => (Vec::new(), true),
        }
    } else {
```

- [ ] **Step 2: 改无快照分支**

紧接着的 `else { ... }` 块内，现有 `match mcp::read_roster_snapshot(...) { Some(snap) => {...}, None => match mcp::fetch_roster_for_account(...) {...} }`（`:401-424`）中的 **`None =>` 分支**整体替换为：

```rust
            None => {
                // 首次无快照：不再同步阻塞 fetch_roster_for_account（6s 窗口常拿 pending、
                // 占连接名额）。立即返回 syncing:true，后台单飞拉取，前端进只读轮询，
                // 后台写好快照后下一轮普通读秒出。
                mcp::spawn_roster_refresh(state.clone(), ws.clone(), acc.clone());
                (Vec::new(), true)
            }
```

`Some(snap) => { if stale { spawn_roster_refresh } (snap.friends, false) }`（`:402-408`）**保持不变**。

- [ ] **Step 3: 编译 + 确认无用 import**

Run: `cargo check`
Expected: 通过。若 `fetch_roster_for_account` 在 contacts.rs 内不再被直接引用而报未使用 import，删除对应 `use`（`fetch_roster_for_account` 仍被 `spawn_roster_refresh` 内部使用，只是 contacts.rs 不再直接调）。

- [ ] **Step 4: 全量 lib 基线**

Run: `cargo test --lib`
Expected: ≥ 350 passed, 0 failed（本 Task 不加删单测，数量同 Task 2 末）。

- [ ] **Step 5: Commit**

```bash
git add src/routes/contacts.rs
git commit -m "fix(roster): 端点无快照/force 分支改后台单飞+立即返回,不再同步阻塞拉全量"
```

---

### Task 4: 前端自动轮询改只读 + 间隔 8s→10s

**Files:**
- Modify: `frontend/src/features/user-ops/RosterView.tsx:91-97`（自动重拉 effect）
- Test: `frontend/src/__tests__/features/user-ops/roster.test.tsx`（若现有用例断言了自动轮询带 force，更新）

**Interfaces:**
- Consumes: 现有 `refresh(accountId, opts?)`（`RosterView.tsx:59`）、`loadRoster`（`userOpsStore.ts:490`，force 透传 URL 逻辑不变）。
- Produces: 自动轮询用**非 force** 读（只读快照，不触发新 spawn）；手动「刷新」按钮（`:188`）仍 `{force:true}`。

- [ ] **Step 1: 改自动轮询 effect**

`frontend/src/features/user-ops/RosterView.tsx` 的 cache 同步中自动重拉 effect（`:90-97`）替换为：

```tsx
  // cache 同步中时每 10s 自动重拉（只读快照，不带 force→不触发新的后台拉取）；
  // 后台单飞任务写好快照后，普通读自然读到、syncing 变 false、轮询自停。
  useEffect(() => {
    if (!syncing || !effectiveAccountId) return;
    const timer = setInterval(() => {
      void refresh(effectiveAccountId);
    }, 10000);
    return () => clearInterval(timer);
  }, [syncing, effectiveAccountId, refresh]);
```

（改动点：`refresh(effectiveAccountId, { force: true })` → `refresh(effectiveAccountId)`；`8000` → `10000`；注释更新。）

- [ ] **Step 2: 检查现有测试是否断言自动轮询 force**

Run: `cd frontend && npx vitest run src/__tests__/features/user-ops/roster.test.tsx`
Expected: 若全绿 → 无需改测试，进 Step 4。若有用例因"自动轮询不再带 force"失败 → Step 3 更新该用例。

- [ ] **Step 3: （条件）更新受影响用例**

仅当 Step 2 有失败时：把断言"自动轮询请求 URL 含 `force=true`"的用例，改为断言"自动轮询 URL **不含** `force=true`、手动点刷新按钮 URL **含** `force=true`"。示例断言形态（按现有测试的 mock 风格适配）：

```tsx
// 自动轮询（syncing 态定时器触发）→ 请求不带 force
expect(getMock).toHaveBeenLastCalledWith(
  expect.not.stringContaining("force=true")
);
// 用户点「刷新」按钮 → 请求带 force
fireEvent.click(screen.getByRole("button", { name: /刷新/ }));
await waitFor(() =>
  expect(getMock).toHaveBeenLastCalledWith(expect.stringContaining("force=true"))
);
```

- [ ] **Step 4: 前端类型检查 + 全量前端测试**

Run: `cd frontend && npx tsc --noEmit && npx vitest run`
Expected: 0 type error，vitest 全绿。

- [ ] **Step 5: Commit**

```bash
git add frontend/src/features/user-ops/RosterView.tsx
# 若改了测试也一并 add：
# git add frontend/src/__tests__/features/user-ops/roster.test.tsx
git commit -m "fix(roster): 前端自动轮询改只读(不带force)+间隔8s→10s,消除高频force叠加"
```

---

### Task 5: 本地全量验证 + 前端构建

**Files:** 无改动，仅验证。

**Interfaces:** 消费 Task 1-4 全部改动。

- [ ] **Step 1: 后端基线门**

Run: `cargo test --lib`
Expected: ≥ 350 passed, 0 failed。

- [ ] **Step 2: 后端编译无 warning-as-error**

Run: `cargo check --tests`
Expected: 通过（复刻 CI baseline step2，确保集成测试构造点未因字段新增而 E0063——本计划未改 config/公开构造体，预期无影响）。

- [ ] **Step 3: 前端构建**

Run: `cd frontend && npm run build`
Expected: 构建成功，写出 `frontend/dist`。

- [ ] **Step 4: no-human-takeover lint 自检**

Run: `scripts/check-no-human-takeover.sh`（或 Windows `scripts/check-no-human-takeover.ps1`）
Expected: 通过（roster 改动不含禁词）。

- [ ] **Step 5: 无需 commit（纯验证）。** 若前序 Task 有遗漏在此暴露，回对应 Task 修复。

---

### Task 6: 部署 117 + 生产真实验证

**Files:** 无改动，部署 + 线上验证。

**前置：** 需用户在场 / 已授权部署（Task8 一条龙授权仅覆盖 #161；本次为新修复，部署前向用户确认）。

- [ ] **Step 1: 删除现有快照（复现首次进频道场景）**

用 paramiko 脚本（VPN bypass）执行：
```
mongosh wechatagent --quiet --eval 'db.roster_snapshots.deleteMany({account_id:"102"}); print("deleted, count="+db.roster_snapshots.countDocuments({}))'
```
Expected: `count=0`。

- [ ] **Step 2: 部署新二进制 + 前端 dist**

按 [[deploy-server-117]]：`cargo build --release` + `cd frontend && npm run build` → git bundle 增量包经 paramiko 上传 → 117 构建 → `systemctl restart wechatagent`。部署完整性三查（后端二进制 mtime > 本次 commit 时间 + 前端 dist 内容含新 bundle）。

- [ ] **Step 3: 进通讯录频道触发首次拉取，观察后台无刷屏**

删快照后，从前端进「用户运营 → 通讯录」（或 curl 后端 roster 端点触发）。随即：
```
journalctl -u wechatagent --since '<刚才时刻 CST>' --no-pager | grep -iE 'roster|contacts_fetch|error decoding' | tail -40
```
Expected:
- 首次端点响应立即返回 `syncing:true`（不阻塞）。
- 后台 roster 刷新日志**无刷屏**（同账号同时只有一个任务；不再是每秒多条交错 attempt）。
- **不再出现** `error decoding response body` 大量刷屏。

- [ ] **Step 4: 确认约 10s 后快照写入**

```
mongosh wechatagent --quiet --eval 'var s=db.roster_snapshots.findOne({account_id:"102"}); print(s? ("total="+s.total+" fetched_at="+s.fetched_at) : "NO SNAPSHOT")'
```
Expected: `total=4832`（或当前真实好友数），`fetched_at` 为刚才时刻。

- [ ] **Step 5: 前端只读轮询秒出好友**

前端页面在快照写入后的下一轮自动轮询（≤10s）应从"正在从微信同步好友…"切换为好友卡片网格，显示全量好友。

- [ ] **Step 6: 并发压力验证（防回归）**

快速连点「刷新」按钮多次 / 开多个标签页进通讯录，随后：
```
journalctl -u wechatagent --since '<刚才时刻 CST>' --no-pager | grep -iE 'roster' | tail -30
```
Expected: 后台任务**不叠加**（single-flight 去重生效，无多任务交错 attempt 日志、无 `error decoding` 复发）。

- [ ] **Step 7: 记录验证结果，更新 memory**

验证通过后更新 [[project-roster-backend-snapshot-deployed]] 记录 single-flight 修复已上线，并记录本次根因（PR#162 低估并发后果）。

---

## Self-Review

**1. Spec coverage（对照设计三支柱）:**
- 支柱1（single-flight 锁）→ Task 1（字段）+ Task 2（抢锁+guard+测试）✓
- 支柱2（前端只读轮询 + 10s）→ Task 4 ✓
- 支柱3（同步端点不阻塞）→ Task 3 ✓
- 测试计划（锁去重/guard释放/panic释放/多账号独立）→ Task 2 Step 1 四个单测 ✓
- 验证计划（本地基线+前端build+117真实验证）→ Task 5 + Task 6 ✓

**2. Placeholder scan:** 无 TBD/TODO；所有代码步给出完整代码块；Task 4 Step 3 为条件步骤（明确"仅当 Step 2 失败时"）并给出示例断言，非占位。

**3. Type consistency:** `RosterRefreshGuard { map: Arc<DashMap<String,()>>, key: String }` 在 Task 2 定义并在测试与 `spawn_roster_refresh` 中一致使用；`roster_refreshing` 字段名在 Task 1 定义、Task 2 读取一致；`spawn_roster_refresh(state, workspace_id, account_id)` 签名全程不变；端点返回 `(friends, syncing)` 元组类型与现有一致。

**说明（不做项，YAGNI 已在 Global Constraints / 设计 YAGNI 覆盖）:** 昵称/头像 null 补 getBriefInfo、MCP server 改动、分布式锁、24h 阈值调整——均不在本计划。
