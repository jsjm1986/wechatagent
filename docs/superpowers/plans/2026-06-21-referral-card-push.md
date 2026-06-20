# 专属顾问名片引荐能力 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 AI 在销售对话中按"提示词注入"模型，识别契合人类标注的高价值客户时，自主把真人专属顾问微信名片推送给客户（辅助模式），AI 退为辅助答疑。

**Architecture:** 新建独立 `referral_cards` 集合（人类标注 target_wxid + 触发提示）；账号级（OperationDomainConfig）+ 客户级（Contact.domain_attributes）开关；候选名片注入 decision prompt，AI 在一次决策里输出 `namecard_to_send`；经现有 outbox 幂等通道，dispatcher 分流调 MCP `message_send_namecard`；发送成功后置「已引荐」态收敛 AI 行为。发送内核形状对齐素材发送计划（`2026-06-21-sales-media-asset-send`，目前 0 行落地），将来收敛。

**Tech Stack:** Rust 2021 / Axum / MongoDB(mongodb crate) / 现有 MCP JSON-RPC client / React 19 + Vite + TS。

设计来源：`docs/superpowers/specs/2026-06-21-referral-card-push-design.md`。

## Global Constraints

- **向后兼容红线**：`AgentDecision` / `OutboxEntry` / `EnqueueRequest` / `ConversationMessage` / `OperationDomainConfig` 所有新增字段必须 `Option`/`Vec` + `#[serde(default)]`；旧文档（无新字段）必须仍能反序列化。
- **三处接线红线**：新增 `namecard_to_send` 必须同时加到 `AgentDecision`（types.rs:224 旁）+ `RawAgentDecision`（types.rs:380 旁）+ `validate_and_promote` carry-through（types.rs:952 旁 `if raw.x.is_some()` 模式）。漏 carry-through 则 LLM 输出被静默丢弃。
- **幂等红线**：approved 发送必须先进 `agent_send_outbox` 拿幂等键再调 MCP；名片条目空 content 须含 `referral_card_id` 进幂等键，否则多张不同名片 hash 撞键误去重。
- **AI 不自我核验红线**：名片默认 `review_status="draft"`，必须人类标 `approved` 且 `enabled=true` 才允许 AI 选。
- **prompt 版本门控**：改 `prompts.rs` 的 `user.reply.task` 字面量后，必须 bump `PROMPT_PACK_VERSION`（prompts.rs:15）或走 reset-system-pack，否则 `ensure_prompt_pack_v2` 不重种、改动不生效。
- **no-human-takeover lint**：`scripts/check-no-human-takeover.sh` 扫 `src/agent/ src/routes/ src/evolution/ frontend/src/` 的 diff 新增行，禁词 `人工接管|takeover|hand-off|人工介入|人工托管|接管|人工`。本功能一律用 AI 内部口径命名（referral/引荐/专属顾问/已引荐），不得出现禁词。
- **红线定位**：本功能为"客户永远只跟 AI 对话、永不直接面对真人"红线开**辅助模式受控例外**；全自治模式红线不动。
- **测试铁律**：纯函数确定性测试为主；不接受 skip 假绿；新增测试只 append 不删旧维度；不过拟合单条样本（触发靠人类标注+LLM 语义，不写关键词词表）；baseline 不回归（`cargo test --lib` ≥350 passed/0 failed，4 个 PBT 累计 ≥33/0）。
- **MCP 工具未决**：`message_send_namecard` 仓内零书面依据，仅用户口头确认。入参字段名（recipient / 目标真人字段）以 server `tools/list` schema 为准。本计划用占位形态，集成时对齐（见 Task 8）。
- **Shell**：bash on Windows，项目根含非 ASCII（`工作项目`），用绝对路径。本地只跑 `cargo test --lib` 和单个 PBT，全量集成留 CI。
- **Subagents**：本项目 spawn 的所有 subagent 必须 `model: "opus"`。

---

## File Structure

**后端新建：**
- `src/agent/referral.rs` — 名片引荐核心：候选过滤/渲染纯函数、辅助模式开关判定纯函数、`send_outbound_namecard`、置「已引荐」态。
- `src/routes/referral_cards.rs` — 名片库 CRUD + 审核 route handler。

**后端修改：**
- `src/models.rs` — 新增 `ReferralCard` 结构体 + `referred_specialist` 相关常量；`AgentDecision`/`RawAgentDecision` 不在此（在 types.rs）；`OutboxEntry` 加 `referral_card_id`；`ConversationMessage` 加 `msg_type`/`media_ref`；`OperationDomainConfig` 加 `assist_mode_enabled`。
- `src/agent/types.rs` — `AgentDecision` + `RawAgentDecision` 加 `namecard_to_send`；`validate_and_promote` carry-through；定义 `NamecardDirective`。
- `src/agent/outbox.rs` — `EnqueueRequest` 加 `referral_card_id`；放宽空 content 校验；`compute_synthetic_key` 含 card_id。
- `src/agent/outbox_dispatcher.rs` — dispatch 按 `referral_card_id` 分流；崩溃恢复核对适配。
- `src/agent/decision.rs` — `load_referral_cards` + 候选注入 prompt；「已引荐」态信号注入。
- `src/agent/gateway.rs` — `namecard_to_send` 准入校验 + 转 outbox 名片条目（文本循环后）。
- `src/agent/escalation/logic.rs` — `build_decision_signals_text` 加「已引荐」态指引（或同款新函数）。
- `src/prompts.rs` — `user.reply.task` 加专属顾问引荐指引 + bump `PROMPT_PACK_VERSION`。
- `src/db/mod.rs` — `referral_cards()` typed accessor。
- `src/db/indexes.rs` — `referral_cards` 索引。
- `src/routes/mod.rs` — 挂载 `referral_cards` 路由 + `mod referral_cards;`。
- `src/agent/mod.rs` — `mod referral;` + 必要 re-export。

**前端修改：**
- `frontend/src/features/referral-cards/*` — 名片库管理页（录入/审核/启停）。
- `frontend/src/types/index.ts` + 账号配置页 — 辅助模式开关；`Message` 加 `msgType`/`mediaRef`。
- 对话渲染处 — 名片消息卡片。

**测试新建：**
- 各模块内联 `#[cfg(test)]`。
- `tests/referral_card_push_integration.rs`（`#[ignore]`，CI）— 端到端 + outbox 幂等。

---

### Task 1: ReferralCard 数据模型 + OperationDomainConfig 开关字段 + 向后兼容测试

**Files:**
- Modify: `src/models.rs`（新增 `ReferralCard` 结构体；`OperationDomainConfig`（:807）加 `assist_mode_enabled`；新增常量）
- Test: `src/models.rs` 内联 `#[cfg(test)] mod referral_card_compat_tests`

**Interfaces:**
- Consumes: 无（地基任务）
- Produces:
  - `pub struct ReferralCard { id, workspace_id, account_id: Option<String>, target_wxid: String, display_name: String, send_trigger_hint: String, target_stages: Vec<String>, enabled: bool, review_status: String, review_note: Option<String>, created_at, updated_at }`
  - `OperationDomainConfig.assist_mode_enabled: Option<bool>`
  - 常量 `pub const REFERRED_SPECIALIST_AT_ATTR: &str = "referred_specialist_at";`、`pub const REFERRED_CARD_ID_ATTR: &str = "referred_card_id";`、`pub const ASSIST_MODE_OVERRIDE_ATTR: &str = "assist_mode_override";`

- [ ] **Step 1: 写失败测试**

在 `src/models.rs` 末尾 `#[cfg(test)]` 区追加：

```rust
#[cfg(test)]
mod referral_card_compat_tests {
    use super::{ReferralCard, OperationDomainConfig};
    use mongodb::bson::{doc, DateTime};

    #[test]
    fn referral_card_roundtrips() {
        let card = ReferralCard {
            id: None,
            workspace_id: "ws1".into(),
            account_id: None,
            target_wxid: "wxid_boss".into(),
            display_name: "销售总监-老王".into(),
            send_trigger_hint: "客户明确要签约或要来公司参观时引荐".into(),
            target_stages: vec!["意向".into()],
            enabled: true,
            review_status: "approved".into(),
            review_note: None,
            created_at: DateTime::now(),
            updated_at: DateTime::now(),
        };
        let d = mongodb::bson::to_document(&card).unwrap();
        let back: ReferralCard = mongodb::bson::from_document(d).unwrap();
        assert_eq!(back.target_wxid, "wxid_boss");
        assert_eq!(back.target_stages.len(), 1);
        assert!(back.enabled);
    }

    #[test]
    fn legacy_domain_config_without_assist_flag_deserializes_none() {
        // 旧 OperationDomainConfig 行无 assist_mode_enabled
        let legacy = doc! {
            "workspace_id": "ws1", "domain": "user_operations", "name": "x",
            "goal": "g", "methodology": "m", "workflow": "w",
            "tool_policy": "t", "automation_policy": "a", "review_policy": "r",
            "status": "active", "updated_at": DateTime::now(),
        };
        let cfg: OperationDomainConfig = mongodb::bson::from_document(legacy)
            .expect("legacy domain config must still deserialize");
        assert_eq!(cfg.assist_mode_enabled, None);
    }
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --lib referral_card_compat_tests`
Expected: 编译失败（`ReferralCard` 未定义、`assist_mode_enabled` 缺字段）。

- [ ] **Step 3: 加 ReferralCard 结构体 + 常量**

在 `src/models.rs` 合适位置（靠近 `ContentAsset` 或 `OperationDomainConfig`）新增。`ReferralCard` 用 `#[derive(Debug, Clone, Serialize, Deserialize)]`，不加 `rename_all`（snake_case 落库，与 OperationDomainConfig 同款）：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferralCard {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    pub target_wxid: String,
    pub display_name: String,
    #[serde(default)]
    pub send_trigger_hint: String,
    #[serde(default)]
    pub target_stages: Vec<String>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub review_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_note: Option<String>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}
```

在已有常量区（`AWAITING_PRINCIPAL_DECISION_ATTR` 附近，models.rs:2890）追加：

```rust
/// 「已引荐」态：发送名片成功后写入 Contact.domain_attributes 的时间戳键。
pub const REFERRED_SPECIALIST_AT_ATTR: &str = "referred_specialist_at";
/// 已引荐推了哪张名片（card_id hex）。
pub const REFERRED_CARD_ID_ATTR: &str = "referred_card_id";
/// 客户级辅助模式覆盖："force_on" | "force_off"。
pub const ASSIST_MODE_OVERRIDE_ATTR: &str = "assist_mode_override";
```

- [ ] **Step 4: OperationDomainConfig 加字段**

在 `src/models.rs:807` 的 `OperationDomainConfig` 末尾（`high_risk_escalation_mode` 之后）加：

```rust
    /// 辅助模式开关：true=本账号启用专属顾问名片引荐。None/false=纯全自治(默认)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assist_mode_enabled: Option<bool>,
```

补齐所有 `OperationDomainConfig {` 字面量构造点的 `assist_mode_enabled: None`（grep `OperationDomainConfig {` 找全，含 tests 与 prompts.rs 的 seed）。

- [ ] **Step 5: 运行确认通过 + 全 lib 不回归**

Run: `cargo test --lib referral_card_compat_tests && cargo test --lib 2>&1 | tail -5`
Expected: 2 passed；全 lib passed ≥ 350。

- [ ] **Step 6: Commit**

```bash
git add src/models.rs
git commit -m "feat(referral-card): ReferralCard模型+OperationDomainConfig辅助模式开关+常量(向后兼容)"
```

---

### Task 2: referral_cards 集合 accessor + 索引

**Files:**
- Modify: `src/db/mod.rs`（加 `referral_cards()` typed accessor）
- Modify: `src/db/indexes.rs`（`ensure_all` 加 referral_cards 索引）
- Test: `src/db/indexes.rs` 内联测试（若已有索引测试模块则 append；否则跳过——索引建立靠集成测试覆盖）

**Interfaces:**
- Consumes: `ReferralCard`（Task 1）
- Produces: `pub fn referral_cards(&self) -> Collection<ReferralCard>`（`src/db/mod.rs`）；`referral_cards` 集合索引 `{workspace_id:1, account_id:1, enabled:1, review_status:1}`

- [ ] **Step 1: 加 typed accessor**

`src/db/mod.rs`，在 `agent_principal_escalations()`（:183）附近追加（确认 `ReferralCard` 已在该文件 `use crate::models::{...}` 引入；若没有则补）：

```rust
    pub fn referral_cards(&self) -> Collection<ReferralCard> {
        self.db.collection("referral_cards")
    }
```

- [ ] **Step 2: 加索引**

`src/db/indexes.rs` 的 `ensure_all`（:10）里，参照现有 `create_index` 写法（如 content_assets :190 附近）追加：

```rust
    db.referral_cards()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "workspace_id": 1,
                    "account_id": 1,
                    "enabled": 1,
                    "review_status": 1,
                })
                .build(),
            None,
        )
        .await?;
```

（确认 `IndexModel` / `doc!` 已在该文件 use；跟随现有 create_index 模式。无需 migration——集合首次 insert 自动建 + ensure_indexes 幂等。）

- [ ] **Step 3: 编译 + lib 不回归**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: 全绿，passed ≥ 350。

- [ ] **Step 4: Commit**

```bash
git add src/db/mod.rs src/db/indexes.rs
git commit -m "feat(referral-card): referral_cards 集合 accessor + 选材索引"
```

---

### Task 3: 辅助模式判定 + 候选过滤/渲染纯函数

**Files:**
- Create: `src/agent/referral.rs`
- Modify: `src/agent/mod.rs`（加 `mod referral;`）
- Test: `src/agent/referral.rs` 内联 `#[cfg(test)]`

**Interfaces:**
- Consumes: `ReferralCard`、`OperationDomainConfig`、`Contact`、`ASSIST_MODE_OVERRIDE_ATTR` / `REFERRED_SPECIALIST_AT_ATTR` / `REFERRED_CARD_ID_ATTR`（Task 1）
- Produces:
  - `pub(crate) fn assist_mode_active(account_enabled: Option<bool>, override_attr: Option<&str>) -> bool` — 客户级 override > 账号级 > 默认关
  - `pub(crate) fn validate_card_sendable(card: &ReferralCard) -> bool` — enabled && review_status=="approved"
  - `pub(crate) fn filter_referral_candidates<'a>(cards: &'a [ReferralCard], customer_stage: Option<&str>) -> Vec<&'a ReferralCard>`
  - `pub(crate) struct AlreadyReferred { pub display_name: String, pub card_id: String }`
  - `pub(crate) fn render_referral_lines(candidates: &[&ReferralCard], already: Option<&AlreadyReferred>) -> String`

- [ ] **Step 1: 写失败测试**

新建 `src/agent/referral.rs`，先写测试模块：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ReferralCard;
    use mongodb::bson::DateTime;

    fn card(enabled: bool, review: &str, stages: Vec<&str>) -> ReferralCard {
        ReferralCard {
            id: None, workspace_id: "ws".into(), account_id: None,
            target_wxid: "wxid_boss".into(), display_name: "老王".into(),
            send_trigger_hint: "要签约时引荐".into(),
            target_stages: stages.into_iter().map(|s| s.to_string()).collect(),
            enabled, review_status: review.into(), review_note: None,
            created_at: DateTime::now(), updated_at: DateTime::now(),
        }
    }

    #[test]
    fn assist_mode_override_beats_account_flag() {
        // 账号关 + 客户 force_on → 开
        assert!(assist_mode_active(Some(false), Some("force_on")));
        // 账号开 + 客户 force_off → 关
        assert!(!assist_mode_active(Some(true), Some("force_off")));
        // 账号开 + 无 override → 开
        assert!(assist_mode_active(Some(true), None));
        // 账号 None + 无 override → 默认关
        assert!(!assist_mode_active(None, None));
        // 无关脏值 override 视为无覆盖
        assert!(assist_mode_active(Some(true), Some("garbage")));
        assert!(!assist_mode_active(Some(false), Some("garbage")));
    }

    #[test]
    fn validate_excludes_draft_and_disabled() {
        assert!(validate_card_sendable(&card(true, "approved", vec![])));
        assert!(!validate_card_sendable(&card(false, "approved", vec![])));
        assert!(!validate_card_sendable(&card(true, "draft", vec![])));
    }

    #[test]
    fn filter_matches_stage_or_empty() {
        let all = vec![
            card(true, "approved", vec!["意向"]),   // 命中
            card(true, "approved", vec!["已成交"]), // 不命中
            card(true, "approved", vec![]),          // 空 = 总命中
            card(false, "approved", vec!["意向"]),  // 排除：disabled
        ];
        let kept = filter_referral_candidates(&all, Some("意向"));
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn render_includes_hint_and_already_referred_note() {
        let c = card(true, "approved", vec!["意向"]);
        let line = render_referral_lines(&[&c], None);
        assert!(line.contains("要签约时引荐"));
        assert!(line.contains("老王"));
        let already = AlreadyReferred { display_name: "老王".into(), card_id: "c1".into() };
        let line2 = render_referral_lines(&[&c], Some(&already));
        assert!(line2.contains("已") && line2.contains("老王"));
    }

    #[test]
    fn render_empty_candidates_is_empty() {
        assert_eq!(render_referral_lines(&[], None), "");
    }
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --lib agent::referral`
Expected: 编译失败（函数/类型未定义）。

- [ ] **Step 3: 实现纯函数**

在测试模块之上写实现：

```rust
//! 专属顾问名片引荐：辅助模式判定、候选过滤/渲染（纯函数）、
//! send_outbound_namecard、置「已引荐」态。
use crate::models::ReferralCard;

/// 辅助模式是否对本客户生效。客户级 override > 账号级 enabled > 默认关。
pub(crate) fn assist_mode_active(account_enabled: Option<bool>, override_attr: Option<&str>) -> bool {
    match override_attr {
        Some("force_on") => true,
        Some("force_off") => false,
        _ => account_enabled.unwrap_or(false),
    }
}

/// 发送前准入：仅 enabled 且 approved 的名片可被 AI 选/发。
pub(crate) fn validate_card_sendable(card: &ReferralCard) -> bool {
    card.enabled && card.review_status == "approved"
}

pub(crate) fn filter_referral_candidates<'a>(
    cards: &'a [ReferralCard],
    customer_stage: Option<&str>,
) -> Vec<&'a ReferralCard> {
    cards
        .iter()
        .filter(|c| validate_card_sendable(c))
        .filter(|c| {
            c.target_stages.is_empty()
                || customer_stage
                    .map(|cs| c.target_stages.iter().any(|s| s == cs))
                    .unwrap_or(false)
        })
        .collect()
}

/// 本客户已引荐过的顾问（防重推上下文）。
pub(crate) struct AlreadyReferred {
    pub display_name: String,
    pub card_id: String,
}

pub(crate) fn render_referral_lines(
    candidates: &[&ReferralCard],
    already: Option<&AlreadyReferred>,
) -> String {
    if candidates.is_empty() {
        return String::new();
    }
    let mut out =
        String::from("可引荐的专属顾问（仅在客户契合触发提示时引荐，没有契合的就不引荐）：\n");
    for c in candidates {
        let id = c.id.map(|i| i.to_hex()).unwrap_or_default();
        let stages = c.target_stages.join(",");
        out.push_str(&format!(
            "- [card:{id}] {} | 阶段:{stages} | 触发提示:{}\n",
            c.display_name, c.send_trigger_hint
        ));
    }
    match already {
        Some(a) => out.push_str(&format!(
            "（本客户引荐历史：已引荐给 {}[card:{}]——除非出现与上次不同的新需求场景，否则不要重复引荐）\n",
            a.display_name, a.card_id
        )),
        None => out.push_str("（本客户引荐历史：尚未引荐）\n"),
    }
    out
}
```

在 `src/agent/mod.rs` 加 `mod referral;`（与其它子模块声明并列）。

- [ ] **Step 4: 运行确认通过**

Run: `cargo test --lib agent::referral`
Expected: 5 passed。

- [ ] **Step 5: Commit**

```bash
git add src/agent/referral.rs src/agent/mod.rs
git commit -m "feat(referral-card): 辅助模式判定+候选过滤/渲染纯函数(含已引荐防重推)"
```

---

### Task 4: AgentDecision.namecard_to_send 三处接线

**Files:**
- Modify: `src/agent/types.rs`（定义 `NamecardDirective`；`AgentDecision`（:80）加字段；`RawAgentDecision`（:307）加字段；`validate_and_promote` carry-through（:952 旁）；`AgentDecision::default()`（:227 区）补字段）
- Test: `src/agent/types.rs` 内联 `#[cfg(test)] mod namecard_directive_tests`

**Interfaces:**
- Consumes: 无（与 referral.rs 解耦——这里只定契约）
- Produces:
  - `pub struct NamecardDirective { pub card_id: String, pub reason: Option<String> }`
  - `AgentDecision.namecard_to_send: Option<NamecardDirective>`
  - `RawAgentDecision.namecard_to_send: Option<NamecardDirective>`
  - promote carry-through 透传

- [ ] **Step 1: 写失败测试**

`src/agent/types.rs` 测试区追加：

```rust
#[cfg(test)]
mod namecard_directive_tests {
    use super::{AgentDecision, RawAgentDecision};

    #[test]
    fn decision_without_namecard_defaults_none() {
        // 旧 LLM 输出（无 namecardToSend）必须仍能反序列化
        let json = r#"{"replyText":"你好","shouldReply":true}"#;
        let d: AgentDecision = serde_json::from_str(json).expect("must deserialize");
        assert!(d.namecard_to_send.is_none());
    }

    #[test]
    fn raw_parses_namecard_and_promote_carries_through() {
        let json = r#"{"replyText":"我请负责人跟您对接","namecardToSend":{"cardId":"c1","reason":"客户要签约"}}"#;
        let raw: RawAgentDecision = serde_json::from_str(json).unwrap();
        assert!(raw.namecard_to_send.is_some());
        // promote 后必须仍带 namecard（carry-through 生效）
        let runtime = crate::agent::runtime::UserRuntimeParameters::default();
        let (decision, _violations) = raw.validate_and_promote(&runtime);
        assert_eq!(
            decision.namecard_to_send.as_ref().map(|n| n.card_id.as_str()),
            Some("c1")
        );
    }
}
```

（注：`UserRuntimeParameters::default()` 若不存在或签名不同，以 types.rs 现有 `validate_and_promote` 测试的构造方式为准——grep 现有 `validate_and_promote(` 测试调用点照搬入参。）

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --lib namecard_directive_tests`
Expected: 编译失败（`namecard_to_send` / `NamecardDirective` 未定义）。

- [ ] **Step 3: 定义 NamecardDirective + AgentDecision 字段**

`src/agent/types.rs`，定义结构体（跟随文件 `#[serde(rename_all = "camelCase")]` 风格，放在 `FollowUpDecision` 附近）：

```rust
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NamecardDirective {
    pub card_id: String,
    #[serde(default)]
    pub reason: Option<String>,
}
```

`AgentDecision`（:224，`escalation_request` 字段旁）加：

```rust
    /// AI 决定本轮引荐某专属顾问名片；None=不引荐。
    #[serde(default)]
    pub namecard_to_send: Option<NamecardDirective>,
```

`AgentDecision::default()`（:287 `escalation_request: None` 旁）加 `namecard_to_send: None,`。

- [ ] **Step 4: RawAgentDecision 字段 + carry-through**

`RawAgentDecision`（:380，`escalation_request` 旁）加：

```rust
    pub namecard_to_send: Option<NamecardDirective>,
```

`validate_and_promote` carry-through（:952 `if raw.escalation_request.is_some()` 之后）加：

```rust
    if raw.namecard_to_send.is_some() {
        decision.namecard_to_send = raw.namecard_to_send;
    }
```

- [ ] **Step 5: 运行确认通过 + lib 不回归**

Run: `cargo test --lib namecard_directive_tests && cargo test --lib 2>&1 | tail -5`
Expected: 2 passed；全 lib passed ≥ 350。

- [ ] **Step 6: Commit**

```bash
git add src/agent/types.rs
git commit -m "feat(referral-card): AgentDecision.namecardToSend三处接线(Raw+carry-through防丢字段)"
```

---

### Task 5: outbox 名片条目（referral_card_id + 空 content 放宽 + 幂等键）

**Files:**
- Modify: `src/models.rs`（`OutboxEntry`（:2370）加 `referral_card_id`）
- Modify: `src/agent/outbox.rs`（`EnqueueRequest`（:126）加 `referral_card_id`；放宽 content 校验；`compute_synthetic_key`（:379）含 card_id；构造 `OutboxEntry`（:232）带字段）
- Test: `src/agent/outbox.rs` 内联测试

**Interfaces:**
- Consumes: 现有 `enqueue` / `EnqueueRequest` / `compute_synthetic_key`
- Produces: `EnqueueRequest.referral_card_id: Option<String>`、`OutboxEntry.referral_card_id: Option<String>`、`pub(crate) fn content_required_for(referral_card_id: &Option<String>) -> bool`

- [ ] **Step 1: 写失败测试**

`src/agent/outbox.rs` 测试区追加：

```rust
#[cfg(test)]
mod referral_outbox_tests {
    use super::*;

    #[test]
    fn namecard_entry_allows_empty_content() {
        assert!(content_required_for(&None));                       // 纯文本 → 需 content
        assert!(!content_required_for(&Some("card1".to_string()))); // 名片条目 → 不需
    }

    #[test]
    fn synthetic_key_differs_per_card() {
        // 同 run/contact、空 content，不同 card → key 必须不同（防撞键误去重）
        let k1 = compute_synthetic_key_with_card(
            "inbound_message", "acct", "wx", "run1", "EMPTYHASH", 0, &Some("c1".into()));
        let k2 = compute_synthetic_key_with_card(
            "inbound_message", "acct", "wx", "run1", "EMPTYHASH", 0, &Some("c2".into()));
        assert_ne!(k1, k2);
        // 无 card 时与旧行为一致
        let k_text = compute_synthetic_key_with_card(
            "inbound_message", "acct", "wx", "run1", "H", 0, &None);
        assert_eq!(k_text, compute_synthetic_key("inbound_message", "acct", "wx", "run1", "H", 0));
    }
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --lib referral_outbox_tests`
Expected: 编译失败（`content_required_for` / `compute_synthetic_key_with_card` 未定义）。

- [ ] **Step 3: OutboxEntry + EnqueueRequest 加字段**

`src/models.rs` 的 `OutboxEntry`（:2380 `content` 附近）加：

```rust
    /// 名片引荐条目：非空表示这条 outbox 发的是专属顾问名片而非文本。
    /// dispatcher 据此走 send_outbound_namecard。`#[serde(default)]` 兼容旧文档。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub referral_card_id: Option<String>,
```

`src/agent/outbox.rs` 的 `EnqueueRequest`（:126）加 `pub referral_card_id: Option<String>,`。

- [ ] **Step 4: 放宽 content 校验 + card 幂等键**

`src/agent/outbox.rs` 加纯函数：

```rust
/// 名片条目（referral_card_id 有值）允许空 content；纯文本条目仍要求非空。
pub(crate) fn content_required_for(referral_card_id: &Option<String>) -> bool {
    referral_card_id.is_none()
}

/// synthetic 幂等键带可选 card_id：名片空 content 时靠 card_id 区分多张不同名片。
pub(crate) fn compute_synthetic_key_with_card(
    source_kind: &str,
    account_id: &str,
    contact_wxid: &str,
    run_id: &str,
    content_hash: &str,
    day_bucket: i64,
    referral_card_id: &Option<String>,
) -> String {
    let base = compute_synthetic_key(source_kind, account_id, contact_wxid, run_id, content_hash, day_bucket);
    match referral_card_id {
        Some(card) => format!("{base}:card:{card}"),
        None => base,
    }
}
```

把 `enqueue` 的 content 校验（:164-166）改为：

```rust
    if content_required_for(&req.referral_card_id) && req.content.trim().is_empty() {
        return Err(OutboxError::Invalid("content is empty".to_string()));
    }
```

把 synthetic 分支（:186-195）的 `compute_synthetic_key(...)` 调用替换为 `compute_synthetic_key_with_card(..., &req.referral_card_id)`。非 synthetic 分支（有 source_event_id，:197-200）的 key 也要含 card：把 `format!("{}:{}:{}", source_event_id, contact_wxid, content_hash)` 改为命中名片时追加 `:card:{card_id}`（同 with_card 逻辑——抽一个小 helper 或内联 if）。

构造 `OutboxEntry`（:232）补 `referral_card_id: req.referral_card_id.clone(),`。

- [ ] **Step 5: 补齐所有 EnqueueRequest 构造点**

Run: `grep -rn "EnqueueRequest {" src/`
对每个构造点补 `referral_card_id: None,`（现有文本路径，含 gateway.rs:350、gateway.rs:1774）。

- [ ] **Step 6: 运行确认通过 + lib 不回归**

Run: `cargo test --lib referral_outbox_tests && cargo test --lib 2>&1 | tail -5`
Expected: 2 passed；全 lib passed ≥ 350。

- [ ] **Step 7: Commit**

```bash
git add src/models.rs src/agent/outbox.rs
git commit -m "feat(referral-card): outbox名片条目(空content放宽+card_id幂等键)"
```

---

### Task 6: ConversationMessage 媒体字段 + send_outbound_namecard + 置「已引荐」态

**Files:**
- Modify: `src/models.rs`（`ConversationMessage` 加 `msg_type` / `media_ref`）
- Modify: `src/agent/referral.rs`（加 `send_outbound_namecard` + `mark_referred`）
- Test: `src/agent/referral.rs` 内联测试（mark_referred 的 $set doc 形状纯函数）

**Interfaces:**
- Consumes: `validate_card_sendable`（Task 3）、`ReferralCard`、`Contact`、`mcp::logged_call_for_account`、`REFERRED_SPECIALIST_AT_ATTR` / `REFERRED_CARD_ID_ATTR`（Task 1）
- Produces:
  - `pub(crate) async fn send_outbound_namecard(state, contact: &Contact, card_id: &str) -> AppResult<Value>`
  - `pub(crate) fn build_referred_set_doc(card_id: &str, now: DateTime) -> Document`（置「已引荐」态的 $set 子文档，纯函数便于单测）

- [ ] **Step 1: 写失败测试 + ConversationMessage 兼容测试**

`src/agent/referral.rs` 测试区追加：

```rust
    #[test]
    fn referred_set_doc_has_dotted_keys_and_updated_at() {
        use mongodb::bson::DateTime;
        let now = DateTime::now();
        let d = build_referred_set_doc("c1", now);
        assert!(d.contains_key("domain_attributes.referred_specialist_at"));
        assert!(d.contains_key("domain_attributes.referred_card_id"));
        assert!(d.contains_key("domain_attributes_updated_at")); // 同步刷新（铁律）
        assert_eq!(d.get_str("domain_attributes.referred_card_id").ok(), Some("c1"));
    }
```

`src/models.rs` 的 `referral_card_compat_tests` 加一条（验证旧 ConversationMessage 无新字段可反序列化）：

```rust
    #[test]
    fn legacy_conversation_message_without_msg_type_deserializes() {
        use super::ConversationMessage;
        let legacy = doc! {
            "workspace_id": "ws", "account_id": "a", "contact_wxid": "wx",
            "direction": "outbound", "content": "hi", "created_at": DateTime::now(),
        };
        let m: ConversationMessage = mongodb::bson::from_document(legacy)
            .expect("legacy message must deserialize");
        assert_eq!(m.msg_type, None);
        assert_eq!(m.media_ref, None);
    }
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --lib agent::referral && cargo test --lib referral_card_compat_tests`
Expected: 编译失败。

- [ ] **Step 3: ConversationMessage 加字段**

`src/models.rs` 的 `ConversationMessage`（在 `raw` 与 `created_at` 之间）加：

```rust
    /// 出站消息类型："text"(默认/缺省) | "namecard"。供前端渲染名片卡片。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub msg_type: Option<String>,
    /// 名片消息引用的 referral_cards._id（hex），前端据此显示引荐了谁。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_ref: Option<String>,
```

补齐所有 `ConversationMessage {` 构造点的 `msg_type: None, media_ref: None,`（grep 找全，含 gateway.rs:1897、gateway.rs:1972 等）。

- [ ] **Step 4: 实现 build_referred_set_doc + send_outbound_namecard**

`src/agent/referral.rs`（use 区补 `crate::routes::AppState`、`crate::models::{Contact, ConversationMessage, MessageDirection, REFERRED_SPECIALIST_AT_ATTR, REFERRED_CARD_ID_ATTR}`、`crate::error::{AppError, AppResult}`、`crate::mcp`、`serde_json::{json, Value}`、`mongodb::bson::{doc, oid::ObjectId, to_document, DateTime, Document}`）：

```rust
/// 置「已引荐」态的 $set 子文档（dotted-key，不覆盖其它 domain_attributes）。
pub(crate) fn build_referred_set_doc(card_id: &str, now: DateTime) -> Document {
    doc! {
        format!("domain_attributes.{}", REFERRED_SPECIALIST_AT_ATTR): now,
        format!("domain_attributes.{}", REFERRED_CARD_ID_ATTR): card_id,
        "domain_attributes_updated_at": now,
        "updated_at": now,
    }
}

/// 发送名片给客户。调用方（dispatcher）已确保经 outbox 幂等。
/// 发送成功后落 conversation_messages(msg_type=namecard) + 置「已引荐」态。
/// MCP 入参字段名以 server tools/list 为准（见计划 Task 8）；此处用占位形态。
pub(crate) async fn send_outbound_namecard(
    state: &AppState,
    contact: &Contact,
    card_id: &str,
) -> AppResult<Value> {
    let oid = ObjectId::parse_str(card_id)
        .map_err(|_| AppError::BadRequest("bad referral card_id".into()))?;
    let card = state.db.referral_cards()
        .find_one(doc! { "_id": oid }, None).await?
        .ok_or_else(|| AppError::BadRequest("referral card not found".into()))?;
    // 发送前准入二次校验（防 AI 幻觉/已撤下名片漏到发送）
    if !validate_card_sendable(&card) {
        return Err(AppError::BadRequest("referral card not sendable (draft/disabled)".into()));
    }

    // ⚠️ MCP message_send_namecard 入参字段名待 server tools/list 确认，此处占位。
    let response = crate::mcp::logged_call_for_account(
        state,
        &contact.account_id,
        "message_send_namecard",
        json!({ "recipient": contact.wxid, "targetWxid": card.target_wxid }),
    ).await?;

    let now = DateTime::now();
    // MCP 已成功 = 名片已送达，此后 DB 写失败不得返 Err（同 send_outbound_message 纪律，
    // 否则 dispatcher retry 会重发名片给客户）。
    let raw = to_document(&response).ok();
    if let Err(err) = state.db.messages().insert_one(
        ConversationMessage {
            id: None,
            workspace_id: contact.workspace_id.clone(),
            account_id: contact.account_id.clone(),
            contact_wxid: contact.wxid.clone(),
            message_id: response.get("newMsgId").and_then(|v| v.as_str()).map(ToString::to_string),
            dedupe_key: None,
            direction: MessageDirection::Outbound,
            content: String::new(),
            raw,
            msg_type: Some("namecard".into()),
            media_ref: Some(card_id.to_string()),
            created_at: now,
        },
        None,
    ).await {
        tracing::error!(error = %err, contact_wxid = %contact.wxid,
            "namecard sent but persisting conversation_messages failed");
    }
    // 置「已引荐」态
    if let Err(err) = state.db.contacts().update_one(
        doc! { "_id": contact.id },
        doc! { "$set": build_referred_set_doc(card_id, now) },
        None,
    ).await {
        tracing::error!(error = %err, contact_wxid = %contact.wxid,
            "namecard sent but marking referred state failed");
    }
    Ok(response)
}
```

- [ ] **Step 5: 运行确认通过 + lib 不回归**

Run: `cargo test --lib agent::referral && cargo test --lib 2>&1 | tail -5`
Expected: 全过；passed ≥ 350。

- [ ] **Step 6: Commit**

```bash
git add src/models.rs src/agent/referral.rs
git commit -m "feat(referral-card): ConversationMessage媒体字段+send_outbound_namecard+置已引荐态"
```

---

### Task 7: dispatcher 按 referral_card_id 分流 + 崩溃恢复适配

**Files:**
- Modify: `src/agent/outbox_dispatcher.rs`（发送点 :556 分流；崩溃恢复 post-hoc 核对 :547 前适配）
- Test: `src/agent/outbox_dispatcher.rs` 内联测试（分流判定纯函数，若可抽）

**Interfaces:**
- Consumes: `OutboxEntry.referral_card_id`（Task 5）、`send_outbound_namecard`（Task 6）、`send_outbound_message`（现有）
- Produces: dispatcher 对名片条目调 `send_outbound_namecard`

- [ ] **Step 1: 发送点分流**

`src/agent/outbox_dispatcher.rs` 把 :555-556 的发送点：

```rust
    let send_fut =
        super::gateway::send_outbound_message(state, &contact, &entry.content, extra_raw);
```

改为按条目类型分流：

```rust
    let send_result = if let Some(card_id) = entry.referral_card_id.as_deref() {
        tokio::time::timeout(
            Duration::from_secs(MCP_SEND_TIMEOUT_SECONDS),
            super::referral::send_outbound_namecard(state, &contact, card_id),
        ).await
    } else {
        tokio::time::timeout(
            Duration::from_secs(MCP_SEND_TIMEOUT_SECONDS),
            super::gateway::send_outbound_message(state, &contact, &entry.content, extra_raw),
        ).await
    };
```

（保持原有 `match send_result { Ok(Ok(_)) => ... }` 结构不变；只是把 send_fut 构造换成分流。`extra_raw` 仅 text 分支用，名片分支不需要——若编译报 extra_raw 未用，把它的构造移进 else 分支。）

- [ ] **Step 2: 崩溃恢复 post-hoc 核对适配**

`mcp_already_succeeded`（:392）按 `tool_name=message_send_text` + `request.content` 匹配，对名片条目（空 content、工具不同）会恒返 false。崩溃恢复路径（:505 区 reclaimed_in_flight 分支 + :609 timeout 分支）对名片条目须**跳过 post-hoc 核对**——名片条目走 retry 重发的风险是"重复发名片"，但 reclaimed 路径已是边缘场景；保守做法是名片条目崩溃恢复时不做 text 形态核对、直接按正常 retry（重发名片的概率极低，且名片重复推送危害小于文本重复）。

在两处调用 `mcp_already_succeeded` 前加守卫：

```rust
    // 名片条目不适用 text 形态的 post-hoc 核对（content 空、tool 不同），跳过。
    if entry.referral_card_id.is_none() {
        if let Ok(true) = mcp_already_succeeded(state, &entry.account_id, &entry.contact_wxid, &entry.content, entry.created_at).await {
            // ... 原有标 sent 逻辑
        }
    }
```

（精确改法：把现有 `if let Ok(true) = mcp_already_succeeded(...)` 的两处包进 `if entry.referral_card_id.is_none() { ... }`。实现时按现有代码块结构对齐。）

- [ ] **Step 3: 编译 + lib 不回归**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: passed ≥ 350。

- [ ] **Step 4: Commit**

```bash
git add src/agent/outbox_dispatcher.rs
git commit -m "feat(referral-card): dispatcher按referral_card_id分流+崩溃恢复核对跳过名片"
```

---

### Task 8: 候选名片注入 prompt + 已引荐态信号

**Files:**
- Modify: `src/agent/decision.rs`（`load_referral_cards` DB 查询；候选清单 + 已引荐态注入 prompt，:298 `load_context_assets` 调用点附近）
- Modify: `src/agent/escalation/logic.rs`（或 decision.rs）（`build_decision_signals_text`（logic.rs:206）加已引荐态指引）
- Test: `src/agent/decision.rs` 内联测试（load_referral_cards query 形状纯函数，仿 reaction_hint_filter_tests）

**Interfaces:**
- Consumes: `referral::{filter_referral_candidates, render_referral_lines, AlreadyReferred, assist_mode_active}`（Task 3）、`ReferralCard`、`referral_cards()` accessor（Task 2）
- Produces: `pub(crate) async fn load_referral_cards(state, account_id: &str) -> AppResult<Vec<ReferralCard>>`；prompt 注入了候选清单

- [ ] **Step 1: 写失败测试（query 形状纯函数）**

仿 decision.rs 现有 `reaction_hint_loader_tests`（:1059），把 `load_referral_cards` 的 filter 抽成纯函数 `build_referral_cards_filter(workspace_id, account_id) -> Document` 并测：

```rust
#[cfg(test)]
mod referral_loader_tests {
    use super::*;
    use mongodb::bson::doc;

    #[test]
    fn referral_filter_pins_workspace_account_enabled_approved() {
        let f = build_referral_cards_filter("ws", "acct");
        assert_eq!(f.get_str("workspace_id").ok(), Some("ws"));
        assert_eq!(f.get_bool("enabled").ok(), Some(true));
        assert_eq!(f.get_str("review_status").ok(), Some("approved"));
        // account 维度：null 或 =account（$or）
        assert!(f.contains_key("$or"));
    }
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --lib referral_loader_tests`
Expected: 编译失败。

- [ ] **Step 3: 实现 load_referral_cards + filter 纯函数**

`src/agent/decision.rs`（仿 `load_context_assets` :1025）：

```rust
pub(crate) fn build_referral_cards_filter(workspace_id: &str, account_id: &str) -> mongodb::bson::Document {
    use mongodb::bson::doc;
    doc! {
        "workspace_id": workspace_id,
        "$or": [ { "account_id": null }, { "account_id": account_id } ],
        "enabled": true,
        "review_status": "approved",
    }
}

pub(crate) async fn load_referral_cards(
    state: &AppState,
    account_id: &str,
) -> AppResult<Vec<crate::models::ReferralCard>> {
    use futures::TryStreamExt;
    use mongodb::options::FindOptions;
    let filter = build_referral_cards_filter(&state.config.default_workspace_id, account_id);
    let mut cursor = state.db.referral_cards().find(
        filter,
        FindOptions::builder().sort(mongodb::bson::doc! { "updated_at": -1 }).limit(20).build(),
    ).await?;
    let mut out = Vec::new();
    while let Some(c) = cursor.try_next().await? { out.push(c); }
    Ok(out)
}
```

- [ ] **Step 4: 注入 prompt（仅辅助模式生效）**

在 decision.rs 组装 prompt 处（:298 `let assets = load_context_assets(...)` 之后），加：

```rust
    // 辅助模式才加载/注入名片候选。customer_stage 从 contact.domain_attributes 读（同 :640 现有逻辑）。
    let assist_override = contact.domain_attributes.as_ref()
        .and_then(|d| d.get_str(crate::models::ASSIST_MODE_OVERRIDE_ATTR).ok());
    let assist_on = crate::agent::referral::assist_mode_active(
        domain_config.and_then(|c| c.assist_mode_enabled),
        assist_override,
    );
    let referral_block = if assist_on {
        let cards = load_referral_cards(state, &contact.account_id).await?;
        let customer_stage = contact.domain_attributes.as_ref()
            .and_then(|d| d.get_str("customer_stage").ok());
        let candidates = crate::agent::referral::filter_referral_candidates(&cards, customer_stage);
        // 已引荐态：读 referred_card_id + 在 cards 里找 display_name
        let already = contact.domain_attributes.as_ref()
            .and_then(|d| d.get_str(crate::models::REFERRED_CARD_ID_ATTR).ok())
            .and_then(|cid| cards.iter().find(|c| c.id.map(|i| i.to_hex()).as_deref() == Some(cid))
                .map(|c| crate::agent::referral::AlreadyReferred {
                    display_name: c.display_name.clone(), card_id: cid.to_string(),
                }));
        crate::agent::referral::render_referral_lines(&candidates, already.as_ref())
    } else {
        String::new()
    };
```

把 `referral_block` 拼进 user prompt 的业务上下文层（与 `assets` 同一 `format!`，紧邻"可引用内容资产"段后加一段"{referral_block}"占位）。

- [ ] **Step 5: 已引荐态被动答疑指引**

`build_decision_signals_text`（`src/agent/escalation/logic.rs:206`，仿其读 `AWAITING_PRINCIPAL_DECISION_ATTR` 的写法）加：读到 `REFERRED_SPECIALIST_AT_ATTR` 存在时，push 一行指引：

```rust
    if contact.domain_attributes.as_ref()
        .map(|d| d.contains_key(crate::models::REFERRED_SPECIALIST_AT_ATTR))
        .unwrap_or(false)
    {
        lines.push("【已引荐】本客户已引荐给专属顾问，你退为辅助：客户再问就正常答疑，不再主动推进成交、不重复引荐（除非客户出现与上次完全不同的新需求场景）。".to_string());
    }
```

（该函数输出已注入 prompt——见 decision.rs:627 现有注入点，无需额外接线。）

- [ ] **Step 6: 运行确认通过 + lib 不回归**

Run: `cargo test --lib referral_loader_tests && cargo test --lib 2>&1 | tail -5`
Expected: 全过；passed ≥ 350。

- [ ] **Step 7: no-human-takeover lint 自检**

Run: `bash scripts/check-no-human-takeover.sh origin/main HEAD`（或 `grep -rnE "人工接管|takeover|hand-?off|人工介入|人工托管|接管|人工" src/agent/decision.rs src/agent/referral.rs src/agent/escalation/logic.rs`）
Expected: 无命中。

- [ ] **Step 8: Commit**

```bash
git add src/agent/decision.rs src/agent/escalation/logic.rs
git commit -m "feat(referral-card): 候选名片注入prompt(仅辅助模式)+已引荐态被动答疑指引"
```

---

### Task 9: gateway 把 namecard_to_send 转 outbox 名片条目

**Files:**
- Modify: `src/agent/gateway.rs`（approved 路径，文本分段 enqueue 循环（:1768-1819）之后加名片条目入队 + 准入校验）
- Test: `src/agent/gateway.rs` 内联测试（准入校验已在 referral.rs Task 3 测过；这里测"未开辅助模式时名片指令被忽略"的判定，若可纯函数化）

**Interfaces:**
- Consumes: `decision.namecard_to_send`（Task 4）、`referral::{assist_mode_active, validate_card_sendable}`（Task 3）、`outbox::{enqueue, EnqueueRequest}`（Task 5）、`referral_cards()` accessor
- Produces: approved 路径在文本之后入队名片 outbox 条目

- [ ] **Step 1: 在文本 enqueue 循环后加名片入队**

`src/agent/gateway.rs`，在文本分段 enqueue 循环（:1819 `}` 之后、:1820 `if !enqueue_errors.is_empty()` 之前或之后均可，但须在 approved 块内）加：

```rust
    // 名片引荐：辅助模式开启 + AI 输出了 namecard_to_send + 准入校验通过 → 入队名片 outbox 条目。
    // 追加在文本之后 = 先发铺垫话术、后发名片（D5）。错误不阻断已入队的文本。
    if let Some(directive) = final_decision.namecard_to_send.as_ref() {
        let assist_override = contact.domain_attributes.as_ref()
            .and_then(|d| d.get_str(crate::models::ASSIST_MODE_OVERRIDE_ATTR).ok());
        let assist_on = crate::agent::referral::assist_mode_active(
            domain_config.as_ref().and_then(|c| c.assist_mode_enabled),
            assist_override,
        );
        if assist_on {
            // 准入二次校验：card 必须真实存在 + enabled + approved（防 AI 幻觉 card_id）
            let card = match mongodb::bson::oid::ObjectId::parse_str(&directive.card_id) {
                Ok(oid) => state.db.referral_cards().find_one(doc! { "_id": oid }, None).await.ok().flatten(),
                Err(_) => None,
            };
            match card {
                Some(c) if crate::agent::referral::validate_card_sendable(&c) => {
                    let req = EnqueueRequest {
                        workspace_id: contact.workspace_id.clone(),
                        account_id: contact.account_id.clone(),
                        contact_wxid: contact.wxid.clone(),
                        run_id: run_id.clone(),
                        decision_id: Some(decision_review_id),
                        source_event_id: format!("{source_event_id}#namecard"),
                        source_kind: trigger.kind().to_string(),
                        content: String::new(),
                        referral_card_id: Some(directive.card_id.clone()),
                        max_attempts: 3,
                    };
                    if let Err(err) = outbox_enqueue(state, req).await {
                        tracing::warn!(error = %err, contact_wxid = %contact.wxid, "名片条目入队失败（不阻断已发文本）");
                    }
                }
                _ => {
                    write_event_for_account(state, &contact.account_id, Some(&contact.wxid),
                        "referral_card_rejected", "warn",
                        "AI 选的名片不存在/未审/已停用，已跳过引荐", None).await.ok();
                }
            }
        }
    }
```

（变量 `run_id`/`contact`/`source_event_id`/`trigger`/`decision_review_id`/`domain_config` 以该作用域现有绑定为准——它们都在文本 enqueue 循环同作用域内，已在 Task 7 核实可用。`EnqueueRequest` 含 Task 5 加的 `referral_card_id` 字段。）

- [ ] **Step 2: 编译 + lib 不回归**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: passed ≥ 350。

- [ ] **Step 3: no-human-takeover lint 自检**

Run: `grep -nE "人工接管|takeover|hand-?off|人工介入|人工托管|接管|人工" src/agent/gateway.rs | grep -iE "referral|namecard|引荐|专属顾问"`
Expected: 无命中（新增行不含禁词）。

- [ ] **Step 4: Commit**

```bash
git add src/agent/gateway.rs
git commit -m "feat(referral-card): gateway转namecard_to_send为outbox名片条目(辅助模式+准入校验)"
```

---

### Task 10: prompt 引荐指引 + bump PROMPT_PACK_VERSION

**Files:**
- Modify: `src/prompts.rs`（`user.reply.task` 字面量加引荐指引；bump `PROMPT_PACK_VERSION`（:15））
- Test: `src/prompts.rs` 内联测试（若有 prompt pack 测试则确认版本号变化；否则确认字面量含关键指引串）

**Interfaces:**
- Consumes: 无（纯文案 + 版本号）
- Produces: decision prompt 含专属顾问引荐指引

- [ ] **Step 1: 加引荐指引到 user.reply.task**

`src/prompts.rs` 找到 `user.reply.task` 的 `PromptSpec` 字面量（escalation 指引文案在 :1182-1200 附近），在合适位置（escalation 指引之后）加（**确认不含禁词**）：

```
【专属顾问引荐】仅当 prompt 中出现「可引荐的专属顾问」候选清单时（=本账号启用辅助模式），你可按需选择引荐给客户，输出到 namecardToSend（{cardId, reason}）。规则：
- 只在客户真正契合某顾问的触发提示时引荐（如明确要签约/要到店参观/深入技术细节），没有契合的就不引荐（namecardToSend 留空），不要为引荐而引荐。
- 引荐时 replyText 先用你自己的口吻做一句自然铺垫（如"这块我请我们负责人直接跟您对接，更高效"），名片会随后自动附上。
- 只能选候选清单里列出的 cardId，不要编造。
- 看到【已引荐】信号时：客户已引荐过，正常答疑即可，不再主动推进成交、不重复引荐（除非出现与上次完全不同的新需求场景）。
```

- [ ] **Step 2: bump PROMPT_PACK_VERSION**

`src/prompts.rs:15` 的 `PROMPT_PACK_VERSION` 常量 bump（如 `..._v3_2026_05_22` → `..._v4_2026_06_21`），否则 `ensure_prompt_pack_v2` 版本门控不重种、指引不生效。

- [ ] **Step 3: 编译 + lib 不回归**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: passed ≥ 350（注意：若有测试断言旧 PROMPT_PACK_VERSION 字符串，同步更新）。

- [ ] **Step 4: no-human-takeover lint 自检**

Run: `grep -nE "人工接管|takeover|hand-?off|人工介入|人工托管|接管|人工" src/prompts.rs`
Expected: 新增引荐指引段无命中（prompts.rs 虽不在 lint 扫描目录，仍自检保险）。

- [ ] **Step 5: Commit**

```bash
git add src/prompts.rs
git commit -m "feat(referral-card): decision prompt加专属顾问引荐指引+bump PROMPT_PACK_VERSION"
```

---

### Task 11: 名片库 CRUD + 审核 API

**Files:**
- Create: `src/routes/referral_cards.rs`
- Modify: `src/routes/mod.rs`（`mod referral_cards;` + 挂载路由）
- Test: `src/routes/referral_cards.rs` 内联测试（请求体校验纯函数）

**Interfaces:**
- Consumes: `ReferralCard`（Task 1）、`referral_cards()` accessor（Task 2）、`AuthenticatedAdmin`（现有 auth）
- Produces:
  - `POST /api/referral-cards`（创建 draft 名片）→ `{ id }`
  - `GET /api/referral-cards`（列表）
  - `POST /api/referral-cards/:id/review`（`{status: "approved"|"draft", note?}`）
  - `POST /api/referral-cards/:id/toggle`（`{enabled: bool}`）
  - `DELETE /api/referral-cards/:id`

- [ ] **Step 1: 实现 handler**

新建 `src/routes/referral_cards.rs`（参照现有 route handler 形态，如 `src/routes/assets.rs` 或 `contacts.rs`——grep 确认 `AuthenticatedAdmin` 提取器与 `AppState` 用法）：

```rust
//! 名片库：专属顾问名片 CRUD + 审核/启停。AI 不自我核验红线——创建默认 draft。
use axum::{extract::{Path, State}, Extension, Json};
use mongodb::bson::{doc, oid::ObjectId, DateTime};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{auth::AuthenticatedAdmin, error::{AppError, AppResult}, models::ReferralCard};
use super::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreateReferralCard {
    target_wxid: String,
    display_name: String,
    #[serde(default)] send_trigger_hint: String,
    #[serde(default)] target_stages: Vec<String>,
    #[serde(default)] account_id: Option<String>,
}

pub(super) async fn create_referral_card(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(body): Json<CreateReferralCard>,
) -> AppResult<Json<Value>> {
    if body.target_wxid.trim().is_empty() || body.display_name.trim().is_empty() {
        return Err(AppError::BadRequest("target_wxid 和 display_name 必填".into()));
    }
    let now = DateTime::now();
    let card = ReferralCard {
        id: None,
        workspace_id: admin.current_workspace.clone(),
        account_id: body.account_id,
        target_wxid: body.target_wxid,
        display_name: body.display_name,
        send_trigger_hint: body.send_trigger_hint,
        target_stages: body.target_stages,
        enabled: false,                  // 默认停用，审核+启用后才生效
        review_status: "draft".into(),   // AI 不自我核验红线
        review_note: None,
        created_at: now,
        updated_at: now,
    };
    let res = state.db.referral_cards().insert_one(card, None).await?;
    Ok(Json(json!({ "id": res.inserted_id.as_object_id().map(|i| i.to_hex()) })))
}

pub(super) async fn list_referral_cards(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
) -> AppResult<Json<Value>> {
    use futures::TryStreamExt;
    let mut cursor = state.db.referral_cards()
        .find(doc! { "workspace_id": &admin.current_workspace }, None).await?;
    let mut items = Vec::new();
    while let Some(c) = cursor.try_next().await? { items.push(c); }
    Ok(Json(json!({ "items": items })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReviewBody { status: String, note: Option<String> }

pub(super) async fn review_referral_card(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(body): Json<ReviewBody>,
) -> AppResult<Json<Value>> {
    if !matches!(body.status.as_str(), "approved" | "draft") {
        return Err(AppError::BadRequest("status must be approved|draft".into()));
    }
    let oid = ObjectId::parse_str(&id).map_err(|_| AppError::BadRequest("bad id".into()))?;
    let res = state.db.referral_cards().update_one(
        doc! { "_id": oid, "workspace_id": &admin.current_workspace },
        doc! { "$set": { "review_status": &body.status, "review_note": body.note.clone(), "updated_at": DateTime::now() }},
        None,
    ).await?;
    if res.matched_count == 0 { return Err(AppError::BadRequest("card not found".into())); }
    Ok(Json(json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ToggleBody { enabled: bool }

pub(super) async fn toggle_referral_card(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(body): Json<ToggleBody>,
) -> AppResult<Json<Value>> {
    let oid = ObjectId::parse_str(&id).map_err(|_| AppError::BadRequest("bad id".into()))?;
    let res = state.db.referral_cards().update_one(
        doc! { "_id": oid, "workspace_id": &admin.current_workspace },
        doc! { "$set": { "enabled": body.enabled, "updated_at": DateTime::now() }},
        None,
    ).await?;
    if res.matched_count == 0 { return Err(AppError::BadRequest("card not found".into())); }
    Ok(Json(json!({ "ok": true })))
}

pub(super) async fn delete_referral_card(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let oid = ObjectId::parse_str(&id).map_err(|_| AppError::BadRequest("bad id".into()))?;
    state.db.referral_cards().delete_one(
        doc! { "_id": oid, "workspace_id": &admin.current_workspace }, None).await?;
    Ok(Json(json!({ "ok": true })))
}
```

（`AuthenticatedAdmin.current_workspace` 字段名以现有 auth 为准——grep `current_workspace` 确认；若不同则对齐。`AppError` 变体以 src/error.rs 为准。）

- [ ] **Step 2: 挂载路由**

`src/routes/mod.rs` 加 `mod referral_cards;`，在合适位置注册：

```rust
        .route("/referral-cards", post(referral_cards::create_referral_card).get(referral_cards::list_referral_cards))
        .route("/referral-cards/:id/review", post(referral_cards::review_referral_card))
        .route("/referral-cards/:id/toggle", post(referral_cards::toggle_referral_card))
        .route("/referral-cards/:id", axum::routing::delete(referral_cards::delete_referral_card))
```

- [ ] **Step 3: 编译 + lib 不回归**

Run: `cargo check 2>&1 | tail -15 && cargo test --lib 2>&1 | tail -5`
Expected: 编译通过；passed ≥ 350。

- [ ] **Step 4: no-human-takeover lint 自检**

Run: `grep -nE "人工接管|takeover|hand-?off|人工介入|人工托管|接管|人工" src/routes/referral_cards.rs`
Expected: 无命中。

- [ ] **Step 5: Commit**

```bash
git add src/routes/referral_cards.rs src/routes/mod.rs
git commit -m "feat(referral-card): 名片库CRUD+审核+启停API(创建默认draft)"
```

---

### Task 12: 前端名片库管理 + 辅助模式开关 + 对话名片渲染

**Files:**
- Create: `frontend/src/features/referral-cards/index.tsx`（名片库管理页）
- Modify: `frontend/src/lib/api.ts`（若无对应 api 封装则加 referral-cards 调用）
- Modify: 账号配置页（加 `assist_mode_enabled` 开关——grep 找现有 OperationDomainConfig/账号配置编辑组件）
- Modify: `frontend/src/types/index.ts`（`Message` 加 `msgType`/`mediaRef`）
- Modify: 对话消息渲染处（grep `message.content` 找气泡渲染点，按 msgType 分支）

**Interfaces:**
- Consumes: Task 11 的 5 个 API；后端 `ConversationMessage.msg_type`/`media_ref`（Task 6）
- Produces: 名片库 UI + 辅助模式开关 UI + 对话名片卡片渲染

- [ ] **Step 1: 名片库管理页**

新建 `frontend/src/features/referral-cards/index.tsx`，遵循 `docs/frontend-design-system.md` 企业白色基调。功能：
- 录入表单：display_name、target_wxid（可结合现有 contacts 搜索选好友）、send_trigger_hint(textarea，自然语言)、target_stages(多选/逗号)、accountId(可选)；
- 列表：每行显示 display_name / target_wxid / review_status(draft/approved) / enabled；
- 操作：draft 行"标记为可引荐(approved)"按钮 → `POST /referral-cards/:id/review {status:"approved"}`；"启用/停用"开关 → `/toggle`；删除。

```tsx
// 关键 action（接 Task 11 API）
async function createCard(form: { targetWxid: string; displayName: string; sendTriggerHint: string; targetStages: string[]; }) {
  return api.post("/api/referral-cards", form);
}
async function approveCard(id: string) {
  return api.post(`/api/referral-cards/${id}/review`, { status: "approved" });
}
async function toggleCard(id: string, enabled: boolean) {
  return api.post(`/api/referral-cards/${id}/toggle`, { enabled });
}
```

（措辞用"专属顾问名片 / 引荐 / 待审核 / 可引荐"，**不得**出现"转人工/接管"等禁词——frontend/src 在 lint 扫描范围内。）

- [ ] **Step 2: 辅助模式开关**

在账号/运营域配置编辑组件加一个开关绑定 `assist_mode_enabled`（写回 OperationDomainConfig 的现有 PUT/PATCH 路径——grep 找现有 domain config 编辑 API）。配说明文案："开启后，AI 会在客户契合引荐条件时主动把专属顾问名片推送给客户。"

- [ ] **Step 3: Message 类型 + 对话渲染**

`frontend/src/types/index.ts` 的 `Message` 接口加：

```ts
  msgType?: "text" | "namecard";
  mediaRef?: string;
```

对话气泡渲染处（grep `message.content` 找渲染点）按 msgType 分支：

```tsx
{message.msgType === "namecard" ? (
  <div className="namecard-bubble">已为客户引荐专属顾问</div>
) : (
  <p>{message.content}</p>
)}
```

- [ ] **Step 4: 前端构建验证**

Run: `cd frontend && npm run build 2>&1 | tail -15`
Expected: 构建成功，无 TS 错误。

- [ ] **Step 5: no-human-takeover lint 自检（前端在扫描范围）**

Run: `grep -rnE "人工接管|takeover|hand-?off|人工介入|人工托管|接管|人工" frontend/src/features/referral-cards/`
Expected: 无命中。

- [ ] **Step 6: Commit**

```bash
git add frontend/src/features/referral-cards frontend/src/types/index.ts frontend/src/lib/api.ts
git commit -m "feat(referral-card): 前端名片库管理+辅助模式开关+对话名片渲染"
```

---

### Task 13: 端到端集成测试（CI）

**Files:**
- Create: `tests/referral_card_push_integration.rs`（`#[ignore]`，CI integration job 跑）

**Interfaces:**
- Consumes: 全链路（Task 1-11）

- [ ] **Step 1: 写集成测试**

新建 `tests/referral_card_push_integration.rs`（用项目现有 testcontainers helper 起 Mongo + AppState——参照 tests/ 其它 `#[ignore]` 集成测试 setup）：

```rust
//! 名片引荐端到端：审核门 + outbox 名片条目幂等。需 Docker(testcontainers Mongo)，
//! 默认 #[ignore]，CI integration job 跑。
#![cfg(test)]

#[tokio::test]
#[ignore = "requires docker mongo"]
async fn only_approved_enabled_card_is_loadable() {
    // 1. insert draft+disabled 名片 → load_referral_cards 不返回它
    // 2. update enabled=true + review_status="approved" → load_referral_cards 返回它
    // 3. filter_referral_candidates(.., Some("意向")) 命中；Some("已成交") 不命中
}

#[tokio::test]
#[ignore = "requires docker mongo"]
async fn namecard_outbox_entry_idempotent_per_card() {
    // enqueue 两次同 (run_id, contact, referral_card_id) → 第二次 IdempotentSkip
    // enqueue 同 run 不同 referral_card_id → 两条都 Created（验证幂等键含 card_id）
}

#[tokio::test]
#[ignore = "requires docker mongo"]
async fn assist_mode_off_account_ignores_namecard_directive() {
    // assist_mode_enabled=None 的账号：即使 decision.namecard_to_send 有值，
    // gateway 也不入队名片条目（assist_mode_active=false 短路）
}
```

- [ ] **Step 2: 本地编译验证（不跑 ignored）**

Run: `cargo test --test referral_card_push_integration --no-run 2>&1 | tail -5`
Expected: 编译通过（CI integration job 带 `--ignored` 真跑）。

- [ ] **Step 3: Commit**

```bash
git add tests/referral_card_push_integration.rs
git commit -m "test(referral-card): 端到端审核门+outbox名片幂等+辅助模式短路集成测试(CI)"
```

---

### Task 14: CLAUDE.md + agent-policy.md 红线文档化

**Files:**
- Modify: `CLAUDE.md`（"无人工接管的精确含义"段加辅助模式受控例外）
- Modify: `docs/agent-policy.md`（新增"辅助模式/专属顾问引荐"小节）

**Interfaces:**
- Consumes: 无（文档任务）
- Produces: 红线例外的书面依据，避免未来误判为红线违规

- [ ] **Step 1: 改 CLAUDE.md 红线段**

在 CLAUDE.md 的"无人工接管的精确含义"段（:23 附近）后补一段（**注意 CLAUDE.md 不在 lint 扫描目录，但仍用清晰中性表述**）：

```
**辅助模式（账号级可选，默认关）的受控例外**：当账号显式开启「辅助模式」且 AI 判定客户契合人类预先标注的引荐条件（如明确要签约/到店参观）时，AI 会主动把真人专属顾问的微信名片推送给客户，由客户与顾问对接完成临门一脚。此时 AI 退为辅助答疑角色。这是管理员显式配置的业务动作、AI 仍是发起方与辅助方，不改变全自治模式（默认）下"客户永远只跟 AI 对话"的红线——后者一字不动。详见 docs/superpowers/specs/2026-06-21-referral-card-push-design.md。
```

- [ ] **Step 2: 改 docs/agent-policy.md**

新增"辅助模式 / 专属顾问引荐"小节，写清：触发依据（人类标注 send_trigger_hint 注入 prompt，非关键词）、开关粒度（账号级 assist_mode_enabled + 客户级 override，默认关）、「已引荐」态语义（转被动答疑）、与全自治模式的关系（受控例外）、被推真人 ≠ 幕后 principal_decider。

- [ ] **Step 3: Commit**

```bash
git add CLAUDE.md docs/agent-policy.md
git commit -m "docs(referral-card): CLAUDE.md+agent-policy.md记录辅助模式红线受控例外"
```

---

## Self-Review

**1. Spec coverage（逐节核对 spec → task）：**
- spec §4.1 ReferralCard 模型 → Task 1 ✓
- spec §4.2 账号级/客户级开关 → Task 1（字段+常量）+ Task 3（assist_mode_active 判定）✓
- spec §4.3 AgentDecision 三处接线 → Task 4 ✓
- spec §4.4 outbox 名片条目（空 content + card 幂等键）→ Task 5 ✓
- spec §5.1 候选注入（load + filter + render）→ Task 3（纯函数）+ Task 8（DB load + 注入）✓
- spec §5.2 prompt 引荐指引 + bump 版本 → Task 10 ✓
- spec §6.1 发送链路（dispatcher 分流 + send_outbound_namecard + 落库 + 已引荐态）→ Task 6（发送函数+落库+置态）+ Task 7（dispatcher 分流）✓
- spec §6.2 闸门（辅助模式开关/准入校验/防重推/频控）→ Task 9（开关+准入）+ Task 8（防重推注入）+ 频控复用现有 gateway ✓
- spec §6.3 「已引荐」态（转态 + 被动答疑）→ Task 6（置态）+ Task 8（被动答疑指引）✓
- spec §7 红线/命名 → 各 Task 命名守 AI 内部口径 + lint 自检步骤；CLAUDE.md/agent-policy.md 文档化 → Task 14 ✓
- spec §8 前端 → Task 12 ✓
- spec §9 配置/数据库 → Task 2（集合+索引）；无新 env（spec 明确名片无文件存储）✓
- spec §10 测试 → 各 Task 内联 + Task 13 集成 ✓
- spec §7.1 改 CLAUDE.md + docs/agent-policy.md 记录辅助模式红线例外 → Task 14 ✓

**2. Placeholder scan：** 无 TBD/TODO。MCP `message_send_namecard` 入参字段名（recipient/targetWxid）标注为"以 server tools/list 为准、实现时对齐"，属已知未决（Global Constraints + spec §12 记录），非占位。

**3. Type consistency：** `ReferralCard`（Task 1，Task 2/3/6/8/11 消费）、`NamecardDirective`（Task 4，Task 9 消费）、`namecard_to_send`（Task 4，Task 9 读）、`referral_card_id`（Task 5，Task 7/9 用）、`assist_mode_active(account_enabled, override_attr)`（Task 3，Task 8/9 调）、`validate_card_sendable`（Task 3，Task 6/9 调）、`send_outbound_namecard(state, contact, card_id)`（Task 6，Task 7 调）、`build_referred_set_doc`（Task 6）、`load_referral_cards`/`build_referral_cards_filter`（Task 8）、`content_required_for`/`compute_synthetic_key_with_card`（Task 5）— 跨任务签名一致。

**4. 已知实现期对齐点（非占位，需实现者 grep 确认）：** `AppError` 变体名（BadRequest 以 src/error.rs 为准）；`AuthenticatedAdmin.current_workspace` 字段名；`UserRuntimeParameters` 构造方式（Task 4 测试）；gateway approved 作用域变量名（run_id/decision_review_id/source_event_id/domain_config）；所有 `EnqueueRequest {`/`ConversationMessage {`/`OperationDomainConfig {` 构造点补新字段。每个 Task 的编译步骤会暴露这些。
