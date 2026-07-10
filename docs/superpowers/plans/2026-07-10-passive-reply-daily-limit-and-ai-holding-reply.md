# 被动回复豁免每日触达上限 + 过渡回复改 AI 生成 —— 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 `daily_limit` 只限制 AI 主动触达（被动回复豁免），并把给客户的过渡/占位回复从硬编码改为 AI 生成（带独立预算旁路、运行期禁词守卫、硬编码降级兜底）。

**Architecture:** 修复一是 `precheck_send_gateway` 中一道门的守卫收窄（一行条件 + 注释/文档/测试）。修复二新增一个统一生成器 `generate_holding_reply`，用 `RUN_BUDGET.scope` 起独立预算调 `generate_agent_json`，经 `evolution::lint::passes_forbidden_words`（+ C 类数字守卫）审查后返回 AI 文案，任何失败/耗尽/命中即回落既有硬编码文案；三个既有发送点改调该生成器取文案。

**Tech Stack:** Rust 2021 / Axum / tokio task-local（`RUN_BUDGET`）/ MongoDB（mongodb crate）/ serde_json。LLM 走 `generate_agent_json`（唯一 JSON 入口）。

## Global Constraints

- 红线：改任何一行前必须已 100% 读懂受影响代码路径（本计划的 file:line 均已亲验）。
- 无人工接管红线：新增/改动的**运行期客户出站文本**必须过 `evolution::lint::passes_forbidden_words`；命中回落已知无禁词的硬编码文案。禁词词表复用 `src/evolution/lint.rs:13-28`，**不在 `src/agent/` 下新写禁词字面量**（会被 `scripts/check-no-human-takeover` CI lint 扫 `src/agent/` 新增行时自噬）。
- 客户永不被晾死：硬编码三文案（`escalation/logic.rs:85/92/99`）保留，是所有降级路径终点。
- 反过拟合：守卫/降级均为可单测的确定性逻辑，多形态变体测试，绝不为过单条测试改业务阈值。
- 测试基线（合并门）：`cargo test --lib` **≥ 350 passed, 0 failed**；4 个 PBT 文件累计 **≥ 33 passed, 0 failed**。新工作只增测试不降基线。
- 子 agent 红线：派实现/修复子 agent 必须要求先读码给 file:line 证据；省略 `model` 参数（传 "opus" 会 400）。
- 本地磁盘紧：只跑 `cargo test --lib` 与单个 PBT，集成测试留 CI。
- 提交纪律：只 `git add` 具名文件（共享 worktree），绝不 `git add -A`；提交信息中文，尾部带 Co-Authored-By。
- 发送路径差异保留：A 类占位走 outbox（`ensure_customer_acknowledged`），C 类走裸 MCP 直发（`escalation/mod.rs`）。本计划**只换文案来源**，不改各自的发送路径。

---

## File Structure

- `src/agent/gateway.rs`
  - 修复一：`precheck_send_gateway` daily_limit 门（:3129）加 `FollowUp` 守卫 + 注释。
  - 修复二接入：`ensure_customer_acknowledged`（:913）改调 `generate_holding_reply`。
- `src/agent/escalation/logic.rs`
  - 新增 `HoldingReplyScene` 枚举 + `scene_fallback_text(scene) -> &'static str` 映射（把三个既有硬编码文案按场景归类，纯函数，可单测）。
  - 三个 `&'static str` 文案函数保留不动。
- `src/agent/escalation/holding_reply.rs`（**新建**）
  - `generate_holding_reply(...)`：独立预算旁路调 LLM + 守卫 + 降级。单一职责，便于单测降级链。
- `src/agent/escalation/mod.rs`
  - `mod holding_reply;` 声明 + re-export。
  - C1 链尾（:390）、C2 授权过期（:201）改调 `generate_holding_reply`。
- `docs/agent-policy.md`
  - :110 每日触达上限语义改写。

---

## Task 1: daily_limit 门豁免被动回复

**Files:**
- Modify: `src/agent/gateway.rs:3129-3131`（daily_limit 门）
- Modify: `src/agent/gateway.rs:3100-3136`（门控顺序注释）
- Modify: `src/agent/gateway.rs` 文件末 `mod tests`（新增单测；修正 :5234 注释）
- Modify: `docs/agent-policy.md:110`

**Interfaces:**
- Consumes: `AgentTrigger`（`src/agent/types.rs`，变体 `Inbound(&ConversationMessage)` / `FollowUp(&AgentTask)`）；`daily_touch_count(state, contact) -> AppResult<i64>`（gateway.rs:3436）；`blocked(status, reason) -> SendGatewayResult`（gateway.rs:3425）。
- Produces: 无新公共接口；仅收窄 daily_limit 门触发条件。

- [ ] **Step 1: 写失败测试（inbound 超限应放行，follow_up 超限仍拦）**

在 `src/agent/gateway.rs` 文件末 `mod tests` 内新增。注意：`precheck_send_gateway` 需要 `AppState` + DB（`daily_touch_count` 查 mongo），不宜在 lib 单测直接跑。改为**抽出纯判定函数**测试更稳妥——先加一个纯函数把"是否对该 trigger 施加 daily_limit"独立出来：

```rust
// 在 gateway.rs daily_touch_count 附近新增纯函数（本步骤先写测试，函数下一步实现）：
// pub(crate) fn daily_limit_applies_to(trigger: &AgentTrigger<'_>) -> bool

#[test]
fn daily_limit_applies_only_to_follow_up() {
    use crate::agent::types::AgentTrigger;
    // 构造一个最小 AgentTask 与 ConversationMessage 用于 trigger 变体判定。
    let task = crate::models::AgentTask::default();
    let msg = crate::models::ConversationMessage::default();
    // 主动触达：受 daily_limit
    assert!(daily_limit_applies_to(&AgentTrigger::FollowUp(&task)));
    // 被动回复：豁免 daily_limit
    assert!(!daily_limit_applies_to(&AgentTrigger::Inbound(&msg)));
}
```

> 若 `AgentTask` / `ConversationMessage` 无 `Default`，用测试内最小构造（参照同文件既有测试如 :5311 `forged_sentinel_trigger_is_not_relay_exempt` 如何构造 trigger）。实现步骤前先 grep 确认构造方式：`grep -n "AgentTrigger::Inbound" src/agent/gateway.rs` 看既有测试怎么造。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib daily_limit_applies_only_to_follow_up`
Expected: 编译失败 `cannot find function daily_limit_applies_to`。

- [ ] **Step 3: 实现纯函数 + 接入门**

在 `gateway.rs` `daily_touch_count`（:3436）上方新增纯函数：

```rust
/// daily_limit（每日触达上限）仅约束 AI **主动触达**（FollowUp）。
/// 客户主动发消息 → AI 被动回复（Inbound）属"客户期待内的被动应答"，永不受此上限限制
/// （语义同 quiet_hours 门 gateway.rs:3154 / relay 豁免 logic.rs:172-173）。
pub(crate) fn daily_limit_applies_to(trigger: &AgentTrigger<'_>) -> bool {
    matches!(trigger, AgentTrigger::FollowUp(_))
}
```

把 daily_limit 门（:3129-3131）改为：

```rust
if daily_limit_applies_to(trigger)
    && daily_touch_count(state, contact).await? >= runtime.max_daily_touches
{
    return Ok(blocked("daily_limit", "已达到每日触达上限"));
}
```

更新 :3100-3104 附近注释：daily_limit 现仅作用于 FollowUp 主动触达（被动回复豁免）。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --lib daily_limit_applies_only_to_follow_up`
Expected: PASS。

- [ ] **Step 5: 修正既有测试注释**

`gateway.rs:5234` 那条 `"daily_limit", // 每日触达上限：客户主动问也须 ack（全兜底）` 的语义前提已变（inbound 不再命中 daily_limit）。把该注释改为：`"daily_limit", // 仅 FollowUp 会命中；此用例验证 should_send_ack_placeholder 对该状态串的黑名单判定，与门是否触发无关`。不改断言本身（`should_send_ack_placeholder("inbound","daily_limit")==true` 仍为纯函数正确行为）。

- [ ] **Step 6: 更新文档**

`docs/agent-policy.md:110`：把"任何自动发送，包括私聊自动回复和 follow-up 定时任务……每日触达上限……"改为明确区分——每日触达上限（max_daily_touches）**仅约束 AI 主动触达（follow-up）**；客户主动消息的被动回复不受此上限限制，仅受最小回复间隔与账号级软上限约束。

- [ ] **Step 7: 跑基线确认不回归**

Run: `cargo test --lib`
Expected: ≥ 350 passed, 0 failed。

- [ ] **Step 8: 提交**

```bash
git add src/agent/gateway.rs docs/agent-policy.md
git commit -m "fix(gateway): daily_limit 每日触达上限仅约束主动触达,被动回复豁免

客户主动发消息的被动回复(Inbound)不再受 max_daily_touches 限制,
该上限语义收窄为纯主动骚扰防护(与 quiet_hours 门 FollowUp-only 范式一致)。
防刷屏仍靠 min_reply_interval + 账号级软上限。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 2: HoldingReplyScene 枚举 + 场景→硬编码兜底映射

**Files:**
- Modify: `src/agent/escalation/logic.rs`（新增枚举 + 映射函数，紧邻三个文案函数 :85-101）
- Modify: `src/agent/escalation/logic.rs` 文件末 `mod tests`（新增单测）

**Interfaces:**
- Consumes: 既有 `fallback_holding_reply()` / `chain_tail_holding_reply()` / `expired_authorization_neutral_reply()`（logic.rs:85/92/99）。
- Produces:
  - `pub(crate) enum HoldingReplyScene { GateHold, ChainTail, ExpiredAuthorization }`
  - `pub(crate) fn scene_fallback_text(scene: HoldingReplyScene) -> &'static str`

- [ ] **Step 1: 写失败测试**

在 `src/agent/escalation/logic.rs` 文件末 `mod tests` 内新增：

```rust
#[test]
fn scene_fallback_text_maps_each_scene_to_its_hardcoded_copy() {
    assert_eq!(scene_fallback_text(HoldingReplyScene::GateHold), fallback_holding_reply());
    assert_eq!(scene_fallback_text(HoldingReplyScene::ChainTail), chain_tail_holding_reply());
    assert_eq!(
        scene_fallback_text(HoldingReplyScene::ExpiredAuthorization),
        expired_authorization_neutral_reply()
    );
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib scene_fallback_text_maps_each_scene`
Expected: 编译失败 `cannot find type HoldingReplyScene`。

- [ ] **Step 3: 实现枚举 + 映射**

在 `logic.rs` 三个文案函数（:85-101）下方新增：

```rust
/// 过渡/占位回复的场景分类。决定 AI 生成失败时回落到哪条硬编码兜底文案，
/// 以及生成 prompt 的语境框定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HoldingReplyScene {
    /// 闸门拦截后的客户回应保障占位（held/blocked/budget/revision_failed 等）。
    GateHold,
    /// 请示领导链尾失联，持续安抚。
    ChainTail,
    /// relay 转述时领导授权已过期，中性收尾。
    ExpiredAuthorization,
}

/// 场景 → 确定性硬编码兜底文案。AI 生成失败/禁词命中/预算耗尽时的最终回落。
pub(crate) fn scene_fallback_text(scene: HoldingReplyScene) -> &'static str {
    match scene {
        HoldingReplyScene::GateHold => fallback_holding_reply(),
        HoldingReplyScene::ChainTail => chain_tail_holding_reply(),
        HoldingReplyScene::ExpiredAuthorization => expired_authorization_neutral_reply(),
    }
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --lib scene_fallback_text_maps_each_scene`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add src/agent/escalation/logic.rs
git commit -m "feat(escalation): 新增 HoldingReplyScene 场景枚举 + 硬编码兜底映射

三类过渡回复场景(闸门占位/链尾失联/授权过期)统一分类,
scene_fallback_text 映射到既有三条硬编码文案作为降级终点。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 3: generate_holding_reply 生成器（新建 holding_reply.rs）

**Files:**
- Create: `src/agent/escalation/holding_reply.rs`
- Modify: `src/agent/escalation/mod.rs`（加 `mod holding_reply;` + `pub(crate) use holding_reply::generate_holding_reply;`）
- Test: 在 `holding_reply.rs` 文件内 `mod tests`（纯函数守卫/降级判定的单测）

**Interfaces:**
- Consumes:
  - `HoldingReplyScene` / `scene_fallback_text`（Task 2）
  - `crate::evolution::lint::passes_forbidden_words(&str) -> bool`（lint.rs:33，`true`=无禁词）
  - `crate::agent::escalation::logic::relay_introduces_unauthorized_number(reply, authorized) -> bool`（logic.rs:234）
  - `crate::agent::generate_agent_json(state, account_id, contact_wxid, run_id, prompt_key, system, user) -> AppResult<Value>`（mod.rs:215）
  - `crate::agent::budget::{RunBudget, RUN_BUDGET, current_run_budget}`（budget.rs:75/196/200）
  - `RunBudget::new(run_id, token_budget, max_llm_calls, tool_call_budget)`（4 参数）
- Produces:
  - `pub(crate) fn holding_reply_text_is_safe(text: &str, scene: HoldingReplyScene, authorized_substance: Option<&str>) -> bool`（纯函数：非空 + 无禁词 +（授权类）无授权外数字）
  - `pub(crate) async fn generate_holding_reply(state: &AppState, account_id: &str, contact_wxid: &str, scene: HoldingReplyScene, authorized_substance: Option<&str>) -> String`（保证返回非空、安全的文案；失败回落 `scene_fallback_text`）

- [ ] **Step 1: 写失败测试（纯函数守卫 holding_reply_text_is_safe）**

Create `src/agent/escalation/holding_reply.rs`，先只写测试与函数签名占位：

```rust
//! 过渡/占位回复的 AI 生成器：独立预算旁路调 LLM + 运行期出站守卫 + 硬编码降级兜底。

use crate::agent::escalation::logic::{scene_fallback_text, HoldingReplyScene};

/// 拟发给客户的过渡文案是否安全可发：非空 + 无 no-human-takeover 禁词 +
/// （授权类场景）不含授权 substance 之外的数字事实。任一不满足即不安全，调用方回落硬编码。
pub(crate) fn holding_reply_text_is_safe(
    text: &str,
    scene: HoldingReplyScene,
    authorized_substance: Option<&str>,
) -> bool {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_is_unsafe() {
        assert!(!holding_reply_text_is_safe("   ", HoldingReplyScene::GateHold, None));
    }

    #[test]
    fn forbidden_word_is_unsafe() {
        // 含 no-human-takeover 禁词 → 不安全
        assert!(!holding_reply_text_is_safe(
            "稍等，我帮您转人工处理",
            HoldingReplyScene::GateHold,
            None
        ));
    }

    #[test]
    fn clean_text_is_safe() {
        assert!(holding_reply_text_is_safe(
            "这个我先帮您了解下，稍后同步您～",
            HoldingReplyScene::GateHold,
            None
        ));
    }

    #[test]
    fn expired_scene_rejects_unauthorized_number() {
        // 授权 substance 无数字，文案编出"8折" → 不安全
        assert!(!holding_reply_text_is_safe(
            "这边给您争取到 8 折",
            HoldingReplyScene::ExpiredAuthorization,
            Some("已确认可以帮您跟进")
        ));
    }

    #[test]
    fn expired_scene_allows_authorized_number() {
        // 文案数字在授权内 → 安全
        assert!(holding_reply_text_is_safe(
            "之前说的 9 折还在，稍等我再帮您确认下",
            HoldingReplyScene::ExpiredAuthorization,
            Some("可以给 9 折")
        ));
    }
}
```

在 `src/agent/escalation/mod.rs` 顶部模块声明区加 `mod holding_reply;`（先不 re-export 异步函数，Task 3 末尾再加）。grep 确认声明位置：`grep -n "^mod \|^pub(crate) mod \|^pub mod " src/agent/escalation/mod.rs`。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib holding_reply`
Expected: FAIL（`unimplemented!()` panic）。

- [ ] **Step 3: 实现纯函数 holding_reply_text_is_safe**

替换 `unimplemented!()`：

```rust
pub(crate) fn holding_reply_text_is_safe(
    text: &str,
    scene: HoldingReplyScene,
    authorized_substance: Option<&str>,
) -> bool {
    if text.trim().is_empty() {
        return false;
    }
    // 运行期 no-human-takeover 禁词守卫（复用 evolution lint 同款词表）。
    if !crate::evolution::lint::passes_forbidden_words(text) {
        return false;
    }
    // 授权类场景：不得编造领导授权之外的数字事实（复用 relay 数字护栏）。
    if scene == HoldingReplyScene::ExpiredAuthorization {
        if let Some(substance) = authorized_substance {
            if crate::agent::escalation::logic::relay_introduces_unauthorized_number(text, substance)
            {
                return false;
            }
        }
    }
    true
}
```

> 注意：`passes_forbidden_words` 现为 `pub fn`（lint.rs:33）；`relay_introduces_unauthorized_number` 为 `pub(crate)`（logic.rs:234）。两者可见性已足够跨模块调用，无需改。实现前 grep 复核：`grep -n "pub fn passes_forbidden_words\|pub(crate) fn relay_introduces_unauthorized_number" src/`。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --lib holding_reply`
Expected: 5 个 `holding_reply_text_is_safe` 相关测试 PASS。

- [ ] **Step 5: 实现异步生成器 generate_holding_reply**

在 `holding_reply.rs` 追加（先加所需 `use`）：

```rust
use std::sync::Arc;
use crate::agent::budget::{current_run_budget, RunBudget, RUN_BUDGET};
use crate::state::AppState;

/// 过渡/占位回复的场景化 prompt（system 段）。约束 AI 口吻、绝不提转接类措辞、
/// 短句、不复述内部字段。禁词最终由 holding_reply_text_is_safe 运行期守卫兜底。
fn holding_reply_system_prompt(scene: HoldingReplyScene) -> &'static str {
    match scene {
        HoldingReplyScene::GateHold =>
            "你是私域运营 AI。客户刚发来消息，但你此刻还不能给出最终答复（需要先核实）。\
             用你自己的口吻写一句简短、自然、真诚的过渡安抚话术，表达『已收到、正在帮你确认、稍后给准信』。\
             要求：①一句话，口语化，不客套堆砌；②绝不出现『转人工/人工/接管/客服』等字眼（你就是唯一对接人）；\
             ③不承诺具体结果/数字/时间点。只输出 JSON：{\"reply\":\"...\"}",
        HoldingReplyScene::ChainTail =>
            "你是私域运营 AI。客户的问题你已在帮他向内部核实，但还需要更多时间。\
             用你自己的口吻写一句简短、真诚、让客户安心的话，表达『还在核实、需要点时间、有结果马上同步』。\
             要求：①一句话，口语化；②绝不出现『转人工/人工/接管/客服』等字眼；③不承诺结果/数字。\
             只输出 JSON：{\"reply\":\"...\"}",
        HoldingReplyScene::ExpiredAuthorization =>
            "你是私域运营 AI。客户之前问的事你已在跟进，现在需要再确认下最新情况。\
             用你自己的口吻写一句简短中性的话，表达『会继续帮你核实最新情况、有确切消息第一时间同步』。\
             要求：①一句话，口语化；②绝不出现『转人工/人工/接管/客服』等字眼；\
             ③绝不编造任何折扣/金额/百分比等数字。只输出 JSON：{\"reply\":\"...\"}",
    }
}

/// 生成一条给客户的过渡/占位回复。
/// 独立预算旁路：用新 RunBudget scope 包住 LLM 调用，主 run 预算耗尽也能生成一次。
/// 任一失败/超时/耗尽/禁词命中/数字越界 → 回落 scene 对应硬编码文案。
/// **保证返回非空、经守卫的文案**（客户永不被晾死）。
pub(crate) async fn generate_holding_reply(
    state: &AppState,
    account_id: &str,
    contact_wxid: &str,
    scene: HoldingReplyScene,
    authorized_substance: Option<&str>,
) -> String {
    let fallback = scene_fallback_text(scene).to_string();
    // 独立小预算：仅够一次短文案生成，与主 run 隔离。
    let run_id = format!("holding-{}", uuid::Uuid::new_v4());
    let side_budget = Arc::new(RunBudget::new(
        run_id.clone(),
        state.config.holding_reply_token_budget,
        1, // 至多一次 LLM 调用
        0, // 不用工具
    ));
    let system = holding_reply_system_prompt(scene);
    let user = "请只输出 JSON。";
    let gen = async {
        // 预算已耗尽（理论上新预算不会，但保持与既有降级点一致的防御）→ 回落。
        if current_run_budget().map(|b| b.is_exceeded()).unwrap_or(false) {
            return None;
        }
        match crate::agent::generate_agent_json(
            state,
            Some(account_id),
            Some(contact_wxid),
            Some(run_id.as_str()),
            "holding.reply",
            system,
            user,
        )
        .await
        {
            Ok(value) => value
                .get("reply")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string()),
            Err(e) => {
                tracing::warn!(error = %e, scene = ?scene, "过渡回复 AI 生成失败，回落硬编码");
                None
            }
        }
    };
    let generated: Option<String> = RUN_BUDGET.scope(side_budget, gen).await;
    match generated {
        Some(text) if holding_reply_text_is_safe(&text, scene, authorized_substance) => text,
        Some(text) => {
            tracing::warn!(
                scene = ?scene,
                rejected = %text,
                "过渡回复 AI 文案未过出站守卫(禁词/数字/空)，回落硬编码"
            );
            fallback
        }
        None => fallback,
    }
}
```

> 依赖确认（实现前必做，grep 亲验）：
> 1. `state.config.holding_reply_token_budget` **可能不存在**——Task 4 负责加这个 config 字段。若本步骤编译报 `no field holding_reply_token_budget`，先做 Task 4 再回来。或本步骤先硬编码 `3000i64`，Task 4 再替换为 config 字段。**推荐：本步骤先用 `3000i64` 字面量，Task 4 引入 config 后替换**，以保持每个 Task 可独立编译通过。
> 2. `AppState` 路径：grep `grep -rn "pub struct AppState" src/` 确认 `use` 路径（可能是 `crate::AppState` 或 `crate::state::AppState`）。
> 3. `uuid` 已是依赖（reaction.rs:39 用了 `uuid::Uuid::new_v4()`）。

- [ ] **Step 6: 跑编译 + 测试**

Run: `cargo test --lib holding_reply`
Expected: 编译通过，纯函数测试仍 PASS（异步函数无新单测，靠 Task 5/6 的接入 + 纯函数守卫覆盖；异步降级链的完整验证在集成层，留 CI）。

- [ ] **Step 7: re-export + 提交**

在 `src/agent/escalation/mod.rs` 加 `pub(crate) use holding_reply::{generate_holding_reply, HoldingReplyScene 若需};`（`HoldingReplyScene` 已在 logic，按实际 import 路径调整；grep 确认 mod.rs 既有 re-export 写法：`grep -n "pub(crate) use\|pub use" src/agent/escalation/mod.rs`）。

```bash
git add src/agent/escalation/holding_reply.rs src/agent/escalation/mod.rs
git commit -m "feat(escalation): 新增 generate_holding_reply 过渡回复 AI 生成器

独立 RunBudget 旁路调 LLM 生成场景化安抚话术,经运行期禁词守卫
(复用 evolution::lint::passes_forbidden_words)+ 授权类数字护栏审查;
任一失败/耗尽/命中回落硬编码兜底,保证客户永不被晾死。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 4: 新增 holding_reply_token_budget 配置字段

**Files:**
- Modify: `src/config.rs`（`AppConfig` 加字段 + 从 env 读取 + 默认值）
- Modify: `src/agent/escalation/holding_reply.rs`（把 Task 3 的 `3000i64` 字面量替换为 `state.config.holding_reply_token_budget`）
- Modify: 所有 `AppConfig { ... }` 构造点（含 tests helper）——按 memory `config_field_add_test_helpers` 教训，加字段须补全所有字面量构造点否则 E0063
- Modify: `.env.example`（补一行文档）

**Interfaces:**
- Consumes: 既有 `AppConfig` env 读取范式（参照 `holding_reply_min_interval_hours` 如何定义——escalation/mod.rs:379 已在用 `state.config.holding_reply_min_interval_hours`，说明 config 里已有同前缀字段，照抄其定义）。
- Produces: `AppConfig.holding_reply_token_budget: i64`（默认 3000）。

- [ ] **Step 1: 定位既有 holding_reply_min_interval_hours 定义作范式**

Run: `grep -n "holding_reply_min_interval_hours" src/config.rs`
读它在 `AppConfig` 结构体的字段声明、env 解析、默认值三处写法，照抄。

- [ ] **Step 2: 加字段声明 + env 解析 + 默认值**

在 `src/config.rs` `AppConfig` 结构体加：

```rust
/// 过渡/占位回复 AI 生成的独立预算 token 上限（仅够一次短文案）。默认 3000。
pub holding_reply_token_budget: i64,
```

在 env 解析处（参照 `holding_reply_min_interval_hours` 的解析行）加：

```rust
holding_reply_token_budget: parse_env_or("HOLDING_REPLY_TOKEN_BUDGET", 3000),
```

> `parse_env_or` 是占位名——用 grep 出的**该文件实际使用的 env 读取 helper**（如 `env_var_parse` / `read_env` 等），照抄 `holding_reply_min_interval_hours` 那一行的确切函数名与写法。

- [ ] **Step 3: 补全所有 AppConfig 构造点**

Run: `grep -rn "AppConfig {" src/ tests/`
对每个字面量构造 `AppConfig { ... }`（尤其 tests helper），加 `holding_reply_token_budget: 3000,`（或测试用值）。漏一个就 E0063。

- [ ] **Step 4: 替换 holding_reply.rs 的字面量**

`src/agent/escalation/holding_reply.rs` Task 3 Step 5 里的 `3000i64` 改为 `state.config.holding_reply_token_budget`。

- [ ] **Step 5: 补 .env.example**

加一行：`# 过渡回复 AI 生成的独立预算 token 上限（默认 3000）\nHOLDING_REPLY_TOKEN_BUDGET=3000`。

- [ ] **Step 6: cargo check --tests 复刻 CI 编译门**

Run: `cargo check --tests`
Expected: 编译通过（无 E0063）。这一步专门复刻 CI baseline step2，`cargo test --lib` 不编译集成测试会漏掉 tests/ 里的构造点。

- [ ] **Step 7: 跑 lib 基线 + 提交**

Run: `cargo test --lib`
Expected: ≥ 350 passed, 0 failed。

```bash
git add src/config.rs .env.example src/agent/escalation/holding_reply.rs
git commit -m "feat(config): 加 holding_reply_token_budget(过渡回复 AI 生成独立预算,默认3000)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 5: 接入 A 类 —— ensure_customer_acknowledged 改调生成器

**Files:**
- Modify: `src/agent/gateway.rs:913-963`（`ensure_customer_acknowledged`）
- Modify: `src/agent/gateway.rs:3402-3423`（`build_ack_enqueue_request`：content 不再固定取 `fallback_holding_reply()`，改为接收传入文案）

**Interfaces:**
- Consumes: `generate_holding_reply(state, account_id, contact_wxid, HoldingReplyScene::GateHold, None) -> String`（Task 3）；`EnqueueRequest`（既有）。
- Produces: 无新公共接口；`build_ack_enqueue_request` 增加一个 `content: String` 入参（或新增重载）。

- [ ] **Step 1: 读懂现状调用链**

`ensure_customer_acknowledged`（:913）当前：`should_send_ack_placeholder` 门（:922）→ `should_abort_send` 复查（:925）→ `build_ack_enqueue_request`（:935，content 固定 `fallback_holding_reply()` 于 :3418）→ `outbox_enqueue`（:943）。改动：在 build 之前先 `generate_holding_reply` 拿文案，传给 build。

- [ ] **Step 2: 改 build_ack_enqueue_request 接收 content**

`build_ack_enqueue_request`（:3402）签名加 `content: String` 参数，把 :3418 的 `content: escalation::fallback_holding_reply().to_string()` 改为 `content`。更新该纯函数的既有单测（gateway.rs:5280 `build_ack_enqueue_request_shape` / :5301）——给它们传一个测试文案参数，断言 content 等于传入值。

- [ ] **Step 3: ensure_customer_acknowledged 先生成文案再入队**

在 `ensure_customer_acknowledged` 的 `should_abort_send` 复查通过后、`build_ack_enqueue_request` 调用前插入：

```rust
let holding_text = escalation::generate_holding_reply(
    state,
    &contact.account_id,
    &contact.wxid,
    escalation::HoldingReplyScene::GateHold,
    None,
)
.await;
```

把 `build_ack_enqueue_request(...)` 调用加上 `holding_text` 参数。

> 注意：`generate_holding_reply` 保证返回非空安全文案（内部已回落硬编码），故此处无需再判空——`ensure_customer_acknowledged` 的 fail-soft 语义（入队失败只 warn）不变。

- [ ] **Step 4: cargo check + 跑既有 ack 测试**

Run: `cargo test --lib ack_placeholder && cargo test --lib build_ack_enqueue_request`
Expected: PASS（纯函数测试适配新签名后仍绿）。

- [ ] **Step 5: 跑 lib 基线 + 提交**

Run: `cargo test --lib`
Expected: ≥ 350 passed, 0 failed。

```bash
git add src/agent/gateway.rs
git commit -m "feat(gateway): 客户回应保障占位改 AI 生成(A类闸门拦截)

ensure_customer_acknowledged 先经 generate_holding_reply 生成场景化安抚,
build_ack_enqueue_request 接收生成文案入 outbox;LLM 失败内部已回落硬编码。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 6: 接入 C 类 —— 链尾失联 + 授权过期改调生成器

**Files:**
- Modify: `src/agent/escalation/mod.rs:384-393`（C1 链尾，`scan_escalation_timeouts` 内）
- Modify: `src/agent/escalation/mod.rs:195-204`（C2 授权过期，`handle_principal_decision_relay` 内）

**Interfaces:**
- Consumes: `generate_holding_reply(state, account_id, contact_wxid, scene, authorized_substance) -> String`（Task 3）。
- Produces: 无新接口。

- [ ] **Step 1: C1 链尾改调生成器**

`escalation/mod.rs:390`，把 `serde_json::json!({... "content": chain_tail_holding_reply()})` 改为先生成：

```rust
let holding_text = holding_reply::generate_holding_reply(
    state,
    &entry.account_id,
    &entry.contact_wxid,
    logic::HoldingReplyScene::ChainTail,
    None,
)
.await;
let _ = mcp::logged_call_for_account(
    state,
    &entry.account_id,
    "message_send_text",
    serde_json::json!({
        "recipient": &entry.contact_wxid,
        "content": holding_text
    }),
)
.await;
```

> 保留裸 MCP 直发路径不变（C 类不走 outbox），仅换文案来源。去重逻辑（`last_holding_reply_ms` / `touch_last_holding_reply_ms`）原样保留。`HoldingReplyScene` / `generate_holding_reply` 的实际 import 路径按 Task 3 re-export 结果 grep 确认。

- [ ] **Step 2: C2 授权过期改调生成器**

`escalation/mod.rs:201`，把 `"content": expired_authorization_neutral_reply()` 改为：

```rust
let holding_text = holding_reply::generate_holding_reply(
    state,
    &contact.account_id,
    &contact.wxid,
    logic::HoldingReplyScene::ExpiredAuthorization,
    None, // 授权已过期,不传 substance(过期即不可用作事实源),AI 只发中性收尾
)
.await;
```

并把 MCP 调用的 `content` 改为 `holding_text`。

> 关键语义：授权过期场景 `authorized_substance` 传 `None`——因为授权已不可用，AI 不该复述任何过期数字，`holding_reply_text_is_safe` 在 substance=None 时数字守卫不生效（符合中性收尾语义：本就不该带数字，靠 prompt 约束 + 无 substance 可依）。这与既有 `relay_substance_if_usable` 过期返 None（logic.rs:120）语义一致。

- [ ] **Step 3: cargo check + lib 基线**

Run: `cargo test --lib`
Expected: 编译通过，≥ 350 passed, 0 failed。

- [ ] **Step 4: 提交**

```bash
git add src/agent/escalation/mod.rs
git commit -m "feat(escalation): 链尾失联/授权过期安抚改 AI 生成(C类)

scan_escalation_timeouts 链尾 + handle_principal_decision_relay 授权过期
两处过渡话术改调 generate_holding_reply(保留各自裸 MCP 直发路径与去重);
授权过期场景 substance 传 None(过期不可用作事实源,AI 只发中性收尾)。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 7: 收尾验证 + 死代码检查

**Files:**
- 只读验证；如有过时注释则修正。

- [ ] **Step 1: 确认三个硬编码文案仍被引用（未变死代码）**

Run: `grep -rn "fallback_holding_reply\|chain_tail_holding_reply\|expired_authorization_neutral_reply" src/`
Expected: 三者都应至少被 `scene_fallback_text`（Task 2）引用；确认无 `dead_code` 警告。若 `cargo check` 报未使用，说明 Task 2 映射未覆盖，回查。

- [ ] **Step 2: 确认 logic.rs:80-84 过时注释修正**

`fallback_holding_reply` 上方注释（logic.rs:80-84）自称"仅作回落参考，不由网关直接发送"——现状它是 AI 生成失败的降级兜底（经 scene_fallback_text）。若注释与新语义矛盾，改为："AI 生成失败/禁词命中/预算耗尽时的确定性降级兜底，经 scene_fallback_text 回落"。

- [ ] **Step 3: 全量 lib 基线 + PBT**

Run: `cargo test --lib`
Expected: ≥ 350 passed, 0 failed。

Run: `cargo test --test state_transition_pbt --test memory_card_invariants --test wiki_chunk_revision_pbt --test llm_retry_jitter`
Expected: 累计 ≥ 33 passed, 0 failed。

- [ ] **Step 4: no-human-takeover lint 自检**

Run: `bash scripts/check-no-human-takeover.sh`（或 `.ps1`）
Expected: exit 0。确认新增代码（含 prompt 里的"绝不出现转人工"这类**否定式提及**）不触 lint——注意：prompt 字符串里写"不要说『转人工』"这类会**命中** lint（它扫字面量不判语义）。若命中，改用不含禁词字面量的表述（如"你就是唯一对接人，不存在其他对接角色"），把禁词判断完全交给运行期 `passes_forbidden_words`（词表在 tests-excluded 的 lint.rs）。**这是本计划最易踩的坑，务必这步验证。**

- [ ] **Step 5: 提交任何收尾修正**

```bash
git add -p   # 只加本次收尾涉及的具名文件
git commit -m "chore(escalation): 收尾——修正过时注释 + lint 自检通过

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Self-Review 记录

- **Spec 覆盖**：修复一→Task 1；生成器三支柱（独立预算/禁词守卫/降级兜底）→Task 3；场景映射→Task 2；config→Task 4；A 类接入→Task 5；C 类接入→Task 6（B 类由 A 承担，spec 已述，无独立任务）；死代码/lint 收尾→Task 7。全覆盖。
- **占位符扫描**：Task 4 的 env helper 名（`parse_env_or`）与 Task 3 的 `AppState` 路径明确标注"grep 亲验后照抄"，因这两处依项目既有写法而定，计划已给出定位命令而非留空。
- **类型一致性**：`HoldingReplyScene` 三变体（GateHold/ChainTail/ExpiredAuthorization）在 Task 2 定义、Task 3/5/6 一致引用；`generate_holding_reply` 5 参数签名在 Task 3 定义、Task 5/6 一致调用；`scene_fallback_text` / `holding_reply_text_is_safe` 签名前后一致。
- **最大风险点**：Task 7 Step 4 的 lint 自噬（prompt 里否定式提及禁词会命中静态 lint）——已在计划中显式标注并给出规避方案。
