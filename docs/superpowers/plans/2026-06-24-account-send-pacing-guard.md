# 账号级最小发送间隔闸 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 outbox dispatcher 发送前加一道账号级最小发送间隔闸，让同一微信账号相邻两次实际发送之间强制留 1-4 秒随机间隔，消除"连珠炮"机器化风控特征。

**Architecture:** 新建 `pacing.rs` 纯函数算间隔（随机由调用点注入，便于确定性测试）；在 `process_entry` 的 reclaim 幂等门之后、MCP 发送之前插入闸，命中则 reschedule（仿 `defer_account_offline`，不耗 attempt、不阻塞 worker）；查 outbox 最大 `sent_at` 判定间隔，并新增支撑索引。

**Tech Stack:** Rust 2021, Axum, MongoDB (mongodb driver), fastrand, tokio。

设计依据：`docs/superpowers/specs/2026-06-24-account-send-pacing-guard-design.md`

## Global Constraints

- 测试基线门（不可回归）：`cargo test --lib` ≥350 passed / 0 failed；四 PBT（state_transition_pbt、memory_card_invariants、wiki_chunk_revision_pbt、llm_retry_jitter）累计 ≥33/0。
- 字符串 lint 门：禁用词（人工接管/takeover/hand-off 等）。事件 kind 用 AI 内部状态名 `agent.send_deferred_account_pacing`。
- 本地磁盘受限：只跑 `cargo test --lib` 和单个 PBT 文件；全量集成 `cargo test --test outbox_integration -- --ignored` 留 CI（需 Docker）。
- 新增测试只增量叠加，绝不删改旧维度。
- AppConfig 无 `..Default::default()`，全字段字面量初始化——加配置项必须同步 6 个落点，漏一处编译失败。
- 提交需用户显式批准。提交时精确 `git add` 命名具体文件，排除工作树并行产物（`src/prompts.rs` 等不属于本功能的改动）。
- 关键时序事实：dispatcher 每 tick 间隔 `DEFAULT_POLL_INTERVAL_SECONDS = 5` 秒（outbox_dispatcher.rs:908）。reschedule 后最坏要等下个 tick 才重发，故实际间隔会被 poll 周期量化到 ~5 秒粒度——这是可接受的（1-4 秒是"最小"间隔下界，poll 量化使实际间隔 ≥ 配置值，方向安全）。

---

## 文件结构

| 文件 | 责任 |
| --- | --- |
| `src/agent/pacing.rs` | **新建**。纯函数 `account_send_interval_ms(jitter01, min_ms, max_ms)` + 内联单测 |
| `src/agent/mod.rs` | 注册 `pub(crate) mod pacing;` |
| `src/agent/outbox_dispatcher.rs` | 新增 `defer_account_pacing` + 在 `process_entry` 插入账号闸 + 新增 `account_last_sent_at` 查询 helper |
| `src/db/indexes.rs` | `ensure_agent_send_outbox_indexes` 加 `(account_id,status,sent_at:-1)` 索引 |
| `src/config.rs` | 加 `account_send_min_interval_ms` / `account_send_max_interval_ms` 两配置项 |
| `tests/common/mod.rs`、`tests/jwt_auth.rs`、`src/evolution/budget.rs`、`src/routes/evolution.rs` | 同步 AppConfig 新字段字面量 |
| `.env.example` | 补 2 行默认值 |
| `tests/outbox_integration.rs` | 账号闸集成测试 |

---

### Task 1: pacing.rs 纯函数模块

**Files:**
- Create: `src/agent/pacing.rs`
- Modify: `src/agent/mod.rs`（:47 附近加模块声明）

**Interfaces:**
- Produces: `pub(crate) fn account_send_interval_ms(jitter01: f64, min_ms: i64, max_ms: i64) -> i64` — jitter01∈[0,1] 线性映射到 [min_ms, max_ms]，越界 clamp。

- [ ] **Step 1: 创建 pacing.rs 含纯函数 + 失败测试**

Create `src/agent/pacing.rs`:

```rust
//! 发送节奏拟人化纯函数。
//!
//! 账号级最小发送间隔闸用它把随机抖动映射成毫秒间隔。随机由调用点用
//! `fastrand::f64()` 注入（对称 `outbox::backoff_with_jitter_seeded` 的纯函数模式），
//! 故本函数确定性可测。

/// 把 `jitter01 ∈ [0,1]` 线性映射到 `[min_ms, max_ms]` 毫秒区间。
///
/// - `jitter01 = 0.0` → `min_ms`
/// - `jitter01 = 1.0` → `max_ms`
/// - `jitter01 = 0.5` → 中点
///
/// 越界的 `jitter01` 会被 clamp 到 `[0,1]`。`max_ms < min_ms` 时返回 `min_ms`
/// （调用方应保证 `min_ms <= max_ms`）。
pub(crate) fn account_send_interval_ms(jitter01: f64, min_ms: i64, max_ms: i64) -> i64 {
    let j = jitter01.clamp(0.0, 1.0);
    let span = (max_ms - min_ms).max(0);
    min_ms + (span as f64 * j).round() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_zero_to_min() {
        assert_eq!(account_send_interval_ms(0.0, 1000, 4000), 1000);
    }

    #[test]
    fn maps_one_to_max() {
        assert_eq!(account_send_interval_ms(1.0, 1000, 4000), 4000);
    }

    #[test]
    fn maps_half_to_midpoint() {
        assert_eq!(account_send_interval_ms(0.5, 1000, 4000), 2500);
    }

    #[test]
    fn clamps_out_of_range_jitter() {
        assert_eq!(account_send_interval_ms(-1.0, 1000, 4000), 1000);
        assert_eq!(account_send_interval_ms(2.0, 1000, 4000), 4000);
    }

    #[test]
    fn degenerate_range_returns_min() {
        // max < min：span clamp 到 0，恒返 min。
        assert_eq!(account_send_interval_ms(0.7, 4000, 1000), 4000);
    }
}
```

- [ ] **Step 2: 注册模块**

Modify `src/agent/mod.rs` — 在 `pub(crate) mod outbox;`（约 :47）附近加一行（按字母/就近聚类）：

```rust
pub(crate) mod pacing;
```

- [ ] **Step 3: 运行测试验证通过**

Run: `cargo test --lib pacing`
Expected: 5 passed; 0 failed（`maps_zero_to_min`/`maps_one_to_max`/`maps_half_to_midpoint`/`clamps_out_of_range_jitter`/`degenerate_range_returns_min`）

- [ ] **Step 4: 提交**

```bash
git add src/agent/pacing.rs src/agent/mod.rs
git commit -m "feat(pacing): account_send_interval_ms 纯函数(账号闸间隔映射)"
```

---

### Task 2: 新增支撑索引

**Files:**
- Modify: `src/db/indexes.rs`（`ensure_agent_send_outbox_indexes`，:722-758）

**Interfaces:**
- Produces: `agent_send_outbox` 新增复合索引 `(account_id:1, status:1, sent_at:-1)`，支撑"查某账号 status=sent 的最大 sent_at"。

- [ ] **Step 1: 加索引**

Modify `src/db/indexes.rs` — 在 `ensure_agent_send_outbox_indexes` 的 `Ok(())` 之前，追加一个 `create_index` 块（与现有四个同结构）：

```rust
    // 账号级发送间隔闸：查某账号 status=sent 的最大 sent_at（pacing guard）。
    // 现有 (account_id,status,next_retry_at) 排序键不是 sent_at，无法支撑 sent_at 倒序，
    // 会触发内存 SORT 随历史线性恶化，故单建此索引。
    db.collection_agent_send_outbox()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "account_id": 1, "status": 1, "sent_at": -1 })
                .build(),
            None,
        )
        .await?;
```

- [ ] **Step 2: 编译验证**

Run: `cargo check --lib`
Expected: 0 error（索引创建是幂等的，ensure_indexes 重复运行安全）

- [ ] **Step 3: 提交**

```bash
git add src/db/indexes.rs
git commit -m "feat(indexes): agent_send_outbox 加 (account_id,status,sent_at) 支撑账号闸查询"
```

---

### Task 3: 配置项（6 处同步）

**Files:**
- Modify: `src/config.rs`（结构体定义 :6 区域 + 初始化 :390 区域）
- Modify: `tests/common/mod.rs`（:228 区域）、`tests/jwt_auth.rs`（:30 区域）、`src/evolution/budget.rs`（:61 区域）、`src/routes/evolution.rs`（:756 区域）
- Modify: `.env.example`

**Interfaces:**
- Produces: `AppConfig.account_send_min_interval_ms: i64`（默认 1000）、`AppConfig.account_send_max_interval_ms: i64`（默认 4000）。

- [ ] **Step 1: 结构体定义加字段**

Modify `src/config.rs` — 在 AppConfig 结构体里（参照 `agent_min_reply_interval_seconds` 字段附近）加：

```rust
    /// 账号级最小发送间隔闸：同账号相邻两次发送的最小间隔下界（毫秒）。
    pub account_send_min_interval_ms: i64,
    /// 账号级最小发送间隔闸：上界（毫秒）。实际间隔在 [min,max] 间随机。
    pub account_send_max_interval_ms: i64,
```

- [ ] **Step 2: 初始化加 env_or（config.rs from_env）**

Modify `src/config.rs` — 在 from_env 初始化块（参照 :406 `agent_min_reply_interval_seconds`）加：

```rust
            account_send_min_interval_ms: env_or("ACCOUNT_SEND_MIN_INTERVAL_MS", "1000").parse()?,
            account_send_max_interval_ms: env_or("ACCOUNT_SEND_MAX_INTERVAL_MS", "4000").parse()?,
```

- [ ] **Step 3: 同步 4 个测试/构造点字面量**

每处在 AppConfig 字面量初始化里加两行（值用默认 1000/4000）。四个文件：
- `tests/common/mod.rs`（:228 区域的 AppConfig 字面量）
- `tests/jwt_auth.rs`（:30 区域）
- `src/evolution/budget.rs`（:61 区域）
- `src/routes/evolution.rs`（:756 区域）

每处插入：

```rust
            account_send_min_interval_ms: 1000,
            account_send_max_interval_ms: 4000,
```

- [ ] **Step 4: .env.example 补默认值**

Modify `.env.example` — 加两行：

```
ACCOUNT_SEND_MIN_INTERVAL_MS=1000
ACCOUNT_SEND_MAX_INTERVAL_MS=4000
```

- [ ] **Step 5: 编译验证（确认 6 处全同步）**

Run: `cargo check --lib && cargo check --tests`
Expected: 0 error。若报 "missing field account_send_*_interval_ms" 说明漏了某个字面量点，补上。

- [ ] **Step 6: 提交**

```bash
git add src/config.rs tests/common/mod.rs tests/jwt_auth.rs src/evolution/budget.rs src/routes/evolution.rs .env.example
git commit -m "feat(config): account_send_{min,max}_interval_ms 配置项(账号闸)"
```

---

### Task 4: dispatcher 账号间隔闸接线 + 集成测试

**Files:**
- Modify: `src/agent/outbox_dispatcher.rs`（新增 `account_last_sent_at` helper + `defer_account_pacing` + `process_entry` 插入闸）
- Test: `tests/outbox_integration.rs`

**Interfaces:**
- Consumes: `pacing::account_send_interval_ms`（Task 1）、`AppConfig.account_send_{min,max}_interval_ms`（Task 3）、`(account_id,status,sent_at)` 索引（Task 2）。
- Consumes: 现有 `defer_account_offline`（:396）模式、`write_event_with_cap`、`update_run_log_outbox_status`、`OutboxEntry`、`fastrand::f64()`。

- [ ] **Step 1: 加 account_last_sent_at 查询 helper**

Modify `src/agent/outbox_dispatcher.rs` — 在 `defer_account_offline`（:444 之后）加一个查询函数：

```rust
/// 查某账号 `agent_send_outbox` 中 `status=sent` 的最大 `sent_at`（毫秒）。
/// 无 sent 历史返回 None。靠 (account_id,status,sent_at:-1) 索引取 limit(1)。
async fn account_last_sent_at_ms(state: &AppState, account_id: &str) -> AppResult<Option<i64>> {
    use mongodb::options::FindOneOptions;
    let collection = state.db.collection_agent_send_outbox();
    let opts = FindOneOptions::builder()
        .sort(doc! { "sent_at": -1 })
        .build();
    let doc = collection
        .find_one(
            doc! { "account_id": account_id, "status": OutboxStatus::Sent.as_str() },
            opts,
        )
        .await?;
    Ok(doc.and_then(|e| e.sent_at).map(|d| d.timestamp_millis()))
}
```

- [ ] **Step 2: 加 defer_account_pacing（仿 defer_account_offline）**

Modify `src/agent/outbox_dispatcher.rs` — 在 `account_last_sent_at_ms` 之后加：

```rust
/// 账号级发送间隔闸命中：把本条 reschedule 到 `last_sent_at + interval`。
/// 仿 [`defer_account_offline`]——attempt 不变、不走 terminal、$unset 锁、写事件。
async fn defer_account_pacing(
    state: &AppState,
    entry_id: ObjectId,
    entry: &OutboxEntry,
    next_send_at_ms: i64,
) -> AppResult<()> {
    let collection = state.db.collection_agent_send_outbox();
    let now = DateTime::now();
    let next_retry = DateTime::from_millis(next_send_at_ms);
    collection
        .update_one(
            doc! {
                "_id": entry_id,
                "status": OutboxStatus::InFlight.as_str(),
            },
            doc! {
                "$set": {
                    // attempt 刻意不变——间隔闸非发送失败，不耗重试额度、不走 terminal。
                    "status": OutboxStatus::Pending.as_str(),
                    "next_retry_at": next_retry,
                    "updated_at": now,
                },
                "$unset": {
                    "worker_id": "",
                    "locked_until": "",
                }
            },
            None,
        )
        .await?;
    let _ = write_event_with_cap(
        state,
        entry_id,
        &entry.account_id,
        Some(&entry.contact_wxid),
        "agent.send_deferred_account_pacing",
        "deferred",
        "账号发送过于密集，本条已按拟人节奏推迟（AI 自治控制外发频率，稍后自动续发），不消耗重试额度",
        Some(doc! {
            "outbox_id": entry_id,
            "run_id": &entry.run_id,
            "attempt": entry.attempt,
        }),
    )
    .await;
    update_run_log_outbox_status(state, &entry.run_id, "pending").await;
    Ok(())
}
```

- [ ] **Step 3: 在 process_entry 插入闸（reclaim 幂等门后、send_fut 前）**

Modify `src/agent/outbox_dispatcher.rs` — 在 reclaim 幂等门 `}` 结束（:649 的 `}`）之后、`let extra_raw = Some(doc! {`（:651）之前插入：

```rust
    // 账号级最小发送间隔闸：查该账号上次实发时刻，距今 < 随机间隔则 reschedule。
    // 防"连珠炮"——单 worker 串行 for 循环里跨客户/多段消息背靠背零间隔发出 = 机器特征。
    // 位置在 reclaim 幂等门之后（不误拦本该 post-hoc 标 sent 的条目）、发送之前。
    // 查询失败 fail-soft 放行（宁可漏限一次也不丢消息）。
    if let Ok(Some(last_sent_ms)) = account_last_sent_at_ms(state, &entry.account_id).await {
        let interval_ms = super::pacing::account_send_interval_ms(
            fastrand::f64(),
            state.config.account_send_min_interval_ms,
            state.config.account_send_max_interval_ms,
        );
        let now_ms = DateTime::now().timestamp_millis();
        if now_ms - last_sent_ms < interval_ms {
            defer_account_pacing(state, entry_id, entry, last_sent_ms + interval_ms).await?;
            return Ok(());
        }
    }
```

- [ ] **Step 4: 编译验证**

Run: `cargo check --lib`
Expected: 0 error, 0 warning（注意 fastrand 已 import，若报未引入则在文件顶部确认 `use` 或全限定 `fastrand::f64()`）

- [ ] **Step 5: 写集成测试**

Modify `tests/outbox_integration.rs` — 加测试（参照 `happy_path_enqueue_claim_send_sent`:155 的构造方式 + `crash_recovery` 的 reschedule 断言）。测试用真实 testcontainers Mongo + wiremock，标 `#[ignore]`（与文件内其它集成测试一致）：

```rust
/// 账号闸：同账号刚发过一条（sent_at=now），紧接的第二条在间隔内 → 被 reschedule
/// 回 pending、attempt 不变、next_retry_at 在未来、写 pacing deferred 事件。
#[tokio::test]
#[ignore = "requires docker (testcontainers mongo)"]
async fn account_pacing_gate_reschedules_back_to_back_send() {
    // Arrange: 起 Mongo + mock MCP server，构造 state（account_send_min/max=2000/2000 固定间隔便于断言）。
    // 在 agent_send_outbox 插一条该账号 status=sent、sent_at=now 的历史条目。
    // 再 enqueue 一条同账号 pending 条目。
    // Act: 跑一次 dispatcher tick（或直接 atomic_claim_pending + process_entry）。
    // Assert: 第二条 status=pending、attempt 仍为 0、next_retry_at > now；
    //         events 含 kind="agent.send_deferred_account_pacing"；
    //         MCP mock 未收到第二条的发送调用。
}

/// 账号闸：间隔已过（last sent_at 在 interval 之前）→ 第二条正常发出。
#[tokio::test]
#[ignore = "requires docker (testcontainers mongo)"]
async fn account_pacing_gate_allows_after_interval() {
    // Arrange: 历史 sent 条目 sent_at = now - 10s（远超 2s 间隔）。
    // Act: process_entry。
    // Assert: 第二条 status=sent，MCP mock 收到发送调用。
}

/// 账号闸：不同账号互不影响（A 刚发不拦 B）。
#[tokio::test]
#[ignore = "requires docker (testcontainers mongo)"]
async fn account_pacing_gate_isolates_accounts() {
    // Arrange: 账号 A 有 sent_at=now 历史；enqueue 账号 B 的 pending 条目。
    // Act: process_entry(B)。
    // Assert: B status=sent（A 的发送历史不拦 B）。
}

/// 账号闸：该账号无 sent 历史 → 第一条不被拦。
#[tokio::test]
#[ignore = "requires docker (testcontainers mongo)"]
async fn account_pacing_gate_first_send_not_blocked() {
    // Arrange: 账号无任何 sent 条目；enqueue 一条 pending。
    // Act: process_entry。
    // Assert: status=sent。
}
```

> 注：测试体内的 Arrange/Act/Assert 注释需实现者按 `tests/outbox_integration.rs` 现有 helper（起 Mongo、构造 AppState、mock MCP、enqueue）补成真实代码。固定 min=max 间隔消除随机性便于断言。

- [ ] **Step 6: 编译测试（本地只编译，集成跑留 CI）**

Run: `cargo test --test outbox_integration --no-run`
Expected: 编译成功（`#[ignore]` 的集成测试本地不实跑，需 Docker；CI 跑 `-- --ignored`）

- [ ] **Step 7: 基线门**

Run: `cargo test --lib`
Expected: ≥350 passed / 0 failed（含 Task 1 的 5 个 pacing 测试）

- [ ] **Step 8: 提交**

```bash
git add src/agent/outbox_dispatcher.rs tests/outbox_integration.rs
git commit -m "feat(outbox): 账号级最小发送间隔闸防连珠炮(reschedule不耗attempt)"
```

---

## Self-Review

**Spec coverage（逐节对照设计文档）：**
- §2.1 闸落点（reclaim 后 send 前）→ Task 4 Step 3 ✅
- §2.2 查最大 sent_at + timestamp_millis 比较 → Task 4 Step 1 ✅
- §2.3 reschedule 仿 defer_account_offline → Task 4 Step 2 ✅
- §2.4 间隔纯函数 1-4 秒 → Task 1 ✅
- §3 新增索引 → Task 2 ✅
- §4 配置 6 处同步 → Task 3 ✅
- §6 查询失败 fail-soft → Task 4 Step 3（`if let Ok(Some(..))` 失败/None 都放行）✅
- §7 测试策略（纯函数确定性 + 4 个集成场景）→ Task 1 Step 1 + Task 4 Step 5 ✅
- §9 事件 kind 用 AI 内部状态名 → Task 4 Step 2 `agent.send_deferred_account_pacing` ✅

**Placeholder scan：** 集成测试体是 Arrange/Act/Assert 骨架注释（已标注需实现者按现有 helper 补真实代码）——这是集成测试依赖大量 testcontainers/wiremock 上下文的合理留白，纯函数与生产代码均为完整可抄代码，无 TBD/TODO。

**Type consistency：** `account_send_interval_ms(jitter01, min_ms, max_ms)` 签名 Task 1 定义、Task 4 调用一致；`account_send_{min,max}_interval_ms: i64` Task 3 定义、Task 4 消费一致；`defer_account_pacing(state, entry_id, entry, next_send_at_ms)` 定义与调用一致；事件 kind 字符串 `agent.send_deferred_account_pacing` 在 Task 4 Step 2/Step 5 一致。

---

## Execution Handoff

计划保存于 `docs/superpowers/plans/2026-06-24-account-send-pacing-guard.md`。
