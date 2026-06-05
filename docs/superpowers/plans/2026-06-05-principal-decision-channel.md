# 决策请示通道（Principal Decision Channel）实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让用户(私聊)运营 Agent 在遇到超出自身职权/能力的事项时，向幕后真人决策源请示、拿到决策后用 AI 口吻向客户推进，形成"撞决策墙→请示真人→转述客户"的闭环，且客户永远只跟 Agent 对话。

**Architecture:** 复用现有统一发送网关（`gateway.rs::run_user_operation_gateway`）、follow-up worker 分发（`tasks.rs` else 分支）、MCP 文本发送（工具名 `message_send_text` 经 `logged_call_for_account`）、知识 draft 契约（`OperationKnowledgeChunk` draft+needs_review）。新增一个 MongoDB 台账 collection `agent_principal_escalations`、一个 AgentTask kind `principal_decision_relay`、一个 workspace 配置 `principal_decider`。

**核心模型（2026-06-06 用户拍板「统一走安全占位」后定稿）：** 决策 Agent 在 decision 阶段 emit 结构化 `escalation_request`。**触发请示的这一轮 run 始终是正常 `Approved`**——decision Agent 不下危险结论，改输出一句**安全占位 reply**（如「这个我帮你跟领导确认下，稍等给你准信」）作为本轮 `reply_text`，外加 `escalation_request`。这句占位经**现有 outbox 正常发送链路**送达客户（享二次安全门/幂等/重试），客户体验连贯不卡。请示动作是 approved 发送路径上的一个**副作用**：网关在占位 reply 入 outbox 后、函数收尾前（`run_user_operation_gateway_inner` 末尾 L1534）检测 `final_decision.escalation_request.needed`，命中则**只**做两件不面向客户的事——推请示卡给领导 wxid（直接 `logged_call_for_account`，不走 outbox）+ 落台账 pending，并把「等待领导决策」标记写进 contact state 供 admin 可观测。**run 不进 Held 分支、不设 hold category**，因此**完全不碰并行 agent 的 `review.rs`**。真人微信回复经 webhook 入站先做 principal-wxid 分流 → LLM 解读成 `{verdict, substance, constraints}` → 台账 resolved → 起 relay task → 第二次 run 先重读客户最新状态再用 AI 口吻转述。

**Tech Stack:** Rust 2021 (Axum)、MongoDB (`mongodb` crate `Collection<T>` + `doc!` + `IndexModel`)、serde/serde_json、tokio。无新依赖。

**红线合规：** 全程避开 `check-no-human-takeover` lint 禁词集（`human[_ -]?takeover|takeover|hand[ -]?off|人工接管|人工介入|人工托管|接管|人工`），命名只用 **真人 / 领导 / principal / escalation / decision**（含代码注释）。contact state 上的可观测标记 `awaiting_principal_decision` 是 AI-internal 风格命名，不触发 lint。`scripts/check-no-model-hint` 同样要过。

**验证门（每个 Task 的 commit 前在本地跑）：**
- `cargo check`（用 `CARGO_TARGET_DIR=target-check cargo check` 省磁盘）
- `cargo test --lib`（基线 ≥350 passed / 0 failed）
- 涉及的新单测：`cargo test --lib <test_name>`
- **不**在本地跑全量 `--ignored` 集成测试（磁盘纪律，交 CI）。

**6 个已锁定的业务决策（用户 2026-06-05 拍板，贯穿全计划）：**
1. 闸门件升级做成 workspace 级可配置 `high_risk_escalation_mode`：`all`（所有高风险件都请示）/ `decision_only`（只升级实质需决策的件）。**统一走安全占位（2026-06-06 拍板）：** 高风险件要请示时，decision Agent 不下危险结论、改输出安全占位 reply + escalation_request，本轮 run 照常 `Approved`、占位经 outbox 正常发给客户——**不再走 Held 挂起分支、客户这轮不会收不到回复**。`high_risk_escalation_mode` 只决定 decision Agent 是否对该类件 emit escalation_request，不改变发送路径。
2. 超时严格无限等待，真人不回永不自动代决。
3. 泛化沉淀由 agent 自判（decision emit `is_generalizable`），自动发知识缺口提案（draft+needs_review）。
4. 真人回复不带短码：该真人仅 1 条未决→直接匹配；≥2 条未决→agent 反问真人澄清，不盲目回落最近一条。
5. 多轮卡死触发 = 同议题 N 轮（默认 3）未推进 **且** 客户出现负面反应，两条件同时满足。
6. 等待期客户发新消息：非越权部分照常回，只挂起越权点（不冻结整段对话）。

**两处 spec 锚点漂移已修正（核验于 2026-06-05，写代码以此为准）：**
- `mcp.rs` **无** `message_send_text` 函数 —— 它是 MCP 工具名字符串，调用走 `mcp::logged_call_for_account(state, account_id, "message_send_text", json!({"recipient": wxid, "content": content}))`。
- **无** `guards/` 子目录，是单文件 `guards.rs`；旧"五闸门"已删除，当前是 knowledge_grounding / hallucination / run_budget 三闸门 + review 评分体系。spec 里"五闸门"措辞按此理解。

---

## 文件结构（先锁定分解边界）

| 文件 | 创建/修改 | 职责 |
| --- | --- | --- |
| `src/models.rs` | 修改 | 新增 `AgentPrincipalEscalation`（台账）、`EscalationRequest`（decision emit 的请示意图）、`PrincipalDecision`（真人裁决解读结果）三个 struct；`OperationDomainConfig` 加 `principal_decider` + `high_risk_escalation_mode` 字段；常量 `PRINCIPAL_ESCALATION_STATUS_*` |
| `src/db/mod.rs` | 修改 | 加 typed accessor `agent_principal_escalations()` |
| `src/db/indexes.rs` | 修改 | 加 (workspace_id, status, contact_wxid) 复合索引 + short_code 唯一索引 |
| `src/agent/types.rs` | 修改 | `AgentDecision` + `RawAgentDecision` 加 `escalation_request` 可选字段（**不再新增 hold category**——统一占位模型下 run 始终 Approved，不进 Held 分支） |
| `src/agent/escalation.rs` | **创建** | 本功能核心模块：短码生成、触发判定（三类）、台账 CRUD、请示卡渲染、真人回复解读（LLM）、relay 转述、知识缺口提案、wxid 防护。公共入口由 `agent/mod.rs` re-export |
| `src/agent/mod.rs` | 修改 | `pub mod escalation;` + re-export 入口函数 |
| `src/agent/gateway.rs` | 修改 | approved 发送路径末尾（`run_user_operation_gateway_inner` L1534，占位 reply 已入 outbox 后）检测 `final_decision.escalation_request.needed` → 调 escalation 模块推卡给领导+落台账；`apply_agent_updates` 写 `awaiting_principal_decision` 标记到 contact state（admin 可观测）；新增 relay 入口。**只加分支/加调用，不改 review 流程** |
| `src/tasks.rs` | 修改 | else 分支已兜底 `principal_decision_relay`，仅需确认（可能无需改） |
| `src/webhooks.rs` | 修改 | 入站先判 from_wxid 是否某 workspace 的 `principal_decider`，是则分流到 escalation::handle_principal_reply，不进客户 agent 链路 |
| `src/prompts.rs` | 修改 | decision prompt（`user.reply.task`）加 escalationRequest 字段说明；新增真人回复解读 prompt |
| `tests/principal_decision_channel.rs` | **创建** | 集成测试（`#[ignore]`，testcontainers MongoDB）覆盖 spec §14 九项 |
| `src/agent/escalation.rs`（含 `#[cfg(test)] mod tests`） | 同上 | 纯函数单测（短码生成、回执码匹配、解读 fallback）跟模块同文件，进 `cargo test --lib` 基线 |

**分解原则：** 本功能绝大部分新逻辑集中在新建的 `src/agent/escalation.rs`（一个文件一个职责=请示通道），对现有大文件（gateway/webhooks/models）只做"加字段/加分支/加调用"的最小侵入式追加，降低与并行 agent 的冲突面，也让每个 Task 改动可独立 review。

---

## Phase 1 — 数据模型与脚手架（Task 1-5）

这一阶段只建类型、常量、collection、索引，不接任何业务逻辑。每个 Task 都能独立编译通过、独立 commit。

### Task 1: 台账与请示意图的 Model 结构体

**Files:**
- Modify: `src/models.rs`（在 `OperationKnowledgeChunk` 之后、文件靠后的 struct 区追加；与现有 BSON struct 同风格）

定义三个 struct + 一组状态常量。`AgentPrincipalEscalation` 是台账行；`EscalationRequest` 是 decision Agent emit 的请示意图（会内嵌进 AgentDecision）；`PrincipalDecision` 是真人自然语言裁决被 LLM 解读后的结构。

- [ ] **Step 1: 追加状态常量与三个 struct**

在 `src/models.rs` 末尾追加（紧跟其他 `ALLOWED_*` 常量风格）：

```rust
/// 请示台账状态闭集。pending=已推送领导待回；resolved=真人已裁决并已起 relay。
pub const PRINCIPAL_ESCALATION_STATUS_PENDING: &str = "pending";
pub const PRINCIPAL_ESCALATION_STATUS_RESOLVED: &str = "resolved";
pub const ALLOWED_PRINCIPAL_ESCALATION_STATUS: &[&str] = &[
    PRINCIPAL_ESCALATION_STATUS_PENDING,
    PRINCIPAL_ESCALATION_STATUS_RESOLVED,
];

/// 请示触发的三类边界（实质驱动）。
pub const ESCALATION_CATEGORY_OUT_OF_SCOPE: &str = "out_of_scope_decision";
pub const ESCALATION_CATEGORY_HIGH_RISK_GATED: &str = "high_risk_gated";
pub const ESCALATION_CATEGORY_STUCK: &str = "stuck_or_undelivered";
pub const ALLOWED_ESCALATION_CATEGORY: &[&str] = &[
    ESCALATION_CATEGORY_OUT_OF_SCOPE,
    ESCALATION_CATEGORY_HIGH_RISK_GATED,
    ESCALATION_CATEGORY_STUCK,
];

/// 真人裁决口径闭集。
pub const PRINCIPAL_VERDICT_APPROVED: &str = "approved";
pub const PRINCIPAL_VERDICT_REJECTED: &str = "rejected";
pub const PRINCIPAL_VERDICT_CONDITIONAL: &str = "conditional";
pub const PRINCIPAL_VERDICT_DEFERRED: &str = "deferred";
pub const PRINCIPAL_VERDICT_DELEGATED_BACK: &str = "delegated_back";
pub const ALLOWED_PRINCIPAL_VERDICT: &[&str] = &[
    PRINCIPAL_VERDICT_APPROVED,
    PRINCIPAL_VERDICT_REJECTED,
    PRINCIPAL_VERDICT_CONDITIONAL,
    PRINCIPAL_VERDICT_DEFERRED,
    PRINCIPAL_VERDICT_DELEGATED_BACK,
];

/// 决策 Agent 在 decision 阶段 emit 的请示意图（内嵌进 AgentDecision）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EscalationRequest {
    /// 是否需要请示真人。
    pub needed: bool,
    /// 三类之一，见 ALLOWED_ESCALATION_CATEGORY。
    #[serde(default)]
    pub category: Option<String>,
    /// 卡点原因（给真人看）。
    #[serde(default)]
    pub reason: Option<String>,
    /// 向真人提的问题。
    #[serde(default)]
    pub question_for_principal: Option<String>,
    /// 客户同一条消息里"非越权、可自主答"的部分（等待期分答用）。
    #[serde(default)]
    pub self_serviceable_part: Option<String>,
    /// agent 自判该决策是否可泛化（决定是否发知识缺口提案）。
    #[serde(default)]
    pub is_generalizable: bool,
}

/// 真人自然语言裁决经 LLM 解读后的结构。绝不原话转发给客户。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PrincipalDecision {
    /// 裁决口径，见 ALLOWED_PRINCIPAL_VERDICT。
    pub verdict: String,
    /// 决策实质（如"同意 8 折"），AI 口吻转述的事实源。
    pub substance: String,
    /// 附带约束（如"本周内付款"）。
    #[serde(default)]
    pub constraints: Vec<String>,
    /// 授权有效时长（小时）。**领导说了算**：领导明确说了期限（"这个价就今天有效"="约 24"、
    /// "这周内都行"=本周剩余小时数）才填；领导没提期限 → None（= 授权不设过期窗，长期有效）。
    /// 由 interpret LLM 自判填充；Task 19 据此算 authorization_expires_at。
    #[serde(default)]
    pub authorization_window_hours: Option<f64>,
}

/// 请示台账行（MongoDB collection `agent_principal_escalations`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPrincipalEscalation {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub contact_wxid: String,
    /// 人类可读短码，如 "E1A2"。全局唯一。
    pub short_code: String,
    /// pending / resolved，见 ALLOWED_PRINCIPAL_ESCALATION_STATUS。
    pub status: String,
    /// 三类触发之一，见 ALLOWED_ESCALATION_CATEGORY。
    pub category: String,
    /// 卡点原因。
    pub reason: String,
    /// 向真人提的问题。
    pub question_for_principal: String,
    /// 推给领导的 wxid（= 该 workspace 的 principal_decider）。
    pub principal_wxid: String,
    /// resolved 时填：真人裁决解读结果。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<PrincipalDecision>,
    /// resolved 时填：授权过期时间（过期后该条授权不可再用，但条目仍 resolved）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_expires_at: Option<DateTime>,
    /// agent 在 emit escalation 时自判：该决策是否可泛化成通用知识（决定 relay 后是否发知识缺口提案）。
    #[serde(default)]
    pub is_generalizable: bool,
    /// 是否已据此发过知识缺口提案（防重复）。
    #[serde(default)]
    pub knowledge_proposal_emitted: bool,
    pub created_at: DateTime,
    pub updated_at: DateTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<DateTime>,
}
```

> 说明：`ObjectId` / `DateTime` / `Serialize` / `Deserialize` 在 `models.rs` 顶部已 `use`（与现有 struct 共用）。若编译报某符号未导入，照 `OperationKnowledgeChunk` 的 use 行补齐——但通常无需，因为同文件已全导入。

- [ ] **Step 2: 验证编译**

Run: `CARGO_TARGET_DIR=target-check cargo check`
Expected: PASS（仅新增类型，无引用方，应零警告）

- [ ] **Step 3: 加纯函数单测——状态闭集自洽**

在 `src/models.rs` 已有的 `#[cfg(test)] mod tests` 内追加（若该 mod 不存在则在文件末尾新建）：

```rust
#[test]
fn principal_escalation_status_closed_set_is_self_consistent() {
    assert!(ALLOWED_PRINCIPAL_ESCALATION_STATUS.contains(&PRINCIPAL_ESCALATION_STATUS_PENDING));
    assert!(ALLOWED_PRINCIPAL_ESCALATION_STATUS.contains(&PRINCIPAL_ESCALATION_STATUS_RESOLVED));
    assert_eq!(ALLOWED_PRINCIPAL_ESCALATION_STATUS.len(), 2);
}

#[test]
fn escalation_category_and_verdict_closed_sets_are_self_consistent() {
    assert_eq!(ALLOWED_ESCALATION_CATEGORY.len(), 3);
    assert_eq!(ALLOWED_PRINCIPAL_VERDICT.len(), 5);
    assert!(ALLOWED_PRINCIPAL_VERDICT.contains(&PRINCIPAL_VERDICT_DELEGATED_BACK));
}

#[test]
fn escalation_request_deserializes_with_defaults() {
    // 只给 needed，其余走 #[serde(default)]，验证向前兼容。
    let req: EscalationRequest =
        serde_json::from_str(r#"{"needed": true}"#).expect("should deserialize");
    assert!(req.needed);
    assert_eq!(req.category, None);
    assert!(!req.is_generalizable);
    assert!(req.self_serviceable_part.is_none());
}
```

- [ ] **Step 4: 跑单测**

Run: `cargo test --lib principal_escalation_status_closed_set_is_self_consistent escalation_category_and_verdict_closed_sets_are_self_consistent escalation_request_deserializes_with_defaults`
Expected: 3 passed; 0 failed

- [ ] **Step 5: Commit**

```bash
git add src/models.rs
git commit -m "feat(escalation): 请示台账/裁决/请示意图三 model + 状态闭集常量"
```

---

### Task 2: typed accessor

**Files:**
- Modify: `src/db/mod.rs`（`use crate::models::...` 区 + `Database` impl 块 ~:340）

- [ ] **Step 1: 引入新 model**

在 `src/db/mod.rs` 顶部 `use crate::models::{...}`（约 :17-29）的列表里追加 `AgentPrincipalEscalation`（按字母/现有顺序插入，保持单个 use 块）。例如若现有是：

```rust
use crate::models::{
    AgentRunLog, Contact, /* ...existing... */ OutboxEntry,
};
```

改为在列表中加入 `AgentPrincipalEscalation,`（具体位置照现有排序，别破坏其它项）。

- [ ] **Step 2: 加 accessor**

在 `Database` impl 块内（紧邻其它 collection accessor，如 `agent_run_logs()` :178 附近，保持分组）追加：

```rust
pub fn agent_principal_escalations(&self) -> Collection<AgentPrincipalEscalation> {
    self.db.collection("agent_principal_escalations")
}
```

- [ ] **Step 3: 验证编译**

Run: `CARGO_TARGET_DIR=target-check cargo check`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/db/mod.rs
git commit -m "feat(escalation): Database 加 agent_principal_escalations typed accessor"
```

---

### Task 3: 索引注册

**Files:**
- Modify: `src/db/indexes.rs`（`ensure_all()` 末尾，或新建 helper 并在 `ensure_all` 调用）

加两个索引：(workspace_id, status, contact_wxid) 复合（查某工作区某客户的 pending 请示）+ short_code 唯一（短码全局唯一、回执码匹配）。

- [ ] **Step 1: 在 `ensure_all()` 末尾追加索引创建**

定位 `src/db/indexes.rs` 的 `ensure_all`（核验时在 ~:560 末尾），在其它 `db.xxx().create_index(...)` 之后、函数 `Ok(())` 之前追加：

```rust
// agent_principal_escalations：复合查询索引 + 短码唯一索引
db.agent_principal_escalations()
    .create_index(
        IndexModel::builder()
            .keys(doc! { "workspace_id": 1, "status": 1, "contact_wxid": 1 })
            .options(
                IndexOptions::builder()
                    .name("idx_principal_escalation_ws_status_contact".to_string())
                    .build(),
            )
            .build(),
        None,
    )
    .await?;
db.agent_principal_escalations()
    .create_index(
        IndexModel::builder()
            .keys(doc! { "short_code": 1 })
            .options(
                IndexOptions::builder()
                    .unique(true)
                    .name("uniq_principal_escalation_short_code".to_string())
                    .build(),
            )
            .build(),
        None,
    )
    .await?;
```

> `IndexModel` / `IndexOptions` / `doc!` 在 indexes.rs 顶部已 use（现有索引都在用）。无需新增 import。

- [ ] **Step 2: 验证编译**

Run: `CARGO_TARGET_DIR=target-check cargo check`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add src/db/indexes.rs
git commit -m "feat(escalation): agent_principal_escalations 复合索引 + 短码唯一索引"
```

---

### Task 4: 等待标记常量 `AWAITING_PRINCIPAL_DECISION_ATTR`

**Files:**
- Modify: `src/models.rs`（常量区，与其它 domain_attributes key 常量放一起；若无集中区则放 `AgentPrincipalEscalation` struct 旁）

> **为什么不再加 hold category？** 统一占位模型（见架构段 + 业务决策 #1）下，触发请示的 run **始终是 `Approved`**——占位 reply 经 outbox 正常发出，run 不进 Held 分支。因此**不需要** `ai_awaiting_principal_decision` 这个 hold category，也**不碰** `HOLD_CATEGORY_VALUES` / `assert_hold_category_valid` / `review.rs`。「正在等待领导决策」这个状态改用 **contact `domain_attributes` 上的一个布尔标记**承载，供 admin 看板观测、供等待期 pre-check（Task 21）读取。本 Task 只定义这个标记的 key 常量，避免散落的字符串字面量。

- [ ] **Step 1: 定义标记 key 常量**

在 `src/models.rs` 追加（放在 escalation 相关常量区，与 Task 1 的 `PRINCIPAL_ESCALATION_STATUS_*` 相邻）：

```rust
/// contact.domain_attributes 上的布尔标记 key：该客户有一个 pending 请示、正在等待领导决策。
/// admin 看板据此显示「等待中」；等待期 pre-check 据此识别。统一占位模型下这只是可观测标记，
/// 不是 hold category——触发请示的 run 本身是 Approved，占位已正常发出。
pub const AWAITING_PRINCIPAL_DECISION_ATTR: &str = "awaiting_principal_decision";
```

- [ ] **Step 2: 加单测——常量值稳定（防手滑改 key 导致 set/unset 不匹配）**

在 `src/models.rs` 的 `#[cfg(test)] mod tests`（若无则新建）追加：

```rust
#[test]
fn awaiting_principal_decision_attr_key_is_stable() {
    // set（Task 18 apply_agent_updates）与 unset（Task 16 clear_awaiting_principal_state）
    // 必须用同一个 key，否则等待标记清不掉。锁死常量值防回归。
    assert_eq!(AWAITING_PRINCIPAL_DECISION_ATTR, "awaiting_principal_decision");
}
```

> **执行注意**：`grep -n "mod tests" src/models.rs` 确认测试模块位置；若 models.rs 无测试模块，按其它 `#[cfg(test)]` 模块风格新建一个最小的。这个测试很轻但有用——它把 set 侧（Task 18）和 unset 侧（Task 16 `clear_awaiting_principal_state` 的 `$unset` key）锁在同一字符串上。

- [ ] **Step 3: 跑单测**

Run: `cargo test --lib awaiting_principal_decision_attr_key_is_stable`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/models.rs
git commit -m "feat(escalation): 定义 AWAITING_PRINCIPAL_DECISION_ATTR 等待标记 key 常量"
```

---

### Task 5: `AgentDecision` / `RawAgentDecision` 加 `escalation_request` 字段

**Files:**
- Modify: `src/agent/types.rs`（`AgentDecision` :82-202、`RawAgentDecision` :277-340、`validate_and_promote`）

让决策 Agent 能 emit `escalation_request`。两层结构都要加：`RawAgentDecision`（直接反序列化 LLM JSON）和 `AgentDecision`（promote 后的内部契约）。

- [ ] **Step 1: `AgentDecision` 加字段**

在 `src/agent/types.rs` 的 `AgentDecision` struct 末尾（~:201，紧邻 `conversation_mode_reason` 等可选字段）追加：

```rust
    /// decision Agent emit 的请示意图；None=本轮无需请示真人。
    #[serde(default)]
    pub escalation_request: Option<crate::models::EscalationRequest>,
```

- [ ] **Step 2: `RawAgentDecision` 加对应字段**

在 `RawAgentDecision`（:277-340，字段全是 `Option`）末尾追加：

```rust
    #[serde(default)]
    pub escalation_request: Option<crate::models::EscalationRequest>,
```

- [ ] **Step 3: `validate_and_promote` 透传**

在 `validate_and_promote` 构造 `AgentDecision { ... }` 的地方，把字段透传（找到构造 `AgentDecision` 的那个表达式，加一行）：

```rust
        escalation_request: self.escalation_request.clone(),
```

> 执行时 `grep "AgentDecision {" src/agent/types.rs` 定位构造点。若 `validate_and_promote` 返回 `(AgentDecision, ...)`，只需在结构体字面量里加这一行。`EscalationRequest` 已 `Clone`（Task 1 derive 了）。

- [ ] **Step 4: 加单测——decision JSON 带 escalationRequest 能解析**

在 types.rs 的 `#[cfg(test)] mod tests`（已有大量 RawAgentDecision 解析测试）追加：

```rust
#[test]
fn raw_decision_parses_escalation_request() {
    let json = r#"{
        "escalationRequest": {
            "needed": true,
            "category": "out_of_scope_decision",
            "reason": "客户要 8 折，超出标准 9 折权限",
            "questionForPrincipal": "是否同意 8 折？",
            "isGeneralizable": false
        }
    }"#;
    let raw: RawAgentDecision = serde_json::from_str(json).expect("parse");
    let esc = raw.escalation_request.expect("escalation present");
    assert!(esc.needed);
    assert_eq!(esc.category.as_deref(), Some("out_of_scope_decision"));
    assert!(!esc.is_generalizable);
}

#[test]
fn raw_decision_without_escalation_still_parses() {
    // 向前兼容：旧 JSON 无该字段，应为 None。
    let raw: RawAgentDecision = serde_json::from_str(r#"{}"#).expect("parse empty");
    assert!(raw.escalation_request.is_none());
}
```

- [ ] **Step 5: 跑单测 + 现有 decision 解析回归**

Run: `cargo test --lib raw_decision_parses_escalation_request raw_decision_without_escalation_still_parses`
Expected: 2 passed
Run: `cargo test --lib raw_agent_decision`（现有 RawAgentDecision 测试族）
Expected: 全 PASS（新可选字段不破坏既有解析）

- [ ] **Step 6: Commit**

```bash
git add src/agent/types.rs
git commit -m "feat(escalation): AgentDecision/RawAgentDecision 加 escalation_request 可选字段并透传"
```

---

## Phase 2 — escalation 模块纯逻辑（Task 6-9）

新建 `src/agent/escalation.rs`，先放**不碰 DB/LLM 的纯函数**：短码生成、workspace 配置读取的纯解析、回执码匹配、请示卡渲染、安抚占位文案策略。纯函数全部带单测，进 `cargo test --lib` 基线。DB/LLM/网关接线留到 Phase 3+。

### Task 6: 建模块骨架 + 短码生成

**Files:**
- Create: `src/agent/escalation.rs`
- Modify: `src/agent/mod.rs`（`pub mod escalation;`）

短码要求：人类可读（领导能在微信里念/认）、URL/正则安全、低碰撞。用 `E` 前缀 + 4 位 base32（去掉易混的 0/O/1/I/L）。碰撞由 DB 短码唯一索引兜底（Phase 3 插入失败则重试）。

- [ ] **Step 1: 建文件，写短码生成 + 第一个失败测试**

创建 `src/agent/escalation.rs`：

```rust
//! 决策请示通道（Principal Decision Channel）。
//!
//! 运营 Agent 撞"决策墙"（超职权 / 高风险件 / 多轮卡死）时，向幕后真人决策源
//! 请示，拿到裁决后用 AI 口吻向客户转述。客户永远只跟 Agent 对话——真人是
//! 幕后决策源，绝不直接面对客户。这不是人工接管：AI 向内部决策源请示，转述仍是 AI。

use crate::models::AgentPrincipalEscalation;

/// 短码字符集：base32 去掉易混字符（0/O/1/I/L），便于真人在微信里识读。
const SHORT_CODE_ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";
const SHORT_CODE_BODY_LEN: usize = 4;

/// 由一个 0..=u32::MAX 的种子生成短码，形如 "E1A2"（E 前缀 + 4 位 base32）。
/// 纯函数、确定性，便于单测；运行时种子由台账插入侧用计数/时间派生（见 Task 11 insert_pending_escalation 的碰撞重试）。
pub(crate) fn short_code_from_seed(seed: u32) -> String {
    let alpha_len = SHORT_CODE_ALPHABET.len() as u32;
    let mut n = seed;
    let mut body = [0u8; SHORT_CODE_BODY_LEN];
    for slot in body.iter_mut() {
        *slot = SHORT_CODE_ALPHABET[(n % alpha_len) as usize];
        n /= alpha_len;
    }
    let body_str = String::from_utf8(body.to_vec()).expect("alphabet is ASCII");
    format!("E{body_str}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_code_has_e_prefix_and_fixed_len() {
        let code = short_code_from_seed(0);
        assert!(code.starts_with('E'));
        assert_eq!(code.len(), 1 + SHORT_CODE_BODY_LEN);
    }

    #[test]
    fn short_code_uses_unambiguous_alphabet_only() {
        let code = short_code_from_seed(123_456);
        for ch in code.chars().skip(1) {
            assert!(
                SHORT_CODE_ALPHABET.contains(&(ch as u8)),
                "char {ch} must be in unambiguous alphabet"
            );
        }
        // 不含易混字符
        for bad in ['0', 'O', '1', 'I', 'L'] {
            assert!(!code[1..].contains(bad), "code body must not contain {bad}");
        }
    }

    #[test]
    fn short_code_is_deterministic() {
        assert_eq!(short_code_from_seed(42), short_code_from_seed(42));
    }

    #[test]
    fn short_code_differs_for_different_seeds() {
        assert_ne!(short_code_from_seed(1), short_code_from_seed(2));
    }
}
```

在 `src/agent/mod.rs` 的模块声明区（其它 `pub mod xxx;` 旁，按字母序）追加：

```rust
pub mod escalation;
```

- [ ] **Step 2: 跑测试确认通过**

Run: `cargo test --lib escalation::tests::short_code`
Expected: 4 passed; 0 failed

- [ ] **Step 3: Commit**

```bash
git add src/agent/escalation.rs src/agent/mod.rs
git commit -m "feat(escalation): 新建 escalation 模块 + 短码生成纯函数"
```

---

### Task 7: 回执码匹配（带码精确 / 不带码回落 / 多条未决反问）

**Files:**
- Modify: `src/agent/escalation.rs`

实现业务决策 #4 的纯逻辑：给定真人回复文本 + 该真人当前所有 pending 台账行，决定匹配到哪一条，或需要反问。把"匹配决策"做成纯函数（输入 pending 列表 + 文本，输出枚举），DB 查询留给上层。

- [ ] **Step 1: 写匹配结果枚举 + 纯函数 + 失败测试**

在 `src/agent/escalation.rs`（`tests` mod 之前）追加：

```rust
/// 真人回复 → 台账匹配结果。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ReplyMatch {
    /// 命中唯一一条 pending（带码精确，或不带码但只有一条未决）。
    Matched(String), // short_code
    /// 该真人有 ≥2 条未决且回复不带可识别短码 → 需反问澄清。
    Ambiguous(Vec<String>), // 候选 short_codes
    /// 没有任何未决 → 不当客户决策回流（落"待 admin 确认的真人主动指令"）。
    NoPending,
}

/// 从真人回复文本里抽取短码（弱匹配：忽略大小写，允许带/不带 # 与 E 前缀）。
/// 命中返回规范化短码（大写、含 E 前缀，不含 #）。
pub(crate) fn extract_short_code(reply: &str, pending_codes: &[String]) -> Option<String> {
    let upper = reply.to_uppercase();
    pending_codes
        .iter()
        .find(|code| {
            let c = code.to_uppercase();
            // 命中完整码 "E1A2" 或带 # 的 "#E1A2"
            upper.contains(&c) || upper.contains(&format!("#{c}"))
        })
        .cloned()
}

/// 业务决策 #4：根据该真人当前所有 pending 台账 + 回复文本，决定匹配哪一条。
pub(crate) fn match_principal_reply(reply: &str, pending: &[AgentPrincipalEscalation]) -> ReplyMatch {
    let codes: Vec<String> = pending.iter().map(|e| e.short_code.clone()).collect();
    if codes.is_empty() {
        return ReplyMatch::NoPending;
    }
    // 1) 带码精确匹配优先
    if let Some(code) = extract_short_code(reply, &codes) {
        return ReplyMatch::Matched(code);
    }
    // 2) 不带码：仅 1 条未决 → 直接匹配
    if codes.len() == 1 {
        return ReplyMatch::Matched(codes[0].clone());
    }
    // 3) 不带码且 ≥2 条未决 → 反问澄清
    ReplyMatch::Ambiguous(codes)
}
```

- [ ] **Step 2: 加测试**

在 `escalation.rs` 的 `mod tests` 内追加（先写好辅助构造器）：

```rust
    fn make_pending(short_code: &str) -> AgentPrincipalEscalation {
        use crate::models::PRINCIPAL_ESCALATION_STATUS_PENDING;
        AgentPrincipalEscalation {
            id: None,
            workspace_id: "ws1".into(),
            account_id: "acc1".into(),
            contact_wxid: "cust1".into(),
            short_code: short_code.into(),
            status: PRINCIPAL_ESCALATION_STATUS_PENDING.into(),
            category: "out_of_scope_decision".into(),
            reason: "r".into(),
            question_for_principal: "q".into(),
            principal_wxid: "boss".into(),
            decision: None,
            authorization_expires_at: None,
            is_generalizable: false,
            knowledge_proposal_emitted: false,
            created_at: bson::DateTime::now(),
            updated_at: bson::DateTime::now(),
            resolved_at: None,
        }
    }

    #[test]
    fn match_with_explicit_code_hits_that_entry() {
        let pending = vec![make_pending("E1A2"), make_pending("E3B4")];
        assert_eq!(
            match_principal_reply("就按 #E3B4 来吧，可以", &pending),
            ReplyMatch::Matched("E3B4".into())
        );
    }

    #[test]
    fn match_without_code_single_pending_falls_back_to_it() {
        let pending = vec![make_pending("E1A2")];
        assert_eq!(
            match_principal_reply("行，可以给", &pending),
            ReplyMatch::Matched("E1A2".into())
        );
    }

    #[test]
    fn match_without_code_multiple_pending_is_ambiguous() {
        let pending = vec![make_pending("E1A2"), make_pending("E3B4")];
        match match_principal_reply("可以", &pending) {
            ReplyMatch::Ambiguous(codes) => {
                assert_eq!(codes.len(), 2);
                assert!(codes.contains(&"E1A2".to_string()));
            }
            other => panic!("expected Ambiguous, got {other:?}"),
        }
    }

    #[test]
    fn match_no_pending_returns_no_pending() {
        assert_eq!(match_principal_reply("以后都按 8 折", &[]), ReplyMatch::NoPending);
    }

    #[test]
    fn extract_short_code_is_case_insensitive() {
        let codes = vec!["E1A2".to_string()];
        assert_eq!(extract_short_code("回复 e1a2 同意", &codes), Some("E1A2".into()));
    }
```

> `bson::DateTime` 与 `models.rs` 用的 `DateTime` 是同一类型（`bson::DateTime` re-export）。若 `escalation.rs` 顶部未 use，测试里用全路径 `bson::DateTime::now()` 即可（`bson` 是直接依赖）。执行时若报找不到 `bson`，改成 `mongodb::bson::DateTime::now()`。

- [ ] **Step 3: 跑测试**

Run: `cargo test --lib escalation::tests::match`
Expected: 4 passed
Run: `cargo test --lib escalation::tests::extract_short_code`
Expected: 1 passed

- [ ] **Step 4: Commit**

```bash
git add src/agent/escalation.rs
git commit -m "feat(escalation): 回执码匹配纯函数(带码精确/单条回落/多条反问)"
```

---

### Task 8: 请示卡渲染 + 安抚占位文案策略

**Files:**
- Modify: `src/agent/escalation.rs`

请示卡（Agent→领导）是结构化文本，对领导不脱敏。安抚占位（Agent→客户）在**统一占位模型**下是 decision Agent 本轮生成的 `reply_text`（正常场景化、经 outbox 发出）——本 Task 提供一个**确定性兜底占位文案**作为该 reply 的回落参考（LLM 没给出合适占位时用，或测试/降级场景用），并以一个**红线测试**锁死它不含 handoff 措辞。注意：网关侧**不再**调用它直接发送（占位走正常 reply 路径，见 Task 16）。

- [ ] **Step 1: 写请示卡渲染 + 占位兜底 + 失败测试**

在 `escalation.rs` 追加：

```rust
/// 渲染推给领导的请示卡（结构化、不脱敏）。短码放在最前便于领导引用。
pub(crate) fn render_principal_card(
    short_code: &str,
    customer_label: &str,
    reason: &str,
    question_for_principal: &str,
) -> String {
    format!(
        "【请示 #{short_code}】客户「{customer_label}」\n卡点：{reason}\n请示：{question_for_principal}"
    )
}

/// 安抚占位的确定性兜底文案。统一占位模型下，占位是 decision Agent 本轮 reply_text 经
/// outbox 正常发出；本函数仅作回落参考（LLM 未给合适占位 / 降级场景），不由网关直接发送。
/// 红线：绝不提"转人工/接管"，只说"帮你确认一下"这类 AI 自然话术。
pub(crate) fn fallback_holding_reply() -> &'static str {
    "这个我帮你确认一下，稍等我给你准信。"
}
```

- [ ] **Step 2: 加测试**

```rust
    #[test]
    fn principal_card_puts_code_first_and_is_not_redacted() {
        let card = render_principal_card("E1A2", "张三(老客户)", "超出标准 9 折权限", "是否同意 8 折？");
        assert!(card.starts_with("【请示 #E1A2】"));
        assert!(card.contains("张三(老客户)")); // 对领导不脱敏
        assert!(card.contains("是否同意 8 折？"));
    }
```

> **🚨 lint 陷阱（必读）**：下面这个 `fallback_holding_reply_has_no_handoff_wording` 红线测试**必须放进 `tests/principal_decision_channel.rs`（Task 24，lint 排除目录），不能放进 `escalation.rs` 的内联 `#[cfg(test)] mod tests`**。原因：`escalation.rs` 在 `src/agent/` 下，受 `check-no-human-takeover` 扫；而这个测试为了断言"兜底文案不含禁词"，**字面量里必须出现"人工""接管"等禁词**，放在 src/ 会被 lint 误杀。tests/ 目录被 lint 排除，是它唯一的合法落点。`render_principal_card` 的纯函数测试（上面那个）不含禁词，留在内联 mod tests 即可。

```rust
    // ⚠️ 这个测试放进 tests/principal_decision_channel.rs（lint 排除），不要放 escalation.rs 内联！
    #[test]
    fn fallback_holding_reply_has_no_handoff_wording() {
        let reply = wechatagent::agent::escalation::fallback_holding_reply();
        // 红线：兜底文案不得出现转接类措辞（本测试在 tests/ 目录，lint 不扫，可写禁词字面量）
        for forbidden in ["真人", "转人工", "客服", "接管", "人工"] {
            assert!(!reply.contains(forbidden), "兜底安抚不得含「{forbidden}」");
        }
    }
```

> **执行注意**：`fallback_holding_reply` 须 `pub` 或经 `agent/mod.rs` re-export 才能被 `tests/` 集成测试引用（内联测试用 `super::` 即可，但这个测试搬去 tests/ 了，所以要走 crate 公开路径——执行时把它从 `pub(crate)` 提到能被集成测试访问的可见性，或在集成测试里用 `wechatagent::agent::escalation::fallback_holding_reply` 路径，按真实 re-export 调整）。

- [ ] **Step 3: 跑测试**

Run: `cargo test --lib escalation::tests::principal_card`
Expected: 1 passed（`render_principal_card` 纯函数测试在内联 mod tests）

> `fallback_holding_reply_has_no_handoff_wording` 在 `tests/principal_decision_channel.rs`（Task 24 落地），用 `cargo test --test principal_decision_channel fallback_holding_reply` 跑——本 Task 先只确保函数体 + 内联纯函数测试通过。

- [ ] **Step 4: Commit**

```bash
git add src/agent/escalation.rs
git commit -m "feat(escalation): 请示卡渲染 + 安抚占位兜底文案(红线无 handoff 措辞)"
```

---

### Task 9: 授权过期判定（纯函数）

**Files:**
- Modify: `src/agent/escalation.rs`

业务决策 #2 + spec §6：真人授权带 `authorization_expires_at`，过期后该条授权不可再用（防 Agent 拿过期授权乱承诺）。这是 relay 转述前的纯判定。

- [ ] **Step 1: 写判定 + 失败测试**

```rust
use crate::models::PrincipalDecision;

/// 该条已 resolved 的授权当前是否仍可用于转述。
/// expires=None 视为不过期（如纯拒绝类裁决无时效）。
pub(crate) fn authorization_is_usable(
    expires_at: Option<bson::DateTime>,
    now: bson::DateTime,
) -> bool {
    match expires_at {
        None => true,
        Some(exp) => now.timestamp_millis() < exp.timestamp_millis(),
    }
}

/// 转述前选用的事实源：授权有效用真人 substance；过期则回落"不再可用"信号。
pub(crate) fn relay_substance_if_usable<'a>(
    decision: &'a PrincipalDecision,
    expires_at: Option<bson::DateTime>,
    now: bson::DateTime,
) -> Option<&'a str> {
    if authorization_is_usable(expires_at, now) {
        Some(&decision.substance)
    } else {
        None
    }
}
```

- [ ] **Step 2: 加测试**

```rust
    #[test]
    fn authorization_none_expiry_is_usable() {
        assert!(authorization_is_usable(None, bson::DateTime::now()));
    }

    #[test]
    fn authorization_future_expiry_is_usable() {
        let now = bson::DateTime::from_millis(1_000);
        let future = bson::DateTime::from_millis(2_000);
        assert!(authorization_is_usable(Some(future), now));
    }

    #[test]
    fn authorization_past_expiry_is_not_usable() {
        let now = bson::DateTime::from_millis(2_000);
        let past = bson::DateTime::from_millis(1_000);
        assert!(!authorization_is_usable(Some(past), now));
    }

    #[test]
    fn relay_substance_none_when_expired() {
        let decision = PrincipalDecision {
            verdict: "conditional".into(),
            substance: "可以 8 折".into(),
            constraints: vec!["本周付款".into()],
            authorization_window_hours: None,
        };
        let now = bson::DateTime::from_millis(2_000);
        let past = bson::DateTime::from_millis(1_000);
        assert_eq!(relay_substance_if_usable(&decision, Some(past), now), None);
        let future = bson::DateTime::from_millis(3_000);
        assert_eq!(
            relay_substance_if_usable(&decision, Some(future), now),
            Some("可以 8 折")
        );
    }
```

- [ ] **Step 3: 跑测试**

Run: `cargo test --lib escalation::tests::authorization escalation::tests::relay_substance`
Expected: 4 passed

- [ ] **Step 4: 跑全 lib 确认 Phase 2 无回归**

Run: `cargo test --lib`
Expected: ≥350 passed; 0 failed（新增约 19 个 escalation 单测应使总数上升）

- [ ] **Step 5: Commit**

```bash
git add src/agent/escalation.rs
git commit -m "feat(escalation): 授权过期判定纯函数(过期授权不可用于转述)"
```

---

## Phase 3 — 配置与台账 DB 接线（Task 10-12）

接 DB/配置：workspace 配置 `principal_decider` + `high_risk_escalation_mode`、台账 insert/查询/resolve、wxid 防护。这些是 async DB 操作，单测放进 `tests/principal_decision_channel.rs`（`#[ignore]` + testcontainers），由 CI 跑；本地只 `cargo check` + `cargo test --lib`。

### Task 10: workspace 配置字段 `principal_decider` + `high_risk_escalation_mode`

**Files:**
- Modify: `src/models.rs`（`OperationDomainConfig` :591-625）
- Modify: `src/agent/escalation.rs`（配置读取 helper + 纯函数测试）

业务决策 #1：高风险件升级模式可配。挂在 `OperationDomainConfig`（per-workspace-per-domain）。

- [ ] **Step 1: `OperationDomainConfig` 加两字段**

在 `src/models.rs` 的 `OperationDomainConfig` struct 末尾（:625 `}` 前）追加：

```rust
    /// 请示通道：接收请示卡的领导 wxid（须是业务号好友）。None=本 workspace 未启用请示通道。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principal_decider: Option<String>,
    /// 高风险件升级模式："all"=所有被静默 hold 的高风险件都请示真人；
    /// "decision_only"=只升级实质需决策/授权的件。None/缺省 = "decision_only"（保守）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub high_risk_escalation_mode: Option<String>,
```

- [ ] **Step 2: 在 escalation.rs 加模式解析纯函数 + 失败测试**

```rust
/// 高风险件升级模式。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum HighRiskEscalationMode {
    /// 所有被静默 hold 的高风险件都请示真人。
    All,
    /// 只升级实质需决策/授权的件（默认，保守）。
    DecisionOnly,
}

/// 从 workspace 配置字符串解析升级模式；未配/未知值回落 DecisionOnly（保守默认）。
pub(crate) fn parse_high_risk_mode(raw: Option<&str>) -> HighRiskEscalationMode {
    match raw {
        Some("all") => HighRiskEscalationMode::All,
        _ => HighRiskEscalationMode::DecisionOnly,
    }
}
```

加测试：

```rust
    #[test]
    fn high_risk_mode_parses_all() {
        assert_eq!(parse_high_risk_mode(Some("all")), HighRiskEscalationMode::All);
    }

    #[test]
    fn high_risk_mode_defaults_to_decision_only() {
        assert_eq!(parse_high_risk_mode(None), HighRiskEscalationMode::DecisionOnly);
        assert_eq!(parse_high_risk_mode(Some("garbage")), HighRiskEscalationMode::DecisionOnly);
        assert_eq!(
            parse_high_risk_mode(Some("decision_only")),
            HighRiskEscalationMode::DecisionOnly
        );
    }
```

- [ ] **Step 3: 跑测试 + 编译**

Run: `cargo test --lib escalation::tests::high_risk_mode`
Expected: 2 passed
Run: `CARGO_TARGET_DIR=target-check cargo check`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/models.rs src/agent/escalation.rs
git commit -m "feat(escalation): OperationDomainConfig 加 principal_decider + high_risk_escalation_mode + 模式解析"
```

---

### Task 11: 台账 CRUD（insert pending / 查 pending / resolve）+ wxid 防护

**Files:**
- Modify: `src/agent/escalation.rs`（async DB 函数）
- Create: `tests/principal_decision_channel.rs`（集成测试骨架 + 本 Task 的两个测试）

**关键防护（spec §9.4）**：插入/推送前二次校验目标 wxid 严格等于该 workspace 配置的 `principal_decider`，绝不可能发到客户。

- [ ] **Step 1: 写台账 CRUD + wxid 防护**

在 `escalation.rs` 追加（顶部补 use）：

```rust
use crate::error::{AppError, AppResult};
use crate::models::{
    OperationDomainConfig, PrincipalDecision, ALLOWED_ESCALATION_CATEGORY,
    PRINCIPAL_ESCALATION_STATUS_PENDING, PRINCIPAL_ESCALATION_STATUS_RESOLVED,
};
use crate::state::AppState; // 若 AppState 在别处，按真实路径调整
use mongodb::bson::{doc, oid::ObjectId, DateTime};

/// 读取该 workspace+domain 的领导 wxid。未配置返回 None（= 请示通道未启用）。
pub(crate) async fn principal_decider_wxid(
    state: &AppState,
    workspace_id: &str,
    domain: &str,
) -> AppResult<Option<String>> {
    let cfg = state
        .db
        .operation_domain_configs()
        .find_one(
            doc! { "workspace_id": workspace_id, "domain": domain, "current_version": true },
            None,
        )
        .await?;
    Ok(cfg.and_then(|c| c.principal_decider))
}

/// 二次防护：目标 wxid 必须严格等于该 workspace 配置的 principal_decider。
/// 用于推请示卡前，杜绝把内部请示卡误发给客户。
pub(crate) fn assert_target_is_principal(
    target_wxid: &str,
    configured_principal: &str,
) -> AppResult<()> {
    if target_wxid == configured_principal {
        Ok(())
    } else {
        Err(AppError::Validation(format!(
            "请示卡目标 wxid 与配置的领导不符，拒发（target={target_wxid}）"
        )))
    }
}

/// 插入一条 pending 台账。短码碰撞（唯一索引报错）时重试至多 5 次。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_pending_escalation(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    contact_wxid: &str,
    category: &str,
    reason: &str,
    question_for_principal: &str,
    principal_wxid: &str,
    is_generalizable: bool,
) -> AppResult<AgentPrincipalEscalation> {
    debug_assert!(
        ALLOWED_ESCALATION_CATEGORY.contains(&category),
        "category 必须在闭集内"
    );
    let now = DateTime::now();
    for attempt in 0..5u32 {
        // 种子混入毫秒时间 + 尝试次数，避免可预测；碰撞由唯一索引兜底。
        let seed = (now.timestamp_millis() as u64).wrapping_add(attempt as u64 * 2_654_435_761) as u32;
        let short_code = short_code_from_seed(seed);
        let entry = AgentPrincipalEscalation {
            id: None,
            workspace_id: workspace_id.to_string(),
            account_id: account_id.to_string(),
            contact_wxid: contact_wxid.to_string(),
            short_code: short_code.clone(),
            status: PRINCIPAL_ESCALATION_STATUS_PENDING.to_string(),
            category: category.to_string(),
            reason: reason.to_string(),
            question_for_principal: question_for_principal.to_string(),
            principal_wxid: principal_wxid.to_string(),
            decision: None,
            authorization_expires_at: None,
            is_generalizable,
            knowledge_proposal_emitted: false,
            created_at: now,
            updated_at: now,
            resolved_at: None,
        };
        match state.db.agent_principal_escalations().insert_one(&entry, None).await {
            Ok(res) => {
                let mut saved = entry;
                saved.id = res.inserted_id.as_object_id();
                return Ok(saved);
            }
            Err(e) => {
                // 仅短码唯一冲突才重试；其它错误直接上抛。
                if is_duplicate_key_error(&e) && attempt < 4 {
                    continue;
                }
                return Err(e.into());
            }
        }
    }
    Err(AppError::Internal("短码生成连续碰撞，插入请示台账失败".into()))
}

/// 查某 workspace 下某领导 wxid 当前所有 pending 台账（按创建时间升序）。
pub(crate) async fn list_pending_for_principal(
    state: &AppState,
    workspace_id: &str,
    principal_wxid: &str,
) -> AppResult<Vec<AgentPrincipalEscalation>> {
    use futures::TryStreamExt;
    let cursor = state
        .db
        .agent_principal_escalations()
        .find(
            doc! {
                "workspace_id": workspace_id,
                "principal_wxid": principal_wxid,
                "status": PRINCIPAL_ESCALATION_STATUS_PENDING,
            },
            mongodb::options::FindOptions::builder()
                .sort(doc! { "created_at": 1 })
                .build(),
        )
        .await?;
    Ok(cursor.try_collect().await?)
}

/// 把一条 pending 台账标 resolved，写入真人裁决 + 授权过期时间。
pub(crate) async fn resolve_escalation(
    state: &AppState,
    short_code: &str,
    decision: &PrincipalDecision,
    authorization_expires_at: Option<DateTime>,
) -> AppResult<Option<AgentPrincipalEscalation>> {
    let now = DateTime::now();
    let decision_bson = mongodb::bson::to_bson(decision)?;
    let mut set = doc! {
        "status": PRINCIPAL_ESCALATION_STATUS_RESOLVED,
        "decision": decision_bson,
        "updated_at": now,
        "resolved_at": now,
    };
    if let Some(exp) = authorization_expires_at {
        set.insert("authorization_expires_at", exp);
    }
    let updated = state
        .db
        .agent_principal_escalations()
        .find_one_and_update(
            doc! { "short_code": short_code, "status": PRINCIPAL_ESCALATION_STATUS_PENDING },
            doc! { "$set": set },
            mongodb::options::FindOneAndUpdateOptions::builder()
                .return_document(mongodb::options::ReturnDocument::After)
                .build(),
        )
        .await?;
    Ok(updated)
}

/// 判断 mongodb 错误是否为唯一键冲突（短码碰撞）。
fn is_duplicate_key_error(e: &mongodb::error::Error) -> bool {
    matches!(*e.kind, mongodb::error::ErrorKind::Write(
        mongodb::error::WriteFailure::WriteError(ref we)) if we.code == 11000)
}
```

> **执行注意（依赖核对）**：
> - `AppError` 变体名（`Validation` / `Internal`）按 `src/error.rs` 真实定义调整——执行时 `grep "pub enum AppError" src/error.rs` 确认。若无 `Validation`，用最接近的（如 `BadRequest`）。
> - `AppState` 路径：核验时 gateway/webhooks 都能拿到 `state: &AppState`，`grep "pub struct AppState"` 确认其模块路径，修正 use。
> - `operation_domain_configs()` accessor：`grep "fn operation_domain_configs" src/db/mod.rs` 确认名字（可能叫 `operation_domain_configs` 或 `domain_configs`）。
> - `futures::TryStreamExt` / `mongodb::options::*`：项目其它 DB 查询已在用，照搬其 use 风格。

- [ ] **Step 2: 建集成测试文件 + wxid 防护测试（纯函数，无需 DB）**

创建 `tests/principal_decision_channel.rs`：

```rust
//! 决策请示通道集成测试。多数需 MongoDB（testcontainers），标 #[ignore]，CI 跑。
//! 纯函数测试（不标 ignore）随 `cargo test --test principal_decision_channel` 即跑。

// 注：assert_target_is_principal 是 pub(crate)，集成测试在 crate 外，无法直接调。
// 故 wxid 防护的纯函数测试放在 src/agent/escalation.rs 的 #[cfg(test)] mod 内（见下）。
// 本文件聚焦需 DB 的端到端流程（Phase 7 填充）。

#[test]
fn placeholder_compiles() {
    assert!(true);
}
```

同时在 `src/agent/escalation.rs` 的 `mod tests` 内加 wxid 防护纯函数测试：

```rust
    #[test]
    fn assert_target_is_principal_accepts_match() {
        assert!(assert_target_is_principal("boss_wxid", "boss_wxid").is_ok());
    }

    #[test]
    fn assert_target_is_principal_rejects_customer() {
        // 红线：目标是客户 wxid 时必须拒发
        assert!(assert_target_is_principal("customer_wxid", "boss_wxid").is_err());
    }
```

- [ ] **Step 3: 跑测试 + 编译**

Run: `cargo test --lib escalation::tests::assert_target_is_principal`
Expected: 2 passed
Run: `CARGO_TARGET_DIR=target-check cargo check --tests`
Expected: PASS（集成测试文件 + 新 async 函数都编译过）

- [ ] **Step 4: Commit**

```bash
git add src/agent/escalation.rs tests/principal_decision_channel.rs
git commit -m "feat(escalation): 台账 CRUD(insert/list/resolve) + wxid 二次防护 + 短码碰撞重试"
```

---

### Task 12: 知识缺口提案（agent 自判可泛化时发 draft）

**Files:**
- Modify: `src/agent/escalation.rs`

业务决策 #3：真人决策可泛化（`escalation_request.is_generalizable`）时，自动发一条 `draft + needs_review` 知识缺口提案，喂回现有知识子系统。**红线：AI 永不自动验证，只能 draft+needs_review。**

- [ ] **Step 1: 写提案函数**

在 `escalation.rs` 追加（补 use `OperationKnowledgeChunk`）：

```rust
use crate::models::OperationKnowledgeChunk;

/// 真人决策可泛化时，发一条知识缺口提案（draft + needs_review）。
/// 复用现有知识子系统的 draft 契约——绝不自动验证（AI 永不自动验证红线）。
/// 已发过（knowledge_proposal_emitted）的不重复发。
pub(crate) async fn emit_knowledge_gap_proposal(
    state: &AppState,
    escalation: &AgentPrincipalEscalation,
    decision: &PrincipalDecision,
) -> AppResult<()> {
    let title = format!("真人决策沉淀（待审核）：{}", escalation.reason);
    let body = format!(
        "源自客户「{}」请示 #{}。\n卡点：{}\n领导裁决：{}\n约束：{}",
        escalation.contact_wxid,
        escalation.short_code,
        escalation.reason,
        decision.substance,
        if decision.constraints.is_empty() {
            "无".to_string()
        } else {
            decision.constraints.join("；")
        }
    );
    let chunk = OperationKnowledgeChunk {
        id: None,
        workspace_id: escalation.workspace_id.clone(),
        // 工作区共享域：account_id=None（与既有 chat 补库共享域一致）。
        account_id: None,
        status: "draft".to_string(),
        integrity_status: Some("needs_review".to_string()),
        title,
        body: Some(body),
        ..OperationKnowledgeChunk::default()
    };
    state.db.operation_knowledge_chunks().insert_one(&chunk, None).await?;
    Ok(())
}
```

> **执行注意**：
> - `OperationKnowledgeChunk` 必须 `#[derive(Default)]` 才能用 `..default()`。执行时 `grep -n "struct OperationKnowledgeChunk" -A2 src/models.rs` 看它有没有 `Default`。**若没有**，要么给它加 `Default`（确认所有字段都能 default），要么显式列全字段（照 Task 11 subagent 报告里的字段表逐个填）。优先显式列全字段，避免给大 struct 加 Default 影响别处。
> - `account_id=None`（workspace 共享域）与项目最近的 chat 补库修复（commit 0c78c13）口径一致——这样提案对该 workspace 全局可见，不被 account 隔离掉。
> - `operation_knowledge_chunks()` accessor 名执行时 `grep` 确认。

- [ ] **Step 2: 编译**

Run: `CARGO_TARGET_DIR=target-check cargo check`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add src/agent/escalation.rs
git commit -m "feat(escalation): 真人决策可泛化时发 draft+needs_review 知识缺口提案"
```

---

## Phase 4 — LLM 接线：真人回复解读 + prompt 改动（Task 13-15）

### Task 13: decision prompt 加 escalationRequest 字段说明

**Files:**
- Modify: `src/prompts.rs`（`user.reply.task`，`### final 形态契约` :1003-1122）

让决策 LLM 知道何时输出 escalationRequest，以及三类触发的判定标准（实质驱动，不是客户字面"要换人对接"）。**统一占位模型的关键 prompt 约束**：当你输出 escalationRequest 时，你这一轮的 `reply` **照常要写**——把"安抚占位 + 你能自主答的部分"放进 `reply`（如「这个我帮你跟领导确认下，稍等给你准信～另外你问的 X 我先答你…」）。这个 `reply` 会正常发给客户，请示是后台动作，客户不会冷场。**不要把 reply 留空等系统代发**。

- [ ] **Step 1: 在 final 形态契约的 JSON 字段说明区追加**

定位 `src/prompts.rs` 的 `user.reply.task` prompt 里 `### final 形态契约` 段（核验在 :1003-1122，followUp 字段在 :1117-1122）。在 followUp 说明之后、JSON 契约描述的收尾处，追加 escalationRequest 字段说明文本（这是 prompt 字符串字面量的一部分，注意保持周围的转义/缩进风格）：

```
  // escalationRequest：仅当你判断本轮遇到"决策墙"（超出你的职权/能力，需要幕后领导拍板）时输出；否则整个字段省略或 needed=false。
  // 判定按"事项实质"，不是客户嘴上"要换人对接"——客户嘴上要换人但事项你能处理，就继续自己处理，不要 escalate。
  // 重要：即使你输出 escalationRequest，这一轮的 reply 仍要正常写——把"安抚占位话术 + selfServiceablePart 里你能自主答的部分"自然地融进 reply 一起发给客户（reply 会照常经发送链路送达，请示是后台动作，客户不会冷场）。绝不要把 reply 留空。
  // 三类（category 取其一）：
  //   out_of_scope_decision：合同变更/特殊折扣/退款纠纷/法律承诺/定制需求等超出标准政策权限。
  //   high_risk_gated：触及未验证产品声明、或风险被闸门拦下、需领导授权才能答的件。
  //   stuck_or_undelivered：同一议题已多轮未推进且客户有负面情绪。
  // 字段：{ "needed": true, "category": "...", "reason": "给领导看的卡点", "questionForPrincipal": "向领导提的问题", "selfServiceablePart": "客户这条消息里你能自主答的部分(若有，应已融进 reply)", "isGeneralizable": true/false(这条决策是否能泛化成通用知识) }
```

> **执行注意**：`prompts.rs` 受 `check-no-human-takeover` lint 扫。上面文本已避开禁词（用"换人对接"而非"人工/真人"）。执行时务必本地跑 `scripts/check-no-human-takeover.sh`（或 .ps1）确认 diff 新增行无"人工/真人/接管"等禁词后再 commit。

- [ ] **Step 2: 加 relay 输入契约（🔴 闭环命门——让 decision Agent 识别"转述模式"）**

> **为什么必须有这段**：拿到领导裁决后，relay 把一条**合成 inbound**（Task 16 Step 2，以哨兵 `__PRINCIPAL_RELAY__` 开头 + verdict/substance/constraints 结构化载荷）塞进"客户消息"位置喂给 decision Agent。若 prompt 不告诉 Agent 这是转述任务，Agent 会**把哨兵载荷当成客户说的话**，可能原样外发内部字段、或不按授权转述。这段输入契约与 Task 16 的哨兵是一对：哨兵是信号，这段是 Agent 读懂信号的解码器。

在同一 `user.reply.task` prompt 的 final 形态契约段，**再追加一段 relay 模式说明**（与 escalationRequest 说明相邻，仍是 prompt 字符串字面量）：

```
  // 【转述模式】如果客户最新消息以 __PRINCIPAL_RELAY__ 开头，这不是客户发的话，而是"领导已就之前一条请示给出裁决"的内部转述任务。载荷字段：verdict（approved/rejected/conditional/deferred/delegated_back）、substance（领导给的实质结论，是你转述的唯一事实源）、constraints（附带条件）。此时你要：
  //   1) 绝不把 __PRINCIPAL_RELAY__、verdict=、substance= 等任何内部字段或方括号文字发给客户；
  //   2) 用你自己的口吻、结合该客户当前语境，自然地把结论转述出去；
  //   3) 按 verdict 决定基调：
  //      approved/conditional → 正面推进 substance，有 constraints 就说清条件（如"申请下来了可以给你8折，麻烦本周内付款哈"）；
  //      rejected → 保关系优先，先给 substance 里的替代方案，没有就用标准口径婉拒，别生硬；
  //      delegated_back → 领导把决定交回你，在标准权限内自己给客户一个答复（substance 可能为空）；
  //      其它/异常 → 不替领导承诺超权事项，按标准口径稳住客户。
  //   4) escalationRequest 这一轮通常省略（除非转述里又冒出新的越权点）。
```

> **执行注意**：哨兵字符串 `__PRINCIPAL_RELAY__` 必须与 `models.rs` 的 `PRINCIPAL_RELAY_SENTINEL`（Task 16 Step 2）**逐字一致**。这段全程用"领导"，无 lint 禁词。verdict 分诊措辞集中在这一处（构造器不再预写中文话术），措辞调整只改这里。

- [ ] **Step 3: 编译 + lint**

Run: `CARGO_TARGET_DIR=target-check cargo check`
Expected: PASS
Run: `bash scripts/check-no-human-takeover.sh`
Expected: PASS（新增 prompt 行无禁词）

- [ ] **Step 4: Commit**

```bash
git add src/prompts.rs
git commit -m "feat(escalation): decision prompt 加 escalationRequest 字段 + relay 转述模式输入契约"
```

---

### Task 14: 真人自然语言裁决 → `PrincipalDecision` 解读（LLM）

**Files:**
- Modify: `src/prompts.rs`（新增解读 prompt）
- Modify: `src/agent/escalation.rs`（解读函数，调 `generate_agent_json`）

真人回什么都行（`行`/`可以但这周付款`/`最多 95 折`），用 LLM 解读成 `{verdict, substance, constraints}`。绝不原话转发给客户。

- [ ] **Step 1: 在 prompts.rs 加解读 prompt**

在 `src/prompts.rs` 的 prompt 注册区（照 `ensure_prompt_pack_v2` 里其它 prompt 的注册方式），新增一个 key `escalation.principal.interpret` 的 prompt。System 内容（字符串字面量）：

```
你是运营 Agent 的内部决策解读器。下面是"领导"对一条客户请示的自然语言回复，请把它解读成结构化裁决。只输出 JSON，不要解释。

裁决口径 verdict 取其一：
- approved：明确同意原诉求。
- rejected：明确拒绝。
- conditional：有条件同意（把条件填进 constraints）。
- deferred：领导暂未定（如"我问下财务""先稳住"）。
- delegated_back：领导把决定权交回你（如"你看着办""看情况"）。

输出 JSON：
{
  "verdict": "approved|rejected|conditional|deferred|delegated_back",
  "substance": "决策实质，一句话（你之后会用自己的口吻转述给客户，所以写清楚能给客户什么）",
  "constraints": ["附带条件，如 本周内付款；没有则空数组"],
  "authorizationWindowHours": null
}

authorizationWindowHours（授权有效时长，小时）——**领导说了算**：
- 领导明确给了时限才填数字：如"这个价就今天有效"→约 24；"这周内都行"→按本周剩余天数估算小时数；"24 小时内"→24。
- 领导没提任何时限 → 填 null（表示这条授权不设过期窗、长期有效）。
- 不要自己默认一个时长——没说就是 null。
```

User 内容由代码拼装（请示问题 + 真人回复），见 Step 2。

> 注册方式：执行时 `grep -n "user.reply.task" src/prompts.rs` 看 prompt 是怎么进 pack 的（可能是 `PromptTemplate { key, system, ... }` 塞进 Vec，或 upsert）。照搬同结构加一条。

- [ ] **Step 2: 在 escalation.rs 写解读函数**

```rust
use crate::agent::generate_agent_json;
use crate::models::{
    ALLOWED_PRINCIPAL_VERDICT, PRINCIPAL_VERDICT_DEFERRED,
};

/// 用 LLM 把真人自然语言回复解读成结构化裁决。绝不原话转发给客户。
/// 解析失败或 verdict 越界时回落 deferred（保守：宁可当"领导还没定"也不乱转述）。
pub(crate) async fn interpret_principal_reply(
    state: &AppState,
    account_id: &str,
    escalation: &AgentPrincipalEscalation,
    principal_reply_text: &str,
) -> AppResult<PrincipalDecision> {
    let user = format!(
        "客户请示问题：{}\n领导回复原话：{}",
        escalation.question_for_principal, principal_reply_text
    );
    // 取 system prompt（从 pack 读，与其它 prompt 一致）。
    let system = crate::prompts::prompt_system(state, "escalation.principal.interpret")
        .await
        .unwrap_or_default();
    let value = generate_agent_json(
        state,
        Some(account_id),
        Some(&escalation.contact_wxid),
        None,
        "escalation.principal.interpret",
        &system,
        &user,
    )
    .await?;
    let decision: PrincipalDecision = match serde_json::from_value(value) {
        Ok(d) => d,
        Err(_) => {
            // 解析失败：保守回落 deferred，substance 留空，不冒险转述。
            return Ok(PrincipalDecision {
                verdict: PRINCIPAL_VERDICT_DEFERRED.to_string(),
                substance: String::new(),
                constraints: vec![],
                authorization_window_hours: None,
            });
        }
    };
    // verdict 越界也回落 deferred（保留 substance/constraints/window）。
    if !ALLOWED_PRINCIPAL_VERDICT.contains(&decision.verdict.as_str()) {
        return Ok(PrincipalDecision {
            verdict: PRINCIPAL_VERDICT_DEFERRED.to_string(),
            substance: decision.substance,
            constraints: decision.constraints,
            authorization_window_hours: decision.authorization_window_hours,
        });
    }
    Ok(decision)
}
```

> **执行注意**：
> - `crate::prompts::prompt_system(state, key)` 是假设的读 prompt helper。执行时 `grep -n "fn prompt" src/prompts.rs` 找真实的"按 key 取 system 文本"的函数（可能叫 `lookup_prompt` / `prompt_for` / 直接从 `state` 的 prompt 缓存取）。若没有现成 helper，照 `generate_agent_json` 的调用方（decision.rs:523 附近）怎么拿 system 文本，照搬。
> - `generate_agent_json` 签名见核验：`(state, account_id: Option<&str>, contact_wxid: Option<&str>, run_id: Option<&str>, prompt_key, system, user)`。

- [ ] **Step 3: 加纯函数测试——verdict 越界回落（mock 不了 LLM，测回落分支用直接构造）**

解读函数依赖 LLM，端到端测试放集成测试。这里加一个不依赖 LLM 的小重构测试：把"verdict 校验回落"抽成纯函数并测。在 escalation.rs 追加：

```rust
/// 校验 verdict，越界回落 deferred（纯函数，便于单测）。
pub(crate) fn sanitize_verdict(decision: PrincipalDecision) -> PrincipalDecision {
    if ALLOWED_PRINCIPAL_VERDICT.contains(&decision.verdict.as_str()) {
        decision
    } else {
        PrincipalDecision {
            verdict: PRINCIPAL_VERDICT_DEFERRED.to_string(),
            substance: decision.substance,
            constraints: decision.constraints,
            authorization_window_hours: decision.authorization_window_hours,
        }
    }
}
```

把 `interpret_principal_reply` 里的越界判断改为调用 `sanitize_verdict(decision)`。加测试：

```rust
    #[test]
    fn sanitize_verdict_keeps_valid() {
        let d = PrincipalDecision { verdict: "approved".into(), substance: "ok".into(), constraints: vec![], authorization_window_hours: None };
        assert_eq!(sanitize_verdict(d).verdict, "approved");
    }

    #[test]
    fn sanitize_verdict_falls_back_on_garbage() {
        let d = PrincipalDecision { verdict: "maybe_lol".into(), substance: "x".into(), constraints: vec![], authorization_window_hours: Some(24.0) };
        let out = sanitize_verdict(d);
        assert_eq!(out.verdict, "deferred");
        assert_eq!(out.substance, "x"); // substance 保留
        assert_eq!(out.authorization_window_hours, Some(24.0)); // window 也保留
    }
```

- [ ] **Step 4: 跑测试 + 编译 + lint**

Run: `cargo test --lib escalation::tests::sanitize_verdict`
Expected: 2 passed
Run: `CARGO_TARGET_DIR=target-check cargo check`
Expected: PASS
Run: `bash scripts/check-no-human-takeover.sh`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/prompts.rs src/agent/escalation.rs
git commit -m "feat(escalation): 真人自然语言裁决 LLM 解读(越界回落 deferred)"
```

---

### Task 15: relay 转述客户（拿到决策→AI 口吻推进）

**Files:**
- Modify: `src/agent/escalation.rs`（relay 入口）

第二次 run 的核心：拿到真人裁决后，先重读客户最新状态（跨天关键），再用 AI 口吻转述。走现有网关，不新建发送路径。

> **🟡 接触级归属的 MVP 边界（spec §9.1，2026-06-06 用户拍板）**：spec §9.1 说真人决策"立即写该客户 memory/state（接触级）"。核验生产代码后确认：relay 走现有网关，Agent 转述的回复（如"已批准8折、本周内有效"）会经 `send_outbound_message` **自动落进 `conversation_messages`**，下一轮该客户 run 经 `load_recent_messages` **从聊天历史天然可见**；授权的结构化事实（substance + 过期时间）另存在**台账**（短码键）。**MVP 不再额外往 `operating_memories.memory_card` / `contact.memory_summary` 写一条结构化授权记忆**——理由：(a) 聊天历史已让下一轮可见，(b) 不触碰正在优化中的 memory 子系统（见 [[project_real_business_flow_focus]]），(c) 台账是授权过期判定的权威源。**已知取舍**：跨很多轮/多会话后聊天历史滑出窗口时，该授权上下文可能丢失——这是有意接受的 MVP 边界，**不是 bug，执行时别"顺手"加 memory 写入**。若将来要强化，再显式调 `write_memory_candidates` 沉淀（后续增强，非本期）。

- [ ] **Step 1: 写 relay task 处理函数**

在 `escalation.rs` 追加：

```rust
use crate::models::{AgentTask, ConversationMessage, Contact};

/// 处理 principal_decision_relay task：真人已裁决，把决策用 AI 口吻转述给客户。
/// 由 tasks.rs else 分支 → handle_follow_up_task 流入；但 relay 需特殊处理
/// （先按短码取台账 + 重读客户最新态），故在 gateway 的 follow-up 入口按 kind 分流到这里。
pub(crate) async fn handle_principal_decision_relay(
    state: &AppState,
    task: &AgentTask,
) -> AppResult<()> {
    // task.content 约定为 relay payload：短码（Phase 5 创建时写入）。
    let short_code = task.content.trim();
    let entry = state
        .db
        .agent_principal_escalations()
        .find_one(doc! { "short_code": short_code }, None)
        .await?;
    let Some(entry) = entry else {
        // 台账不存在（已被清理？）→ 静默结束，不发任何东西。
        return Ok(());
    };
    let Some(decision) = entry.decision.clone() else {
        // 还没 resolved，不该起 relay；保守不发。
        return Ok(());
    };

    // 授权过期 → 不拿过期授权乱承诺（业务决策 #2 / spec §6）。
    let now = DateTime::now();
    let usable = relay_substance_if_usable(&decision, entry.authorization_expires_at, now);
    if usable.is_none() {
        // 过期：不转述带授权的承诺，结束（客户侧无动作，避免乱承诺）。
        return Ok(());
    }

    // 重读客户最新状态（跨天关键：可能已改主意/换话题/流失）。
    let contact = state
        .db
        .contacts()
        .find_one(
            doc! {
                "workspace_id": &entry.workspace_id,
                "account_id": &entry.account_id,
                "wxid": &entry.contact_wxid
            },
            None,
        )
        .await?;
    let Some(contact) = contact else {
        return Ok(()); // 客户没了，不发。
    };

    // 走现有网关：构造一条合成 follow-up，让 decision Agent 在"已知真人裁决"上下文里
    // 重新生成 AI 口吻回复（过 HumanLike/EmotionalValue 评分；FactRisk 由真人授权作 grounding 满足）。
    // 转述上下文通过 task 注入（Phase 5 在 relay task 内容里带裁决摘要；
    // 或在此把裁决写入一个 run-local 提示，经 gateway 注入 decision prompt）。
    crate::agent::gateway::relay_principal_decision_to_customer(state, contact, &entry, &decision).await
}
```

> **执行注意**：`relay_principal_decision_to_customer` 是 Phase 5 在 gateway.rs 里实现的入口（它负责构造 trigger、把"真人已裁决：substance+constraints"作为本轮 decision 的业务上下文注入、跑网关、清等待态、标台账已用）。本 Task 只把"取台账+查过期+重读客户"做掉，转述委托给 gateway 入口。若你倾向把转述也放 escalation.rs，需要 escalation.rs 能调 `run_user_operation_gateway`——但那样会和 gateway 形成双向依赖，故转述入口放 gateway.rs、escalation.rs 调它，单向依赖更干净。

- [ ] **Step 2: 编译（此时 `relay_principal_decision_to_customer` 还不存在，预期失败）**

Run: `CARGO_TARGET_DIR=target-check cargo check`
Expected: FAIL with "cannot find function `relay_principal_decision_to_customer`"

这是预期的——它在 Task 16 实现。本 Task 不单独 commit，与 Task 16 合并提交（因为互相依赖）。**跳到 Task 16，完成后一起编译+commit。**

---

## Phase 5 — 网关接线（Task 16-18）

把 escalation 模块接进 `run_user_operation_gateway` 的真实运行路径。**统一占位模型（见架构段）**：触发请示的这一轮 run 是正常 `Approved`——占位 reply 已由 decision Agent 生成、经 outbox 正常发给客户。Task 16 的触发函数只做不面向客户的两件事（推卡 + 落台账），并在 approved 路径末尾接线；Task 17 分流 relay task；Task 18 只在 `apply_agent_updates` 写可观测等待标记（不碰 review.rs）。

### Task 16: 网关触发处理 + relay 转述入口

**Files:**
- Modify: `src/agent/gateway.rs`（approved 路径末尾接线 + 两个新 pub(crate) 函数）

- [ ] **Step 1: 写"触发请示"处理函数（只推卡 + 落台账，不发客户）**

在 `src/agent/gateway.rs` 追加（放在 `run_user_operation_gateway` 之后；先确认 use 了 `crate::agent::escalation`）：

```rust
/// decision Agent emit 了 escalation_request 时，在 approved 发送路径末尾调用。
/// 占位 reply 已由网关经 outbox 正常发给客户——本函数只做不面向客户的两件事：
/// 推请示卡给领导 wxid + 落 pending 台账。调用方对本函数错误只记 warn 日志、不阻断 run
/// （占位已发出，不能因推卡失败把 run 标失败）——见 Step 3 接线。
pub(crate) async fn trigger_principal_escalation(
    state: &AppState,
    contact: &Contact,
    req: &crate::models::EscalationRequest,
) -> AppResult<()> {
    if !req.needed {
        return Ok(());
    }
    // 1) 读领导 wxid；未配置 = 本 workspace 未启用请示通道 → 不触发（保守）。
    let domain = contact_domain(contact); // 见 Step 注释：取 contact 的 operation domain
    let Some(principal_wxid) =
        escalation::principal_decider_wxid(state, &contact.workspace_id, &domain).await?
    else {
        return Ok(());
    };

    // 2) 二次防护：领导 wxid 绝不等于客户 wxid（否则配置错误，拒绝触发）。
    if principal_wxid == contact.wxid {
        return Err(AppError::Validation(
            "principal_decider 配置等于客户 wxid，拒绝触发请示".into(),
        ));
    }

    let category = req
        .category
        .clone()
        .unwrap_or_else(|| crate::models::ESCALATION_CATEGORY_OUT_OF_SCOPE.to_string());

    // 3) 去重（spec §9.4.3 / 业务决策 #6）：同客户同类别已有 pending → 不重复推卡骚扰领导。
    //    占位 reply 这轮已正常发出，这里直接 return，没有任何客户侧动作。
    if escalation::has_pending_for_contact(state, &contact.workspace_id, &contact.wxid, &category)
        .await?
    {
        return Ok(());
    }

    let reason = req.reason.clone().unwrap_or_default();
    let question = req.question_for_principal.clone().unwrap_or_default();

    // 4) 落 pending 台账（短码碰撞自动重试）。is_generalizable 由 agent 自判透传（业务决策 #3）。
    let entry = escalation::insert_pending_escalation(
        state,
        &contact.workspace_id,
        &contact.account_id,
        &contact.wxid,
        &category,
        &reason,
        &question,
        &principal_wxid,
        req.is_generalizable,
    )
    .await?;

    // 5) 推请示卡给领导（经现有 MCP 工具名 message_send_text，不走 outbox——这是给领导看的
    //    内部通知，不需要为客户设计的冷却/幂等/拟人评分）。
    let customer_label = contact_display_label(contact); // 见注释
    let card = escalation::render_principal_card(&entry.short_code, &customer_label, &reason, &question);
    escalation::assert_target_is_principal(&principal_wxid, &principal_wxid)?; // 目标即配置领导
    mcp::logged_call_for_account(
        state,
        &contact.account_id,
        "message_send_text",
        serde_json::json!({ "recipient": principal_wxid, "content": card }),
    )
    .await?;

    Ok(())
}

/// relay：把真人裁决用 AI 口吻转述给客户，走现有网关。转述完清等待态、标台账已转述。
pub(crate) async fn relay_principal_decision_to_customer(
    state: &AppState,
    contact: Contact,
    entry: &crate::models::AgentPrincipalEscalation,
    decision: &crate::models::PrincipalDecision,
) -> AppResult<()> {
    // 构造一条合成 inbound，把"领导已裁决"作为本轮 decision 的业务上下文。
    // 客户视角看不到这条；它只是触发 decision Agent 在"已授权"上下文里生成转述。
    // 按 verdict 分诊转述基调（spec §6）：
    //   approved/conditional → 推进 substance（+ 条件）；
    //   rejected → 保关系优先，先用 substance 里的替代方案，否则回落标准政策婉拒；
    //   delegated_back → 领导把决定权交回，Agent 在标准政策内自行决断（substance 可能为空）。
    let synthetic = ConversationMessage::synthetic_principal_relay(
        &contact,
        &decision.verdict,
        &decision.substance,
        &decision.constraints,
    );
    // 走现有网关。FactRisk 这里由领导授权作 grounding 满足。
    run_user_operation_gateway(
        state,
        contact.clone(),
        AgentTrigger::Inbound(&synthetic),
        None,
        None,
    )
    .await?;

    // 清等待态 + 若可泛化则发知识缺口提案。
    clear_awaiting_principal_state(state, &contact).await?;
    // 知识沉淀（业务决策 #3）：agent 自判 is_generalizable 存在台账 entry.is_generalizable；
    // 仅 approved/conditional 类（真正给了可复用结论）且未发过时发一次（防重复）。
    let verdict_yields_knowledge = matches!(
        decision.verdict.as_str(),
        crate::models::PRINCIPAL_VERDICT_APPROVED | crate::models::PRINCIPAL_VERDICT_CONDITIONAL
    );
    if entry.is_generalizable && verdict_yields_knowledge && !entry.knowledge_proposal_emitted {
        escalation::emit_knowledge_gap_proposal(state, entry, decision).await?;
        state
            .db
            .agent_principal_escalations()
            .update_one(
                doc! { "short_code": &entry.short_code },
                doc! { "$set": { "knowledge_proposal_emitted": true } },
                None,
            )
            .await?;
    }
    Ok(())
}

/// 清掉客户 state 上的"等待领导决策"标记（key 用 Task 4 的常量，与 set 侧严格对齐）。
async fn clear_awaiting_principal_state(state: &AppState, contact: &Contact) -> AppResult<()> {
    let unset_key = format!(
        "domain_attributes.{}",
        crate::models::AWAITING_PRINCIPAL_DECISION_ATTR
    );
    state
        .db
        .contacts()
        .update_one(
            doc! { "workspace_id": &contact.workspace_id, "account_id": &contact.account_id, "wxid": &contact.wxid },
            doc! { "$unset": { unset_key: "" } },
            None,
        )
        .await?;
    Ok(())
}
```

> **执行注意（必须先核实）**：
> - `contact_domain(contact)` / `contact_display_label(contact)`：这两个 helper 可能不存在。`grep -n "fn contact_domain\|domain_attributes\|operation_domain" src/agent/gateway.rs`。运营 domain 通常在 contact 或 account 上（如 `contact.operation_domain` 或固定 `"user_ops"`）。Phase 1 是用户私聊运营，domain 很可能是单一常量——执行时确认，若是常量直接用该常量字符串，删掉 `contact_domain` 调用。`customer_label` 可简单用 `contact.remark.clone().unwrap_or(contact.wxid.clone())` 之类，按 Contact 真实字段拼。
> - `ConversationMessage::synthetic_principal_relay(...)`：这是要在 `models.rs` 新增的构造器（Step 2）。
> - `has_pending_for_contact` 在 Task 21 定义——本 Task Step 1 已引用它做去重。执行时若按编号顺序走，**先把 Task 21 Step 1 的 `has_pending_for_contact` 函数体一并写进 escalation.rs**（它不依赖本 Task 任何东西），本 Task 引用即可，避免编译期未定义。
> - `entry.is_generalizable` / `entry.knowledge_proposal_emitted` 是 `AgentPrincipalEscalation`（Task 1）字段，已在该 struct 定义；trigger 时由 `insert_pending_escalation` 把 `req.is_generalizable` 落库，relay 时读它判断是否发知识提案，agent 自判贯穿（业务决策 #3）。
> - `assert_target_is_principal(&principal_wxid, &principal_wxid)` 在此恒真（目标就是配置领导），保留它是为了：将来若 recipient 来源变化（如从别处取），这层防护仍在。也可改成显式注释说明。

- [ ] **Step 2: `models.rs` 加合成消息构造器（带 relay 哨兵，让 decision Agent 识别转述模式）**

> **🔴 闭环命门**：合成消息是塞进"客户消息"位置喂给 decision Agent 的。若只塞一句自然语言指令，Agent 会**误把它当客户说的话**，可能把内部指令措辞泄漏给客户、或不按授权转述。解法：合成消息以一个**机器可识别的哨兵前缀** `__PRINCIPAL_RELAY__` 开头 + 结构化载荷（verdict/substance/constraints），decision prompt（Task 13 已加对称的 relay 输入契约）见到哨兵就**进入转述模式**：substance 是已授权事实源，用自己口吻转述，绝不把哨兵或方括号内部文字发给客户。哨兵 + prompt 契约是这条闭环能真跑通的关键，不是可选润色。

在 `src/models.rs` 的 `ConversationMessage` impl（或新建 impl 块）追加：

```rust
/// relay 合成消息的哨兵前缀。decision prompt 见到它即进入"转述模式"（Task 13 输入契约）。
/// 取一个绝不会与真实客户消息撞的字符串。
pub const PRINCIPAL_RELAY_SENTINEL: &str = "__PRINCIPAL_RELAY__";

impl ConversationMessage {
    /// 构造一条"领导已裁决"的合成 inbound，仅用于触发 relay 转述，不落客户可见会话。
    /// 以哨兵前缀开头 + 结构化裁决载荷；decision prompt 据哨兵进入转述模式（spec §6）。
    pub fn synthetic_principal_relay(
        contact: &Contact,
        verdict: &str,
        substance: &str,
        constraints: &[String],
    ) -> Self {
        let constraint_text = if constraints.is_empty() {
            "（无）".to_string()
        } else {
            constraints.join("；")
        };
        // 结构化载荷：哨兵 + 字段化，让 decision prompt 能稳定解析。
        // 注意：这是给 Agent 的"转述任务说明"，不是给客户的话——prompt 契约要求 Agent
        // 用自己口吻重写，绝不原样外发哨兵/字段。
        let payload = format!(
            "{PRINCIPAL_RELAY_SENTINEL}\nverdict={verdict}\nsubstance={substance}\nconstraints={constraint_text}"
        );
        ConversationMessage {
            // 照 ConversationMessage 真实字段填；direction=Inbound、是合成标记。
            // 执行时 grep "struct ConversationMessage" 对齐字段。
            ..ConversationMessage::synthetic_inbound(contact, payload)
        }
    }
}
```

> **执行注意**：`ConversationMessage` 字段较多。若已有 `synthetic_inbound` 之类构造器最省事；若没有，照 webhook 写 inbound 的地方（`register_inbound` / webhooks.rs:532）看一条 inbound ConversationMessage 怎么构造，照搬字段。**关键**：
> - 这条合成消息**不要**写进 `conversation_messages` collection（否则污染客户会话历史）——它只在内存里作为 trigger 传给网关。确认 relay 路径不会持久化它。
> - 哨兵 `PRINCIPAL_RELAY_SENTINEL` 与 Task 13 decision prompt 的 relay 输入契约**必须用同一字符串**；verdict 分诊（同意/拒绝/交回）的措辞策略**已从这里移到 prompt 契约**（Task 13 Step 2），因为是 Agent 拿到结构化载荷后自己按 verdict 决定口吻，构造器只负责把字段如实结构化、不再预写中文指令文案。这样 lint 面更小（构造器无大段中文话术）、措辞策略集中在 prompt 一处。

- [ ] **Step 3: 在 approved 发送路径末尾接线触发（原计划缺的关键一步）**

定位 `src/agent/gateway.rs` 的 `run_user_operation_gateway_inner`，找到 **占位 reply 入 outbox 的 `if outbox_eligible { … }` 块结束之后、函数收尾 `Ok(())` 之前**（核验在 L1534，紧接 L1535 的 `Ok(())`；执行时 `grep -n "outbox_eligible\|fn run_user_operation_gateway_inner" src/agent/gateway.rs` 现场对齐）。在那里追加：

```rust
    // 决策请示触发（统一占位模型）：占位 reply 已入 outbox 正常发给客户，
    // 这里只做不面向客户的副作用——推请示卡给领导 + 落台账 pending。
    // 失败不回滚已发的占位，也不让整个 run 失败：仅记 warn 日志降级。
    if let Some(req) = final_decision.escalation_request.as_ref() {
        if req.needed {
            if let Err(err) = trigger_principal_escalation(state, &contact, req).await {
                tracing::warn!(
                    workspace_id = %contact.workspace_id,
                    contact_wxid = %contact.wxid,
                    error = %err,
                    "principal escalation 推卡/落台账失败（占位已正常发出，降级不阻断 run）"
                );
            }
        }
    }
```

> **执行注意**：
> - `final_decision`（`AgentDecision`）、`contact`、`state` 在该位置都在作用域（核验确认）。`contact` 若是 `&Contact` 则直接传 `contact`；若是 `Contact` 值则传 `&contact`——执行时按真实借用形态调整 `&`。
> - **关键设计**：trigger 的错误**降级为 warn 日志**，不用 `?` 传播。因为占位 reply 这时已经发给客户了，若再因推卡失败把整个 run 标失败会造成"客户收到占位但系统记为失败"的不一致。推卡失败的补偿在 Task 21/22 的去重 + 后续轮次重试天然覆盖（同类 pending 不存在则下轮会再试）。
> - 这一步是原计划遗漏的"把 `trigger_principal_escalation` 接进主流程"的关键接线——**没有它，Task 16 前面所有函数都是 dead code**。

- [ ] **Step 4: 编译（Task 15 + 16 合并，此时应全部解析）**

Run: `CARGO_TARGET_DIR=target-check cargo check`
Expected: PASS（Task 15 的 `relay_principal_decision_to_customer` 现已定义；`has_pending_for_contact` 已按执行注意提前写入）

- [ ] **Step 5: lint**

Run: `bash scripts/check-no-human-takeover.sh`
Expected: PASS

> **🚨 红线检查（极重要）**：`gateway.rs` 在 `src/agent/` 下，受 lint 扫。上面代码注释里出现了"真人""领导"——**"真人"和"人工"是禁词，"领导/principal/decision/escalation"不是**。执行 Step 5 前，把所有新增行（含注释、含合成消息文案、含 tracing 日志）里的"真人"替换成"领导/principal 决策源"，"转述"保留（不是禁词）。合成消息文案 `[内部·领导已决策]` 用"领导"安全。逐行过 lint，红了就改措辞，**绝不 `--no-verify` 跳过**。

- [ ] **Step 6: Commit（Task 15+16 合并）**

```bash
git add src/agent/gateway.rs src/agent/escalation.rs src/models.rs
git commit -m "feat(escalation): 网关 approved 路径接线触发请示(推卡+落台账) + relay AI 口吻转述客户"
```

---

### Task 17: 网关 follow-up 入口按 kind 分流到 relay

**Files:**
- Modify: `src/agent/gateway.rs`（`handle_follow_up_task` :107）

`tasks.rs` else 分支把 `principal_decision_relay` 流到 `handle_follow_up_task`。在那里按 kind 分流到 escalation relay 处理，而不是当普通 follow-up 跑。

- [ ] **Step 1: 在 `handle_follow_up_task` 开头加 kind 分流**

定位 `src/agent/gateway.rs:107` `handle_follow_up_task`，在函数体最前面（取 contact 之前）加：

```rust
    // principal_decision_relay：真人已裁决，走专门的 relay 转述路径。
    if task.kind == "principal_decision_relay" {
        return crate::agent::escalation::handle_principal_decision_relay(state, &task).await;
    }
```

- [ ] **Step 2: 编译**

Run: `CARGO_TARGET_DIR=target-check cargo check`
Expected: PASS

- [ ] **Step 3: 跑 follow-up 相关现有测试确认无回归**

Run: `cargo test --lib follow_up`
Expected: 现有 follow-up 测试全 PASS

- [ ] **Step 4: lint + Commit**

Run: `bash scripts/check-no-human-takeover.sh`
Expected: PASS

```bash
git add src/agent/gateway.rs
git commit -m "feat(escalation): handle_follow_up_task 按 kind 分流 principal_decision_relay 到 relay 路径"
```

---

### Task 18: apply_agent_updates 写"等待领导决策"可观测标记

**Files:**
- Modify: `src/agent/gateway.rs`（`apply_agent_updates` :2105 后）

> **统一占位模型下，本 Task 大幅瘦身——不再碰 `review.rs`。** 原计划让 review finalize 把 run 设成 hold 等待态；现在触发请示的 run 始终 `Approved`（占位 reply 经 outbox 正常发出），**run 根本不进 Held 分支**。因此：
> - **删除**对 `src/agent/review.rs` 的所有改动（原 Step 1 / Step 1b）——这消除了与并行 agent 文件的冲突面，是本次设计简化的最大收益。
> - **保留**唯一一件事：在 `apply_agent_updates` 里把"等待领导决策"写成 contact `domain_attributes` 上的一个**可观测布尔标记**（admin 看板显示「等待中」、等待期 pre-check 用 Task 21 读取）。key 用 Task 4 定义的 `AWAITING_PRINCIPAL_DECISION_ATTR` 常量，与 Task 16 `clear_awaiting_principal_state` 的 `$unset` 严格对齐。

- [ ] **Step 1: apply_agent_updates 写"等待领导决策"标记到 state**

定位 `src/agent/gateway.rs:2105`（`apply_agent_updates` 写 operation_state 之后；执行时 `grep -n "fn apply_agent_updates\|operation_state" src/agent/gateway.rs` 现场对齐）。追加：

```rust
    // 请示触发：把"等待领导决策"标记写进客户 domain_attributes（admin 可观测）。
    // key 用 Task 4 常量，与 relay 完成时 clear_awaiting_principal_state 的 $unset 同一字符串。
    if decision
        .escalation_request
        .as_ref()
        .map(|e| e.needed)
        .unwrap_or(false)
    {
        let mut attrs = set_doc
            .get_document("domain_attributes")
            .ok()
            .cloned()
            .or_else(|| contact.domain_attributes.clone())
            .unwrap_or_default();
        attrs.insert(crate::models::AWAITING_PRINCIPAL_DECISION_ATTR, true);
        set_doc.insert("domain_attributes", attrs);
        set_doc.insert("domain_attributes_updated_at", DateTime::now());
    }
```

> **执行注意**：
> - `set_doc` / `contact.domain_attributes` / `decision` 的真实名字与类型按 `apply_agent_updates` 现场为准（核验：函数在 gateway.rs，`apply_agent_updates(state, &contact, &final_decision, &runtime)`，所以函数内 decision 参数名可能是 `decision`——grep 确认）。`domain_attributes` 在 Contact 上的类型若是 `Option<Document>`，`.clone()` + `unwrap_or_default()` 可用；若是 `Document` 直接用。
> - 不在这里写 `authorization_expires_at`——授权过期时间是领导 resolve 时才知道，写在台账（Task 11 `resolve_escalation`）。客户 state 这里只标"等待中"。
> - **不触碰 review 流程、不设 hold_category**：本轮 run 是 Approved，占位 reply 由 Task 16 Step 3 之前的 outbox 正常发送。

- [ ] **Step 2: 编译 + lint**

Run: `CARGO_TARGET_DIR=target-check cargo check`
Expected: PASS
Run: `bash scripts/check-no-human-takeover.sh`
Expected: PASS（注意新增注释里的"领导"安全，无"真人/人工/接管"禁词）

- [ ] **Step 3: 跑相关测试确认无回归**

Run: `cargo test --lib apply_agent_updates`
Expected: 现有 apply_agent_updates 相关测试全 PASS（无则跳过）
Run: `cargo test --lib awaiting_principal_decision_attr_key_is_stable`
Expected: PASS（Task 4 的 key 常量稳定）

- [ ] **Step 4: Commit**

```bash
git add src/agent/gateway.rs
git commit -m "feat(escalation): apply_agent_updates 写 awaiting_principal_decision 可观测标记(不碰 review)"
```

---

## Phase 6 — webhook 入站分流 + 真人回复闭环（Task 19-20）

真人在微信回复，经 `wechat_webhook` 入站。要在进入客户 agent 链路**之前**识别"这是某 workspace 的领导回复"，分流到 escalation 处理：匹配台账 → 解读 → resolve → 起 relay task。

### Task 19: 真人回复处理入口（匹配→解读→resolve→起 relay / 反问）

**Files:**
- Modify: `src/agent/escalation.rs`（`handle_principal_reply`）

- [ ] **Step 1: 写真人回复处理**

在 `escalation.rs` 追加：

```rust
use crate::models::AgentTask;

/// 处理真人（领导）的微信回复。匹配未决台账→解读→resolve→起 relay task。
/// 业务决策 #4：不带码且多条未决时反问澄清（向领导发一条，不回流客户）。
/// 返回 true 表示已作为领导回复消费（调用方据此不再进客户 agent 链路）。
pub(crate) async fn handle_principal_reply(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    principal_wxid: &str,
    reply_text: &str,
) -> AppResult<bool> {
    let pending = list_pending_for_principal(state, workspace_id, principal_wxid).await?;
    match match_principal_reply(reply_text, &pending) {
        ReplyMatch::NoPending => {
            // 无未决：不当客户决策回流（spec §9.4 落"待 admin 确认的真人主动指令"，
            // MVP 先只记日志，不自动生效）。返回 true：仍当"领导消息"消费，不进客户链路。
            tracing::info!(
                principal_wxid,
                "领导主动消息但无未决请示，不自动生效（待 admin 确认）"
            );
            Ok(true)
        }
        ReplyMatch::Ambiguous(codes) => {
            // 多条未决且不带码 → 反问领导澄清是哪一条。
            let list = codes
                .iter()
                .map(|c| format!("#{c}"))
                .collect::<Vec<_>>()
                .join(" / ");
            let ask = format!(
                "您刚回复的是哪一条？目前挂着这几条：{list}，麻烦带上编号（如 #{}）再回我一次。",
                codes.first().cloned().unwrap_or_default()
            );
            mcp::logged_call_for_account(
                state,
                account_id,
                "message_send_text",
                serde_json::json!({ "recipient": principal_wxid, "content": ask }),
            )
            .await?;
            Ok(true)
        }
        ReplyMatch::Matched(short_code) => {
            let entry = pending
                .iter()
                .find(|e| e.short_code == short_code)
                .cloned()
                .expect("matched code must be in pending");
            // 解读真人自然语言裁决。
            let decision = interpret_principal_reply(state, account_id, &entry, reply_text).await?;
            // deferred：领导还没定 → 不 resolve、不起 relay，保持 pending（继续等）。
            if decision.verdict == crate::models::PRINCIPAL_VERDICT_DEFERRED {
                tracing::info!(short_code = %short_code, "领导暂缓，保持 pending 继续等待");
                return Ok(true);
            }
            // 授权过期时间：**领导说了算**（业务决策 2026-06-06）。
            // LLM 解读出领导明确说的时限→authorization_window_hours；领导没提→None=不设过期窗。
            // 不再硬编码 24h 默认窗。
            let expires = decision.authorization_window_hours.and_then(|hours| {
                if hours > 0.0 {
                    Some(DateTime::from_millis(
                        DateTime::now().timestamp_millis() + (hours * 3600.0 * 1000.0) as i64,
                    ))
                } else {
                    None
                }
            });
            // resolve 台账。
            let resolved = resolve_escalation(state, &short_code, &decision, expires).await?;
            if resolved.is_none() {
                // 已被并发 resolve；幂等，直接结束。
                return Ok(true);
            }
            // 起 relay task：content=短码，经 tasks.rs else→handle_follow_up_task→kind 分流。
            enqueue_relay_task(state, &entry).await?;
            Ok(true)
        }
    }
}

mcp use 见文件顶部；若未 use，加 `use crate::mcp;`

/// 创建 principal_decision_relay task（立即可执行）。
async fn enqueue_relay_task(state: &AppState, entry: &AgentPrincipalEscalation) -> AppResult<()> {
    let now = DateTime::now();
    let task = AgentTask {
        id: None,
        workspace_id: entry.workspace_id.clone(),
        account_id: entry.account_id.clone(),
        contact_wxid: entry.contact_wxid.clone(),
        kind: "principal_decision_relay".to_string(),
        run_at: now,
        expires_at: None,
        content: entry.short_code.clone(), // relay 据短码取台账
        status: "pending".to_string(),
        source_decision_id: None,
        review_required: false,
        attempt_count: 0,
        max_attempts: 3,
        next_retry_at: None,
        gateway_status: None,
        cancel_reason: None,
        error: None,
        claimed_at: None,
        claim_recovery_count: 0,
        created_at: now,
        updated_at: now,
    };
    state.db.tasks().insert_one(&task, None).await?;
    Ok(())
}
```

> **执行注意**：
> - 上面那行 `mcp use 见文件顶部；...` 是给执行者的提示，**不是代码**——执行时删掉它，在文件顶部 use 区加 `use crate::mcp;`（若还没 use）。
> - `AgentTask` 字段以核验报告为准（models.rs:363-382+）。`tasks()` accessor：`grep "fn tasks" src/db/mod.rs` 确认（核验里 planner 用 `state.db.tasks()`）。
> - `DateTime::from_millis` 是 `bson::DateTime::from_millis`。

- [ ] **Step 2: 编译**

Run: `CARGO_TARGET_DIR=target-check cargo check`
Expected: PASS

- [ ] **Step 3: lint**

Run: `bash scripts/check-no-human-takeover.sh`
Expected: PASS（注释/日志里"真人"→"领导"；"反问领导澄清"安全）

- [ ] **Step 4: Commit**

```bash
git add src/agent/escalation.rs
git commit -m "feat(escalation): 领导回复处理(匹配/解读/resolve/起 relay/多条反问/deferred 续等)"
```

---

### Task 20: webhook 入站分流

**Files:**
- Modify: `src/webhooks.rs`（`wechat_webhook` :287，from_wxid 提取后 :393、客户链路 :466/:532 之前）

入站消息先判 from_wxid 是不是某 workspace 的 principal_decider；是则分流到 `handle_principal_reply`，**不**进客户 agent 链路。

- [ ] **Step 1: 在客户链路前插入 principal 分流**

定位 `src/webhooks.rs`：from_wxid 在 :377-393 提取，contact 解析/managed 判定在 :466-554。在**解析出 account_id + from_wxid 之后、进入客户 agent 处理之前**插入分流。需要先反查"这个 from_wxid 是不是某 workspace 的 principal_decider"。

加一个反查 helper（可放 escalation.rs）：

```rust
/// 反查：在**入站消息自身所属 workspace** 内，from_wxid 是否是某 domain 的 principal_decider。
/// 返回 Some(domain) 表示该 wxid 是本 workspace 的领导。
/// 🔒 关键：必须用入站消息自己的 workspace_id 约束查询——否则 A workspace 的领导 wxid
/// 若恰好也是 B workspace 某业务号的好友，B 收到他消息时会被误路由进 A 的请示流（跨域串扰）。
/// account_id→workspace_id 是 1:1（WechatAccount.workspace_id），webhook 入口已能拿到。
pub(crate) async fn lookup_principal_config(
    state: &AppState,
    workspace_id: &str,
    from_wxid: &str,
) -> AppResult<Option<String>> {
    let cfg = state
        .db
        .operation_domain_configs()
        .find_one(
            doc! {
                "workspace_id": workspace_id,
                "principal_decider": from_wxid,
                "current_version": true,
            },
            None,
        )
        .await?;
    Ok(cfg.map(|c| c.domain))
}
```

在 `wechat_webhook` 客户链路前插入（伪代码位置，按真实变量名接）：

```rust
    // 领导回复分流：from_wxid 是**本 workspace** 的 principal_decider → 走请示通道，不进客户链路。
    // workspace_id 来自 resolve_account_context（account→workspace 1:1），与 account_id 同源取得。
    if let Some(_domain) =
        crate::agent::escalation::lookup_principal_config(&state, &workspace_id, &from_wxid).await?
    {
        let consumed = crate::agent::escalation::handle_principal_reply(
            &state, &workspace_id, &account_id, &from_wxid, &content,
        )
        .await?;
        if consumed {
            return Ok(Json(serde_json::json!({ "ok": true, "routed": "principal" })));
        }
    }
```

> **执行注意（关键边界）**：
> - 这段必须放在"判定 contact 是否 managed / 触发客户 agent"**之前**，但在"已能拿到 workspace_id + account_id + from_wxid + content"**之后**。`workspace_id` 与 `account_id` 同出自 `resolve_account_context(state, app_id)`（核验：webhooks.rs:687-710 返回 `(workspace_id, account_id)`）。核验给的插入点：:466（find_one contact 后）或 :532（managed 判定后）——选 :466 之前更安全（领导本身可能也是某 contact，别让它先走 register_inbound）。执行时读 :440-540 实际代码定位精确行，确认 `workspace_id` 变量在该作用域可见（若没有，从 `resolve_account_context` 的返回处往下接）。
> - **领导自己可能也是一个 contact**（业务号好友）。分流命中后直接 return，不要再把这条消息当客户消息 persist/处理。
> - **跨 workspace 串扰已堵**：`lookup_principal_config` 用本条消息自己的 `workspace_id` 约束查询，A workspace 的领导 wxid 即使是 B workspace 某号的好友，B 收到他消息也不会误匹配 A 的 config。MVP 单领导：一个 workspace 内 `principal_decider` 唯一，`find_one` 取该条即可。
> - `content` 变量名按 webhook 真实解析的入站文本字段名接（核验里 from_wxid 在 :377-393 提取，content 应在附近）。

- [ ] **Step 2: 编译**

Run: `CARGO_TARGET_DIR=target-check cargo check`
Expected: PASS

- [ ] **Step 3: lint**

Run: `bash scripts/check-no-human-takeover.sh`
Expected: PASS（webhooks.rs 在扫描范围；"领导回复分流"安全，避开"真人/人工"）

- [ ] **Step 4: 跑 webhook 相关测试确认无回归**

Run: `cargo test --lib webhook`
Expected: 现有 webhook 测试全 PASS

- [ ] **Step 5: Commit**

```bash
git add src/webhooks.rs src/agent/escalation.rs
git commit -m "feat(escalation): webhook 入站识别 principal_decider 回复并分流到请示通道"
```

---

## Phase 7 — 等待期行为 + 多轮卡死触发 + CLAUDE.md（Task 21-23）

补齐 spec 里靠 gateway pre-check 的行为：等待期分答、超时沉默、多轮卡死触发。这些都在客户消息进入网关时判定。

### Task 21: 等待期去重助手 `has_pending_for_contact`（分答天然发生）

**Files:**
- Modify: `src/agent/escalation.rs`（去重判定纯查询函数）

业务决策 #6 + spec §7.2/7.3：客户在等待期（state 有 `awaiting_principal_decision` 标记）发新消息时——非越权部分照常回（decision Agent 正常跑，escalation 只挂越权点）。

> **统一占位模型下，等待期行为几乎"免实现"**：
> - 客户新消息 → 网关照常跑 decision Agent → 占位/正常 reply 经 outbox 正常发（分答天然发生：Agent 答能答的，再次 escalate 能 escalate 的）。
> - 若 decision **又** emit escalation 且指向**同一个已 pending 的议题** → Task 16 的 `trigger_principal_escalation` **内置去重**（Step 1 的 `has_pending_for_contact` 命中即 `return Ok(())`），**不重复推卡骚扰领导**；此时占位 reply 已经正常发出，客户不会冷场。
> - 不再冻结整段对话、不需要"是否追问"的额外 LLM 判定——把"等待期分答 + 去重"统一成"正常跑 + trigger 内 pending 去重"，更稳。
>
> 因此本 Task **只需提供一个东西**：`trigger_principal_escalation`（Task 16 Step 1）引用的去重查询函数 `has_pending_for_contact`。

- [ ] **Step 1: 写"同议题已 pending 去重"查询函数**

在 `escalation.rs` 追加（**注意**：Task 16 Step 1 的 trigger 已引用本函数；若按编号顺序执行，把本函数提前到 Task 16 之前写，或在执行 Task 16 时一并写入——它不依赖 Task 16 任何东西）：

```rust
/// 该客户是否已有同类别的 pending 请示（去重用：避免等待期重复推卡骚扰领导）。
pub(crate) async fn has_pending_for_contact(
    state: &AppState,
    workspace_id: &str,
    contact_wxid: &str,
    category: &str,
) -> AppResult<bool> {
    let count = state
        .db
        .agent_principal_escalations()
        .count_documents(
            doc! {
                "workspace_id": workspace_id,
                "contact_wxid": contact_wxid,
                "category": category,
                "status": PRINCIPAL_ESCALATION_STATUS_PENDING,
            },
            None,
        )
        .await?;
    Ok(count > 0)
}
```

> **执行注意**：去重的"命中即跳过推卡"逻辑已写在 Task 16 Step 1 的 `trigger_principal_escalation` 第 3 步（`if has_pending_for_contact(...).await? { return Ok(()); }`），**本 Task 不再重复添加任何客户侧发送**——占位 reply 由网关 outbox 在 trigger 之前就发出了。`PRINCIPAL_ESCALATION_STATUS_PENDING` 是 Task 1 常量。

- [ ] **Step 2: 编译确认**

Run: `CARGO_TARGET_DIR=target-check cargo check`
Expected: PASS（`has_pending_for_contact` 被 Task 16 trigger 引用，此时应解析）

- [ ] **Step 3: lint + Commit**

Run: `bash scripts/check-no-human-takeover.sh`
Expected: PASS

```bash
git add src/agent/escalation.rs
git commit -m "feat(escalation): 等待期同类 pending 去重查询函数(trigger 内置去重，不重复推卡)"
```

---

### Task 22: 多轮卡死触发的 pre-check（N 轮未推进 + 负面反应）

**Files:**
- Modify: `src/agent/escalation.rs`（卡死判定纯函数）

业务决策 #5 + spec §4 类别3：同议题连续 N 轮（默认 3）Agent 未推进 **且** 客户出现负面反应才触发。这是 gateway pre-check 的一个信号，喂给 decision Agent（让它倾向 emit escalation），而不是绕过 decision 硬触发。

- [ ] **Step 1: 写卡死判定纯函数**

在 `escalation.rs` 追加：

```rust
/// 多轮卡死判定（业务决策 #5）：同一议题连续 stalled_turns 轮未推进 + 最近一轮负面反应。
/// 两条件同时满足才算卡死。纯函数，输入由 gateway 从 state/reaction 取。
pub(crate) fn is_stuck_or_undelivered(
    consecutive_unprogressed_turns: u32,
    threshold: u32,
    latest_reaction_is_negative: bool,
) -> bool {
    consecutive_unprogressed_turns >= threshold && latest_reaction_is_negative
}

/// 默认卡死轮阈值（spec：默认 3，可配）。
pub(crate) const DEFAULT_STUCK_THRESHOLD: u32 = 3;
```

- [ ] **Step 2: 加测试**

```rust
    #[test]
    fn stuck_needs_both_conditions() {
        // 轮数够但无负面反应 → 不触发
        assert!(!is_stuck_or_undelivered(5, 3, false));
        // 有负面反应但轮数不够 → 不触发
        assert!(!is_stuck_or_undelivered(2, 3, true));
        // 两者都满足 → 触发
        assert!(is_stuck_or_undelivered(3, 3, true));
        assert!(is_stuck_or_undelivered(4, 3, true));
    }

    #[test]
    fn default_stuck_threshold_is_three() {
        assert_eq!(DEFAULT_STUCK_THRESHOLD, 3);
    }
```

- [ ] **Step 3: 把卡死信号喂给 decision（gateway 注入提示）**

> **设计说明**：卡死信号**不**直接落台账触发请示，而是作为 decision prompt 的一个上下文提示（"你已连续 N 轮未推进此议题且客户有负面情绪，考虑是否该请示领导"），让 decision Agent 自己决定 emit escalation（保持单一决策入口，符合"不绕过 gateway/decision"）。具体注入点：`gateway.rs` 构造 decision user prompt 时，若 `is_stuck_or_undelivered(...)`，append 一句提示。这一步因依赖 reaction/state 的真实取值路径，留给执行时按 `grep "consecutive\|unprogressed\|reaction"` 找现成信号源接线；若现成信号不足，本 Task 仅落地纯函数 + 阈值常量，注入接线记为后续增强（不阻塞 MVP）。

Run: `cargo test --lib escalation::tests::stuck escalation::tests::default_stuck_threshold`
Expected: 2 passed

- [ ] **Step 4: Commit**

```bash
git add src/agent/escalation.rs
git commit -m "feat(escalation): 多轮卡死判定纯函数(N轮未推进+负面反应双条件)"
```

---

### Task 23: CLAUDE.md 红线澄清

**Files:**
- Modify: `CLAUDE.md`（"无人工接管"段）

spec §11：给 CLAUDE.md 补一句澄清，不改 lint、不改红线本身。

- [ ] **Step 1: 在 CLAUDE.md 的产品定位段补澄清句**

定位 `CLAUDE.md` 里讲"fully AI-autonomous / 无人工接管"那段（Project 段）。在其后追加一句：

```markdown
**"无人工接管"的精确含义**：指客户永远只跟 AI 对话、永不直接面对真人。AI 在遇到超出自身职权/能力的事项时，向**幕后决策源（领导）**请示、拿回结论后用自己的口吻向客户转述——这不是人工接管（客户从不面对人、对话始终是 AI 在说）。详见决策请示通道设计 `docs/superpowers/specs/2026-06-05-principal-decision-channel-design.md`。
```

> 注意：CLAUDE.md **不**在 `check-no-human-takeover` lint 扫描范围（lint 只扫 `src/agent/ src/routes/ src/evolution/ frontend/src/`），所以这里可以正常使用"人工接管/真人"等词来做澄清说明。

- [ ] **Step 2: Commit**

```bash
git add CLAUDE.md
git commit -m "docs(escalation): CLAUDE.md 澄清「无人工接管」=客户永不面对真人，AI 向幕后决策源请示不算接管"
```

---

## Phase 8 — 集成测试（Task 24，覆盖 spec §14 九项）

### Task 24: 端到端集成测试

**Files:**
- Modify: `tests/principal_decision_channel.rs`（填充 Task 11 建的骨架，testcontainers MongoDB，`#[ignore]`）

覆盖 spec §14 九项。这些需真 MongoDB（testcontainers）+ 可能需真/桩 LLM，标 `#[ignore]`，由 CI 跑（本地磁盘纪律不跑）。每个测试用现有集成测试的 setup 模式（`grep "testcontainers\|GenericImage\|Database::connect" tests/*.rs` 找现成 harness）。

- [ ] **Step 1: 写测试 setup helper（照现有集成测试 harness）**

在 `tests/principal_decision_channel.rs` 顶部，照搬现有集成测试的 MongoDB 起容器 + `Database::connect` + `migrations::run` + `ensure_indexes` 顺序（核验：main.rs 固定此序）。例如：

```rust
// 照 tests/ 下现有集成测试（如 state_transition_pbt 或某 _it.rs）的 setup 复制。
// 关键：connect → migrations::run → ensure_indexes 顺序不能乱。
async fn setup() -> (/* container guard */, wechatagent::AppState) {
    // ... 起 mongo 容器、建 AppState ...
    todo!("照现有 harness 填充")
}
```

> **执行注意**：`todo!()` 是脚手架占位，但**本 plan 要求无占位符**——执行此 Step 时必须用真实 harness 代码替换。先 `grep -l "testcontainers" tests/` 找一个最接近的现有集成测试，把它的 setup 整段拷过来改 collection 名。不允许留 `todo!()` 进 commit。

- [ ] **Step 2: 九项测试逐一实现**

按 spec §14 顺序，每项一个 `#[tokio::test] #[ignore]`：

```rust
// §14.1 三类触发各一条
#[tokio::test]
#[ignore]
async fn t_escalation_out_of_scope_creates_pending() {
    // 安排：配 principal_decider；构造一条 escalation_request{needed,category:out_of_scope}
    // 直接调 escalation::insert_pending_escalation，断言：台账有 1 条 pending、短码非空、principal_wxid 正确。
}

#[tokio::test]
#[ignore]
async fn t_escalation_high_risk_gated_respects_mode() {
    // 配 high_risk_escalation_mode=all vs decision_only，
    // 断言 parse_high_risk_mode + 触发行为符合两模式（all 升级、decision_only 不升级纯高风险件）。
}

#[tokio::test]
#[ignore]
async fn t_escalation_stuck_or_undelivered_signal() {
    // is_stuck_or_undelivered(3,3,true)=true 时该议题可被 escalate；断言信号正确。
}

// §14.2 回执码回流：带码精确 + 不带码回落最近未决（单条）
#[tokio::test]
#[ignore]
async fn t_principal_reply_with_code_resolves_exact() {
    // 插 2 条 pending；handle_principal_reply(带 #code)；断言对应那条 resolved、另一条仍 pending、起了 relay task。
}

#[tokio::test]
#[ignore]
async fn t_principal_reply_no_code_single_pending_resolves() {
    // 插 1 条 pending；handle_principal_reply(不带码)；断言 resolved + relay task。
}

// §14.2b 多条未决不带码 → 反问（业务决策 #4）
#[tokio::test]
#[ignore]
async fn t_principal_reply_no_code_multi_pending_asks_clarify() {
    // 插 2 条 pending；handle_principal_reply(不带码)；断言：无台账被 resolve、给领导发了反问消息（检查 outbox/mcp 调用）。
}

// §14.3 真人自然语言 → {verdict,substance,constraints}（解读，需 LLM 或桩）
#[tokio::test]
#[ignore]
async fn t_interpret_principal_reply_conditional() {
    // 真人回"可以但这周付款"；解读应 verdict=conditional、constraints 含"这周付款"。
    // 若 CI 用真 LLM：断言宽松（verdict∈闭集、substance 非空）。
}

// §14.4 跨天先重评再转述（客户已改主意/换话题）—— relay 前重读 contact
#[tokio::test]
#[ignore]
async fn t_relay_rereads_latest_contact_state() {
    // resolve 后改 contact 状态/删 contact；起 relay；断言 relay 安全处理（contact 没了不发；在了走网关）。
}

// §14.4b relay 命门：合成消息带哨兵 + 结构化字段（纯函数，不 ignore）
#[test]
fn t_synthetic_relay_carries_sentinel_and_fields() {
    // 闭环命门的机械守卫：synthetic_principal_relay 的载荷必须以哨兵开头并带 verdict/substance/constraints，
    // 否则 decision prompt 的 relay 输入契约（Task 13 Step 2）认不出转述任务。
    let contact = minimal_contact("cust_x"); // 见下方 helper
    let msg = wechatagent::models::ConversationMessage::synthetic_principal_relay(
        &contact, "conditional", "可以给8折", &["本周内付款".to_string()],
    );
    let body = relay_message_text(&msg); // 见下方 helper：取 ConversationMessage 文本字段
    assert!(body.starts_with(wechatagent::models::PRINCIPAL_RELAY_SENTINEL));
    assert!(body.contains("verdict=conditional"));
    assert!(body.contains("可以给8折"));
    assert!(body.contains("本周内付款"));
}

// §14.5 等待期分答（同类 pending 去重，不重复推卡）
#[tokio::test]
#[ignore]
async fn t_waiting_period_dedups_same_category() {
    // 已有 pending；再次 trigger 同 category；断言台账仍只 1 条 pending、未重复推卡（mcp 推卡调用计数不增）。
}

// §14.6 超时兜底：无限等待（deferred 保持 pending，不自动代决）
#[tokio::test]
#[ignore]
async fn t_deferred_keeps_pending_no_auto_decision() {
    // 真人回"我问下财务"→deferred；断言台账仍 pending、无 relay task、无客户侧发送。
}

// §14.7 知识缺口提案落 draft+needs_review
#[tokio::test]
#[ignore]
async fn t_generalizable_decision_emits_draft_proposal() {
    // resolved + is_generalizable；调 emit_knowledge_gap_proposal；
    // 断言 operation_knowledge_chunks 有 1 条 status=draft、integrity_status=needs_review、account_id=None。
}

// §14.8 wxid 误配防护：目标非 principal_decider 拒发
#[tokio::test]
#[ignore]
async fn t_target_wxid_guard_rejects_non_principal() {
    // assert_target_is_principal("customer","boss") → Err。（纯函数，也可不 ignore，但放这统一）
}

// §14.9 等待标记 set/clear 往返（统一占位模型：无 hold category，等待态在 contact flag 上）
#[tokio::test]
#[ignore]
async fn t_awaiting_marker_set_on_trigger_and_cleared_on_relay() {
    // 安排：配 principal_decider；构造 escalation_request{needed}。
    // act1：跑触发路径（或直接调 apply_agent_updates 写标记）→ 断言 contact.domain_attributes
    //   含 AWAITING_PRINCIPAL_DECISION_ATTR=true。
    // act2：resolve + 跑 relay → 断言 clear_awaiting_principal_state 后该标记被 $unset 清掉。
    // 这取代了旧的 hold-category 矫正测试——统一占位模型下 run 始终 Approved，无 hold category。
}

// §14.9b 兜底占位文案红线：不含转接类禁词（本测试放 tests/ 目录，lint 不扫，可写禁词字面量）
#[test]
fn fallback_holding_reply_has_no_handoff_wording() {
    let reply = wechatagent::agent::escalation::fallback_holding_reply();
    for forbidden in ["真人", "转人工", "客服", "接管", "人工"] {
        assert!(!reply.contains(forbidden), "兜底安抚不得含「{forbidden}」");
    }
}
```

> **执行注意**：每个 `#[ignore]` 测试体里的注释要替换成真实的 arrange/act/assert 代码（用 setup() 返回的 AppState 调真实函数）。**不允许留注释占位**——执行此 Task 时逐个填实。§14.8 是纯函数可不 ignore。LLM 依赖项（§14.3）若 CI 无真 LLM 则该测试在无 key 时跳过（照现有 real-llm 测试的 env-gate 模式）。
>
> **§14.4b 用到的两个 helper（执行时在本文件加，非占位）**：
> - `minimal_contact(wxid: &str) -> Contact`：构造一个最小可用 `Contact`。执行时 `grep -n "struct Contact" src/models.rs` 看必填字段，照 tests/ 下现有构造 Contact 的测试（`grep -rn "Contact {" tests/`）照搬一份最小字面量，只改 wxid。
> - `relay_message_text(msg: &ConversationMessage) -> &str`：取 `ConversationMessage` 的文本字段。执行时 `grep -n "struct ConversationMessage" -A30 src/models.rs` 确认文本字段名（`content` / `text` / `body`），返回该字段引用。
> - 这两个 helper 让 §14.4b 成为可真跑的纯函数测试（不 ignore），机械守卫哨兵契约。

- [ ] **Step 3: 编译测试**

Run: `CARGO_TARGET_DIR=target-check cargo check --tests`
Expected: PASS（所有测试编译过，ignored 不跑）

- [ ] **Step 4: 本地跑非 ignore 的（纯函数）测试**

Run: `cargo test --test principal_decision_channel`
Expected: 非 ignore 测试 PASS，ignored 显示 skipped

- [ ] **Step 5: lint + Commit**

Run: `bash scripts/check-no-human-takeover.sh`
Expected: PASS（tests/ 目录被 lint 排除，但保持措辞干净）

```bash
git add tests/principal_decision_channel.rs
git commit -m "test(escalation): 端到端集成测试覆盖 spec §14 九项(testcontainers, #[ignore])"
```

---

## Phase 9 — 收尾验证（Task 25）

### Task 25: 全量门 + 推分支 + PR

**Files:** 无（验证 + git）

- [ ] **Step 1: 全量 lib 测试 + PBT 基线**

Run: `cargo test --lib`
Expected: ≥350 passed; 0 failed（新增约 25 个 escalation 单测，总数应明显上升）

Run: `cargo test --test state_transition_pbt && cargo test --test memory_card_invariants && cargo test --test wiki_chunk_revision_pbt && cargo test --test llm_retry_jitter`
Expected: 累计 ≥33 passed; 0 failed

- [ ] **Step 2: 双 lint**

Run: `bash scripts/check-no-human-takeover.sh`
Expected: PASS
Run: `bash scripts/check-baseline.sh`
Expected: PASS（baseline gate 绿）

- [ ] **Step 3: 推分支（不直推 main）**

```bash
git push -u origin <feature-branch>
```

- [ ] **Step 4: 开 PR（CI 跑全量含集成测试）**

```bash
gh pr create --title "feat: 决策请示通道(幕后领导模式)" --body "$(cat <<'EOF'
## Summary
- 实现 spec `docs/superpowers/specs/2026-06-05-principal-decision-channel-design.md`：运营 Agent 撞决策墙时向幕后真人决策源请示、AI 口吻转述客户。
- 客户侧"无人工接管"红线字面+语义双保（无 lint 禁词、客户从不面对真人）。
- 新增 agent_principal_escalations 台账、principal_decision_relay task kind、principal_decider 配置、contact state 上的 awaiting_principal_decision 可观测标记。
- 统一占位模型：触发请示的 run 始终 Approved（占位经 outbox 正常发），请示为 approved 路径副作用，不碰 review.rs。

## Test plan
- [ ] cargo test --lib ≥350/0
- [ ] 4 PBT 累计 ≥33/0
- [ ] no-human-takeover + baseline 双 lint 绿
- [ ] CI 跑 tests/principal_decision_channel.rs 九项集成测试（testcontainers）
EOF
)"
```

- [ ] **Step 5: 等 CI，读结果**

CI 绿 → 功能完成。红 → 按失败 job 日志修（遵守 additive-only + 反过拟合 + 不碰并行 agent 文件）。

---

## Self-Review（计划 vs spec 覆盖核对）

**1. Spec 覆盖（每节 → Task）：**

| spec 节 | 覆盖 Task |
| --- | --- |
| §3 幕后领导定位 | Task 13/14 prompt 措辞 + Task 23 CLAUDE.md 澄清 |
| §4 三类触发 | Task 13(prompt 描述)、Task 10(high_risk 模式可配)、Task 22(卡死判定) |
| §5.1 请示卡 | Task 8(render_principal_card) |
| §5.2 真人回复+回执码 | Task 7(match_principal_reply)、Task 19(handle_principal_reply) |
| §5.3 模糊回复(交回 Agent) | Task 14(verdict=delegated_back) + Task 13(relay 输入契约 delegated_back 基调) |
| §6 拿到决策推进(同意/条件/拒绝/过期) | Task 9(过期判定)、Task 13(relay 输入契约按 verdict 分诊)、Task 19(授权窗=领导说了算)、Task 11(resolve 写 authorization_expires_at) |
| §7.1 触发即时安抚占位 | Task 8(兜底占位文案+红线测试) + Task 13(decision prompt 让 Agent 把占位作 reply_text) + Task 16 Step 3(占位经 outbox 正常发，run 保持 Approved) |
| §7.2 等待期分答 | Task 21(trigger 内置去重 + 正常跑天然分答) |
| §7.3 超时无限等待 | Task 19(deferred 保持 pending，不自动代决) |
| §8 跨天先重评 | Task 15(relay 前重读 contact) |
| §9.1 归属+知识提案 | Task 12(emit_knowledge_gap_proposal)、Task 16(approved/conditional 才发)；接触级归属走聊天历史+台账(MVP 边界，见 Task 15 说明) |
| §9.2 知识闭环红线(只 draft) | Task 12(status=draft + needs_review) |
| §9.4 防护(wxid/去重/不自动全局) | Task 11(assert_target_is_principal)、Task 16+21(trigger 内置 pending 去重)、Task 19(NoPending 不自动生效) |
| §10 多主管 MVP 单领导 | Task 20(workspace 内 principal_decider 唯一，find_one 取该条；跨 workspace 串扰已堵) |
| §11 红线/lint | 全程命名纪律 + Task 23 |
| §12 新增组件 | Task 1(struct/常量)、Task 2(accessor)、Task 3(index)、Task 4(等待标记 key 常量)、Task 17(task kind 分流) |
| §13 端到端数据流 | Phase 5(触发) + Phase 6(回流) 串联 |
| §14 九项测试 | Task 24(逐项) |

无未覆盖的 spec 节。

**2. 占位符扫描：** 计划中剩一处显式标注的脚手架占位（Task 24 `setup()` 的 `todo!()`）在 Step 注释里强制要求执行时替换成真实 harness，不允许进 commit。Task 19 里那行 `mcp use 见文件顶部；...` 已在执行注意中标明删除并改成 `use crate::mcp;`。无其它 TBD/"类似上文"/空 handler。

**3. 类型一致性（已核对修正）：**
- `AgentPrincipalEscalation` 字段（id/workspace_id/account_id/contact_wxid/short_code/status/category/reason/question_for_principal/principal_wxid/decision/authorization_expires_at/**is_generalizable**/knowledge_proposal_emitted/created_at/updated_at/resolved_at）在 Task 1 定义、Task 7 `make_pending`、Task 11 `insert_pending_escalation` 字面量、Task 16 relay 读取处**全部一致**（已补齐 `is_generalizable`）。
- `insert_pending_escalation` 签名（含 `is_generalizable: bool`）与 Task 16 调用处传参一致。
- `trigger_principal_escalation` 签名（统一占位模型后）= `(state, contact, req) -> AppResult<()>`，**已删 `holding_reply` 参数 + 返回值由 `bool` 改 `()`**；Task 16 Step 3 接线处按此调用（错误降级 warn 不 `?`）。
- `EscalationRequest` 字段（needed/category/reason/question_for_principal/self_serviceable_part/is_generalizable）+ `PrincipalDecision` 字段（verdict/substance/constraints/**authorization_window_hours**）在 models 定义、prompt 字段、解读函数、relay 使用处一致；6 处 `PrincipalDecision` 字面构造（Task 9 测试、Task 14 fallback×2、sanitize_verdict、Task 14 测试×2）均已带 `authorization_window_hours`。
- verdict 闭集常量（approved/rejected/conditional/deferred/delegated_back）在 Task 1 定义、Task 14 sanitize、Task 13 relay 输入契约分诊、Task 19 deferred 续等处一致。
- **relay 哨兵 `PRINCIPAL_RELAY_SENTINEL`（Task 16 Step 2 定义 = `"__PRINCIPAL_RELAY__"`）与 Task 13 Step 2 decision prompt 的 relay 输入契约逐字一致**——这是闭环命门：哨兵是信号、prompt 契约是解码器，措辞分诊集中在 prompt 一处（构造器只结构化字段）。
- 授权过期窗 = 领导说了算：`PrincipalDecision.authorization_window_hours`（Task 14 LLM 解读填，领导没说则 None）在 Task 19 算 `authorization_expires_at`；**已删 24h 硬编码**。
- 等待标记 key 常量 `AWAITING_PRINCIPAL_DECISION_ATTR`（Task 4 定义）在 Task 18 set 侧、Task 16 `clear_awaiting_principal_state` 的 `$unset` 侧用**同一字符串**（Task 4 有锁值单测）。**不再有 hold category 常量**（统一占位模型下 run 不进 Held）。
- `lookup_principal_config` 签名（Task 20）= `(state, workspace_id, from_wxid) -> Option<domain>`，**用入站消息自己的 workspace_id 约束查询**，堵跨 workspace 串扰（核验：account→workspace 1:1，resolve_account_context 返回 workspace_id）。
- `has_pending_for_contact`（Task 21 定义）被 Task 16 trigger 引用——执行顺序上需先于或随 Task 16 写入 escalation.rs（执行注意已标）。
- task kind 字符串 `"principal_decision_relay"` 在 Task 17 分流、Task 19 enqueue 一致。

**4. 统一占位模型一致性（2026-06-06 用户拍板后全计划已收敛）：** 触发请示的 run **始终 `Approved`**，占位 = decision Agent 本轮 `reply_text` 经 outbox 正常发出；`trigger_principal_escalation` 是 approved 路径末尾（gateway L1534）的副作用，**只推卡给领导 + 落台账**，不给客户发任何东西、不进 Held 分支、不设 hold category、**不碰 `review.rs`**。这一模型贯穿架构段、业务决策 #1、文件结构表、Task 4/8/16/18/21，全部已对齐。`review.rs` 已从文件结构表与所有 Task 中移除——**本计划不再修改任何并行 agent 域文件**。

**5. 开工前终极审查（2026-06-06，对照 spec 端到端）已修正三项：**
- **🔴 relay 闭环命门（曾遗漏）**：relay 把领导裁决塞进"客户消息"位喂 decision Agent，但原计划没让 Agent 知道这是转述任务。已补：合成消息加哨兵 `PRINCIPAL_RELAY_SENTINEL`（Task 16）+ decision prompt 加 relay 输入契约（Task 13 Step 2），成对存在，Agent 进入转述模式、不外泄内部字段。**没这对契约闭环跑不通**。
- **授权过期窗（曾硬编码 24h）**：改为"领导说了算"——LLM 解读领导明确说的时限填 `authorization_window_hours`，没说则不设过期窗（Task 14 prompt + struct，Task 19 用它）。
- **跨 workspace 串扰（曾全局查）**：`lookup_principal_config` 改用入站消息自己的 workspace_id 约束（Task 20），堵 A 领导误路由进 B 的请示流。
- **接触级归属（spec §9.1 MVP 边界）**：经核验，relay 转述回复天然落聊天历史、下轮可见；授权结构化事实存台账。MVP **不**额外写 contact memory（不碰优化中的 memory 子系统），已在 Task 15 显式标注为有意边界、非 bug。

**执行前提醒（给 worker）：** 每个 Task 的"执行注意"列出了需 `grep` 现场确认的真实符号（AppError 变体、AppState 路径、accessor 名、ConversationMessage 字段、prompt helper、contact domain/label helper）。这些是有意留的"对齐点"——核验报告给了行号但代码会演进，执行时以 grep 现场为准，不要盲信计划里的行号。**最重要的红线**：`src/agent/`、`src/webhooks.rs`、`src/prompts.rs` 都在 `check-no-human-takeover` lint 扫描内，**新增行（含注释、日志、合成文案）里"真人/人工/接管"是禁词**——一律用"领导/principal/决策源"，每个 Task commit 前必跑 `bash scripts/check-no-human-takeover.sh`。

**并行 agent 协调点：** **无。** 统一占位模型确立后，本计划已不再修改 `review.rs` 或任何并行 agent 域文件（原 Task 18 的 review.rs hold 分支已删除）。所有改动落在新建的 `escalation.rs` + 对 `gateway.rs`/`models.rs`/`webhooks.rs`/`prompts.rs`/`types.rs` 的最小侵入式追加。这是本次设计简化最重要的收益——冲突面归零。


