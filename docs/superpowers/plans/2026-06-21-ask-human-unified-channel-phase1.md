# Ask-Human 统一频道 Phase 1（后端地基）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给 ask-human 决策请示通道补齐后端地基：可配置策略（决策人链/升级范围/骚扰频率/超时）+ admin REST 处置端点 + 只读聚合器，并把配置接进推送路径让它真生效。

**Architecture:** 配置挂在现有 `OperationDomainConfig`（不新建 collection），用 `#[serde(default)]` 向后兼容；新策略 `ask_human_policy` 经纯函数 `resolve_ask_human_policy` 解析（旧字段 None 时回落、字节等价）。escalation 新增三 REST 端点复用现有 `resolve_escalation`/`enqueue_relay_task`/`sanitize_verdict` 下游。聚合器只读扇出查 8 类来源归一成 `InboxItem`，零侵入。

**Tech Stack:** Rust 2021 / Axum / MongoDB（mongodb crate）/ serde。无新依赖。

## Global Constraints

逐条 verbatim 来自 spec + CLAUDE.md，每个 task 隐含遵守：

- **红线② serde 向后兼容**：所有新字段 `#[serde(default)]`；`ask_human_policy=None` / 旧库缺字段 → 现有行为字节等价。`coreFacts` 等既有字段的反序列化不受影响。
- **红线③ AI 永不自动 verify**：聚合器对知识审核项**只读列出**；配置层**不提供**"知识免审"开关。
- **红线④ 不造双真相源**：`high_risk_escalation_mode`/`principal_decider` 旧字段保留，运行时只读 `resolve_ask_human_policy` 解析结果，旧字段仅 None 时兜底映射。
- **红线⑤ 无人工接管**：新增端点/字段命名一律 `principal`/`escalation`/`decider`/`ask_human`/`ask-human`，**绝不出现** `takeover`/`人工接管`/`转人工`/`人工介入`/`接管`/`人工`。过 `scripts/check-no-human-takeover.sh`（扫 `src/agent/`、`src/routes/` 新增行）。
- **红线⑥ 反过拟合**：所有阈值（卡死轮数/骚扰窗/超时/每日上限）可配 + 纯函数判定。
- **红线⑦ boundary_protection 不放宽**：配置层不引入任何降低安全门/grounding 的开关；`escalate_safety_guard=false` 只是"不额外请示"，安全门本身仍拦截。
- **IDOR 纪律**：所有 DB filter 含 `workspace_id`（取自 `admin.current_workspace`）。
- **测试基线不回归**：`cargo test --lib` ≥ 350/0；四 PBT 累计 ≥ 33/0。新增纯函数测试进 lib 硬门。
- **磁盘纪律**：编译前 `rm -rf target/debug/incremental` + `CARGO_INCREMENTAL=0`；本地只 `cargo test --lib` + 单 PBT，集成测试（`#[ignore]`）靠 CI。
- **提交纪律**：精确 `git add` 具名文件，**排除并行会话产物**（`.kiro/specs/universal-test-coverage/*`、`AGENTS.md`、`agent_t*.txt`、`t15_single.txt`、`tests/real_llm_*`、`tests/roleplay_*`、`docs/superpowers/plans/2026-06-18-*`）。子代理 model:opus；回复中文。

---

## 文件结构（决策锁定）

| 文件 | 责任 | 改/建 |
| --- | --- | --- |
| `src/models.rs` | `AskHumanPolicy`/`DeciderRef`/`AskHumanQuietHours` 结构 + `ask_human_policy` 字段 + `resolved_via` 字段 | 改 |
| `src/agent/escalation/policy.rs` | **新建**：`ResolvedAskHumanPolicy` + `resolve_ask_human_policy` + 骚扰门/超时纯函数（全部可单测） | 建 |
| `src/agent/escalation/mod.rs` | `escalate_held_decision` 改读 resolved policy；导出 policy 子模块 | 改 |
| `src/agent/escalation/logic.rs` | `should_escalate_held` 改签名收 `&ResolvedAskHumanPolicy` | 改 |
| `src/agent/escalation/ledger.rs` | `resolve_escalation` 写 `resolved_via`；列表/改派 DB helper | 改 |
| `src/agent/gateway.rs` | `trigger_principal_escalation` 取 decider_chain[0] + 骚扰门 | 改 |
| `src/routes/principal_escalations.rs` | **新建**：list / resolve / reassign 三 handler | 建 |
| `src/routes/domains.rs` | `put_ask_human_policy` handler | 改 |
| `src/routes/ask_human_inbox.rs` | **新建**：聚合器 inbox + summary 二 handler | 建 |
| `src/routes/mod.rs` | 注册新路由 + use | 改 |
| `src/tasks.rs` | escalation 超时转备选扫描 | 改 |
| `src/db/migrations/m025_backfill_ask_human_policy.rs` | **新建**：旧字段回填 ask_human_policy | 建 |
| `src/db/migrations/mod.rs` | 注册 m025 | 改 |
| `tests/ask_human_phase1_e2e.rs` | **新建**：集成测试（`#[ignore]`） | 建 |

---

## Block A — 配置模型 + 解析纯函数

### Task 1: AskHumanPolicy 数据模型

**Files:**
- Modify: `src/models.rs`（结构体定义加在 `pub struct OperationDomainConfig`（`src/models.rs:764`）上方；字段加在 `high_risk_escalation_mode`（`src/models.rs:804`）之后）

**Interfaces:**
- Produces: `AskHumanPolicy { decider_chain: Vec<DeciderRef>, escalate_safety_guard: bool, escalate_unverified_product: bool, escalate_ai_policy_hold: bool, escalate_stuck: bool, dedupe_window_hours: Option<f64>, daily_push_cap: Option<u32>, quiet_hours: Option<AskHumanQuietHours>, timeout_hours: Option<f64> }`；`DeciderRef { wxid: String, display_name: Option<String> }`；`AskHumanQuietHours { start_hour: u8, end_hour: u8, tz_offset_hours: i8 }`；`OperationDomainConfig.ask_human_policy: Option<AskHumanPolicy>`

- [ ] **Step 1: 加结构体定义**（插入在 `pub struct OperationDomainConfig` 上方）

```rust
/// 请示通道决策人引用：wxid + 可选展示名（前端选好友时填）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DeciderRef {
    pub wxid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

/// 请示推送静默时段（领导休息时间不推卡）。tz_offset_hours 复用运营时区偏移。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AskHumanQuietHours {
    pub start_hour: u8,
    pub end_hour: u8,
    pub tz_offset_hours: i8,
}

/// 请示通道策略。None/缺省 = 沿用旧 principal_decider/high_risk_escalation_mode 行为（红线②字节等价）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AskHumanPolicy {
    /// 有序决策人链：主 -> 备选1 -> 备选2…。空 = 未启用请示通道。
    #[serde(default)]
    pub decider_chain: Vec<DeciderRef>,
    #[serde(default = "default_true")]
    pub escalate_safety_guard: bool,
    #[serde(default = "default_true")]
    pub escalate_unverified_product: bool,
    #[serde(default)]
    pub escalate_ai_policy_hold: bool,
    #[serde(default = "default_true")]
    pub escalate_stuck: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedupe_window_hours: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily_push_cap: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quiet_hours: Option<AskHumanQuietHours>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_hours: Option<f64>,
}
```

注：`default_true` 已存在于 `src/models.rs:813`，复用，勿重复定义。

- [ ] **Step 2: 在 OperationDomainConfig 末尾加字段**（`src/models.rs:804` `high_risk_escalation_mode` 字段之后、结构体闭合 `}` 之前）

```rust
    /// 请示通道策略（决策人链/升级范围/骚扰频率/超时）。None = 回落旧 principal_decider/high_risk_escalation_mode。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask_human_policy: Option<AskHumanPolicy>,
```

- [ ] **Step 3: 同步所有 OperationDomainConfig 字面量构造点**

Run: `grep -rn "OperationDomainConfig {" src/`
Expected: 列出所有构造点。每处在 `high_risk_escalation_mode: None,` 旁加 `ask_human_policy: None,`。

- [ ] **Step 4: 编译验证**

Run: `CARGO_INCREMENTAL=0 cargo check --lib 2>&1 | tail -20`
Expected: 0 errors（missing field 报错 → 回 Step 3 补漏的构造点）。

- [ ] **Step 5: Commit**

```bash
git add src/models.rs
git commit -m "feat(ask-human): AskHumanPolicy 配置模型挂 OperationDomainConfig(serde default 向后兼容)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: resolve_ask_human_policy 解析纯函数（旧字段映射 + 字节等价）

**Files:**
- Create: `src/agent/escalation/policy.rs`
- Modify: `src/agent/escalation/mod.rs:7-11`（加 `mod policy;` + `pub(crate) use policy::*;`）

**Interfaces:**
- Consumes: `OperationDomainConfig`（Task 1 的 `ask_human_policy` + 旧 `principal_decider`/`high_risk_escalation_mode`）
- Produces: `ResolvedAskHumanPolicy { decider_chain, escalate_safety_guard, escalate_unverified_product, escalate_ai_policy_hold, escalate_stuck, dedupe_window_hours, daily_push_cap, quiet_hours, timeout_hours }`（字段类型同 `AskHumanPolicy`）；`pub(crate) fn resolve_ask_human_policy(config: &OperationDomainConfig) -> ResolvedAskHumanPolicy`

- [ ] **Step 1: 写失败测试**（新建 `src/agent/escalation/policy.rs`）

```rust
//! ask_human 策略解析（纯函数）：ask_human_policy 存在则用它；否则回落旧
//! principal_decider/high_risk_escalation_mode 字段映射（字节等价红线④）。无 IO。

use crate::models::{AskHumanPolicy, AskHumanQuietHours, DeciderRef, OperationDomainConfig};

/// 解析后的请示策略（运行时唯一权威；旧字段仅 None 时兜底）。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResolvedAskHumanPolicy {
    pub decider_chain: Vec<DeciderRef>,
    pub escalate_safety_guard: bool,
    pub escalate_unverified_product: bool,
    pub escalate_ai_policy_hold: bool,
    pub escalate_stuck: bool,
    pub dedupe_window_hours: Option<f64>,
    pub daily_push_cap: Option<u32>,
    pub quiet_hours: Option<AskHumanQuietHours>,
    pub timeout_hours: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::OperationDomainConfig;

    fn base_config() -> OperationDomainConfig {
        OperationDomainConfig {
            id: None,
            workspace_id: "ws1".into(),
            domain: "user_operations".into(),
            name: "n".into(),
            goal: "g".into(),
            methodology: "m".into(),
            workflow: "w".into(),
            tool_policy: "t".into(),
            automation_policy: "a".into(),
            review_policy: "r".into(),
            runtime_parameters: Default::default(),
            state_machine: Default::default(),
            status: "active".into(),
            updated_at: mongodb::bson::DateTime::now(),
            version: 1,
            current_version: true,
            previous_version: None,
            seeded_by: None,
            principal_decider: None,
            high_risk_escalation_mode: None,
            ask_human_policy: None,
        }
    }

    #[test]
    fn legacy_none_maps_to_decision_only_defaults() {
        let cfg = base_config();
        let r = resolve_ask_human_policy(&cfg);
        assert!(r.escalate_safety_guard);
        assert!(r.escalate_unverified_product);
        assert!(r.escalate_stuck);
        assert!(!r.escalate_ai_policy_hold);
        assert!(r.decider_chain.is_empty());
        assert_eq!(r.timeout_hours, None);
    }
}
```

- [ ] **Step 2: 跑测试确认编译失败**

Run: `CARGO_INCREMENTAL=0 cargo test --lib resolve_ask_human_policy 2>&1 | tail -15`
Expected: FAIL —— `cannot find function resolve_ask_human_policy`。

- [ ] **Step 3: 实现 resolve_ask_human_policy**（加在 `ResolvedAskHumanPolicy` 定义之后、`#[cfg(test)]` 之前）

```rust
/// 解析请示策略。优先 ask_human_policy；None 时回落旧字段映射（字节等价）。
pub(crate) fn resolve_ask_human_policy(config: &OperationDomainConfig) -> ResolvedAskHumanPolicy {
    if let Some(p) = &config.ask_human_policy {
        return ResolvedAskHumanPolicy {
            decider_chain: p.decider_chain.clone(),
            escalate_safety_guard: p.escalate_safety_guard,
            escalate_unverified_product: p.escalate_unverified_product,
            escalate_ai_policy_hold: p.escalate_ai_policy_hold,
            escalate_stuck: p.escalate_stuck,
            dedupe_window_hours: p.dedupe_window_hours,
            daily_push_cap: p.daily_push_cap,
            quiet_hours: p.quiet_hours.clone(),
            timeout_hours: p.timeout_hours,
        };
    }
    let all_mode = config.high_risk_escalation_mode.as_deref() == Some("all");
    let chain = config
        .principal_decider
        .clone()
        .map(|w| vec![DeciderRef { wxid: w, display_name: None }])
        .unwrap_or_default();
    ResolvedAskHumanPolicy {
        decider_chain: chain,
        escalate_safety_guard: true,
        escalate_unverified_product: true,
        escalate_ai_policy_hold: all_mode,
        escalate_stuck: true,
        dedupe_window_hours: None,
        daily_push_cap: None,
        quiet_hours: None,
        timeout_hours: None,
    }
}
```

- [ ] **Step 4: 补两个测试**（加进 tests mod）

```rust
    #[test]
    fn legacy_all_mode_enables_ai_policy_hold() {
        let mut cfg = base_config();
        cfg.high_risk_escalation_mode = Some("all".into());
        cfg.principal_decider = Some("boss".into());
        let r = resolve_ask_human_policy(&cfg);
        assert!(r.escalate_ai_policy_hold);
        assert_eq!(r.decider_chain.len(), 1);
        assert_eq!(r.decider_chain[0].wxid, "boss");
    }

    #[test]
    fn ask_human_policy_takes_precedence_over_legacy() {
        let mut cfg = base_config();
        cfg.high_risk_escalation_mode = Some("all".into());
        cfg.ask_human_policy = Some(AskHumanPolicy {
            decider_chain: vec![DeciderRef { wxid: "alice".into(), display_name: Some("决策人A".into()) }],
            escalate_safety_guard: true,
            escalate_unverified_product: false,
            escalate_ai_policy_hold: false,
            escalate_stuck: true,
            dedupe_window_hours: Some(6.0),
            daily_push_cap: Some(10),
            quiet_hours: None,
            timeout_hours: Some(24.0),
        });
        let r = resolve_ask_human_policy(&cfg);
        assert!(!r.escalate_unverified_product);
        assert_eq!(r.decider_chain[0].wxid, "alice");
        assert_eq!(r.timeout_hours, Some(24.0));
    }
```

- [ ] **Step 5: 接 mod.rs 子模块**（`src/agent/escalation/mod.rs:7` `mod ledger;` 区域加 `mod policy;`；`mod.rs:11` `pub(crate) use logic::*;` 下加 `pub(crate) use policy::*;`）

- [ ] **Step 6: 跑测试通过**

Run: `CARGO_INCREMENTAL=0 cargo test --lib escalation::policy 2>&1 | tail -15`
Expected: PASS（3 tests）。

- [ ] **Step 7: Commit**

```bash
git add src/agent/escalation/policy.rs src/agent/escalation/mod.rs
git commit -m "feat(ask-human): resolve_ask_human_policy 解析纯函数(旧字段映射字节等价)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Block D — 执行层纯函数（骚扰门 + 升级范围 + 超时；让配置真生效）

> 先做纯函数（本块），再在 Block B 的 task 里把它们接进推送路径。这样纯函数先进 lib 硬门、可独立验证。

### Task 3: should_escalate_held 改签名收 ResolvedAskHumanPolicy

**Files:**
- Modify: `src/agent/escalation/logic.rs:254-263`（`should_escalate_held` 函数）
- Modify: `src/agent/escalation/logic.rs:770-838`（现有 4 个 `should_escalate_held_*` 测试改用新签名）
- Modify: `src/agent/escalation/mod.rs:46-48`（`escalate_held_decision` 调用点——本 task 只改纯函数+测试，调用点接线在 Task 9）

**Interfaces:**
- Consumes: `ResolvedAskHumanPolicy`（Task 2）；`blocked_status: &str`
- Produces: `pub(crate) fn should_escalate_held(blocked_status: &str, policy: &ResolvedAskHumanPolicy) -> bool`（替换旧 `(blocked_status, mode: HighRiskEscalationMode)` 签名）

- [ ] **Step 1: 改测试到新签名**（`logic.rs:770` 起的 4 个测试，构造 `ResolvedAskHumanPolicy` 替代 `HighRiskEscalationMode`）

把 `should_escalate_held_safety_guard_unconditional` 等 4 个测试里的 `HighRiskEscalationMode::DecisionOnly` / `::All` 调用，改为传 policy。新增一个 helper 在 tests mod：

```rust
    fn policy_with(ai_policy: bool) -> crate::agent::escalation::ResolvedAskHumanPolicy {
        crate::agent::escalation::ResolvedAskHumanPolicy {
            decider_chain: vec![],
            escalate_safety_guard: true,
            escalate_unverified_product: true,
            escalate_ai_policy_hold: ai_policy,
            escalate_stuck: true,
            dedupe_window_hours: None,
            daily_push_cap: None,
            quiet_hours: None,
            timeout_hours: None,
        }
    }
```

四个测试体改为（保持原断言语义）：
```rust
    #[test]
    fn should_escalate_held_safety_guard_unconditional() {
        use crate::agent::types::HOLD_CATEGORY_BLOCKED_BY_SAFETY_GUARD;
        assert!(should_escalate_held(HOLD_CATEGORY_BLOCKED_BY_SAFETY_GUARD, &policy_with(false)));
        assert!(should_escalate_held(HOLD_CATEGORY_BLOCKED_BY_SAFETY_GUARD, &policy_with(true)));
    }

    #[test]
    fn should_escalate_held_unverified_product_unconditional() {
        assert!(should_escalate_held("blocked_unverified_product_claim", &policy_with(false)));
        assert!(should_escalate_held("blocked_unverified_product_claim", &policy_with(true)));
    }

    #[test]
    fn should_escalate_held_ai_policy_only_when_enabled() {
        use crate::agent::types::HOLD_CATEGORY_HELD_BY_AI_POLICY;
        assert!(should_escalate_held(HOLD_CATEGORY_HELD_BY_AI_POLICY, &policy_with(true)));
        assert!(!should_escalate_held(HOLD_CATEGORY_HELD_BY_AI_POLICY, &policy_with(false)));
    }

    #[test]
    fn should_escalate_held_waiting_context_never() {
        use crate::agent::types::HOLD_CATEGORY_AI_WAITING_FOR_MORE_CONTEXT;
        assert!(!should_escalate_held(HOLD_CATEGORY_AI_WAITING_FOR_MORE_CONTEXT, &policy_with(true)));
        assert!(!should_escalate_held(HOLD_CATEGORY_AI_WAITING_FOR_MORE_CONTEXT, &policy_with(false)));
    }
```
注：原 `should_escalate_held_other_terminal_states_never`（`logic.rs:824`）同样把 `HighRiskEscalationMode::All`/`DecisionOnly` 换成 `&policy_with(true)`/`&policy_with(false)`。

- [ ] **Step 2: 跑测试确认失败**

Run: `CARGO_INCREMENTAL=0 cargo test --lib should_escalate_held 2>&1 | tail -15`
Expected: FAIL（签名不匹配 / 类型错误）。

- [ ] **Step 3: 改实现**（`logic.rs:254`）

```rust
/// 被风险闸门 hold 的件是否要升级请示领导。由 ResolvedAskHumanPolicy 的逐类别开关驱动。
/// 取舍：安全门/未验证产品声明默认升级（escalate_safety_guard/escalate_unverified_product，
/// 默认 true）；ai_policy 仅 escalate_ai_policy_hold=true 时升级；其它终态（等待上下文/必填
/// 缺失/预算/context_changed）一律不升级（非决策墙）。
pub(crate) fn should_escalate_held(
    blocked_status: &str,
    policy: &crate::agent::escalation::ResolvedAskHumanPolicy,
) -> bool {
    match blocked_status {
        s if s == crate::agent::types::HOLD_CATEGORY_BLOCKED_BY_SAFETY_GUARD => {
            policy.escalate_safety_guard
        }
        "blocked_unverified_product_claim" => policy.escalate_unverified_product,
        s if s == crate::agent::types::HOLD_CATEGORY_HELD_BY_AI_POLICY => {
            policy.escalate_ai_policy_hold
        }
        _ => false,
    }
}
```

注：`HighRiskEscalationMode` 与 `parse_high_risk_mode`（`logic.rs:116-130`）**保留**（Task 2 的 resolve 回落映射不再用它，但 `build_decision_signals_text` 的 ③ 高风险提示仍可能引用；若 Task 9 移除最后引用则那时再删——本 task 不删，避免编译断裂）。

- [ ] **Step 4: 跑测试通过**

Run: `CARGO_INCREMENTAL=0 cargo test --lib should_escalate_held 2>&1 | tail -15`
Expected: PASS（5 tests）。

- [ ] **Step 5: 编译全 lib**

Run: `CARGO_INCREMENTAL=0 cargo check --lib 2>&1 | tail -20`
Expected: 可能报 `escalate_held_decision`（mod.rs:46-48）调用点签名不匹配——本 task **暂留**该调用点用 `parse_high_risk_mode` 的旧路径吗？不行，会编译错。**处理**：在 Step 3 后立即同步 `mod.rs:46-48`，把 `escalate_held_decision` 里：
```rust
    let mode =
        parse_high_risk_mode(domain_config.and_then(|c| c.high_risk_escalation_mode.as_deref()));
    if !should_escalate_held(blocked_status, mode) {
        return Ok(());
    }
```
改为：
```rust
    let policy = domain_config
        .map(crate::agent::escalation::resolve_ask_human_policy)
        .unwrap_or_else(|| crate::agent::escalation::resolve_ask_human_policy(
            &crate::agent::escalation::fallback_domain_config_for_policy()
        ));
    if !should_escalate_held(blocked_status, &policy) {
        return Ok(());
    }
```
但 `domain_config: Option<&OperationDomainConfig>`，且 `resolve_ask_human_policy` 收 `&OperationDomainConfig`。更简单：domain_config 为 None 时直接保守不升级或用 legacy 默认。改为：
```rust
    let Some(cfg) = domain_config else {
        return Ok(()); // 无 config = 请示通道未配置，不升级
    };
    let policy = crate::agent::escalation::resolve_ask_human_policy(cfg);
    if !should_escalate_held(blocked_status, &policy) {
        return Ok(());
    }
```
（这是真实接线，提前在此 task 完成 mod.rs 的最小改动以保编译通过；decider_chain 取首位在 Task 9 完成。）

Run: `CARGO_INCREMENTAL=0 cargo check --lib 2>&1 | tail -20`
Expected: 0 errors。

- [ ] **Step 6: Commit**

```bash
git add src/agent/escalation/logic.rs src/agent/escalation/mod.rs
git commit -m "feat(ask-human): should_escalate_held 改收 ResolvedAskHumanPolicy(四 escalate_* 布尔驱动)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: 骚扰门纯函数 push_allowed + next_decider_on_timeout

**Files:**
- Modify: `src/agent/escalation/policy.rs`（加两个纯函数 + 测试）

**Interfaces:**
- Consumes: `ResolvedAskHumanPolicy`；`PushHistory { today_count: u32, last_push_at: Option<DateTime>, current_decider_wxid: Option<String> }`（聚合查询结果，调用方在 Task 9 填）
- Produces: `pub(crate) fn push_allowed(policy: &ResolvedAskHumanPolicy, today_count: u32, last_push_ms: Option<i64>, now_ms: i64) -> bool`；`pub(crate) fn next_decider_on_timeout<'a>(policy: &'a ResolvedAskHumanPolicy, current_wxid: &str, age_hours: f64) -> Option<&'a DeciderRef>`；`pub(crate) fn in_quiet_hours(qh: &AskHumanQuietHours, now_ms: i64) -> bool`

- [ ] **Step 1: 写失败测试**（加进 `policy.rs` tests mod）

```rust
    fn resolved_with(daily_cap: Option<u32>, dedupe_h: Option<f64>) -> ResolvedAskHumanPolicy {
        ResolvedAskHumanPolicy {
            decider_chain: vec![],
            escalate_safety_guard: true,
            escalate_unverified_product: true,
            escalate_ai_policy_hold: false,
            escalate_stuck: true,
            dedupe_window_hours: dedupe_h,
            daily_push_cap: daily_cap,
            quiet_hours: None,
            timeout_hours: None,
        }
    }

    #[test]
    fn push_allowed_none_config_always_true() {
        // 无 cap、无 dedupe、无 quiet → 字节等价（全放行）。
        let p = resolved_with(None, None);
        assert!(push_allowed(&p, 999, Some(0), 1_000));
    }

    #[test]
    fn push_blocked_when_daily_cap_reached() {
        let p = resolved_with(Some(3), None);
        assert!(push_allowed(&p, 2, None, 1_000));   // 未达上限
        assert!(!push_allowed(&p, 3, None, 1_000));  // 达上限
    }

    #[test]
    fn push_blocked_within_dedupe_window() {
        let p = resolved_with(None, Some(6.0)); // 6h 窗
        let now = 10 * 3600 * 1000i64;
        let recent = now - 3600 * 1000; // 1h 前推过 → 窗内 → 拦
        assert!(!push_allowed(&p, 0, Some(recent), now));
        let old = now - 7 * 3600 * 1000; // 7h 前 → 超窗 → 放行
        assert!(push_allowed(&p, 0, Some(old), now));
    }

    #[test]
    fn next_decider_picks_following_after_timeout() {
        let mut p = resolved_with(None, None);
        p.timeout_hours = Some(24.0);
        p.decider_chain = vec![
            DeciderRef { wxid: "a".into(), display_name: None },
            DeciderRef { wxid: "b".into(), display_name: None },
        ];
        // 当前 a，已等 25h > 24h → 转 b
        assert_eq!(next_decider_on_timeout(&p, "a", 25.0).map(|d| d.wxid.as_str()), Some("b"));
        // 未超时 → None
        assert_eq!(next_decider_on_timeout(&p, "a", 10.0), None);
        // 已是链尾 b → None（继续等）
        assert_eq!(next_decider_on_timeout(&p, "b", 99.0), None);
    }

    #[test]
    fn next_decider_none_when_timeout_unset() {
        let mut p = resolved_with(None, None);
        p.decider_chain = vec![
            DeciderRef { wxid: "a".into(), display_name: None },
            DeciderRef { wxid: "b".into(), display_name: None },
        ];
        // timeout_hours=None → 无限等待，永不转
        assert_eq!(next_decider_on_timeout(&p, "a", 9999.0), None);
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `CARGO_INCREMENTAL=0 cargo test --lib escalation::policy 2>&1 | tail -15`
Expected: FAIL —— 函数未定义。

- [ ] **Step 3: 实现三个纯函数**（加在 `resolve_ask_human_policy` 之后）

```rust
/// 静默时段判定：now 落在 [start,end) 内（按 tz_offset 折算小时）。支持跨午夜（start>end）。
pub(crate) fn in_quiet_hours(qh: &AskHumanQuietHours, now_ms: i64) -> bool {
    let shifted = now_ms + (qh.tz_offset_hours as i64) * 3600 * 1000;
    let hour = ((shifted / (3600 * 1000)) % 24 + 24) % 24;
    let h = hour as u8;
    if qh.start_hour <= qh.end_hour {
        h >= qh.start_hour && h < qh.end_hour
    } else {
        h >= qh.start_hour || h < qh.end_hour // 跨午夜
    }
}

/// 推卡前骚扰门：daily_push_cap / dedupe_window_hours / quiet_hours 任一不满足 → false（不推）。
/// 全 None → true（字节等价，全放行）。
pub(crate) fn push_allowed(
    policy: &ResolvedAskHumanPolicy,
    today_count: u32,
    last_push_ms: Option<i64>,
    now_ms: i64,
) -> bool {
    if let Some(cap) = policy.daily_push_cap {
        if today_count >= cap {
            return false;
        }
    }
    if let (Some(window_h), Some(last)) = (policy.dedupe_window_hours, last_push_ms) {
        let elapsed_h = (now_ms - last) as f64 / (3600.0 * 1000.0);
        if elapsed_h < window_h {
            return false;
        }
    }
    if let Some(qh) = &policy.quiet_hours {
        if in_quiet_hours(qh, now_ms) {
            return false;
        }
    }
    true
}

/// 超时转备选：当前决策人在链中、已等待 age_hours 超过 timeout_hours，返回链中下一位。
/// timeout_hours=None（无限等待）/ 未超时 / 已是链尾 → None。
pub(crate) fn next_decider_on_timeout<'a>(
    policy: &'a ResolvedAskHumanPolicy,
    current_wxid: &str,
    age_hours: f64,
) -> Option<&'a DeciderRef> {
    let timeout = policy.timeout_hours?;
    if age_hours < timeout {
        return None;
    }
    let idx = policy.decider_chain.iter().position(|d| d.wxid == current_wxid)?;
    policy.decider_chain.get(idx + 1)
}
```

- [ ] **Step 4: 跑测试通过**

Run: `CARGO_INCREMENTAL=0 cargo test --lib escalation::policy 2>&1 | tail -15`
Expected: PASS（8 tests 累计）。

- [ ] **Step 5: Commit**

```bash
git add src/agent/escalation/policy.rs
git commit -m "feat(ask-human): 骚扰门 push_allowed + 超时 next_decider_on_timeout 纯函数

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Block B — escalation REST 端点 + 超时扫描

### Task 5: AgentPrincipalEscalation 加 resolved_via 审计字段 + resolve_escalation 写入

**Files:**
- Modify: `src/models.rs:2938`（`AgentPrincipalEscalation` 在 `resolved_at` 字段旁加 `resolved_via`）
- Modify: `src/agent/escalation/ledger.rs:153-182`（`resolve_escalation` 加 `resolved_via` 参数并写入）
- Modify: `src/agent/escalation/mod.rs`（`handle_principal_reply` 调 `resolve_escalation` 处 `ledger.rs` 内 `:285` 传 `"wechat"`）

**Interfaces:**
- Produces: `AgentPrincipalEscalation.resolved_via: Option<String>`（`"admin"` / `"wechat"`）；`resolve_escalation(state, short_code, decision, authorization_expires_at, resolved_via: &str) -> AppResult<Option<AgentPrincipalEscalation>>`

- [ ] **Step 1: 加字段**（`src/models.rs:2939` `resolved_at` 之后）

```rust
    /// 裁决来源审计："wechat"（领导微信回复）/ "admin"（管理员在后台直接裁决）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_via: Option<String>,
```

- [ ] **Step 2: 同步 ledger.rs 构造点**（`ledger.rs:57-75` `insert_pending_escalation` 的字面量加 `resolved_via: None,`）+ `logic.rs:375` 测试 helper `make_pending` 加 `resolved_via: None,`

- [ ] **Step 3: resolve_escalation 加参数写入**（`ledger.rs:153`）

签名改：
```rust
pub(crate) async fn resolve_escalation(
    state: &AppState,
    short_code: &str,
    decision: &PrincipalDecision,
    authorization_expires_at: Option<DateTime>,
    resolved_via: &str,
) -> AppResult<Option<AgentPrincipalEscalation>> {
```
`set` doc 加（`ledger.rs:161-166` 的 `doc!` 内）：
```rust
        "resolved_via": resolved_via,
```

- [ ] **Step 4: 同步现有调用点**（`mod.rs` 的 `handle_principal_reply` 在 `ledger.rs:285` 调 `resolve_escalation(state, &short_code, &decision, expires)`）

Run: `grep -rn "resolve_escalation(" src/`
Expected: 找到调用点（`handle_principal_reply` 内）。该处加末参 `"wechat"`。

- [ ] **Step 5: 编译**

Run: `CARGO_INCREMENTAL=0 cargo check --lib 2>&1 | tail -20`
Expected: 0 errors。

- [ ] **Step 6: Commit**

```bash
git add src/models.rs src/agent/escalation/ledger.rs src/agent/escalation/mod.rs src/agent/escalation/logic.rs
git commit -m "feat(ask-human): AgentPrincipalEscalation.resolved_via 审计 + resolve_escalation 写来源

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 6: 列表 / 改派 DB helper（ledger 层）

**Files:**
- Modify: `src/agent/escalation/ledger.rs`（加 `list_escalations_by_workspace`、`reassign_escalation`、`count_pushes_today`）

**Interfaces:**
- Produces:
  - `list_escalations_by_workspace(state, workspace_id: &str, status: &str) -> AppResult<Vec<AgentPrincipalEscalation>>`
  - `reassign_escalation(state, workspace_id: &str, short_code: &str, to_wxid: &str) -> AppResult<Option<AgentPrincipalEscalation>>`
  - `count_pushes_today(state, workspace_id: &str, principal_wxid: &str, since_ms: i64) -> AppResult<u32>`（Task 9 骚扰门用）

- [ ] **Step 1: 实现三 helper**（加在 `ledger.rs` 末尾，`enqueue_relay_task` 之后）

```rust
/// 按 workspace + status 列请示台账（admin 收件箱/SLA 看板用），created_at 升序。
pub(crate) async fn list_escalations_by_workspace(
    state: &AppState,
    workspace_id: &str,
    status: &str,
) -> AppResult<Vec<AgentPrincipalEscalation>> {
    use futures::TryStreamExt;
    let cursor = state
        .db
        .agent_principal_escalations()
        .find(
            doc! { "workspace_id": workspace_id, "status": status },
            mongodb::options::FindOptions::builder()
                .sort(doc! { "created_at": 1 })
                .build(),
        )
        .await?;
    Ok(cursor.try_collect().await?)
}

/// 改派 pending 请示到另一位决策人（仅 pending 可改派；workspace 约束防 IDOR）。
pub(crate) async fn reassign_escalation(
    state: &AppState,
    workspace_id: &str,
    short_code: &str,
    to_wxid: &str,
) -> AppResult<Option<AgentPrincipalEscalation>> {
    let updated = state
        .db
        .agent_principal_escalations()
        .find_one_and_update(
            doc! {
                "workspace_id": workspace_id,
                "short_code": short_code,
                "status": PRINCIPAL_ESCALATION_STATUS_PENDING,
            },
            doc! { "$set": { "principal_wxid": to_wxid, "updated_at": DateTime::now() } },
            mongodb::options::FindOneAndUpdateOptions::builder()
                .return_document(mongodb::options::ReturnDocument::After)
                .build(),
        )
        .await?;
    Ok(updated)
}

/// 统计某决策人当日（since_ms 起）已被推送的请示卡数（骚扰门 daily_push_cap 用）。
/// 以 pending 台账 created_at 作为推送时刻近似（每条 pending = 一次推卡）。
pub(crate) async fn count_pushes_today(
    state: &AppState,
    workspace_id: &str,
    principal_wxid: &str,
    since_ms: i64,
) -> AppResult<u32> {
    let count = state
        .db
        .agent_principal_escalations()
        .count_documents(
            doc! {
                "workspace_id": workspace_id,
                "principal_wxid": principal_wxid,
                "created_at": { "$gte": DateTime::from_millis(since_ms) },
            },
            None,
        )
        .await?;
    Ok(count as u32)
}
```

- [ ] **Step 2: 编译**

Run: `CARGO_INCREMENTAL=0 cargo check --lib 2>&1 | tail -20`
Expected: 0 errors（未用 helper 可能 dead_code warning；下一 task 接线后消除——若 `-Dwarnings` 卡，临时加 `#[allow(dead_code)]`，Task 7/9 用上后移除）。

- [ ] **Step 3: Commit**

```bash
git add src/agent/escalation/ledger.rs
git commit -m "feat(ask-human): ledger 列表/改派/当日推送计数 helper

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 7: escalation 三 REST handler（list / resolve / reassign）

**Files:**
- Create: `src/routes/principal_escalations.rs`
- Modify: `src/routes/mod.rs`（use + 三 route，挂在 `:733` `admin/operation-domains` 路由组附近）

**Interfaces:**
- Consumes: `list_escalations_by_workspace` / `reassign_escalation`（Task 6）；`resolve_escalation`（Task 5 新签名）；`enqueue_relay_task`、`sanitize_verdict`、`resolve_ask_human_policy`
- Produces: 三 handler `list_principal_escalations` / `resolve_principal_escalation` / `reassign_principal_escalation`

- [ ] **Step 1: 写 handler 文件**（参照 `src/routes/domains.rs:155` 的 handler 惯例：`State(state)` + `Extension(admin): Extension<AuthenticatedAdmin>` + `Path`/`Json`，返回 `AppResult<Json<Value>>`）

```rust
//! 决策请示通道 admin REST 端点：列表 / admin 直接裁决 / 改派。
//! admin 在此是"幕后决策人"（真人决策），客户仍只收 AI 口吻转述（relay 下游不变）。

use axum::extract::{Path, Query, State};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::agent::escalation::{
    enqueue_relay_task, list_escalations_by_workspace, reassign_escalation, resolve_ask_human_policy,
    resolve_escalation, sanitize_verdict,
};
use crate::auth::AuthenticatedAdmin;
use crate::error::{AppError, AppResult};
use crate::models::PrincipalDecision;
use crate::routes::AppState;
use mongodb::bson::DateTime;

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub status: Option<String>,
}

/// GET /api/admin/principal-escalations?status=pending|resolved
pub async fn list_principal_escalations(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(q): Query<ListQuery>,
) -> AppResult<Json<Value>> {
    let status = q.status.as_deref().unwrap_or("pending");
    if status != "pending" && status != "resolved" {
        return Err(AppError::BadRequest("status 只能是 pending|resolved".into()));
    }
    let items = list_escalations_by_workspace(&state, &admin.current_workspace, status).await?;
    let now = DateTime::now().timestamp_millis();
    let json_items: Vec<Value> = items
        .iter()
        .map(|e| {
            let age_hours =
                (now - e.created_at.timestamp_millis()) as f64 / (3600.0 * 1000.0);
            json!({
                "shortCode": e.short_code,
                "contactWxid": e.contact_wxid,
                "category": e.category,
                "reason": e.reason,
                "questionForPrincipal": e.question_for_principal,
                "principalWxid": e.principal_wxid,
                "status": e.status,
                "ageHours": age_hours,
                "createdAt": e.created_at,
                "decision": e.decision,
                "authorizationExpiresAt": e.authorization_expires_at,
                "resolvedVia": e.resolved_via,
            })
        })
        .collect();
    Ok(Json(json!({ "items": json_items })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveBody {
    pub verdict: String,
    #[serde(default)]
    pub substance: String,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default)]
    pub authorization_window_hours: Option<f64>,
}

/// POST /api/admin/principal-escalations/:short_code/resolve
/// admin 结构化裁决 → 复用 relay 下游（跳过 LLM interpret）。
pub async fn resolve_principal_escalation(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(short_code): Path<String>,
    Json(body): Json<ResolveBody>,
) -> AppResult<Json<Value>> {
    // 先确认该条属于本 workspace 且 pending（IDOR + 幂等）。
    let pending = list_escalations_by_workspace(&state, &admin.current_workspace, "pending").await?;
    let Some(entry) = pending.into_iter().find(|e| e.short_code == short_code) else {
        // 不在本 workspace pending 列表：可能已 resolved（幂等）或越权 → 幂等成功避免泄漏存在性。
        return Ok(Json(json!({ "ok": true, "alreadyResolved": true })));
    };
    let decision = sanitize_verdict(PrincipalDecision {
        verdict: body.verdict,
        substance: body.substance,
        constraints: body.constraints,
        authorization_window_hours: body.authorization_window_hours,
    });
    let expires = decision.authorization_window_hours.and_then(|hours| {
        if hours > 0.0 {
            Some(DateTime::from_millis(
                DateTime::now().timestamp_millis() + (hours * 3600.0 * 1000.0) as i64,
            ))
        } else {
            None
        }
    });
    let resolved = resolve_escalation(&state, &short_code, &decision, expires, "admin").await?;
    if resolved.is_none() {
        return Ok(Json(json!({ "ok": true, "alreadyResolved": true })));
    }
    // deferred 不转述（领导/admin 暂缓）；其余起 relay task 用 AI 口吻转述客户。
    if decision.verdict != crate::models::PRINCIPAL_VERDICT_DEFERRED {
        enqueue_relay_task(&state, &entry).await?;
    }
    Ok(Json(json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReassignBody {
    pub to_wxid: String,
}

/// POST /api/admin/principal-escalations/:short_code/reassign
pub async fn reassign_principal_escalation(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(short_code): Path<String>,
    Json(body): Json<ReassignBody>,
) -> AppResult<Json<Value>> {
    // 校验 to_wxid 在 decider_chain 内（取 current_version config 解析）。
    let cfg = state
        .db
        .operation_domain_configs()
        .find_one(
            mongodb::bson::doc! {
                "workspace_id": &admin.current_workspace,
                "domain": "user_operations",
                "current_version": true,
            },
            None,
        )
        .await?;
    let in_chain = cfg
        .as_ref()
        .map(|c| {
            let p = resolve_ask_human_policy(c);
            p.decider_chain.iter().any(|d| d.wxid == body.to_wxid)
        })
        .unwrap_or(false);
    if !in_chain {
        return Err(AppError::BadRequest(
            "to_wxid 不在该 workspace 的决策人链内".into(),
        ));
    }
    let updated =
        reassign_escalation(&state, &admin.current_workspace, &short_code, &body.to_wxid).await?;
    if updated.is_none() {
        return Err(AppError::NotFound("无此 pending 请示或已处置".into()));
    }
    Ok(Json(json!({ "ok": true })))
}
```

注：若 `enqueue_relay_task`/`sanitize_verdict`/`list_escalations_by_workspace` 等当前是 `pub(crate)`，本文件在 `crate::routes` 下属同 crate，可见性 OK。`AuthenticatedAdmin` 导入路径以 `domains.rs` 实际 use 为准（`grep -n "AuthenticatedAdmin" src/routes/domains.rs` 确认）。

- [ ] **Step 2: 注册路由 + 模块**（`src/routes/mod.rs`）

`mod` 声明区加：`mod principal_escalations;`
use 区加：
```rust
use principal_escalations::{
    list_principal_escalations, reassign_principal_escalation, resolve_principal_escalation,
};
```
router（`:733` 附近 admin 组）加：
```rust
        .route("/admin/principal-escalations", get(list_principal_escalations))
        .route(
            "/admin/principal-escalations/:short_code/resolve",
            post(resolve_principal_escalation),
        )
        .route(
            "/admin/principal-escalations/:short_code/reassign",
            post(reassign_principal_escalation),
        )
```

- [ ] **Step 3: no-takeover lint 自检**

Run: `bash scripts/check-no-human-takeover.sh 2>&1 | tail -10`
Expected: clean（新增行无禁词；命名全 principal/escalation/decider）。

- [ ] **Step 4: 编译**

Run: `CARGO_INCREMENTAL=0 cargo check --lib 2>&1 | tail -20`
Expected: 0 errors。

- [ ] **Step 5: Commit**

```bash
git add src/routes/principal_escalations.rs src/routes/mod.rs
git commit -m "feat(ask-human): escalation 三 REST 端点(list/admin-resolve 复用 relay/reassign)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 8: 配置写入端点 PUT ask-human-policy

**Files:**
- Modify: `src/routes/domains.rs`（加 `put_ask_human_policy` handler，仿 `update_operation_domain_state_machine:155`）
- Modify: `src/routes/mod.rs`（route + use）

**Interfaces:**
- Consumes: `AskHumanPolicy`（Task 1）
- Produces: handler `put_ask_human_policy`

- [ ] **Step 1: 写 handler**（加在 `domains.rs` `update_operation_domain_state_machine` 之后）

```rust
/// PUT /api/admin/operation-domains/:domain/ask-human-policy
/// $set ask_human_policy 到 current_version 行（不 bump 版本，贴生产 admin 编辑语义）。
pub async fn put_ask_human_policy(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(domain): Path<String>,
    Json(policy): Json<crate::models::AskHumanPolicy>,
) -> AppResult<Json<Value>> {
    // 校验：decider_chain wxid 非空；quiet_hours 小时范围。
    for d in &policy.decider_chain {
        if d.wxid.trim().is_empty() {
            return Err(AppError::BadRequest("decider_chain wxid 不能为空".into()));
        }
    }
    if let Some(qh) = &policy.quiet_hours {
        if qh.start_hour > 23 || qh.end_hour > 23 {
            return Err(AppError::BadRequest("quiet_hours 小时须 0-23".into()));
        }
    }
    let policy_bson = mongodb::bson::to_bson(&policy)?;
    let res = state
        .db
        .operation_domain_configs()
        .update_one(
            doc! {
                "workspace_id": &admin.current_workspace,
                "domain": &domain,
                "current_version": true,
            },
            doc! { "$set": { "ask_human_policy": policy_bson, "updated_at": DateTime::now() } },
            None,
        )
        .await?;
    if res.matched_count == 0 {
        return Err(AppError::NotFound("operation domain 当前版本不存在".into()));
    }
    Ok(Json(json!({ "ok": true })))
}
```

注：`Value`/`json!`/`doc!`/`DateTime`/`AppError` 在 `domains.rs` 已 use（确认 `grep -n "use " src/routes/domains.rs | grep -E "json|DateTime|AppError"`）。

- [ ] **Step 2: 注册路由**（`src/routes/mod.rs` `:626` `operation-domains/:domain` 路由组加）

```rust
        .route(
            "/admin/operation-domains/:domain/ask-human-policy",
            axum::routing::put(domains::put_ask_human_policy),
        )
```
（若 `put` 已在 use 列表则直接 `put(...)`；否则用全路径 `axum::routing::put`。）

- [ ] **Step 3: lint + 编译**

Run: `bash scripts/check-no-human-takeover.sh 2>&1 | tail -5 && CARGO_INCREMENTAL=0 cargo check --lib 2>&1 | tail -20`
Expected: lint clean；0 errors。

- [ ] **Step 4: Commit**

```bash
git add src/routes/domains.rs src/routes/mod.rs
git commit -m "feat(ask-human): PUT ask-human-policy 写入端点($set current 行不 bump 版本)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 9: 推送路径接线（decider_chain[0] + 骚扰门）

**Files:**
- Modify: `src/agent/escalation/mod.rs:38-136`（`escalate_held_decision` 取 chain[0] + 骚扰门）
- Modify: `src/agent/gateway.rs:485+`（`trigger_principal_escalation` 取 chain[0] + 骚扰门）

**Interfaces:**
- Consumes: `resolve_ask_human_policy`、`push_allowed`、`count_pushes_today`（前序 task）

- [ ] **Step 1: escalate_held_decision 取 chain[0] + 骚扰门**（`mod.rs:51-56` 当前 `principal_decider_wxid(...)` 取单 wxid 处）

替换：
```rust
    let Some(principal_wxid) =
        principal_decider_wxid(state, &contact.workspace_id, super::domain::USER_OPS_DOMAIN_ID)
            .await?
    else {
        return Ok(());
    };
```
为（`cfg` 已在 Task 3 Step5 取得 `let Some(cfg) = domain_config else {...}` + `let policy = resolve_ask_human_policy(cfg);`）：
```rust
    let Some(decider) = policy.decider_chain.first() else {
        return Ok(()); // 决策人链空 = 未启用请示通道
    };
    let principal_wxid = decider.wxid.clone();
    if principal_wxid == contact.wxid {
        return Err(AppError::BadRequest(
            "principal_decider 配置等于客户 wxid，拒绝触发请示".into(),
        ));
    }
    // 骚扰门：daily_push_cap / dedupe_window / quiet_hours。
    let now_ms = mongodb::bson::DateTime::now().timestamp_millis();
    let since_ms = now_ms - 24 * 3600 * 1000;
    let today = count_pushes_today(state, &contact.workspace_id, &principal_wxid, since_ms).await?;
    if !crate::agent::escalation::push_allowed(&policy, today, None, now_ms) {
        return Ok(()); // 骚扰门关：跳过推卡（台账后续可由 admin 处置）
    }
```
注：原 `mod.rs:57-61` 的 `if principal_wxid == contact.wxid` 校验已并入上面，删除重复段。`last_push_ms` 此处传 None（dedupe 主要靠现有 pending 去重 + daily_cap；dedupe_window 的精确 last-push 查询留 Task 留待 Phase 优化，None 时 `push_allowed` 的 dedupe 分支不触发，字节等价）。

- [ ] **Step 2: trigger_principal_escalation 同样接线**（`gateway.rs:485+`）

`gateway.rs:494` 当前 `escalation::principal_decider_wxid(...)`。同 Step 1 改为取 `resolve_ask_human_policy(cfg).decider_chain.first()` + 骚扰门。需先在该函数拿到 `cfg`——查 `trigger_principal_escalation` 签名是否已有 domain_config 参；若无，从 `state.db.operation_domain_configs().find_one(current_version)` 取。

Run: `grep -n "fn trigger_principal_escalation" src/agent/gateway.rs`
然后读该函数体确认 cfg 来源，按 Step1 模式接线。

- [ ] **Step 3: lint + 编译 + 全 lib 测试**

Run: `bash scripts/check-no-human-takeover.sh 2>&1 | tail -5 && CARGO_INCREMENTAL=0 cargo test --lib 2>&1 | tail -15`
Expected: lint clean；lib ≥ 350/0（含新增纯函数测试）。

- [ ] **Step 4: Commit**

```bash
git add src/agent/escalation/mod.rs src/agent/gateway.rs
git commit -m "feat(ask-human): 推送路径接 decider_chain[0] + 骚扰门(push_allowed/daily_cap)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 10: 超时转备选扫描（task worker）

**Files:**
- Modify: `src/tasks.rs:164` 附近（`ensure_today_outcome_aggregation_tasks` 调用处加超时扫描调用）
- Create helper in `src/agent/escalation/ledger.rs` 或 `mod.rs`: `scan_escalation_timeouts(state) -> AppResult<()>`

**Interfaces:**
- Consumes: `next_decider_on_timeout`（Task 4）、`reassign_escalation`（Task 6）、`render_principal_card`、`logged_call_for_account`

- [ ] **Step 1: 实现扫描函数**（加在 `src/agent/escalation/mod.rs`）

```rust
/// 超时转备选：扫所有 pending 请示，age > timeout_hours 且当前决策人非链尾 → 改派下一位 + 重推卡。
/// AI 绝不替决策人拍板——只把请示转给链上下一位真人。timeout=None → 无限等待，不动。
pub(crate) async fn scan_escalation_timeouts(state: &AppState) -> AppResult<()> {
    use futures::TryStreamExt;
    let now_ms = DateTime::now().timestamp_millis();
    // 取所有 current_version config，建 workspace+domain → resolved policy 映射。
    let configs: Vec<OperationDomainConfig> = state
        .db
        .operation_domain_configs()
        .find(doc! { "current_version": true }, None)
        .await?
        .try_collect()
        .await?;
    for cfg in &configs {
        let policy = resolve_ask_human_policy(cfg);
        if policy.timeout_hours.is_none() {
            continue;
        }
        let pending = list_escalations_by_workspace(state, &cfg.workspace_id, "pending").await?;
        for entry in pending {
            let age_hours =
                (now_ms - entry.created_at.timestamp_millis()) as f64 / (3600.0 * 1000.0);
            let Some(next) = next_decider_on_timeout(&policy, &entry.principal_wxid, age_hours)
            else {
                continue;
            };
            let next_wxid = next.wxid.clone();
            if reassign_escalation(state, &cfg.workspace_id, &entry.short_code, &next_wxid)
                .await?
                .is_some()
            {
                let label = entry.contact_wxid.clone();
                let card = render_principal_card(
                    &entry.short_code,
                    &label,
                    &entry.reason,
                    &entry.question_for_principal,
                );
                let _ = mcp::logged_call_for_account(
                    state,
                    &entry.account_id,
                    "message_send_text",
                    serde_json::json!({ "recipient": next_wxid, "content": card }),
                )
                .await; // 推卡失败降级，不阻断扫描
            }
        }
    }
    Ok(())
}
```
注：`OperationDomainConfig` 需 import 进 mod.rs（已有 `use crate::models::{...}`，加该类型）。

- [ ] **Step 2: task worker 调用**（`src/tasks.rs:164` `let _ = ensure_today_outcome_aggregation_tasks(state).await;` 旁加）

```rust
    let _ = crate::agent::escalation::scan_escalation_timeouts(state).await;
```

- [ ] **Step 3: lint + 编译**

Run: `bash scripts/check-no-human-takeover.sh 2>&1 | tail -5 && CARGO_INCREMENTAL=0 cargo check --lib 2>&1 | tail -20`
Expected: lint clean；0 errors。

- [ ] **Step 4: Commit**

```bash
git add src/agent/escalation/mod.rs src/tasks.rs
git commit -m "feat(ask-human): 超时转备选扫描接 task worker(age>timeout 改派链上下一位真人)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Block C — 只读聚合器

> 来源 collection 精确形状（实证）：
> - 关系类型建议：`collection_relationship_type_suggestions()` → `RelationshipTypeSuggestion`（`status="pending"`，无 created_at，用 `last_seen_at`；title 用 `suggested_value`）
> - 知识缺口：`knowledge_gap_signals()` → `KnowledgeGapSignal`（`status="pending"`，`title`、`created_at`）
> - profile 草稿：`domain_profiles()` → `DomainProfile`（待激活 = `current_version=true && is_active=false`；title 用 `display_name`、`created_at`）
> - 进化候选：`proposals()` → `Proposal`（`status="eligible_for_release"`，title 用 `diff_summary`/`proposal_kind`、`created_at`）
> - lessons_learned：**无 typed accessor/struct**，走 `state.db.raw().collection::<Document>("lessons_learned")`（`review_status="pending_review"`，title 用 `pattern_kind`、`created_at`）
> - 知识切片：`operation_knowledge_chunks()`（`integrity_status="needs_review"`，title 用 `title`）
> - 标签候选：`taxonomy_candidates()`（`review_status="pending"`）
> - 请示通道：`agent_principal_escalations()`（`status="pending"`）

### Task 11: InboxItem 聚合器（per-source 降级）

**Files:**
- Create: `src/routes/ask_human_inbox.rs`
- Modify: `src/routes/mod.rs`（use + 二 route）

**Interfaces:**
- Produces: handler `ask_human_inbox` / `ask_human_summary`；内部 `struct InboxItem`（serde Serialize）

- [ ] **Step 1: 写聚合器骨架 + 两个最简单 source（请示 + 知识切片）**

先实现 `InboxItem` + summary + inbox 的请示/知识两 source，跑通端到端，再逐 source 加（每 source 一个独立 fn，失败降级）。

```rust
//! ask-human 只读聚合器：扇出查各待审来源，归一成统一 InboxItem。
//! 每 source 独立查询，失败标 error 不整体崩。零侵入（不动任何写路径）。

use axum::extract::{Query, State};
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::auth::AuthenticatedAdmin;
use crate::error::AppResult;
use crate::routes::AppState;
use mongodb::bson::{doc, DateTime, Document};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxItem {
    pub source: String,
    pub id: String,
    pub title: String,
    pub summary: String,
    pub severity: String,
    pub created_at: Option<DateTime>,
    pub age_hours: f64,
    pub action_kind: String, // "inline" | "rich"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rich_component: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rich_params: Option<Document>,
}

fn age_hours_of(created: Option<DateTime>, now_ms: i64) -> f64 {
    created
        .map(|c| (now_ms - c.timestamp_millis()) as f64 / (3600.0 * 1000.0))
        .unwrap_or(0.0)
}

/// 请示通道 pending → InboxItem（inline）。
async fn collect_escalations(
    state: &AppState,
    ws: &str,
    now_ms: i64,
) -> AppResult<Vec<InboxItem>> {
    let items =
        crate::agent::escalation::list_escalations_by_workspace(state, ws, "pending").await?;
    Ok(items
        .into_iter()
        .map(|e| InboxItem {
            source: "principal_escalation".into(),
            id: e.short_code.clone(),
            title: format!("请示 #{}", e.short_code),
            summary: e.reason.clone(),
            severity: "high".into(),
            created_at: Some(e.created_at),
            age_hours: age_hours_of(Some(e.created_at), now_ms),
            action_kind: "inline".into(),
            rich_component: None,
            rich_params: None,
        })
        .collect())
}

/// 知识切片 needs_review → InboxItem（rich：在统一频道内挂知识核验组件）。
async fn collect_knowledge_review(
    state: &AppState,
    ws: &str,
    now_ms: i64,
) -> AppResult<Vec<InboxItem>> {
    use futures::TryStreamExt;
    let cursor = state
        .db
        .operation_knowledge_chunks()
        .find(
            doc! { "workspace_id": ws, "integrity_status": "needs_review" },
            mongodb::options::FindOptions::builder().limit(100).build(),
        )
        .await?;
    let chunks: Vec<crate::models::OperationKnowledgeChunk> = cursor.try_collect().await?;
    Ok(chunks
        .into_iter()
        .map(|c| {
            let id = c.id.map(|o| o.to_hex()).unwrap_or_default();
            InboxItem {
                source: "knowledge_review".into(),
                id: id.clone(),
                title: c.title.clone(),
                summary: c.body.clone().unwrap_or_default().chars().take(80).collect(),
                severity: "medium".into(),
                created_at: None,
                age_hours: 0.0,
                action_kind: "rich".into(),
                rich_component: Some("knowledgeReview".into()),
                rich_params: Some(doc! { "chunkId": id }),
            }
        })
        .collect())
}

#[derive(Debug, Deserialize)]
pub struct InboxQuery {
    #[serde(default)]
    pub source: Option<String>,
}

/// GET /api/admin/ask-human/inbox?source=<filter>
pub async fn ask_human_inbox(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(q): Query<InboxQuery>,
) -> AppResult<Json<Value>> {
    let ws = &admin.current_workspace;
    let now_ms = DateTime::now().timestamp_millis();
    let mut items: Vec<InboxItem> = Vec::new();
    let mut errors: Vec<Value> = Vec::new();

    // 每 source 独立降级：Err 不整体崩，记进 errors 数组。
    macro_rules! collect_source {
        ($name:expr, $fut:expr) => {
            if q.source.as_deref().map(|s| s == $name).unwrap_or(true) {
                match $fut.await {
                    Ok(mut v) => items.append(&mut v),
                    Err(e) => errors.push(json!({ "source": $name, "error": e.to_string() })),
                }
            }
        };
    }

    collect_source!("principal_escalation", collect_escalations(&state, ws, now_ms));
    collect_source!("knowledge_review", collect_knowledge_review(&state, ws, now_ms));
    // Task 12 在此追加其余 source。

    Ok(Json(json!({ "items": items, "errors": errors })))
}

/// GET /api/admin/ask-human/summary —— 各 source pending 计数。
pub async fn ask_human_summary(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
) -> AppResult<Json<Value>> {
    let ws = &admin.current_workspace;
    let escalations = state
        .db
        .agent_principal_escalations()
        .count_documents(doc! { "workspace_id": ws, "status": "pending" }, None)
        .await
        .unwrap_or(0);
    let knowledge = state
        .db
        .operation_knowledge_chunks()
        .count_documents(doc! { "workspace_id": ws, "integrity_status": "needs_review" }, None)
        .await
        .unwrap_or(0);
    // Task 12 追加其余 source 计数。
    Ok(Json(json!({
        "principalEscalation": escalations,
        "knowledgeReview": knowledge,
    })))
}
```

- [ ] **Step 2: 注册路由**（`src/routes/mod.rs`）

`mod ask_human_inbox;` + use：
```rust
use ask_human_inbox::{ask_human_inbox, ask_human_summary};
```
router（admin 组）：
```rust
        .route("/admin/ask-human/inbox", get(ask_human_inbox))
        .route("/admin/ask-human/summary", get(ask_human_summary))
```

- [ ] **Step 3: lint + 编译**

Run: `bash scripts/check-no-human-takeover.sh 2>&1 | tail -5 && CARGO_INCREMENTAL=0 cargo check --lib 2>&1 | tail -20`
Expected: lint clean；0 errors。

- [ ] **Step 4: Commit**

```bash
git add src/routes/ask_human_inbox.rs src/routes/mod.rs
git commit -m "feat(ask-human): 只读聚合器 inbox/summary 骨架(请示+知识两 source,per-source 降级)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 12: 补齐其余 6 个聚合 source

**Files:**
- Modify: `src/routes/ask_human_inbox.rs`（加 6 个 `collect_*` fn + 接进 inbox/summary）

**Interfaces:**
- Consumes: `taxonomy_candidates()`、`collection_relationship_type_suggestions()`、`knowledge_gap_signals()`、`domain_profiles()`、`proposals()`、`raw().collection("lessons_learned")`

- [ ] **Step 1: 加 6 个 collect fn**（每个独立，失败由 inbox 的 macro 降级）

```rust
/// 标签候选 pending → inline。
async fn collect_taxonomy_candidates(state: &AppState, ws: &str, now_ms: i64) -> AppResult<Vec<InboxItem>> {
    use futures::TryStreamExt;
    let cursor = state.db.collection_taxonomy_candidates()
        .find(doc! { "workspace_id": ws, "status": "pending" },
              mongodb::options::FindOptions::builder().limit(100).build()).await?;
    let rows: Vec<crate::models::TaxonomyCandidate> = cursor.try_collect().await?;
    Ok(rows.into_iter().map(|c| {
        let id = c.id.map(|o| o.to_hex()).unwrap_or_default();
        InboxItem {
            source: "taxonomy_candidate".into(),
            id,
            title: format!("标签候选：{}", c.kind),
            summary: c.raw_value.clone(),
            severity: "low".into(),
            created_at: Some(c.last_seen_at),
            age_hours: age_hours_of(Some(c.last_seen_at), now_ms),
            action_kind: "inline".into(),
            rich_component: None, rich_params: None,
        }
    }).collect())
}
```
注：`TaxonomyCandidate`（`models.rs:2422`）字段实证：`kind`/`raw_value`/`status="pending"`/`last_seen_at`（无 created_at）/`scope`。accessor 是 `collection_taxonomy_candidates()`（`db/mod.rs:244`，非 `taxonomy_candidates()`）。**注意**：`TaxonomyCandidate` 是否有 `workspace_id` 字段需 `grep -n "workspace_id" -A1 src/models.rs | grep -A1 2422` 确认；若该 struct 用 `scope` 而非 `workspace_id` 做隔离键，filter 改用 `scope`（按实证调整，IDOR 纪律仍要求按当前 workspace/account scope 过滤）。其余 5 个同理：

```rust
/// 关系类型建议 pending → inline。无 created_at，用 last_seen_at。
async fn collect_relationship_suggestions(state: &AppState, ws: &str, now_ms: i64) -> AppResult<Vec<InboxItem>> {
    use futures::TryStreamExt;
    let cursor = state.db.collection_relationship_type_suggestions()
        .find(doc! { "workspace_id": ws, "status": "pending" },
              mongodb::options::FindOptions::builder().limit(100).build()).await?;
    let rows: Vec<crate::models::RelationshipTypeSuggestion> = cursor.try_collect().await?;
    Ok(rows.into_iter().map(|r| {
        let id = r.id.map(|o| o.to_hex()).unwrap_or_default();
        InboxItem {
            source: "relationship_suggestion".into(),
            id,
            title: format!("关系类型建议：{}", r.suggested_value),
            summary: r.suggested_value.clone(),
            severity: "low".into(),
            created_at: Some(r.last_seen_at),
            age_hours: age_hours_of(Some(r.last_seen_at), now_ms),
            action_kind: "inline".into(),
            rich_component: None, rich_params: None,
        }
    }).collect())
}

/// 知识缺口信号 pending → inline。
async fn collect_gap_signals(state: &AppState, ws: &str, now_ms: i64) -> AppResult<Vec<InboxItem>> {
    use futures::TryStreamExt;
    let cursor = state.db.knowledge_gap_signals()
        .find(doc! { "workspace_id": ws, "status": "pending" },
              mongodb::options::FindOptions::builder().limit(100).build()).await?;
    let rows: Vec<crate::models::KnowledgeGapSignal> = cursor.try_collect().await?;
    Ok(rows.into_iter().map(|g| {
        let id = g.id.map(|o| o.to_hex()).unwrap_or_default();
        InboxItem {
            source: "gap_signal".into(),
            id,
            title: g.title.clone(),
            summary: g.description.clone(),
            severity: "medium".into(),
            created_at: Some(g.created_at),
            age_hours: age_hours_of(Some(g.created_at), now_ms),
            action_kind: "inline".into(),
            rich_component: None, rich_params: None,
        }
    }).collect())
}

/// profile 待激活草稿(current_version=true && is_active=false) → rich。
async fn collect_profile_drafts(state: &AppState, ws: &str, now_ms: i64) -> AppResult<Vec<InboxItem>> {
    use futures::TryStreamExt;
    let cursor = state.db.domain_profiles()
        .find(doc! { "workspace_id": ws, "current_version": true, "is_active": false },
              mongodb::options::FindOptions::builder().limit(50).build()).await?;
    let rows: Vec<crate::models::DomainProfile> = cursor.try_collect().await?;
    Ok(rows.into_iter().map(|p| {
        let id = p.id.map(|o| o.to_hex()).unwrap_or_default();
        InboxItem {
            source: "profile_risky".into(),
            id: id.clone(),
            title: format!("待激活画像：{}", p.display_name),
            summary: "AI 生成的运营画像草稿待人审激活".into(),
            severity: "high".into(),
            created_at: Some(p.created_at),
            age_hours: age_hours_of(Some(p.created_at), now_ms),
            action_kind: "rich".into(),
            rich_component: Some("profilePublish".into()),
            rich_params: Some(doc! { "profileId": id }),
        }
    }).collect())
}

/// 进化候选 eligible_for_release → rich。
async fn collect_evolution_proposals(state: &AppState, ws: &str, now_ms: i64) -> AppResult<Vec<InboxItem>> {
    use futures::TryStreamExt;
    let cursor = state.db.proposals()
        .find(doc! { "workspace_id": ws, "status": "eligible_for_release" },
              mongodb::options::FindOptions::builder().limit(50).build()).await?;
    let rows: Vec<crate::models::Proposal> = cursor.try_collect().await?;
    Ok(rows.into_iter().map(|p| {
        let id = p.id.map(|o| o.to_hex()).unwrap_or_default();
        InboxItem {
            source: "evolution_proposal".into(),
            id: id.clone(),
            title: format!("进化候选：{}", p.proposal_kind),
            summary: p.diff_summary.clone().unwrap_or_default(),
            severity: "medium".into(),
            created_at: Some(p.created_at),
            age_hours: age_hours_of(Some(p.created_at), now_ms),
            action_kind: "rich".into(),
            rich_component: Some("evolutionRelease".into()),
            rich_params: Some(doc! { "proposalId": id }),
        }
    }).collect())
}

/// lessons_learned pending_review → rich。裸 Document（无 typed accessor）。
async fn collect_lessons_learned(state: &AppState, ws: &str, now_ms: i64) -> AppResult<Vec<InboxItem>> {
    use futures::TryStreamExt;
    let coll = state.db.raw().collection::<Document>("lessons_learned");
    let cursor = coll.find(doc! { "workspace_id": ws, "review_status": "pending_review" },
                           mongodb::options::FindOptions::builder().limit(50).build()).await?;
    let rows: Vec<Document> = cursor.try_collect().await?;
    Ok(rows.into_iter().map(|d| {
        let id = d.get_object_id("_id").map(|o| o.to_hex()).unwrap_or_default();
        let kind = d.get_str("pattern_kind").unwrap_or("").to_string();
        let created = d.get_datetime("created_at").ok().copied();
        InboxItem {
            source: "lessons_learned".into(),
            id,
            title: format!("经验晋升：{kind}"),
            summary: "AI 总结的经验待人审晋升为案例".into(),
            severity: "low".into(),
            created_at: created,
            age_hours: age_hours_of(created, now_ms),
            action_kind: "rich".into(),
            rich_component: Some("lessonsPromote".into()),
            rich_params: None,
        }
    }).collect())
}
```

- [ ] **Step 2: 接进 inbox 的 macro 区**（Task 11 的 `collect_source!` 区）

```rust
    collect_source!("taxonomy_candidate", collect_taxonomy_candidates(&state, ws, now_ms));
    collect_source!("relationship_suggestion", collect_relationship_suggestions(&state, ws, now_ms));
    collect_source!("gap_signal", collect_gap_signals(&state, ws, now_ms));
    collect_source!("profile_risky", collect_profile_drafts(&state, ws, now_ms));
    collect_source!("evolution_proposal", collect_evolution_proposals(&state, ws, now_ms));
    collect_source!("lessons_learned", collect_lessons_learned(&state, ws, now_ms));
```

- [ ] **Step 3: summary 补其余计数**（同 count_documents 模式，`unwrap_or(0)` 降级）

- [ ] **Step 4: 编译（逐 source 修字段名）**

Run: `CARGO_INCREMENTAL=0 cargo check --lib 2>&1 | tail -30`
Expected: 若报字段名错误（如 `RelationshipTypeSuggestion` 无 `suggested_value`），按编译器提示 + `grep -n "struct <Name>" -A 25 src/models.rs` 实证字段名修正。0 errors 为准。

- [ ] **Step 5: Commit**

```bash
git add src/routes/ask_human_inbox.rs
git commit -m "feat(ask-human): 聚合器补齐 6 source(标签/关系/缺口/画像/进化/经验)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 13: 迁移 m025 回填 ask_human_policy

**Files:**
- Create: `src/db/migrations/m025_backfill_ask_human_policy.rs`
- Modify: `src/db/migrations/mod.rs`（声明 + 注册）

**Interfaces:**
- Consumes: 旧 `principal_decider`/`high_risk_escalation_mode`；写 `ask_human_policy`

- [ ] **Step 1: 写迁移**（仿 `m024_seed_relationship_type.rs` 的 `run_step` 结构）

```rust
//! 2026_06_X9_001：回填 ask_human_policy。把现有 (principal_decider,
//! high_risk_escalation_mode) 映射成 ask_human_policy（decider_chain + 四 escalate_*）。
//! 幂等：已有 ask_human_policy 的行跳过。不删旧字段（向后兼容兜底）。

use mongodb::bson::{doc, DateTime};

use crate::db::Database;
use crate::error::AppResult;

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    use futures::TryStreamExt;
    let coll = db.operation_domain_configs();
    let cursor = coll
        .find(doc! { "ask_human_policy": { "$exists": false } }, None)
        .await?;
    let rows: Vec<crate::models::OperationDomainConfig> = cursor.try_collect().await?;
    for cfg in rows {
        let all_mode = cfg.high_risk_escalation_mode.as_deref() == Some("all");
        let chain: Vec<mongodb::bson::Document> = cfg
            .principal_decider
            .as_ref()
            .map(|w| vec![doc! { "wxid": w }])
            .unwrap_or_default();
        let policy = doc! {
            "deciderChain": chain,
            "escalateSafetyGuard": true,
            "escalateUnverifiedProduct": true,
            "escalateAiPolicyHold": all_mode,
            "escalateStuck": true,
        };
        coll.update_one(
            doc! { "_id": cfg.id.unwrap() },
            doc! { "$set": { "ask_human_policy": policy, "updated_at": DateTime::now() } },
            None,
        )
        .await?;
    }
    Ok(())
}
```
注：`ask_human_policy` 是 `#[serde(rename_all="camelCase")]`，故 BSON 键用 camelCase（`deciderChain` 等）。DeciderRef 同样 camelCase（`wxid`/`displayName`），`wxid` 两形态一致。

- [ ] **Step 2: 注册**（`src/db/migrations/mod.rs`）

`mod m025_backfill_ask_human_policy;`（`m024` 声明后，`:61` 附近）
MIGRATIONS 数组末尾（`:168` 之后）：
```rust
    Migration {
        id: "2026_06_X9_001_backfill_ask_human_policy",
        run: |db| Box::pin(m025_backfill_ask_human_policy::run_step(db)),
    },
```

- [ ] **Step 3: 编译**

Run: `CARGO_INCREMENTAL=0 cargo check --lib 2>&1 | tail -20`
Expected: 0 errors。

- [ ] **Step 4: Commit**

```bash
git add src/db/migrations/m025_backfill_ask_human_policy.rs src/db/migrations/mod.rs
git commit -m "feat(ask-human): m025 回填 ask_human_policy(旧字段映射,幂等,不删旧字段)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 14: 集成测试（#[ignore]，CI 跑）

**Files:**
- Create: `tests/ask_human_phase1_e2e.rs`

**Interfaces:**
- Consumes: `tests/common/mod.rs` 的 `TestApp::start()`

- [ ] **Step 1: 写集成测试**（参照现有 `tests/domain_profile_e2e.rs` 的 TestApp 用法 + seed helper 纪律）

测试覆盖（每个 `#[ignore]` + `#[tokio::test]`）：
1. `put_ask_human_policy_persists_and_reads_back`：PUT 配置 → 直接查 config 行 `ask_human_policy` 字段一致，version 未 bump。
2. `admin_resolve_enqueues_relay_and_marks_resolved`：seed 一条 pending escalation → POST resolve → 台账 status=resolved + resolved_via="admin" + 有一条 `principal_decision_relay` task 入队。
3. `admin_resolve_is_idempotent`：对已 resolved 的再 resolve → 返回 alreadyResolved，不重复入队。
4. `reassign_rejects_wxid_not_in_chain`：配置 decider_chain=[a]，reassign 到 b → 400。
5. `inbox_aggregates_and_degrades`：seed 一条 pending escalation + 一条 needs_review chunk → GET inbox 返回 ≥2 items，errors 为空。
6. `summary_counts_pending`：GET summary → principalEscalation 计数正确。

**关键 harness 纪律**：若测试需 seed `operation_domain_configs` 的 `(default,user_operations,v1)` 行，**必须 `replace_one(upsert)` 不能 `insert_one`**（`ensure_prompt_pack_v2` 已 seed，见 [[project_config_seed_in_prompts_not_migrations]]）。配置改动用 `$set` 到既有 current 行。

每个测试体给出完整代码（构造 TestApp、seed、发请求、断言）。示例（测试 2 完整）：

```rust
#[tokio::test]
#[ignore]
async fn admin_resolve_enqueues_relay_and_marks_resolved() {
    let app = TestApp::start().await;
    let ws = &app.state.config.default_workspace_id;
    // seed 一条 pending escalation
    let entry = crate::seed_pending_escalation(&app.state, ws, "cust1", "boss").await;
    // POST resolve
    let resp = app
        .post_json(
            &format!("/api/admin/principal-escalations/{}/resolve", entry.short_code),
            serde_json::json!({ "verdict": "approved", "substance": "可以给 8 折" }),
        )
        .await;
    assert_eq!(resp.status(), 200);
    // 台账 resolved + resolved_via=admin
    let updated = app.state.db.agent_principal_escalations()
        .find_one(doc! { "short_code": &entry.short_code }, None).await.unwrap().unwrap();
    assert_eq!(updated.status, "resolved");
    assert_eq!(updated.resolved_via.as_deref(), Some("admin"));
    // relay task 入队
    let task_count = app.state.db.tasks()
        .count_documents(doc! { "kind": "principal_decision_relay", "content": &entry.short_code }, None)
        .await.unwrap();
    assert_eq!(task_count, 1);
}
```
注：`app.post_json` / `seed_pending_escalation` 以 `tests/common/mod.rs` 实际暴露的 helper 为准；若无 `post_json`，用现有测试里的 reqwest/axum test client 惯例（`grep -n "post\|fn start\|TestApp" tests/common/mod.rs` + 抄 `tests/domain_profile_e2e.rs` 的请求方式）。

- [ ] **Step 2: 编译测试（不跑，本地无 Docker）**

Run: `CARGO_INCREMENTAL=0 cargo test --test ask_human_phase1_e2e --no-run 2>&1 | tail -20`
Expected: 编译通过（测试 `#[ignore]`，CI 才跑）。

- [ ] **Step 3: Commit**

```bash
git add tests/ask_human_phase1_e2e.rs
git commit -m "test(ask-human): Phase 1 集成测试(配置写入/admin裁决/幂等/改派/聚合,#[ignore])

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## 收尾验证（全部 task 后）

- [ ] **基线门**：`CARGO_INCREMENTAL=0 cargo test --lib 2>&1 | tail -5` → ≥ 350/0
- [ ] **PBT 门**：四 PBT 文件各跑一遍累计 ≥ 33/0
- [ ] **no-takeover lint**：`bash scripts/check-no-human-takeover.sh` → clean
- [ ] **磁盘**：编译前若紧 `rm -rf target/debug/incremental`
- [ ] 集成测试留 CI（`.github/workflows/ci.yml` integration job 跑 `--ignored`）

## Phase 1 完成定义

请示通道可在 admin：列表查看 / 直接结构化裁决（复用 relay，客户收 AI 口吻转述）/ 改派；ask_human_policy 可写且**真生效**（升级范围四布尔驱动 should_escalate_held、decider_chain[0] 为推送目标、骚扰门 push_allowed、超时转备选扫描）；收件箱聚合 8 source 就绪（per-source 降级）；旧字段经 m025 回填、字节等价。**不含任何前端**（P2/P3 另起）。
