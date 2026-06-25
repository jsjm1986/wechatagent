# 账号级最小发送间隔闸（防风控连珠炮）设计

- 日期：2026-06-24
- 分支：feat/tag-trust（实现时另开干净分支，不混入 tag-trust）
- 状态：设计待复审

## 1. 背景与动机

WechatAgent 是"全 AI 自治"的微信私域运营 agent。真实微信对个人号有一套**不公开、动态调整**的风控系统，对"机器化/营销化"发送行为建模。后端业务场景审查（2026-06-24，15-agent workflow + 人工核证）把"账号在极短时间窗内连续发出多条消息"列为真实风控特征。

**根本立场（已与用户确认）**：不赌"每天发 X 条就封号"这类魔法数字——精确阈值不可知且会随微信迭代变化。设计目标是**从"像不像机器"的根源降低风控特征**，而非押注猜对某个数字。

### 1.1 风险的精确定位（经代码核证修正）

初判风险是"账号瞬时并发发送"。核证后修正：

- `src/main.rs:207` 经 `spawn_supervised` 只启动**一个** outbox dispatcher 实例。
- `src/agent/outbox_dispatcher.rs:936-949` 的 `tick()` 是 `for _ in 0..PER_TICK_PROCESS_CAP { claim → process_entry().await → 下一轮 }` 的**串行**循环（`PER_TICK_PROCESS_CAP = 16`，outbox_dispatcher.rs:52），一条处理完才处理下一条，无 `tokio::spawn` 并发。

因此严格意义的"瞬时并发"不会发生。**真实风险是"连珠炮"**：单 worker 串行 `for` 循环中，相邻两条发送之间**零间隔**。当多个客户的回复、同一回复的多段、多个到期 follow-up 在同一 tick 内被连续 claim 时，账号会在几十毫秒内一条接一条发出——在风控特征上等同于机器群发。

### 1.2 为什么用"账号级间隔闸"而非入队层错峰

设计过程中曾考虑过"多段回复按字数模拟打字间隔 + follow-up 错峰"的入队层方案。但入队层各算各的偏移，无法感知"同账号还有别的消息也要发"；且预算池"agent 慢就立即发"的逻辑在多客户 agent 同时跑完时反而会制造同 tick 连发。

**账号级间隔闸**在 dispatcher 实际发送的那一刻统一收口：无论消息来自多段、follow-up 还是跨客户并发，只要是同一账号，相邻两次实际发送之间强制留最小间隔。一个机制覆盖全部来源。这是用户确认的最终方案——**砍掉**入队层段间隔/打字速度/预算池/follow-up 错峰，只做这一个闸。

## 2. 机制设计

### 2.1 闸的位置（经核证，落点关键）

插入 `process_entry`（outbox_dispatcher.rs）的发送链路中，位置必须是：

```
... 掉线 defer 检查 (defer_account_offline, :566-569)
... reclaimed_in_flight 崩溃恢复幂等门 (:577-649) —— post-hoc 核对上轮是否已送达，命中标 sent 并 return
>>> 【账号间隔闸插入点：约 :655，reclaim 幂等门之后、send_fut 之前】<<<
... send_fut 定义 (:657-665) 与实际 MCP 发送 (:666-667)
```

**为什么不能放在掉线 defer 之后立即插入**：`reclaimed_in_flight` 幂等门会对"上轮其实已送达但状态没回写"的条目做 post-hoc 核对并标 sent。若账号闸放在它之前，会误把"本该标记完成"的条目又 reschedule 推迟一轮。因此账号闸必须在幂等门**之后**。

### 2.2 判定逻辑

发送前查询"该账号上次实际发送时刻"：

- 查 `collection_agent_send_outbox()` 中 `{account_id, status:"sent"}` 的**最大 `sent_at`**。
- 写法参照 `src/agent/send_ledger.rs:304` 的 `FindOptions::builder().sort(doc!{"sent_at": -1}).limit(1)`。
- `OutboxEntry.account_id: String`（models.rs:2580）、`sent_at: Option<DateTime>`（models.rs:2615）。
- 时间比较统一走 `timestamp_millis()`：`now_ms - last_sent_ms < interval_ms` 则**命中闸**（与 gateway.rs:2967 现有惯例一致）。
- 该账号无任何 sent 历史（查不到）→ 不命中，正常发送。

**为什么不复用现成字段**：`WechatAccount.last_sync_at`（models.rs:71）是心跳/在线同步时间，由 webhooks 在线回调写入，与"实际发送时刻"无关，不可复用。`agent_send_ledger` 只记 media/namecard 两类主动发送，不覆盖普通文本发送，且索引不以 account_id 打头。故必须查 outbox。

### 2.3 命中后的动作：reschedule（不阻塞、不耗 attempt）

命中闸时，该条 **reschedule** 而非 sleep 等待——复用 `defer_account_offline`（outbox_dispatcher.rs:396-444）的模式：

- `$set`：`status → Pending`、`next_retry_at = last_sent_at + interval_ms`、`updated_at = now`。
- **attempt 不变**（不耗重试预算、不走 terminal）。
- `$unset`：`worker_id`、`locked_until`。
- 写事件：新 kind `agent.send_deferred_account_pacing`（区别于 offline 的 `agent.send_deferred_account_offline`），经 `write_event_with_cap`。
- `update_run_log_outbox_status(..., "pending")`。
- `process_entry` 提前 `return Ok(())`，tick 的 for 循环继续 claim 下一条。

**为什么不 sleep**：单 worker sleep 会阻塞整个 dispatcher（其他账号的消息也跟着等），且 sleep 期间进程崩溃丢状态。reschedule 不阻塞、崩溃可恢复。

**无死循环**：`atomic_claim_pending` 的 filter 是 `next_retry_at <= now`（outbox_dispatcher.rs:124-131）。reschedule 后 `next_retry_at` 在未来，本 tick 后续轮次不会重新 claim 到它，下个 tick 到点后才发。

### 2.4 间隔值

随机 **1-4 秒**（用户确认）——防住连珠炮的"瞬时"特征，同时正常对话（客户连发多条、AI 逐条回）节奏接近真人逐条回复，不明显拖慢。

纯函数计算，对称现有 `backoff_with_jitter_seeded`（outbox.rs:457）的"纯函数 + 调用点注入随机"模式：

```rust
// src/agent/pacing.rs
/// jitter01 ∈ [0,1] 线性映射到 [min, max] 毫秒区间。
/// 纯函数：随机由调用点用 fastrand::f64() 注入，便于确定性测试。
pub(crate) fn account_send_interval_ms(jitter01: f64, min_ms: i64, max_ms: i64) -> i64 {
    let j = jitter01.clamp(0.0, 1.0);
    min_ms + ((max_ms - min_ms) as f64 * j).round() as i64
}
```

注意：这是**区间线性映射**（jitter01=0 → min_ms，=1 → max_ms），不是 backoff 的"中心 ±幅度"模式，公式形似但不同，不照抄。调用点注入 `fastrand::f64()`（fastrand 已是依赖，Cargo.toml:17）。

## 3. 新增索引（性能必需）

`agent_send_outbox` 现有 4 条索引（`src/db/indexes.rs:722-757`）：`(account_id,status,next_retry_at)`、`idempotency_key` unique、`(status,locked_until)`、`(source_event_id,contact_wxid)`。

**无一支撑 "(account_id,status) 等值 + sent_at 倒序" 查询**——第一条索引排序键是 next_retry_at 而非 sent_at，会在 account+status 命中文档集上做内存 SORT，随账号 sent 历史增长性能线性恶化。

设计**必须新增**复合索引 `(account_id: 1, status: 1, sent_at: -1)`，加在 `ensure_agent_send_outbox_indexes`（indexes.rs:722）中。

## 4. 配置项

新增一个上下界配置（默认 1000ms / 4000ms）。按 config.rs env_or 惯例（参照 config.rs:406/414）：

```rust
account_send_min_interval_ms: env_or("ACCOUNT_SEND_MIN_INTERVAL_MS", "1000").parse()?,
account_send_max_interval_ms: env_or("ACCOUNT_SEND_MAX_INTERVAL_MS", "4000").parse()?,
```

**AppConfig 无 `..Default::default()`，全字段字面量初始化，必须同步 6 个落点**（漏一处编译失败）：
1. 结构体定义 `src/config.rs:6` 区域
2. 初始化 `src/config.rs:390` 区域
3. `tests/common/mod.rs:228` 区域
4. `tests/jwt_auth.rs:30` 区域
5. `src/evolution/budget.rs:61` 区域
6. `src/routes/evolution.rs:756` 区域

并在 `.env.example` 补两行默认值。

## 5. 代码落点汇总

| 文件 | 改动 |
| --- | --- |
| `src/agent/pacing.rs` | **新建**。纯函数 `account_send_interval_ms` + 内联 `#[cfg(test)] mod tests` |
| `src/agent/mod.rs` | 加 `pub(crate) mod pacing;`（:47 附近就近聚类） |
| `src/agent/outbox_dispatcher.rs` | `process_entry` 在 reclaim 幂等门后、send_fut 前插入账号闸；新增 `defer_account_pacing`（仿 `defer_account_offline`，换延迟值与事件 kind） |
| `src/db/indexes.rs` | `ensure_agent_send_outbox_indexes` 加 `(account_id,status,sent_at:-1)` 复合索引 |
| `src/config.rs` | 加 2 个配置项（定义 + 初始化） |
| `tests/common/mod.rs`、`tests/jwt_auth.rs`、`src/evolution/budget.rs`、`src/routes/evolution.rs` | 同步补 AppConfig 新字段字面量 |
| `.env.example` | 补 2 行默认值 |

## 6. 错误处理

- **查询失败 fail-soft**：查 last_sent_at 失败时，不阻断发送（放行），仿 `account_daily_sent_count` 的 fail-soft 语义（查询失败不影响"已发"语义）。宁可漏限一次也不丢消息。
- **闸命中是正常路径**：reschedule 不是错误，写 info 级事件供观测。
- **MCP 发送已成功后的 DB 写失败**：维持现有"既成事实"语义不变，本设计不触碰发送后的回写逻辑。

## 7. 测试策略

- **pacing.rs 纯函数确定性单测**（内联，参照 outbox.rs:947-981）：
  - `account_send_interval_ms(0.0, 1000, 4000) == 1000`
  - `account_send_interval_ms(1.0, 1000, 4000) == 4000`
  - `account_send_interval_ms(0.5, 1000, 4000) == 2500`
  - jitter01 越界 clamp 行为
- **dispatcher 集成测试**（tests/outbox_integration.rs，参照 `happy_path_enqueue_claim_send_sent`:155 与 `crash_recovery_..._reschedules`:382）：
  - 同账号背靠背两条：第一条发出（status=sent），第二条在间隔内被 reschedule 回 pending、attempt 不变、next_retry_at 在未来、写 `agent.send_deferred_account_pacing` 事件。
  - 间隔已过的第二条正常发出。
  - 不同账号互不影响（A 账号刚发不影响 B 账号立即发）。
  - 该账号无 sent 历史时第一条不被闸拦。
- **基线门**：lib ≥350/0、四 PBT ≥33/0、两个字符串 lint。新增测试只增量叠加。

## 8. 范围边界（YAGNI）

本设计**只做**账号级最小发送间隔闸。明确**不做**（留后续独立专题）：

- P0-A 单账号日发送总量硬上限（当前 gateway.rs:2507 软上限只告警）。
- P1-C 微信风控返回码解析 + 账号风控状态位自动摘号（mcp.rs 不解析微信 ret 码）。
- 入队层多段打字间隔、follow-up 错峰（账号闸已覆盖其风控意义）。

## 9. 安全与合规

- 这是**保护自有账号不被平台风控**的防御性能力，不涉及规避检测做恶意营销。
- 不改变"全 AI 自治"红线：闸只影响发送时机，不影响发送内容或决策。
- 不引入 no-human-takeover 禁用词：事件 kind 用 `agent.send_deferred_account_pacing`（AI 内部状态名）。
