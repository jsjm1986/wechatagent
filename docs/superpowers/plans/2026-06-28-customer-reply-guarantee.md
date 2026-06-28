# 全自治"客户必有回应"保障 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **⚠️ 实现期修正（2026-06-28，已合并 main，权威以代码 + spec §3.5 为准）**：本计划原写"4 处挂载点（含 Task 3 Step 5 的 A3 no_reply）"。实现期 CI 集成测试 `full_flow_a3_no_reply_skips_review_and_outbox` 证伪 A3 挂载——A3 是 AI **主动判定**沉默（非晾死），补占位破坏拟人。最终**去掉挂载点④（A3）**，守卫只挂 **3 处真晾死出口**（两道 precheck + 拦截分支），`no_reply` 入 `ACK_PLACEHOLDER_EXCLUDED_STATUSES` 豁免清单。下文 Task 3 Step 5、行 40/48/80/115/153 等"4 处 / no_reply 补占位"措辞均为原案，已被本修正覆盖；其余 Task（1/2/4）落地与计划一致。

**Goal:** 保证任何 Inbound（客户真发了消息、且 AI 不是主动判定沉默）的一轮 gateway 运行，凡 AI 想回却被 held/blocked/precheck 拦下的，客户都至少收到一条确定性中性占位，堵住晾死漏洞。

**Architecture:** gateway 内加一道 per-run"回应保障守卫"——纯函数 `should_send_ack_placeholder`（黑名单判定）+ 纯函数 `build_ack_enqueue_request`（占位 outbox 入参构造）+ async `ensure_customer_acknowledged`（should_abort 复查后入 outbox）。守卫挂在 **3 个真晾死出口**（实现期修正，原写 4 个）。同时把 escalation 里耦合在"领导骚扰门"之后的客户占位补发移除（方案 B 解耦），客户占位统一由守卫负责。

**Tech Stack:** Rust 2021 / Axum / MongoDB（既有 `agent_send_outbox` + 幂等键）。复用既有 `fallback_holding_reply()` 确定性文案与 `outbox::enqueue`。

## Global Constraints

每个 task 的要求都隐含包含本节，逐字遵守：

- **不过拟合**：绝不为过测试改业务逻辑 / prompt / guards / **硬闸阈值**（review/gates.rs 的 FactRisk/PressureRisk/grounding 数值闸一字不碰）。本修复只补"零回复"漏洞，不碰"为什么被拦"。
- **agent-first**：占位用**确定性文案** `fallback_holding_reply()`，绝不新增 LLM 产出的占位字段（违反 ④/⑨ 的"埋字段 LLM 无视"A/B 教训）。
- **绝不破坏去抖聚合**：现有"客户连发 N 条→4s 窗口去抖→聚合成一轮回复"（webhooks.rs:105-241）零影响。因去抖已把 N 条客户消息塌成 1 run，per-run 守卫每批正常只触发一次——**绝不能变成"对方发 N 条、我们回 N 条"**。守卫内部的 `should_abort_send()` 复查是这一保证的代码级兜底。
- **绝不破坏拟人分段发送**：现有"一条逻辑回复→`split_reply_into_segments` 拆成最多 4 条短消息拟人分发"（gateway.rs:3074）零影响。占位是单条 19 字短文案（< 单段上限），与分段逻辑正交：有真回复才走分段、零回复才补占位，两路径互斥永不同时触发。
- **check-no-human-takeover lint**：占位文案不含任何转接类禁词。`fallback_holding_reply()` 已过该 lint + 已有红线测试（tests/principal_decision_channel.rs §14.9b）。不新增任何含禁词的字面量（连测试断言 / 注释提 lint 名都不行，见 ⑨/④ 教训）。
- **DEFAULT 行为变更已评估**：现状"默认晾死"→改"默认兜底"是行为变更，但符合"客户永不被晾死"红线本意（relay 回程已贯彻，held 首发是漏的）。方案 B 移除 escalation 客户占位会让 **FollowUp + safety_guard/unverified** 路径从"发占位"变"静默"——这是改进（proactive 触达被拦后发"稍等我给你准信"是非所问），但属 DEFAULT 变更，final review 需知悉（见 Task 4 说明）。
- **基线门（不回归）**：`cargo test --lib` ≥ 350/0；4 PBT（state_transition_pbt / memory_card_invariants / wiki_chunk_revision_pbt / llm_retry_jitter）累计 ≥ 33/0。`scripts/check-baseline.{sh,ps1}` + `check-no-human-takeover` 两 lint 绿。
- **本地资源纪律**：本地只跑 `cargo test --lib` + 单 PBT 文件 + `cargo check --tests`（复刻 CI step2）。全量 `--ignored` 集成套件留 CI（磁盘小，testcontainers mongo 会撑爆 target/）。
- **新增测试只增量叠加**：只 append，绝不删改旧维度/旧弧/旧金标。
- **commit 纪律**：用户已整体授权提交；只 `git add` 具名文件，绝不 `-A`。

## 黑名单口径（全程一致，Task 1/3 共用）

守卫是**黑名单语义**（用户定"全兜底，最彻底"）：只要 `trigger.kind() == "inbound"` 且 status **不在**下列豁免清单内，就补占位。豁免清单（这些状态下"客户零回复"是**正确**的）：

| status | 为何豁免（不补） |
|---|---|
| `cooldown` | 客户主动叫停冷却期，补占位违反客户意愿 |
| `rate_limited` | 本批刚回过（min_reply_interval），补占位=重复打扰 |
| `quiet_hours_deferred` | 重排到醒来时真回（仅 FollowUp，Inbound 不会到此） |
| `expired` | 死任务（仅 FollowUp） |
| `superseded_by_new_inbound` | 下一轮用更全上下文真回 |
| `not_managed` | 非 managed 联系人（webhook 不会把它送进 gateway） |
| `context_changed` | FollowUp 上下文已变，下个 inbound 真回 |
| `no_reply` | **（实现期加入）** A3 主动沉默——AI 判定该沉默更拟人（非晾死），补占位破坏拟人 |

**会补占位的零回复状态**（晾死漏洞所在）：`daily_limit`、`policy_cooldown`、`policy_wait_user_reply`、`policy_consecutive_limit`（precheck 类）；`held_by_ai_policy`、`blocked_by_required_field`、`blocked_by_budget`、`blocked_by_safety_guard`、`blocked_unverified_product_claim`（finalize 终态）。〔实现期修正：`no_reply` 已移入上方豁免清单，不再补。〕

> 该黑名单逐字等于 spec §3.2 的排除清单。spec §3.1 只列了"两处"挂载点（拦截分支 + 第二道 precheck），**遗漏了第一道 precheck（gateway.rs:916）**——`daily_limit` 等会在决策前就被第一道 precheck 拦下、零回复 return。本计划据用户"全兜底"决定补上第一道 precheck 挂载点，使黑名单在所有 Inbound 零回复出口生效。

## 文件结构

| 文件 | 职责 | 改动 |
|---|---|---|
| `src/agent/gateway.rs` | 用户运营网关 | 新增 `ACK_PLACEHOLDER_EXCLUDED_STATUSES` const + 纯函数 `should_send_ack_placeholder` / `build_ack_enqueue_request` + async `ensure_customer_acknowledged`；4 处零回复出口挂载调用；纯函数单测追加进既有 `mod tests`（gateway.rs:4931） |
| `src/agent/escalation/mod.rs` | 决策请示通道 | 方案 B：移除 `escalate_held_decision` 里 137-143 客户占位补发，函数回归"只推领导 + 落台账 + 写 awaiting"；同步更新函数 doc 注释（去掉"补发安全占位"措辞） |
| `src/agent/escalation/logic.rs` | 占位文案 | 不改文案；仅 `fallback_holding_reply()` 保持 `pub`（已是），供 gateway 守卫与测试复用 |

---

### Task 1: 守卫判定纯函数 `should_send_ack_placeholder`

**Files:**
- Modify: `src/agent/gateway.rs`（在 `split_reply_into_segments`（约 3074 行）附近、`#[cfg(test)]` 之前的纯函数区追加 const + 函数）
- Test: `src/agent/gateway.rs` 既有 `#[cfg(test)] mod tests`（约 4931 行）追加单测

**Interfaces:**
- Produces:
  - `pub(crate) const ACK_PLACEHOLDER_EXCLUDED_STATUSES: &[&str]`
  - `pub(crate) fn should_send_ack_placeholder(trigger_kind: &str, status: &str) -> bool`

- [ ] **Step 1: 写失败测试**

在 `mod tests`（gateway.rs:4931，`use super::*;` 之后）追加：

```rust
// 客户回应保障守卫判定纯函数（黑名单语义，全兜底）：
// 只要 Inbound 且 status 不在豁免清单内就补占位。
#[test]
fn ack_placeholder_inbound_held_and_blocked_terminals_get_ack() {
    for status in [
        "held_by_ai_policy",
        "blocked_by_required_field",
        "blocked_by_budget",
        "blocked_by_safety_guard",
        "blocked_unverified_product_claim",
        "no_reply",          // A3 主动沉默：Inbound 仍须 ack
        "daily_limit",       // 每日触达上限：客户主动问也须 ack（全兜底）
        "policy_cooldown",   // 运营策略冷却：仍 ack
    ] {
        assert!(
            should_send_ack_placeholder("inbound", status),
            "inbound + {status} 应补占位"
        );
    }
}

#[test]
fn ack_placeholder_excluded_statuses_skip() {
    for status in [
        "cooldown",
        "rate_limited",
        "quiet_hours_deferred",
        "expired",
        "superseded_by_new_inbound",
        "not_managed",
        "context_changed",
    ] {
        assert!(
            !should_send_ack_placeholder("inbound", status),
            "豁免清单内的 {status} 不该补占位"
        );
    }
}

#[test]
fn ack_placeholder_follow_up_never_acks() {
    // FollowUp 是 AI 主动触达，不是客户在等回复——任何状态都不补占位。
    for status in [
        "held_by_ai_policy",
        "blocked_by_safety_guard",
        "no_reply",
        "daily_limit",
    ] {
        assert!(
            !should_send_ack_placeholder("follow_up", status),
            "follow_up + {status} 不该补占位"
        );
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib ack_placeholder`
Expected: FAIL，编译错误 `cannot find function should_send_ack_placeholder`。

- [ ] **Step 3: 写最小实现**

在 gateway.rs 纯函数区（`split_reply_into_segments` 之后、`#[cfg(test)]` 之前）追加：

```rust
/// 客户回应保障——零回复豁免清单（黑名单语义）。这些终态 / precheck 状态下
/// 「客户零回复」是**正确**的，不补占位（口径见 plan「黑名单口径」表）。
/// 逐字等于 spec §3.2 排除清单。
pub(crate) const ACK_PLACEHOLDER_EXCLUDED_STATUSES: &[&str] = &[
    "cooldown",
    "rate_limited",
    "quiet_hours_deferred",
    "expired",
    "superseded_by_new_inbound",
    "not_managed",
    "context_changed",
];

/// 是否该给本轮零回复的客户补一条确定性安抚占位。
///
/// 黑名单语义（全兜底）：只要是 Inbound（`trigger_kind == "inbound"`，客户真发了消息）
/// 且 `status` 不在豁免清单内，就补。`status` 取各零回复出口的状态串：
/// precheck.status / 拦截分支 blocked_status / A3 主动沉默路径的 `"no_reply"`。
///
/// 红线：FollowUp（AI 主动触达，客户没在等回复）任何状态都不补——避免"主动触达被拦"
/// 时发"稍等我给你准信"这类非所问的占位。
pub(crate) fn should_send_ack_placeholder(trigger_kind: &str, status: &str) -> bool {
    trigger_kind == "inbound" && !ACK_PLACEHOLDER_EXCLUDED_STATUSES.contains(&status)
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --lib ack_placeholder`
Expected: PASS（3 个测试全绿）。

- [ ] **Step 5: 提交**

```bash
git add src/agent/gateway.rs
git commit -m "feat(gateway): 客户回应保障守卫判定纯函数(黑名单语义)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: 占位 outbox 入参构造纯函数 `build_ack_enqueue_request`

**Files:**
- Modify: `src/agent/gateway.rs`（紧接 Task 1 的纯函数追加）
- Test: `src/agent/gateway.rs` 既有 `mod tests` 追加单测

**Interfaces:**
- Consumes: `escalation::fallback_holding_reply() -> &'static str`（既有，`src/agent/escalation/logic.rs:85`，crate 内经 `use super::escalation;` 可达）；`super::outbox::EnqueueRequest`（既有，`src/agent/outbox.rs:126`，gateway 已 `use super::outbox::{enqueue as outbox_enqueue, EnqueueOutcome, EnqueueRequest};`）。
- Produces: `pub(crate) fn build_ack_enqueue_request(workspace_id: &str, account_id: &str, contact_wxid: &str, run_id: &str, source_event_id: &str, trigger_kind: &str) -> EnqueueRequest`
- 设计说明：取**三个 contact 字符串字段**而非 `&Contact`——本函数只需 workspace_id/account_id/wxid 三个值，取原语使其成为零依赖纯函数，单测无需构造 40 字段的 `Contact`（`Contact` 未派生 `Default`，gateway 测试模块也从不构造它）。call 站点显式传 `&contact.workspace_id` 等，对"读了哪几个字段"更诚实。

- [ ] **Step 1: 写失败测试**

在 `mod tests` 追加：

```rust
#[test]
fn build_ack_enqueue_request_shape() {
    let req = build_ack_enqueue_request("ws1", "acc1", "cust_wxid", "run_abc", "evt123", "inbound");

    // 幂等键派生：源事件 id 加 `#ack-placeholder` 后缀，与真回复 / 分段 key 天然不碰撞
    assert_eq!(req.source_event_id, "evt123#ack-placeholder");
    // 占位文案 = 确定性兜底（agent-first，不靠 LLM）
    assert_eq!(req.content, escalation::fallback_holding_reply());
    // 占位是纯文本，不带媒体 / 名片
    assert!(req.media_asset_id.is_none());
    assert!(req.referral_card_id.is_none());
    // 占位无决策评审记录
    assert!(req.decision_id.is_none());
    assert_eq!(req.workspace_id, "ws1");
    assert_eq!(req.account_id, "acc1");
    assert_eq!(req.contact_wxid, "cust_wxid");
    assert_eq!(req.run_id, "run_abc");
    assert_eq!(req.source_kind, "inbound");
    assert_eq!(req.max_attempts, 3);
}

#[test]
fn build_ack_enqueue_request_empty_source_event_id_still_suffixed() {
    let req = build_ack_enqueue_request("ws", "acc", "wx", "run1", "", "inbound");
    // 空 source_event_id 仍带后缀（非空），走 outbox 非 synthetic 路径
    assert_eq!(req.source_event_id, "#ack-placeholder");
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib build_ack_enqueue_request`
Expected: FAIL，`cannot find function build_ack_enqueue_request`。

- [ ] **Step 3: 写最小实现**

紧接 Task 1 的函数追加：

```rust
/// 构造"客户回应保障占位"的 outbox 入参。
///
/// 复用 `fallback_holding_reply()` 确定性文案，走 outbox（享受 dispatcher 在线门控 +
/// 幂等键，与正常发送路径一致）。幂等键派生：`{source_event_id}#ack-placeholder` 后缀，
/// 保证同 run 重复挂载只入一条、且与真回复 / 分段（`#seg{idx}`）key 天然不碰撞。
///
/// 取 contact 的三个字符串字段而非 `&Contact`：本函数只需这三个值，原语入参使其成为
/// 零依赖纯函数（单测无需构造 40 字段的 Contact）。
pub(crate) fn build_ack_enqueue_request(
    workspace_id: &str,
    account_id: &str,
    contact_wxid: &str,
    run_id: &str,
    source_event_id: &str,
    trigger_kind: &str,
) -> EnqueueRequest {
    EnqueueRequest {
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        contact_wxid: contact_wxid.to_string(),
        run_id: run_id.to_string(),
        decision_id: None,
        source_event_id: format!("{source_event_id}#ack-placeholder"),
        source_kind: trigger_kind.to_string(),
        content: escalation::fallback_holding_reply().to_string(),
        media_asset_id: None,
        referral_card_id: None,
        max_attempts: 3,
    }
}
```

> 注：取原语字符串入参，单测无需 `Contact` 实例。call 站点（Task 3 守卫内）显式传 `&contact.workspace_id, &contact.account_id, &contact.wxid`。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --lib build_ack_enqueue_request`
Expected: PASS（2 个测试绿）。

- [ ] **Step 5: 提交**

```bash
git add src/agent/gateway.rs
git commit -m "feat(gateway): 客户回应保障占位 outbox 入参构造纯函数

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: async 守卫 `ensure_customer_acknowledged` + 4 处零回复出口挂载

**Files:**
- Modify: `src/agent/gateway.rs`（新增 async 守卫函数；在 4 个零回复出口插入调用）

**Interfaces:**
- Consumes: Task 1 `should_send_ack_placeholder`；Task 2 `build_ack_enqueue_request`；`outbox_enqueue`（既有别名 `enqueue`）；`should_abort_send: &Option<Arc<dyn Fn() -> bool + Send + Sync>>`（`run_user_operation_gateway_inner` 既有入参，gateway.rs:872）。
- Produces: `async fn ensure_customer_acknowledged(state: &AppState, contact: &Contact, run_id: &str, trigger_kind: &str, source_event_id: &str, status: &str, should_abort_send: &Option<Arc<dyn Fn() -> bool + Send + Sync>>)`（无返回值，fail-soft）。

- [ ] **Step 1: 写 async 守卫函数**

在 gateway.rs 私有 async 辅助函数区（如紧邻 `precheck_operation_policy` 之后、或 `run_user_operation_gateway_inner` 之前的私有 fn 区）新增：

```rust
/// 客户回应保障守卫：本轮若是 Inbound 且落到会晾死客户的零回复状态，给客户补一条
/// 确定性中性占位（走 outbox）。统一三道 precheck 出口 + 拦截分支 + A3 主动沉默路径。
///
/// - 黑名单判定见 [`should_send_ack_placeholder`]（仅 Inbound、非豁免状态才补）。
/// - 入队前复查 `should_abort_send()`：客户又发了新消息 → 下一轮会真回，补占位会与下轮
///   回复竞争重复打扰，故跳过（这也是"绝不破坏去抖聚合"的代码级兜底）。
/// - fail-soft：入队失败只记 warn、不阻断 run、不改终态（与 `escalate_held_decision`
///   的 let _ / warn 同纪律）。
async fn ensure_customer_acknowledged(
    state: &AppState,
    contact: &Contact,
    run_id: &str,
    trigger_kind: &str,
    source_event_id: &str,
    status: &str,
    should_abort_send: &Option<Arc<dyn Fn() -> bool + Send + Sync>>,
) {
    if !should_send_ack_placeholder(trigger_kind, status) {
        return;
    }
    if let Some(guard) = should_abort_send {
        if guard() {
            tracing::info!(
                %run_id,
                contact_wxid = %contact.wxid,
                "客户回应保障占位跳过：客户又发新消息，下一轮真回"
            );
            return;
        }
    }
    let req = build_ack_enqueue_request(
        &contact.workspace_id,
        &contact.account_id,
        &contact.wxid,
        run_id,
        source_event_id,
        trigger_kind,
    );
    match outbox_enqueue(state, req).await {
        Ok(outcome) => {
            tracing::info!(
                %run_id,
                contact_wxid = %contact.wxid,
                %status,
                ?outcome,
                "客户回应保障占位已入 outbox"
            );
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                %run_id,
                contact_wxid = %contact.wxid,
                %status,
                "客户回应保障占位入队失败（不阻断 run）"
            );
        }
    }
}
```

> `EnqueueOutcome` 已派生 `Debug`（outbox.rs:107 `pub enum EnqueueOutcome`，确认其上有 `#[derive(Debug)]`；若无则把 `?outcome` 改成不打印 outcome，只记 run_id/status）。

- [ ] **Step 2: 挂载点①——第一道 precheck-abort（决策前）**

定位 gateway.rs:879-916 的 `if !precheck.allowed { ... return Ok(()); }` 块。在 `return Ok(());`（约 916 行）**之前**插入：

```rust
        ensure_customer_acknowledged(
            state,
            &contact,
            &run_id,
            trigger.kind(),
            &envelope_source_event_id,
            &precheck.status,
            &should_abort_send,
        )
        .await;
```

（`envelope_source_event_id` 在本函数 876 行已派生；`precheck`、`contact`、`run_id`、`trigger`、`should_abort_send` 均在作用域内。）

- [ ] **Step 3: 挂载点②——拦截分支（held/blocked/revision_failed）**

定位 gateway.rs:1886-2010 的 `if !matches!(finalize_status, GatewayStatusFinal::Approved) { ... }` 块。在末尾 `return Ok(());`（约 2010 行）**之前**、`escalate_held_decision(...)` 调用**之后**插入：

```rust
        ensure_customer_acknowledged(
            state,
            &contact,
            &run_id,
            trigger.kind(),
            &envelope_source_event_id,
            &blocked_status,
            &should_abort_send,
        )
        .await;
```

（`blocked_status` 在该块 1887 行已绑定。注意：方案 B（Task 4）会让 `escalate_held_decision` 只推领导、不再发客户占位，客户占位改由这里负责——两者解耦。）

- [ ] **Step 4: 挂载点③——第二道 precheck-abort（决策后）**

定位 gateway.rs:2014-2087 的 `if final_decision.should_reply && !final_precheck.allowed { ... return Ok(()); }` 块。在 `return Ok(());`（约 2087 行）**之前**插入：

```rust
        ensure_customer_acknowledged(
            state,
            &contact,
            &run_id,
            trigger.kind(),
            &envelope_source_event_id,
            &final_precheck.status,
            &should_abort_send,
        )
        .await;
```

- [ ] **Step 5: 挂载点④——A3 主动沉默（approved + should_reply=false）**

定位 gateway.rs:2200-2204 的 `if !final_decision.should_reply { if let Some(task_id) = task_id { cancel_task(... "no_reply" ...).await?; } }` 块。在 `cancel_task` 调用**之后**、该 `if` 块闭合**之前**插入（与 cancel_task 同级，置于内层 `if let` 之外以覆盖无 task_id 的 Inbound）：

```rust
    if !final_decision.should_reply {
        if let Some(task_id) = task_id {
            cancel_task(state, task_id, "no_reply", "Agent 判断无需触达").await?;
        }
        ensure_customer_acknowledged(
            state,
            &contact,
            &run_id,
            trigger.kind(),
            &envelope_source_event_id,
            "no_reply",
            &should_abort_send,
        )
        .await;
    }
```

> 这里传字面量 `"no_reply"`（A3 无 blocked_status）。`"no_reply"` 不在黑名单 → Inbound 补占位、FollowUp 不补（纯函数已 gate）。此处是 fall-through（非 return），守卫只入队不影响后续；A3 下 `outbox_eligible=false`、媒体门 false，本轮除占位外不会再发任何东西，无重复。

- [ ] **Step 6: 编译 + 全量 lib 测试**

Run: `cargo test --lib`
Expected: PASS，计数 ≥ 350/0（含 Task 1/2 的 5 个新测试）。无编译错误 / 无 unused 警告（`ensure_customer_acknowledged` 被 4 处调用、`should_abort_send` 已被多处用）。

- [ ] **Step 7: cargo check --tests（复刻 CI step2）**

Run: `cargo check --tests`
Expected: exit 0。

- [ ] **Step 8: 提交**

```bash
git add src/agent/gateway.rs
git commit -m "feat(gateway): 客户回应保障守卫挂 4 处零回复出口(Inbound必有回应)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: 方案 B——解耦 escalate_held_decision（移除客户占位补发）

**Files:**
- Modify: `src/agent/escalation/mod.rs:32-161`（`escalate_held_decision` 函数体 + doc 注释）

**Interfaces:**
- Consumes: 无新增。
- Produces: `escalate_held_decision` 行为变更——不再向客户发任何消息，回归"只推领导卡 + 落 pending 台账 + 写 awaiting 标记"。

**背景（实现者必读）：** 当前 `escalate_held_decision`（mod.rs:40-161）在推完领导卡后（136-143 行）给客户补发 `fallback_holding_reply()`。问题：该补发排在一连串"领导骚扰门"（`should_escalate_held` / `decider_chain` 空 / `push_allowed` / `has_pending_for_contact`）之后，任一门命中即 `return Ok(())`，客户占位一起被跳过——这是"客户被晾死"的根因之一。Task 3 的守卫已无条件覆盖所有 Inbound 零回复终态（含 safety_guard / unverified），故这里安全移除客户占位，让本函数回归单一职责（只跟领导打交道）。

- [ ] **Step 1: 移除客户占位补发**

定位 mod.rs:136-143：

```rust
    // 补发安全占位安抚客户（hold 路径无 outbox，直发；体验与 approved 占位一致）。
    mcp::logged_call_for_account(
        state,
        &contact.account_id,
        "message_send_text",
        serde_json::json!({ "recipient": &contact.wxid, "content": fallback_holding_reply() }),
    )
    .await?;
```

**整段删除**（连同上方注释行）。删除后，紧接其下的"写 awaiting 标记"块（mod.rs:144 起的 `let set_key = ...`）保留不动。

- [ ] **Step 2: 更新函数 doc 注释**

定位 mod.rs:32-39 的 doc 注释，把"并补发安全占位"/"额外**补发安全占位**安抚客户"相关措辞改为反映新职责。替换为：

```rust
/// hold→升级请示：被风险闸门拦下的高风险件，按 workspace 升级模式请示领导。
///
/// 与 `trigger_principal_escalation` 的区别：后者用于 approved 路径（占位已由 outbox 发出）；
/// 本函数用于 hold 路径，只推领导卡 + 落 pending 台账 + 写 awaiting 标记，**不向客户发任何消息**。
/// 客户侧的安抚占位由网关守卫 `ensure_customer_acknowledged` 统一负责（解耦"安抚客户"与
/// "请示领导"：前者对任何 Inbound 零回复无条件补，后者受领导骚扰门 / 去重约束）。
///
/// 调用方对本函数错误只记 warn、不阻断 run、不改终态。
```

- [ ] **Step 3: 处理可能的 unused 警告**

`fallback_holding_reply` 在 mod.rs 内若仅 136-143 使用，删除后 mod.rs 不再引用它。它经 `pub(crate) use logic::*`（glob）+ `pub use logic::fallback_holding_reply`（mod.rs:17）暴露——glob 未用成员不报 unused，`pub use` 是公开再导出也不报。故**预期无 unused 警告**。若 `cargo check` 仍报，按提示处理（不要删 `pub use` 行——tests/principal_decision_channel.rs §14.9b 红线测试依赖它 crate 外可见）。

- [ ] **Step 4: 编译 + 既有请示通道测试不回归**

Run: `cargo test --lib`
Expected: PASS，≥ 350/0。

Run: `cargo test --test principal_decision_channel 2>/dev/null || cargo test --lib principal`
Expected: 请示通道相关单测（含 §14.9b `fallback_holding_reply` 红线纯函数测试）仍绿。若 `principal_decision_channel` 是 `#[ignore]` 集成测试需 Docker，则本地只确认 `cargo check --tests` 编译过，完整跑留 CI。

- [ ] **Step 5: lint——no-human-takeover**

Run: `bash scripts/check-no-human-takeover.sh 2>/dev/null || pwsh scripts/check-no-human-takeover.ps1`
Expected: 0 违规。本 task 是**删除** + 注释改写，新增行（doc 注释）不含禁词（"领导/请示/守卫/占位"均非禁词；不含"转人工/接管/hand-off/人工"）。

- [ ] **Step 6: 提交**

```bash
git add src/agent/escalation/mod.rs
git commit -m "refactor(escalation): 解耦安抚客户与请示领导,escalate_held只推领导(方案B)

客户占位统一由 gateway 守卫 ensure_customer_acknowledged 负责,根治
'安抚客户被领导骚扰门跳过→客户零回复'缺陷。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 5: 基线门 + server117 真模型回归

**Files:** 无代码改动（验证 task）。

**背景：** 验证整波（Task 1-4）不回归基线，且端到端真实修复了"held 后客户零回复"。real-model 回归走 server117（本地资源受限、真测纪律见 Global Constraints）。biz-test 脚本必须从**本地**跑（`_lib.remote_run` 本地→SSH→server，在 server 上跑会嵌套 SSH 失败）。

- [ ] **Step 1: 本地基线门**

Run: `cargo test --lib`
Expected: ≥ 350/0（含 5 个新纯函数测试）。

Run（4 PBT 累计 ≥ 33/0）:
```bash
cargo test --test state_transition_pbt
cargo test --test memory_card_invariants
cargo test --test wiki_chunk_revision_pbt
cargo test --test llm_retry_jitter
```
Expected: 四个累计 ≥ 33 passed / 0 failed。

Run: `cargo check --tests`
Expected: exit 0。

- [ ] **Step 2: no-human-takeover lint（整波 diff）**

Run: `bash scripts/check-no-human-takeover.sh 2>/dev/null || pwsh scripts/check-no-human-takeover.ps1`
Expected: 0 违规。

- [ ] **Step 3: 部署 server117（用户授权时）**

把整波合并/推送后的 commit 部署到 server117（117.72.54.28，/opt/wechatagent，service wechatagent，app 3003）。出海恢复后可 `git fetch origin` + checkout 目标 commit + `cargo build --release` + restart service + 确认 HTTP 200。
> 部署坑（见 memory）：`setsid` 后台 `&` 在 SSH PTY 下进程会丢失（无日志无 rustc）→ 用前台 `cargo build` 输出到 `/tmp` 日志同步等待；SSH 输出流偶断会误判 build 未完成，以 binary relink 时间 + service 启动时间为准。

- [ ] **Step 4: 真模型回归——held 后客户必有回应**

从**本地**跑（先 `step0_preflight.py` 确认 HEAD/ACTIVE model/account online）：

```bash
MSYS_NO_PATHCONV=1 PYTHONUTF8=1 python3 scripts/biz-test/step0_preflight.py
MSYS_NO_PATHCONV=1 PYTHONUTF8=1 python3 scripts/biz-test/batch_a_domain4.py
```

复跑 batch_a_domain4 路径2（assist 开签约引荐）：reviewer 仍可能 held（话术瑕疵，本波不碰硬闸），但**核心断言：本轮即使 final=held_by_ai_policy，客户也收到了占位**——查 `conversation_messages` 该客户 outbound 非空（含 `fallback_holding_reply()` 文案），或查 `agent_send_outbox` 有一条 `source_event_id` 以 `#ack-placeholder` 结尾的条目。

> 真测纪律（见 memory feedback_biztest_fix_loop_no_overfitting）：端点 glitch（`llm_tool_use_instead_of_json`）导致的残缺 run 标 BLOCKED 不假绿；只认 reviewer/守卫真实落库的 run。若端点污染拿不到 clean run，记录 BLOCKED 状态待端点稳定复跑，不强判通过。

- [ ] **Step 5: 换 seed 验证泛化**

至少再换一个 held 触发场景（如构造一条会触发 `blocked_unverified_product_claim` 的客户问句），确认守卫同样补占位（验证方案 B 解耦后 safety/unverified 类不回归——这两类原由 escalate_held_decision 发占位，移除后必须由守卫接住）。

- [ ] **Step 6: 账本记录 + 收尾**

把真测结果（run id / 客户是否收到占位 / 是否 BLOCKED）记入 `.git/sdd-held-customer-reply-exploration.md` 账本。整波 review clean + 基线绿后，转 `superpowers:finishing-a-development-branch` 决定合并方式。

---

## Self-Review（plan vs spec）

**1. Spec coverage：**
- spec §1 根因（垃圾桶终态 / 安抚客户与请示领导耦合 / 三类零回复）→ Task 3（守卫覆盖全部零回复终态）+ Task 4（解耦）。✓
- spec §2.1 核心不变量 "Inbound 必有回应" → Task 1 黑名单 + Task 3 四处挂载（含第一道 precheck，补全 spec §3.1 遗漏）。✓
- spec §3.1 守卫两层（纯函数 + 副作用）→ Task 1（`should_send_ack_placeholder`）+ Task 3（`ensure_customer_acknowledged`）。✓ 签名细化：用统一 `status: &str` 取代 spec 的 `(final_status, should_reply)`，因第一道 precheck 出口无 final_status——已在「黑名单口径」节说明。
- spec §3.2 排除清单 → Task 1 `ACK_PLACEHOLDER_EXCLUDED_STATUSES` 逐字对应。✓
- spec §3.3 走 outbox + 幂等键 `#ack-placeholder` → Task 2。✓
- spec §3.4 方案 B 解耦 + 回归保障（零回复集含 safety_guard/unverified）→ Task 4 + Task 5 Step 5。✓ relay 回程占位（logic.rs `expired_authorization_neutral_reply` / `chain_tail_holding_reply`）不动。✓
- spec §3.5 A3 主动沉默 → Task 3 挂载点④。✓
- spec §5 测试 → Task 1/2 纯函数单测（本地）+ Task 5 真模型回归。✓ 重型 Docker 集成测试（spec §5.2）降级为"靠纯函数单测 + 真模型端到端覆盖"——强制构造 held 终态需 mock LLM、成本高/价值中，纯函数已覆盖黑名单逻辑、真测覆盖端到端，故不单列 Docker 集成 task（YAGNI）。
- spec §7 非目标（不重构垃圾桶终态分类 / 不碰硬闸 / 不改去抖 / 不加 LLM 占位字段）→ Global Constraints + 各 task 未触碰。✓

**2. Placeholder scan：** 无 TBD/TODO；所有代码步骤含完整代码块；测试步骤含完整断言。✓

**3. Type consistency：** `should_send_ack_placeholder(&str, &str)->bool`、`build_ack_enqueue_request(&Contact,&str,&str,&str)->EnqueueRequest`、`ensure_customer_acknowledged(...)` 签名在 Task 1/2/3 定义与 Task 3 调用一致；`EnqueueRequest` 字段与 outbox.rs:126 实际定义逐字对齐（workspace_id/account_id/contact_wxid/run_id/decision_id/source_event_id/source_kind/content/media_asset_id/referral_card_id/max_attempts）。✓
