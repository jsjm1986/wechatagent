# 主动发送台账（agent_send_ledger）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为素材库 + 专属顾问名片引荐两个对称功能建一张共享的"发送事实表" `agent_send_ledger`，闭合缺口 1（效果追踪/统计）与缺口 5（跨 run 防重发软约束）。

**Architecture:** 新建 `agent_send_ledger` 集合，每次 AI 主动发送（素材/名片）MCP 成功后紧贴现有 `ConversationMessage` 落库处 fail-soft 写一条；转化字段（响应率/阶段推进）由复用的 `tasks.rs` worker 异步回扫填充；新增只读 API + 独立「发送成效」前端频道 + decision prompt 已发历史注入。台账是旁路记录 + 只读消费，不改发送决策逻辑。

**Tech Stack:** Rust 2021 / Axum / MongoDB(mongodb crate) / React 19 + Vite + TS（CSS Modules + tokens 变量）。

设计来源：`docs/superpowers/specs/2026-06-21-agent-send-ledger-design.md`。

## Global Constraints

- **既成事实纪律红线**：MCP 发送成功后，台账落库失败**绝不返 Err**（只 `tracing::error!`）。返 Err 会让 dispatcher retry 重发，客户收重复文件/名片。与现有 `media_send.rs` / `referral.rs` 落 ConversationMessage 同纪律。
- **workspace_id scope 红线**：所有 ledger 查询/聚合/API 必须带 `workspace_id` 条件（防跨租户 IDOR）。
- **向后兼容红线**：`AgentSendLedger` 所有转化字段必须 `Option` + `#[serde(default)]`；转化字段全 None 的条目必须能反序列化。
- **agent-first / 不加硬门**：防重发只走 prompt 软约束，**不得**在发送路径加"发过就拦"的硬阈值/词表门。ledger 只喂 prompt 历史。
- **回扫无副作用**：回扫 worker 不调 LLM、不发消息，纯读 + 回写自己表。
- **no-human-takeover lint**：`scripts/check-no-human-takeover.sh` 扫 `src/agent/ src/routes/ src/evolution/ frontend/src/` 的 diff 新增行，禁词 `人工接管|takeover|hand-off|人工介入|人工托管|接管|人工`。命名用"发送成效/响应率/引荐/已发送"等中性词。
- **测试铁律**：纯函数确定性测试为主；不接受 skip 假绿；新增测试只 append 不删旧维度；不过拟合单条样本；baseline 不回归（`cargo test --lib` ≥350 passed/0 failed，4 个 PBT 累计 ≥33/0）。
- **前端设计语言**：新页严格遵循 `docs/frontend-design-system.md` + 对齐 `features/referral-cards/`：CSS Modules + tokens 变量（禁硬编码色值/间距），四级层级不嵌套卡片，表格不套卡片，sub-tab 不引第三级导航，白色企业控制台基调。
- **Shell**：bash on Windows，项目根含非 ASCII（`工作项目`），用绝对路径。本地只跑 `cargo test --lib` 和单个 PBT，全量集成留 CI。
- **Subagents**：本项目 spawn 的所有 subagent 必须 `model: "opus"`。

---

## File Structure

**后端新建：**
- `src/agent/send_ledger.rs` — 台账核心：`AgentSendLedger` 写入纯函数（构造条目）、转化回扫纯函数（`responded` 窗口判定、`stage_advanced` 阶段推进判定）、聚合率计算纯函数。配套内联 `#[cfg(test)]`。
- `src/routes/send_ledger.rs` — 3 个只读 API handler（单客户历史 / 维度聚合 / 总览）。

**后端修改：**
- `src/models.rs` — 新增 `AgentSendLedger` 结构体。
- `src/db/mod.rs` — `agent_send_ledger()` typed accessor（仿 `referral_cards()` :187）。
- `src/db/indexes.rs` — 3 个 ledger 索引（仿 `referral_cards` :217）。
- `src/agent/media_send.rs` — `send_outbound_media`（:192 ConversationMessage insert 后）紧贴写 ledger。
- `src/agent/referral.rs` — `send_outbound_namecard`（:129 ConversationMessage insert 后）紧贴写 ledger。
- `src/agent/mod.rs` — `mod send_ledger;` + 必要 re-export。
- `src/tasks.rs` — `tick`（:160）加 `scan_send_ledger_outcomes` 回扫步骤。
- `src/agent/decision.rs` — prompt 注入已发素材历史（对齐名片侧 `AlreadyReferred`，统一从 ledger 取）。
- `src/routes/mod.rs` — 挂载 send_ledger 路由 + `mod send_ledger;`（仿 referral_cards :62/:380）。

**前端修改：**
- `frontend/src/features/send-analytics/index.tsx` + `SendAnalytics.module.css` — 「发送成效」频道（总览卡 + 素材/名片效果两 sub-tab 排行榜表）。
- `frontend/src/stores/sendAnalyticsStore.ts` — 拉取 stats/overview。
- `frontend/src/app/channels.ts` + `frontend/src/types/index.ts` — 频道接线（Channel union 加 `sendAnalytics`）。
- `frontend/src/features/user-ops/legacy.tsx` — 客户页嵌"AI 已发送"只读历史小面板。

**测试新建：**
- 各模块内联 `#[cfg(test)]`。
- `tests/send_ledger_integration.rs`（`#[ignore]`，CI）— 写入 + 回扫 + API workspace scope。

---

## Task 1: AgentSendLedger 数据模型 + 向后兼容测试

**Files:**
- Modify: `src/models.rs`（新增 `AgentSendLedger`，靠近 `ContentAsset` :678 或 `ReferralCard`）
- Test: `src/models.rs` 内联 `#[cfg(test)] mod send_ledger_compat_tests`

**Interfaces:**
- Consumes: 无（地基任务）
- Produces: `pub struct AgentSendLedger { id, workspace_id, account_id, contact_wxid, send_kind: String, target_id: String, target_title: String, run_id: String, trigger_reason: Option<String>, customer_stage_at_send: Option<String>, sent_at: DateTime, responded: Option<bool>, response_window_hours: Option<i32>, stage_advanced: Option<bool>, outcome_evaluated_at: Option<DateTime> }`

- [ ] **Step 1: 写失败测试**

在 `src/models.rs` 末尾 `#[cfg(test)]` 区追加：

```rust
#[cfg(test)]
mod send_ledger_compat_tests {
    use super::AgentSendLedger;
    use mongodb::bson::{doc, DateTime};

    #[test]
    fn ledger_roundtrips() {
        let row = AgentSendLedger {
            id: None,
            workspace_id: "ws1".into(),
            account_id: "acct1".into(),
            contact_wxid: "wxid_cust".into(),
            send_kind: "media".into(),
            target_id: "asset1".into(),
            target_title: "报价单 2026".into(),
            run_id: "run1".into(),
            trigger_reason: Some("客户问报价".into()),
            customer_stage_at_send: Some("意向".into()),
            sent_at: DateTime::now(),
            responded: None,
            response_window_hours: None,
            stage_advanced: None,
            outcome_evaluated_at: None,
        };
        let d = mongodb::bson::to_document(&row).unwrap();
        let back: AgentSendLedger = mongodb::bson::from_document(d).unwrap();
        assert_eq!(back.send_kind, "media");
        assert_eq!(back.target_id, "asset1");
        assert!(back.responded.is_none());
    }

    #[test]
    fn legacy_row_without_outcome_fields_deserializes() {
        // 转化字段全缺的早期条目必须仍能反序列化（向后兼容红线）
        let legacy = doc! {
            "workspace_id": "ws1", "account_id": "a", "contact_wxid": "w",
            "send_kind": "namecard", "target_id": "c1", "target_title": "张顾问",
            "run_id": "r1", "sent_at": DateTime::now(),
        };
        let row: AgentSendLedger = mongodb::bson::from_document(legacy)
            .expect("legacy ledger row must deserialize");
        assert_eq!(row.send_kind, "namecard");
        assert!(row.responded.is_none());
        assert!(row.outcome_evaluated_at.is_none());
    }
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --lib send_ledger_compat_tests`
Expected: 编译失败（`AgentSendLedger` 未定义）。

- [ ] **Step 3: 加 AgentSendLedger 结构体**

在 `src/models.rs` 合适位置（靠近 `ReferralCard`）新增。用 `#[derive(Debug, Clone, Serialize, Deserialize)]`，snake_case 落库（与 ReferralCard 同款，不加 rename_all）：

```rust
/// 主动发送台账：每次 AI 主动发素材/名片成功后落一条。素材与名片共用
/// （send_kind 区分），供单客户历史 / 维度聚合统计 / prompt 已发历史注入。
/// 转化字段（responded/stage_advanced）发送时留空，由 tasks worker 回扫填充。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSendLedger {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub contact_wxid: String,
    /// "media" | "namecard"
    pub send_kind: String,
    /// asset_id 或 card_id（hex）
    pub target_id: String,
    /// 冗余快照：素材标题 / 顾问名（统计展示不回表，原实体改名/删除后历史仍可读）
    #[serde(default)]
    pub target_title: String,
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_reason: Option<String>,
    /// 发送瞬间客户阶段快照（阶段推进判断的"前值"）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub customer_stage_at_send: Option<String>,
    pub sent_at: DateTime,
    /// sent_at 后 response_window_hours 小时内是否有入站消息（回扫填）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub responded: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_window_hours: Option<i32>,
    /// 发送后 customer_stage 是否前进（回扫填）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_advanced: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome_evaluated_at: Option<DateTime>,
}
```

（确认 `ObjectId` / `DateTime` 已在 models.rs 顶部 use；它们是该文件通用导入，ReferralCard 已在用。）

- [ ] **Step 4: 运行确认通过 + 全 lib 不回归**

Run: `cargo test --lib send_ledger_compat_tests && cargo test --lib 2>&1 | tail -5`
Expected: 2 passed；全 lib passed ≥ 350。

- [ ] **Step 5: Commit**

```bash
git add src/models.rs
git commit -m "feat(send-ledger): AgentSendLedger模型(素材+名片共用,转化字段Option向后兼容)"
```

---

## Task 2: agent_send_ledger 集合 accessor + 索引

**Files:**
- Modify: `src/db/mod.rs`（加 `agent_send_ledger()` accessor，仿 `referral_cards()` :187）
- Modify: `src/db/indexes.rs`（`ensure_all` 加 3 个索引，仿 referral_cards :217）

**Interfaces:**
- Consumes: `AgentSendLedger`（Task 1）
- Produces: `pub fn agent_send_ledger(&self) -> Collection<AgentSendLedger>`

- [ ] **Step 1: 加 typed accessor**

`src/db/mod.rs`，在 `referral_cards()`（:187）附近追加（确认 `AgentSendLedger` 已在该文件 `use crate::models::{...}` 引入；若没有则补）：

```rust
    pub fn agent_send_ledger(&self) -> Collection<AgentSendLedger> {
        self.db.collection("agent_send_ledger")
    }
```

- [ ] **Step 2: 加索引**

`src/db/indexes.rs` 的 `ensure_all`，参照 `referral_cards`（:217）的 create_index 写法追加 3 个：

```rust
    // 单客户发送历史（按时间倒序）
    db.agent_send_ledger()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "contact_wxid": 1, "sent_at": -1 })
                .build(),
            None,
        )
        .await?;
    // 素材/名片维度聚合
    db.agent_send_ledger()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "send_kind": 1, "target_id": 1 })
                .build(),
            None,
        )
        .await?;
    // 回扫待处理（找 outcome_evaluated_at 缺失的条目）
    db.agent_send_ledger()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "outcome_evaluated_at": 1 })
                .build(),
            None,
        )
        .await?;
```

（确认 `IndexModel` / `doc!` 已在该文件 use；跟随现有 create_index 模式。无需 migration——首次 insert 自动建集合 + ensure_indexes 幂等。）

- [ ] **Step 3: 编译 + lib 不回归**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: 全绿，passed ≥ 350。

- [ ] **Step 4: Commit**

```bash
git add src/db/mod.rs src/db/indexes.rs
git commit -m "feat(send-ledger): agent_send_ledger 集合 accessor + 3 索引"
```

---

## Task 3: 转化判定纯函数（responded 窗口 + stage_advanced 推进）

**Files:**
- Create: `src/agent/send_ledger.rs`
- Modify: `src/agent/mod.rs`（加 `mod send_ledger;`）
- Test: `src/agent/send_ledger.rs` 内联 `#[cfg(test)]`

**Interfaces:**
- Consumes: 无（纯函数，与 DB 解耦）
- Produces:
  - `pub(crate) fn responded_within_window(sent_at_ms: i64, window_hours: i32, inbound_ms: &[i64]) -> bool` — 任一入站时间戳落在 (sent_at, sent_at+窗口] 内即 true
  - `pub(crate) fn stage_advanced(stage_at_send: Option<&str>, current_stage: Option<&str>, ordered_stages: &[String]) -> bool` — 当前阶段在有序阶段列表里严格靠后于发送时阶段
  - `pub(crate) fn response_rate(total: u64, responded: u64) -> f64` — total=0 返 0.0，否则 responded/total 保留 4 位

- [ ] **Step 1: 写失败测试**

新建 `src/agent/send_ledger.rs`，先写测试模块：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const HOUR_MS: i64 = 3_600_000;

    #[test]
    fn responded_true_when_inbound_in_window() {
        let sent = 1_000_000_000_000;
        // 窗口 24h，入站在 sent 后 2h → 命中
        assert!(responded_within_window(sent, 24, &[sent + 2 * HOUR_MS]));
    }

    #[test]
    fn responded_false_when_inbound_after_window() {
        let sent = 1_000_000_000_000;
        // 入站在 sent 后 25h，窗口 24h → 不命中
        assert!(!responded_within_window(sent, 24, &[sent + 25 * HOUR_MS]));
    }

    #[test]
    fn responded_false_when_inbound_before_send() {
        let sent = 1_000_000_000_000;
        // 入站早于发送（历史消息）→ 不算响应
        assert!(!responded_within_window(sent, 24, &[sent - HOUR_MS]));
    }

    #[test]
    fn responded_false_when_no_inbound() {
        assert!(!responded_within_window(1_000_000_000_000, 24, &[]));
    }

    #[test]
    fn stage_advanced_true_when_moves_forward() {
        let order = vec!["new_contact".to_string(), "意向".to_string(), "待成交".to_string()];
        assert!(stage_advanced(Some("意向"), Some("待成交"), &order));
    }

    #[test]
    fn stage_advanced_false_when_same_or_backward() {
        let order = vec!["new_contact".to_string(), "意向".to_string(), "待成交".to_string()];
        assert!(!stage_advanced(Some("意向"), Some("意向"), &order)); // 持平
        assert!(!stage_advanced(Some("待成交"), Some("意向"), &order)); // 回退
    }

    #[test]
    fn stage_advanced_false_when_unknown_or_missing() {
        let order = vec!["new_contact".to_string(), "意向".to_string()];
        // 任一阶段不在有序表 → 保守判 false（不算推进）
        assert!(!stage_advanced(Some("意向"), Some("不存在"), &order));
        assert!(!stage_advanced(None, Some("意向"), &order));
    }

    #[test]
    fn response_rate_zero_total_is_zero() {
        assert_eq!(response_rate(0, 0), 0.0);
    }

    #[test]
    fn response_rate_basic() {
        assert_eq!(response_rate(4, 1), 0.25);
    }
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --lib agent::send_ledger`
Expected: 编译失败（函数未定义）。

- [ ] **Step 3: 实现纯函数**

在测试模块之上写实现：

```rust
//! 主动发送台账：转化判定纯函数（responded 窗口 / stage_advanced 推进）、
//! 聚合率计算。写入 / 回扫的 DB 逻辑在 gateway/tasks 调用侧，这里只放可单测的纯逻辑。

/// 任一入站时间戳落在 (sent_at, sent_at + window_hours] 内 → 已响应。
/// 早于/等于发送时刻的入站（历史消息）不算。
pub(crate) fn responded_within_window(sent_at_ms: i64, window_hours: i32, inbound_ms: &[i64]) -> bool {
    let window_end = sent_at_ms + (window_hours.max(0) as i64) * 3_600_000;
    inbound_ms
        .iter()
        .any(|&ms| ms > sent_at_ms && ms <= window_end)
}

/// 当前阶段在 ordered_stages 里严格靠后于发送时阶段 → 推进。
/// 任一阶段缺失或不在有序表 → 保守判 false（不算推进）。
pub(crate) fn stage_advanced(
    stage_at_send: Option<&str>,
    current_stage: Option<&str>,
    ordered_stages: &[String],
) -> bool {
    let (Some(from), Some(to)) = (stage_at_send, current_stage) else {
        return false;
    };
    let idx = |s: &str| ordered_stages.iter().position(|x| x == s);
    match (idx(from), idx(to)) {
        (Some(i), Some(j)) => j > i,
        _ => false,
    }
}

/// 响应率：total=0 返 0.0，否则 responded/total 保留 4 位小数。
pub(crate) fn response_rate(total: u64, responded: u64) -> f64 {
    if total == 0 {
        return 0.0;
    }
    let raw = responded as f64 / total as f64;
    (raw * 10_000.0).round() / 10_000.0
}
```

在 `src/agent/mod.rs` 加 `mod send_ledger;`（与其它子模块声明并列）。

- [ ] **Step 4: 运行确认通过**

Run: `cargo test --lib agent::send_ledger`
Expected: 9 passed。

- [ ] **Step 5: Commit**

```bash
git add src/agent/send_ledger.rs src/agent/mod.rs
git commit -m "feat(send-ledger): 转化判定纯函数(responded窗口+stage_advanced推进+率计算)"
```

---

## Task 4: dispatcher 成功分支写台账（统一覆盖 media + namecard）

**Files:**
- Modify: `src/agent/send_ledger.rs`（加 `build_ledger_entry` 纯函数 + `record_send` async 写入，fail-soft）
- Modify: `src/agent/outbox_dispatcher.rs`（成功分支 :608 outbox 标 Sent 后调 `record_send`）
- Test: `src/agent/send_ledger.rs` 内联测试（`build_ledger_entry` 纯函数）

**设计依据（实现者必读）：** 台账写入点选在 **dispatcher 成功分支**（`outbox_dispatcher.rs:587 Ok(Ok(_))`），不在 `send_outbound_media/namecard` 内部。原因：发送函数签名只有 `(state, contact, id)`，拿不到 `run_id`；而 dispatcher 持有 `entry`（含 `run_id` / `referral_card_id` / `media_asset_id`）+ `contact`（含 workspace_id / domain_attributes 里的 customer_stage）。一处写入对称覆盖两功能。`entry` 的 `referral_card_id` 有值→namecard，`media_asset_id` 有值→media。

**Interfaces:**
- Consumes: `AgentSendLedger`（Task 1）、`OutboxEntry`（现有，字段 `run_id` / `referral_card_id` / `media_asset_id` / `account_id` / `contact_wxid` / `workspace_id`）、`Contact`（现有，`domain_attributes` 里 customer_stage）
- Produces:
  - `pub(crate) fn build_ledger_entry(workspace_id: &str, account_id: &str, contact_wxid: &str, send_kind: &str, target_id: &str, target_title: &str, run_id: &str, customer_stage_at_send: Option<String>, now: DateTime) -> AgentSendLedger`
  - `pub(crate) async fn record_send(state: &AppState, entry: &crate::models::AgentSendLedger)` — fail-soft insert，失败只 log

- [ ] **Step 1: 写失败测试**

`src/agent/send_ledger.rs` 测试区追加：

```rust
    #[test]
    fn build_ledger_entry_sets_kind_and_leaves_outcome_none() {
        use mongodb::bson::DateTime;
        let row = build_ledger_entry(
            "ws", "acct", "wx", "media", "asset1", "报价单", "run1",
            Some("意向".to_string()), DateTime::now(),
        );
        assert_eq!(row.send_kind, "media");
        assert_eq!(row.target_id, "asset1");
        assert_eq!(row.target_title, "报价单");
        assert_eq!(row.customer_stage_at_send.as_deref(), Some("意向"));
        // 转化字段发送时必须留空（回扫才填）
        assert!(row.responded.is_none());
        assert!(row.stage_advanced.is_none());
        assert!(row.outcome_evaluated_at.is_none());
    }
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --lib agent::send_ledger`
Expected: 编译失败（`build_ledger_entry` 未定义）。

- [ ] **Step 3: 实现 build_ledger_entry + record_send**

`src/agent/send_ledger.rs` 顶部补 use（`crate::models::AgentSendLedger`、`crate::routes::AppState`、`mongodb::bson::DateTime`），加：

```rust
use crate::models::AgentSendLedger;
use crate::routes::AppState;
use mongodb::bson::DateTime;

/// 构造一条待写台账。转化字段一律留空（回扫填）。
pub(crate) fn build_ledger_entry(
    workspace_id: &str,
    account_id: &str,
    contact_wxid: &str,
    send_kind: &str,
    target_id: &str,
    target_title: &str,
    run_id: &str,
    customer_stage_at_send: Option<String>,
    now: DateTime,
) -> AgentSendLedger {
    AgentSendLedger {
        id: None,
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        contact_wxid: contact_wxid.to_string(),
        send_kind: send_kind.to_string(),
        target_id: target_id.to_string(),
        target_title: target_title.to_string(),
        run_id: run_id.to_string(),
        trigger_reason: None,
        customer_stage_at_send,
        sent_at: now,
        responded: None,
        response_window_hours: None,
        stage_advanced: None,
        outcome_evaluated_at: None,
    }
}

/// fail-soft 写台账：失败只 log，绝不返 Err（既成事实纪律——发送已成，
/// 台账缺一条不该影响发送结果，更不能让上游误判为失败而重发）。
pub(crate) async fn record_send(state: &AppState, entry: &AgentSendLedger) {
    if let Err(err) = state.db.agent_send_ledger().insert_one(entry, None).await {
        tracing::error!(
            workspace_id = %entry.workspace_id,
            contact_wxid = %entry.contact_wxid,
            send_kind = %entry.send_kind,
            target_id = %entry.target_id,
            error = %err,
            "send succeeded but persisting agent_send_ledger failed; metrics will miss this send",
        );
    }
}
```

- [ ] **Step 4: dispatcher 成功分支调 record_send**

`src/agent/outbox_dispatcher.rs` 成功分支，在 `update_run_log_outbox_status(state, &entry.run_id, "sent").await;`（:624）之后追加。**仅当条目是 media 或 namecard 时写**（纯文本不进台账）：

```rust
            // 主动发送台账：素材/名片条目记一条（纯文本不记）。fail-soft，不影响已成发送。
            let send_kind_target = entry
                .referral_card_id
                .as_deref()
                .map(|id| ("namecard", id))
                .or_else(|| entry.media_asset_id.as_deref().map(|id| ("media", id)));
            if let Some((send_kind, target_id)) = send_kind_target {
                // target_title 冗余快照：回查实体标题，查不到留空（不阻断）。
                let target_title = super::send_ledger::lookup_target_title(
                    state, &entry.workspace_id, send_kind, target_id,
                )
                .await;
                // 发送瞬间客户阶段快照：从 contact.domain_attributes 读 customer_stage。
                let stage_at_send = contact
                    .domain_attributes
                    .as_ref()
                    .and_then(|d| d.get_str("customer_stage").ok())
                    .map(ToString::to_string);
                let ledger_row = super::send_ledger::build_ledger_entry(
                    &entry.workspace_id,
                    &entry.account_id,
                    &entry.contact_wxid,
                    send_kind,
                    target_id,
                    &target_title,
                    &entry.run_id,
                    stage_at_send,
                    now,
                );
                super::send_ledger::record_send(state, &ledger_row).await;
            }
```

- [ ] **Step 5: 实现 lookup_target_title**

`src/agent/send_ledger.rs` 加（回查素材标题/顾问名，查不到返空串——快照容错）：

```rust
use mongodb::bson::{doc, oid::ObjectId};

/// 回查发送物标题做冗余快照。查不到/解析失败返空串（不阻断写台账）。
pub(crate) async fn lookup_target_title(
    state: &AppState,
    workspace_id: &str,
    send_kind: &str,
    target_id: &str,
) -> String {
    let Ok(oid) = ObjectId::parse_str(target_id) else {
        return String::new();
    };
    let filter = doc! { "_id": oid, "workspace_id": workspace_id };
    match send_kind {
        "namecard" => state
            .db
            .referral_cards()
            .find_one(filter, None)
            .await
            .ok()
            .flatten()
            .map(|c| c.display_name)
            .unwrap_or_default(),
        _ => state
            .db
            .content_assets()
            .find_one(filter, None)
            .await
            .ok()
            .flatten()
            .map(|a| a.title)
            .unwrap_or_default(),
    }
}
```

（确认 `ContentAsset.title` 字段名——models.rs :678 区；`ReferralCard.display_name` 已知存在。`contact` 变量在 dispatcher 该作用域已绑定，见 :576/:578 已用 `&contact`。）

- [ ] **Step 6: 运行确认通过 + lib 不回归**

Run: `cargo test --lib agent::send_ledger && cargo test --lib 2>&1 | tail -5`
Expected: 全过；passed ≥ 350。

- [ ] **Step 7: no-human-takeover lint 自检**

Run: `grep -nE "人工接管|takeover|hand-?off|人工介入|人工托管|接管|人工" src/agent/send_ledger.rs src/agent/outbox_dispatcher.rs`
Expected: 无命中。

- [ ] **Step 8: Commit**

```bash
git add src/agent/send_ledger.rs src/agent/outbox_dispatcher.rs
git commit -m "feat(send-ledger): dispatcher成功分支写台账(media+namecard统一,fail-soft)"
```

---

## Task 5: tasks worker 转化回扫

**Files:**
- Modify: `src/agent/send_ledger.rs`（加 `scan_send_ledger_outcomes` async 回扫 + `ordered_stages_from_machine` 纯函数）
- Modify: `src/tasks.rs`（`tick` :160 加回扫调用）
- Test: `src/agent/send_ledger.rs` 内联测试（`ordered_stages_from_machine` 纯函数）

**Interfaces:**
- Consumes: `responded_within_window` / `stage_advanced`（Task 3）、`AgentSendLedger`（Task 1）、`operation_domain_configs` 的 stateMachine Document
- Produces:
  - `pub(crate) fn ordered_stages_from_machine(state_machine: &Document) -> Vec<String>` — 从状态机 states 数组按出现顺序抽 key 列表（作为"阶段序"）
  - `pub(crate) async fn scan_send_ledger_outcomes(state: &AppState) -> AppResult<usize>` — 回扫一批待评估条目，回填 responded/stage_advanced/outcome_evaluated_at，返回处理条数

- [ ] **Step 1: 写失败测试**

`src/agent/send_ledger.rs` 测试区追加：

```rust
    #[test]
    fn ordered_stages_extracts_keys_in_order() {
        use mongodb::bson::doc;
        let machine = doc! {
            "states": [
                { "key": "new_contact", "initial": true },
                { "key": "意向" },
                { "key": "待成交" },
            ]
        };
        let order = ordered_stages_from_machine(&machine);
        assert_eq!(order, vec!["new_contact", "意向", "待成交"]);
    }

    #[test]
    fn ordered_stages_empty_when_no_states() {
        use mongodb::bson::doc;
        assert!(ordered_stages_from_machine(&doc! {}).is_empty());
    }
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --lib agent::send_ledger`
Expected: 编译失败（`ordered_stages_from_machine` 未定义）。

- [ ] **Step 3: 实现 ordered_stages_from_machine + scan_send_ledger_outcomes**

`src/agent/send_ledger.rs` 加（use 区补 `crate::error::AppResult`、`futures::TryStreamExt`、`mongodb::options::FindOptions`、`crate::models::MessageDirection`）：

```rust
use crate::error::AppResult;

/// 从状态机 states 数组按出现顺序抽 key（作为粗略"阶段序"，供 stage_advanced 判定）。
pub(crate) fn ordered_stages_from_machine(state_machine: &Document) -> Vec<String> {
    state_machine
        .get_array("states")
        .map(|states| {
            states
                .iter()
                .filter_map(|s| s.as_document())
                .filter_map(|d| d.get_str("key").ok())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// 回扫一批 outcome_evaluated_at 缺失且已过响应窗口的台账条目，回填转化字段。
/// 纯读 + 回写自己表，不调 LLM、不发消息（无副作用红线）。返回处理条数。
pub(crate) async fn scan_send_ledger_outcomes(state: &AppState) -> AppResult<usize> {
    use futures::TryStreamExt;
    use mongodb::options::FindOptions;

    let default_window_hours: i32 = 24;
    let now = DateTime::now();
    let now_ms = now.timestamp_millis();

    // 待评估：outcome_evaluated_at 缺失。窗口是否已过在内存里按每条 sent_at 判断
    // （避免对 response_window_hours 可空字段做复杂 mongo 时间运算）。
    let filter = doc! { "outcome_evaluated_at": { "$exists": false } };
    let mut cursor = state
        .db
        .agent_send_ledger()
        .find(
            filter,
            FindOptions::builder()
                .limit(200) // 一次限量，防积压时单 tick 过重
                .sort(doc! { "sent_at": 1 })
                .build(),
        )
        .await?;

    let mut processed = 0usize;
    while let Some(row) = cursor.try_next().await? {
        let Some(row_id) = row.id else { continue };
        let window_hours = row.response_window_hours.unwrap_or(default_window_hours);
        let sent_ms = row.sent_at.timestamp_millis();
        let window_end_ms = sent_ms + (window_hours.max(0) as i64) * 3_600_000;
        // 窗口未过 → 跳过本轮（下个 tick 再看）。
        if now_ms < window_end_ms {
            continue;
        }

        // responded：查该 contact 在 (sent, sent+窗口] 内的入站消息时间戳。
        let inbound_filter = doc! {
            "workspace_id": &row.workspace_id,
            "contact_wxid": &row.contact_wxid,
            "direction": "inbound",
            "created_at": {
                "$gt": row.sent_at,
                "$lte": DateTime::from_millis(window_end_ms),
            },
        };
        let inbound_count = state
            .db
            .messages()
            .count_documents(inbound_filter, None)
            .await
            .unwrap_or(0);
        let responded = inbound_count > 0;

        // stage_advanced：取当前 contact.customer_stage vs 发送时快照，按状态机序判断。
        let current_stage = state
            .db
            .contacts()
            .find_one(
                doc! { "workspace_id": &row.workspace_id, "wxid": &row.contact_wxid },
                None,
            )
            .await
            .ok()
            .flatten()
            .and_then(|c| {
                c.domain_attributes
                    .as_ref()
                    .and_then(|d| d.get_str("customer_stage").ok().map(ToString::to_string))
            });
        let ordered = load_user_ops_stage_order(state, &row.workspace_id).await;
        let advanced = stage_advanced(
            row.customer_stage_at_send.as_deref(),
            current_stage.as_deref(),
            &ordered,
        );

        let _ = state
            .db
            .agent_send_ledger()
            .update_one(
                doc! { "_id": row_id },
                doc! { "$set": {
                    "responded": responded,
                    "response_window_hours": window_hours,
                    "stage_advanced": advanced,
                    "outcome_evaluated_at": now,
                }},
                None,
            )
            .await;
        processed += 1;
    }
    Ok(processed)
}

/// 取 user_operations 域当前状态机的阶段序。查不到返空（stage_advanced 保守判 false）。
async fn load_user_ops_stage_order(state: &AppState, workspace_id: &str) -> Vec<String> {
    state
        .db
        .operation_domain_configs()
        .find_one(
            doc! { "workspace_id": workspace_id, "domain": "user_operations", "current_version": true },
            None,
        )
        .await
        .ok()
        .flatten()
        .map(|c| ordered_stages_from_machine(&c.state_machine))
        .unwrap_or_default()
}
```

（确认：`Contact` 有 `wxid` 字段 + `domain_attributes: Option<Document>`；`OperationDomainConfig.state_machine: Document`；`messages()` / `contacts()` / `operation_domain_configs()` accessor 均存在。`count_documents` 是 mongodb crate 现有方法。）

- [ ] **Step 4: tick 挂回扫**

`src/tasks.rs` 的 `tick`（:160），在 `scan_escalation_timeouts`（:166）那组 `let _ = ...` 之后追加：

```rust
    // 主动发送台账：回扫已过响应窗口的条目，回填转化（响应率/阶段推进）。
    let _ = crate::agent::send_ledger::scan_send_ledger_outcomes(state).await;
```

- [ ] **Step 5: 运行确认通过 + lib 不回归**

Run: `cargo test --lib agent::send_ledger && cargo test --lib 2>&1 | tail -5`
Expected: 全过；passed ≥ 350。

- [ ] **Step 6: Commit**

```bash
git add src/agent/send_ledger.rs src/tasks.rs
git commit -m "feat(send-ledger): tasks worker回扫转化(responded窗口+stage_advanced,幂等限量)"
```

---

## Task 6: 只读 API（单客户历史 + 维度聚合 + 总览）

**Files:**
- Create: `src/routes/send_ledger.rs`
- Modify: `src/routes/mod.rs`（`mod send_ledger;` + 挂 3 路由，仿 referral_cards :62/:379）
- Test: `src/routes/send_ledger.rs` 内联测试（聚合 pipeline 构造纯函数）

**Interfaces:**
- Consumes: `AgentSendLedger`（Task 1）、`response_rate`（Task 3）、`AuthenticatedAdmin`（现有，`current_workspace` 字段）、`agent_send_ledger()` accessor（Task 2）
- Produces:
  - `GET /api/contacts/:wxid/send-history` → `{ items: [...] }`
  - `GET /api/send-ledger/stats?kind=media|namecard` → `{ items: [{ targetId, targetTitle, sentCount, contactCount, responseRate, stageAdvanceRate }] }`
  - `GET /api/send-ledger/overview` → `{ totalSends, responseRate, stageAdvanceRate }`

- [ ] **Step 1: 写失败测试（聚合 filter 纯函数）**

新建 `src/routes/send_ledger.rs`，先写测试 + filter 纯函数：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::doc;

    #[test]
    fn stats_match_pins_workspace_and_kind() {
        let m = build_stats_match("ws1", Some("media"));
        assert_eq!(m.get_str("workspace_id").ok(), Some("ws1"));
        assert_eq!(m.get_str("send_kind").ok(), Some("media"));
    }

    #[test]
    fn stats_match_without_kind_omits_kind() {
        let m = build_stats_match("ws1", None);
        assert_eq!(m.get_str("workspace_id").ok(), Some("ws1"));
        assert!(!m.contains_key("send_kind"));
    }
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --lib routes::send_ledger`
Expected: 编译失败（`build_stats_match` 未定义）。

- [ ] **Step 3: 实现 handler + filter 纯函数**

参照现有 route handler 形态（grep `AuthenticatedAdmin` + `State(state): State<AppState>` 的用法，如 `src/routes/referral_cards.rs`）。`/contacts/:wxid/send-history` 的 `:wxid` path 提取参照现有 `/contacts/:wxid` 路由的 `Path(wxid): Path<String>`。

```rust
//! 主动发送台账只读 API：单客户发送历史 / 素材·名片维度聚合 / 总览。
//! 全部带 workspace_id scope（防跨租户 IDOR）。
use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::bson::{doc, Document};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{auth::AuthenticatedAdmin, error::AppResult};
use super::AppState;

/// 聚合 $match：固定 workspace，可选 kind。
pub(super) fn build_stats_match(workspace_id: &str, kind: Option<&str>) -> Document {
    let mut m = doc! { "workspace_id": workspace_id };
    if let Some(k) = kind {
        m.insert("send_kind", k);
    }
    m
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StatsQuery {
    kind: Option<String>,
}

/// 单客户发送历史（按 sent_at 倒序）。
pub(super) async fn contact_send_history(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(wxid): Path<String>,
) -> AppResult<Json<Value>> {
    use mongodb::options::FindOptions;
    let mut cursor = state
        .db
        .agent_send_ledger()
        .find(
            doc! { "workspace_id": &admin.current_workspace, "contact_wxid": &wxid },
            FindOptions::builder().sort(doc! { "sent_at": -1 }).limit(100).build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(r) = cursor.try_next().await? {
        items.push(json!({
            "sendKind": r.send_kind,
            "targetId": r.target_id,
            "targetTitle": r.target_title,
            "sentAt": crate::models::dt_to_string(r.sent_at),
            "triggerReason": r.trigger_reason,
            "responded": r.responded,
            "stageAdvanced": r.stage_advanced,
        }));
    }
    Ok(Json(json!({ "items": items })))
}

/// 素材/名片维度聚合排行榜。
pub(super) async fn send_ledger_stats(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(q): Query<StatsQuery>,
) -> AppResult<Json<Value>> {
    let match_doc = build_stats_match(&admin.current_workspace, q.kind.as_deref());
    let pipeline = vec![
        doc! { "$match": match_doc },
        doc! { "$group": {
            "_id": "$target_id",
            "targetTitle": { "$last": "$target_title" },
            "sentCount": { "$sum": 1 },
            "contacts": { "$addToSet": "$contact_wxid" },
            "respondedCount": { "$sum": { "$cond": [ { "$eq": ["$responded", true] }, 1, 0 ] } },
            "stageAdvancedCount": { "$sum": { "$cond": [ { "$eq": ["$stage_advanced", true] }, 1, 0 ] } },
            "evaluatedCount": { "$sum": { "$cond": [ { "$ifNull": ["$outcome_evaluated_at", false] }, 1, 0 ] } },
        }},
        doc! { "$sort": { "sentCount": -1 } },
        doc! { "$limit": 100 },
    ];
    let mut cursor = state.db.agent_send_ledger().aggregate(pipeline, None).await?;
    let mut items = Vec::new();
    while let Some(d) = cursor.try_next().await? {
        let sent = d.get_i32("sentCount").unwrap_or(0).max(0) as u64;
        let responded = d.get_i32("respondedCount").unwrap_or(0).max(0) as u64;
        let advanced = d.get_i32("stageAdvancedCount").unwrap_or(0).max(0) as u64;
        let evaluated = d.get_i32("evaluatedCount").unwrap_or(0).max(0) as u64;
        let contact_count = d.get_array("contacts").map(|a| a.len()).unwrap_or(0);
        items.push(json!({
            "targetId": d.get_str("_id").unwrap_or_default(),
            "targetTitle": d.get_str("targetTitle").unwrap_or_default(),
            "sentCount": sent,
            "contactCount": contact_count,
            // 率以"已评估条目"为分母（未过窗口的不计入），避免新发未评估拉低率
            "responseRate": crate::agent::send_ledger::response_rate(evaluated, responded),
            "stageAdvanceRate": crate::agent::send_ledger::response_rate(evaluated, advanced),
        }));
    }
    Ok(Json(json!({ "items": items })))
}

/// 总览：总发送数 + 整体响应率/推进率。
pub(super) async fn send_ledger_overview(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
) -> AppResult<Json<Value>> {
    let pipeline = vec![
        doc! { "$match": { "workspace_id": &admin.current_workspace } },
        doc! { "$group": {
            "_id": null,
            "total": { "$sum": 1 },
            "respondedCount": { "$sum": { "$cond": [ { "$eq": ["$responded", true] }, 1, 0 ] } },
            "stageAdvancedCount": { "$sum": { "$cond": [ { "$eq": ["$stage_advanced", true] }, 1, 0 ] } },
            "evaluatedCount": { "$sum": { "$cond": [ { "$ifNull": ["$outcome_evaluated_at", false] }, 1, 0 ] } },
        }},
    ];
    let mut cursor = state.db.agent_send_ledger().aggregate(pipeline, None).await?;
    let (mut total, mut responded, mut advanced, mut evaluated) = (0u64, 0u64, 0u64, 0u64);
    if let Some(d) = cursor.try_next().await? {
        total = d.get_i32("total").unwrap_or(0).max(0) as u64;
        responded = d.get_i32("respondedCount").unwrap_or(0).max(0) as u64;
        advanced = d.get_i32("stageAdvancedCount").unwrap_or(0).max(0) as u64;
        evaluated = d.get_i32("evaluatedCount").unwrap_or(0).max(0) as u64;
    }
    Ok(Json(json!({
        "totalSends": total,
        "responseRate": crate::agent::send_ledger::response_rate(evaluated, responded),
        "stageAdvanceRate": crate::agent::send_ledger::response_rate(evaluated, advanced),
    })))
}
```

（注：`response_rate` 在 Task 3 是 `pub(crate)`，跨模块可见。`aggregate` 返回 `Document` cursor；`$group` 的 `$sum:1` 在 mongo 里是 i32，故用 `get_i32`——若运行时类型为 i64 实现者按编译/测试结果改 `get_i64`。`dt_to_string` 已在 models 中被各 route 使用。）

- [ ] **Step 4: 挂路由**

`src/routes/mod.rs` 加 `mod send_ledger;`（仿 :62），并在 referral-cards 路由块后（:393 之后）追加：

```rust
        .route(
            "/contacts/:wxid/send-history",
            get(send_ledger::contact_send_history),
        )
        .route("/send-ledger/stats", get(send_ledger::send_ledger_stats))
        .route("/send-ledger/overview", get(send_ledger::send_ledger_overview))
```

（确认 `get` 已在 mod.rs 顶部 `use axum::routing::{get, post}` 导入——现有路由在用。）

- [ ] **Step 5: 编译 + lib 不回归**

Run: `cargo test --lib routes::send_ledger && cargo test --lib 2>&1 | tail -5`
Expected: 2 passed；全 lib passed ≥ 350。

- [ ] **Step 6: no-human-takeover lint 自检**

Run: `grep -nE "人工接管|takeover|hand-?off|人工介入|人工托管|接管|人工" src/routes/send_ledger.rs`
Expected: 无命中。

- [ ] **Step 7: Commit**

```bash
git add src/routes/send_ledger.rs src/routes/mod.rs
git commit -m "feat(send-ledger): 只读API(单客户历史+维度聚合排行+总览,workspace scope)"
```

---

## Task 7: decision prompt 注入素材已发历史（支撑防重发软约束）

**Files:**
- Modify: `src/agent/send_ledger.rs`（加 `recent_sends_for_contact` async 查询 + `render_recent_media_lines` 纯函数）
- Modify: `src/agent/decision.rs`（:316 素材候选注入旁，加已发历史段拼进 prompt）
- Test: `src/agent/send_ledger.rs` 内联测试（`render_recent_media_lines` 纯函数）

**设计依据（实现者必读）：** 设计 §6.3 要求 prompt 注入"已发过的素材 + 时间"支撑防重发软约束（缺口 5）。**本期只做素材侧新增**——素材侧此前完全无已发历史注入，是真正的空白。名片侧已有 `AlreadyReferred`（decision.rs:339-352 从 `contact.domain_attributes[REFERRED_CARD_ID_ATTR]` 取）已能防重推，**本期不改其数据源**（避免回归现有已工作逻辑；设计里"统一从 ledger 取单一事实源"作为未来收敛，YAGNI 暂不强做）。

**Interfaces:**
- Consumes: `AgentSendLedger`（Task 1）、`agent_send_ledger()` accessor（Task 2）
- Produces:
  - `pub(crate) async fn recent_sends_for_contact(state: &AppState, workspace_id: &str, contact_wxid: &str, send_kind: &str, limit: i64) -> Vec<AgentSendLedger>`
  - `pub(crate) fn render_recent_media_lines(rows: &[AgentSendLedger]) -> String` — 空 rows 返空串；否则渲染"已发素材历史"段供 Reply Agent 判重

- [ ] **Step 1: 写失败测试**

`src/agent/send_ledger.rs` 测试区追加：

```rust
    #[test]
    fn render_recent_media_empty_when_no_rows() {
        assert_eq!(render_recent_media_lines(&[]), "");
    }

    #[test]
    fn render_recent_media_lists_titles() {
        use mongodb::bson::DateTime;
        let row = build_ledger_entry(
            "ws", "acct", "wx", "media", "a1", "报价单 2026", "run1", None, DateTime::now(),
        );
        let out = render_recent_media_lines(&[row]);
        assert!(out.contains("报价单 2026"));
        // 含"已发"语义提示，供 Reply Agent 判重（不强发同素材）
        assert!(out.contains("已"));
    }
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --lib agent::send_ledger`
Expected: 编译失败（`render_recent_media_lines` 未定义）。

- [ ] **Step 3: 实现查询 + 渲染**

`src/agent/send_ledger.rs` 加：

```rust
/// 取该客户近期某类发送记录（按 sent_at 倒序）。best-effort：故障返空。
pub(crate) async fn recent_sends_for_contact(
    state: &AppState,
    workspace_id: &str,
    contact_wxid: &str,
    send_kind: &str,
    limit: i64,
) -> Vec<AgentSendLedger> {
    use futures::TryStreamExt;
    use mongodb::options::FindOptions;
    let res = state
        .db
        .agent_send_ledger()
        .find(
            doc! { "workspace_id": workspace_id, "contact_wxid": contact_wxid, "send_kind": send_kind },
            FindOptions::builder().sort(doc! { "sent_at": -1 }).limit(limit).build(),
        )
        .await;
    match res {
        Ok(mut cursor) => {
            let mut out = Vec::new();
            while let Ok(Some(r)) = cursor.try_next().await {
                out.push(r);
            }
            out
        }
        Err(_) => Vec::new(),
    }
}

/// 渲染"已发素材历史"段。空返空串（prompt 不多余段）。供 Reply Agent 判重：
/// 不重复给同一客户硬发同一素材（软约束，非硬门——agent-first）。
pub(crate) fn render_recent_media_lines(rows: &[AgentSendLedger]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let mut out = String::from("【近期已发素材】你近期已给该客户发过以下素材，除非客户明确再次需要，否则不要重复发送同一素材：\n");
    for r in rows {
        let title = if r.target_title.is_empty() {
            r.target_id.as_str()
        } else {
            r.target_title.as_str()
        };
        out.push_str(&format!("- {title}\n"));
    }
    out
}
```

- [ ] **Step 4: 注入 decision prompt**

`src/agent/decision.rs`，在 `sendable_candidates_text`（:316）之后加：

```rust
    // 已发素材历史注入（防重发软约束，缺口 5）：查该客户近期已发素材，
    // 渲染成提示段供 Reply Agent 判重。best-effort，空 = 不加段。
    let recent_media_sent = super::send_ledger::recent_sends_for_contact(
        state,
        &contact.workspace_id,
        &contact.wxid,
        "media",
        10,
    )
    .await;
    let recent_media_text = super::send_ledger::render_recent_media_lines(&recent_media_sent);
```

然后把 `recent_media_text` 拼进 user prompt 的业务上下文层——**紧邻 `sendable_candidates_text` 注入处**。grep `sendable_candidates_text` 在本文件的 `format!` 注入点（它已被拼进 prompt），在同一 `format!` 里其后追加 `{recent_media_text}` 占位（参照 referral_block 的拼接方式）。

- [ ] **Step 5: 运行确认通过 + lib 不回归**

Run: `cargo test --lib agent::send_ledger && cargo test --lib 2>&1 | tail -5`
Expected: 全过；passed ≥ 350。

- [ ] **Step 6: no-human-takeover lint 自检**

Run: `grep -nE "人工接管|takeover|hand-?off|人工介入|人工托管|接管|人工" src/agent/send_ledger.rs src/agent/decision.rs`
Expected: 无命中（新增行）。

- [ ] **Step 7: Commit**

```bash
git add src/agent/send_ledger.rs src/agent/decision.rs
git commit -m "feat(send-ledger): decision prompt注入素材已发历史(防重发软约束,素材侧补空白)"
```

---

## Task 8: 前端「发送成效」频道（总览 + 素材/名片排行榜）

**Files:**
- Create: `frontend/src/features/send-analytics/index.tsx`
- Create: `frontend/src/features/send-analytics/SendAnalytics.module.css`
- Create: `frontend/src/stores/sendAnalyticsStore.ts`
- Modify: `frontend/src/types/index.ts`（Channel union 加 `"sendAnalytics"`）
- Modify: `frontend/src/app/channels.ts`（加频道项 + lazy import）

**设计语言约束（实现者必读，违反即返工）：** 见 Global Constraints「前端设计语言」。新页**必须**对齐 `frontend/src/features/referral-cards/`（最新同类页）：CSS Modules + `tokens.css` 变量（禁硬编码色值/间距）、控件全量重置（参照 `ReferralCards.module.css` 顶部注释）、四级层级不嵌套卡片、**排行榜用表格不套卡片**、sub-tab 不引第三级导航、白色企业控制台基调、中性命名守 no-human-takeover 禁词。

**Interfaces:**
- Consumes: Task 6 的 3 个 API（`/api/send-ledger/overview`、`/api/send-ledger/stats?kind=`）
- Produces: 「发送成效」频道入口

- [ ] **Step 1: Channel union 加项**

`frontend/src/types/index.ts` 的 `Channel` union（:4-19），在 `"referralCards"` 旁加一行 `| "sendAnalytics"`。

- [ ] **Step 2: store 拉数据**

新建 `frontend/src/stores/sendAnalyticsStore.ts`（仿 `referralCardStore.ts` 的 zustand + `api.get` 模式）：

```ts
import { create } from "zustand";
import { api } from "../lib/api";
import { useUiStore } from "./uiStore";

export type SendStatRow = {
  targetId: string;
  targetTitle: string;
  sentCount: number;
  contactCount: number;
  responseRate: number;
  stageAdvanceRate: number;
};
export type SendOverview = {
  totalSends: number;
  responseRate: number;
  stageAdvanceRate: number;
};

interface SendAnalyticsState {
  overview: SendOverview | null;
  mediaStats: SendStatRow[];
  namecardStats: SendStatRow[];
  loadOverview: () => Promise<void>;
  loadStats: (kind: "media" | "namecard") => Promise<void>;
}

export const useSendAnalyticsStore = create<SendAnalyticsState>((set) => ({
  overview: null,
  mediaStats: [],
  namecardStats: [],
  loadOverview: async () => {
    try {
      const r = await api.get<SendOverview>("/api/send-ledger/overview");
      set({ overview: r });
    } catch (e) {
      useUiStore.getState().setError(e instanceof Error ? e.message : String(e));
    }
  },
  loadStats: async (kind) => {
    try {
      const r = await api.get<{ items: SendStatRow[] }>(`/api/send-ledger/stats?kind=${kind}`);
      if (kind === "media") set({ mediaStats: r.items });
      else set({ namecardStats: r.items });
    } catch (e) {
      useUiStore.getState().setError(e instanceof Error ? e.message : String(e));
    }
  },
}));
```

- [ ] **Step 3: 频道页**

新建 `frontend/src/features/send-analytics/index.tsx`：默认导出组件，含
- 顶部总览：3 个指标（总发送数 / 响应率 / 阶段推进率），用现有 summary card 模式
- 两个 sub-tab：「素材效果」「名片效果」（本地 `useState` 切换，不引第三级路由）
- 每 tab 一张**表格**：列 = 名称 / 已发次数 / 覆盖客户数 / 响应率 / 阶段推进率，按已发次数倒序（后端已排序）
- `useEffect` 调 `loadOverview` + 当前 tab 的 `loadStats`
- 率显示为百分比（`(rate * 100).toFixed(1) + "%"`）

新建 `SendAnalytics.module.css`：复制 `ReferralCards.module.css` 的顶部注释 + `.page/.panel/.head/.headL/.eyebrow/.title/.headIcon` 结构，加 `.table/.tr/.th/.td/.tab/.tabActive` 表格与 tab 类，**全部走 tokens 变量**。表格不外包卡片。

- [ ] **Step 4: 频道接线**

`frontend/src/app/channels.ts`：
1. 顶部 lazy import：`const SendAnalyticsFeature = lazy(() => import("../features/send-analytics"));`
2. lucide 图标 import 加 `BarChart3`（确认 `lucide-react` 有此图标；若无用 `TrendingUp`）
3. `CHANNELS` 数组加一项（归"系统"组，放 quality 附近）：

```ts
  {
    id: "sendAnalytics",
    group: "系统",
    label: "发送成效",
    caption: "Send Analytics",
    icon: BarChart3,
    eyebrow: "Send Analytics",
    title: "发送成效",
    subtitle: "查看 AI 主动发送的素材与专属顾问名片的使用次数、覆盖客户数、响应率与阶段推进率。",
    Component: SendAnalyticsFeature,
  },
```

- [ ] **Step 5: 前端构建验证**

Run: `cd frontend && npm run build 2>&1 | tail -8`
Expected: 构建成功，无 TS 错误。

- [ ] **Step 6: no-human-takeover lint 自检（前端在扫描范围）**

Run: `grep -rnE "人工接管|takeover|hand-?off|人工介入|人工托管|接管|人工" frontend/src/features/send-analytics/ frontend/src/stores/sendAnalyticsStore.ts`
Expected: 无命中。

- [ ] **Step 7: Commit**

```bash
git add frontend/src/features/send-analytics frontend/src/stores/sendAnalyticsStore.ts frontend/src/types/index.ts frontend/src/app/channels.ts
git commit -m "feat(send-ledger): 前端「发送成效」频道(总览+素材/名片排行榜,对齐设计系统)"
```

---

## Task 9: 客户页嵌入「AI 已发送」只读历史面板

**Files:**
- Modify: `frontend/src/features/user-ops/legacy.tsx`（客户画像/对话区加只读历史小面板）
- Modify: `frontend/src/types/index.ts`（加 `SendHistoryItem` 类型）

**设计语言约束：** 视觉沿用 user-ops 页既有面板/行样式（`styles.css` 既有类名，不新造风格）。只读展示，不加操作按钮。

**Interfaces:**
- Consumes: Task 6 的 `/api/contacts/:wxid/send-history`
- Produces: 客户上下文里的"AI 已发送"可见性

- [ ] **Step 1: 加类型**

`frontend/src/types/index.ts` 加：

```ts
export type SendHistoryItem = {
  sendKind: "media" | "namecard";
  targetId: string;
  targetTitle: string;
  sentAt?: string;
  triggerReason?: string | null;
  responded?: boolean | null;
  stageAdvanced?: boolean | null;
};
```

- [ ] **Step 2: 选中客户时拉历史 + 渲染面板**

`frontend/src/features/user-ops/legacy.tsx`，在展示选中客户画像的区域（grep `核心画像` :548 附近作锚点），加一个只读面板：选中客户 wxid 变化时 `api.get<{items: SendHistoryItem[]}>(\`/api/contacts/${wxid}/send-history\`)`，渲染列表：每行显示 `sendKind`（素材/名片中文）+ `targetTitle` + 发送时间 + 响应标记（responded=true 显示"已响应"青色点，false 显示"未响应"，null 显示"待评估"）。空历史显示 EmptyInline/占位。

实现要点（不强制具体 JSX，但须满足）：
- 数据获取放在选中客户的 effect 里（与现有画像加载同生命周期）
- 用既有面板类名（`styles.css` 里 user-ops 已有的 panel/row 风格），不新建 module.css
- "已响应/未响应/待评估"用既有语义色，不硬编码

- [ ] **Step 3: 前端构建验证**

Run: `cd frontend && npm run build 2>&1 | tail -8`
Expected: 构建成功，无 TS 错误。

- [ ] **Step 4: no-human-takeover lint 自检**

Run: `grep -rnE "人工接管|takeover|hand-?off|人工介入|人工托管|接管|人工" frontend/src/features/user-ops/legacy.tsx | grep -iE "send|history|已发送"`
Expected: 无命中（新增行）。

- [ ] **Step 5: Commit**

```bash
git add frontend/src/features/user-ops/legacy.tsx frontend/src/types/index.ts
git commit -m "feat(send-ledger): 客户页嵌入AI已发送只读历史面板(补素材侧可见性)"
```

---

## Task 10: 端到端集成测试（CI）

**Files:**
- Create: `tests/send_ledger_integration.rs`（`#[ignore]`，CI integration job 跑）

**可见性约束（实现者必读）：** 参照 `tests/referral_card_push_integration.rs` 的封装边界——`scan_send_ledger_outcomes` / `build_ledger_entry` / `record_send` 是 `pub(crate)` 且 `send_ledger` 模块未对外 `pub use`，**跨 crate 不可见**。故集成测试走**公开路径**：直接对 `Database::agent_send_ledger()`（pub accessor）做 CRUD round-trip + workspace scope 验证。转化判定纯函数已由 `src/agent/send_ledger.rs` 内联单测覆盖，本文件不重复。

**Interfaces:**
- Consumes: `Database::agent_send_ledger()`（Task 2，pub）、`AgentSendLedger`（Task 1，pub）

- [ ] **Step 1: 写集成测试**

新建 `tests/send_ledger_integration.rs`（用现有 testcontainers helper 起 Mongo——参照 `tests/referral_card_push_integration.rs` 的 `mod common;` setup）：

```rust
//! 主动发送台账端到端：ledger CRUD round-trip + workspace_id scope 隔离 +
//! 转化字段可空 round-trip。需 Docker(testcontainers Mongo)，默认 #[ignore]，
//! CI 用 `cargo test --test send_ledger_integration -- --ignored` 跑。
//!
//! 可见性：scan_send_ledger_outcomes / build_ledger_entry 为 pub(crate) 跨 crate
//! 不可见（转化判定纯函数已由 src/agent/send_ledger.rs 内联单测覆盖）。本文件测
//! 公开路径：Database::agent_send_ledger() 集合 CRUD + workspace scope。
#![cfg(test)]

mod common;

use futures::TryStreamExt;
use mongodb::bson::{doc, DateTime};
use wechatagent::models::AgentSendLedger;

fn make_row(workspace: &str, contact: &str, kind: &str, target: &str) -> AgentSendLedger {
    AgentSendLedger {
        id: None,
        workspace_id: workspace.into(),
        account_id: "acct1".into(),
        contact_wxid: contact.into(),
        send_kind: kind.into(),
        target_id: target.into(),
        target_title: "fixture".into(),
        run_id: "run1".into(),
        trigger_reason: None,
        customer_stage_at_send: Some("意向".into()),
        sent_at: DateTime::now(),
        responded: None,
        response_window_hours: None,
        stage_advanced: None,
        outcome_evaluated_at: None,
    }
}

#[tokio::test]
#[ignore = "requires docker mongo"]
async fn ledger_roundtrip_and_outcome_update() {
    let (db, _guard) = common::test_db().await;
    let coll = db.agent_send_ledger();
    let res = coll.insert_one(make_row("ws1", "wxA", "media", "asset1"), None).await.unwrap();
    let id = res.inserted_id.as_object_id().unwrap();
    // 回填转化字段后能读回
    coll.update_one(
        doc! { "_id": id },
        doc! { "$set": { "responded": true, "stage_advanced": false, "outcome_evaluated_at": DateTime::now() } },
        None,
    ).await.unwrap();
    let back = coll.find_one(doc! { "_id": id }, None).await.unwrap().unwrap();
    assert_eq!(back.responded, Some(true));
    assert_eq!(back.stage_advanced, Some(false));
    assert!(back.outcome_evaluated_at.is_some());
}

#[tokio::test]
#[ignore = "requires docker mongo"]
async fn ledger_query_is_workspace_scoped() {
    let (db, _guard) = common::test_db().await;
    let coll = db.agent_send_ledger();
    coll.insert_one(make_row("wsA", "wxA", "media", "a1"), None).await.unwrap();
    coll.insert_one(make_row("wsB", "wxB", "namecard", "c1"), None).await.unwrap();
    // 只查 wsA：不能看到 wsB 的条目（IDOR 防护的数据层前提）
    let mut cursor = coll.find(doc! { "workspace_id": "wsA" }, None).await.unwrap();
    let mut count = 0;
    while let Some(r) = cursor.try_next().await.unwrap() {
        assert_eq!(r.workspace_id, "wsA");
        count += 1;
    }
    assert_eq!(count, 1);
}
```

（`common::test_db()` 的确切签名以 `tests/common/mod.rs` 现有为准——grep 现有集成测试的 setup 调用照搬。若返回形态不同，对齐现有 `referral_card_push_integration.rs` 的 setup。）

- [ ] **Step 2: 本地编译验证（不跑 ignored）**

Run: `cargo test --test send_ledger_integration --no-run 2>&1 | tail -5`
Expected: 编译通过（CI integration job 带 `--ignored` 真跑）。

- [ ] **Step 3: Commit**

```bash
git add tests/send_ledger_integration.rs
git commit -m "test(send-ledger): 端到端ledger CRUD round-trip+workspace scope隔离集成测试(CI)"
```

---

## Self-Review

**1. Spec coverage（逐节核对 spec → task）：**
- spec §3 数据模型 AgentSendLedger → Task 1 ✓
- spec §3 集合 accessor + 3 索引 → Task 2 ✓
- spec §4.1 写入（MCP 成功后紧贴 fail-soft）→ Task 4（dispatcher 成功分支，依据已注明为何不在发送函数内）✓
- spec §4.2 回填（复用 tasks worker + responded + stage_advanced + 幂等限量）→ Task 3（纯函数）+ Task 5（worker 回扫）✓
- spec §5 API（单客户历史 / 维度聚合 / 总览，workspace scope）→ Task 6 ✓
- spec §6.1 「发送成效」频道 → Task 8 ✓
- spec §6.2 单客户历史嵌客户页 → Task 9 ✓
- spec §6.3 prompt 历史注入 → Task 7（素材侧新增；名片侧已有 AlreadyReferred 本期不改源，依据已注明）✓
- spec §7 测试（纯函数 / 向后兼容 / 回填幂等 / 集成 / 前端构建）→ Task 1/3/5 内联 + Task 10 集成 + Task 8/9 build ✓
- spec §8 边界（不做硬门/LLM 归因/实时）→ Global Constraints + 各 task 未越界 ✓
- spec §9 红线（fail-soft / workspace scope / 无副作用回扫 / 禁词）→ Global Constraints + 各 task lint 自检步骤 ✓

**2. Placeholder scan：** 无 TBD/TODO。三处"实现期对齐"（`get_i32` vs `get_i64`、`common::test_db()` 签名、decision.rs 的 `format!` 注入点）均为"按现有代码 grep 确认"的已知对齐点，非占位——每个都给了参照锚点 + 编译步骤会暴露。

**3. Type consistency：** `AgentSendLedger`（Task 1，2/4/6/7/10 消费）、`response_rate`（Task 3 pub(crate)，Task 6 跨模块调）、`build_ledger_entry`/`record_send`（Task 4，dispatcher 调）、`scan_send_ledger_outcomes`/`ordered_stages_from_machine`（Task 5）、`recent_sends_for_contact`/`render_recent_media_lines`（Task 7）、`build_stats_match`（Task 6）、`agent_send_ledger()` accessor（Task 2，4/5/6/7/10 用）、`SendStatRow`/`SendOverview`/`SendHistoryItem`（前端 Task 8/9）— 跨任务签名一致。

**4. 关键设计决策已在 task 内联注明依据（实现者不会误判）：**
- 写入点为何在 dispatcher 而非发送函数（Task 4 设计依据段：签名拿不到 run_id）。
- prompt 注入为何只做素材侧、名片侧不改源（Task 7 设计依据段：避免回归 + YAGNI）。
- 率以"已评估条目"为分母而非总发送数（Task 6 注释：避免新发未评估拉低率）。
- 集成测试为何走公开 CRUD 路径（Task 10 可见性约束段：pub(crate) 跨 crate 不可见）。





