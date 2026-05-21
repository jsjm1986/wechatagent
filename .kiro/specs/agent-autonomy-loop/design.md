# Design Document — agent-autonomy-loop

> 中文标题：用户运营 Agent 自治回路 — 技术设计文档
> 关联需求文档：`.kiro/specs/agent-autonomy-loop/requirements.md`（R0–R13 + 实现注记 N1–N7）
> 工作流：requirements-first / specType=feature

## Overview

> 中文小节标题：1. 概述

本设计文档把 requirements.md 的 14 项需求（R0–R13）映射到现有代码库（Rust + MongoDB + LLM + MCP + Vite/React 前端）的具体改造方案，并按实现注记 N1–N7 的约束给出强制顺序的"6 波 + 1 收口"实施波次。设计原则：

- **业务语义保护**：全 AI 自治流程，禁止任何 `human takeover / 人工接管` 语义；hold 类别只用 `held_by_ai_policy / blocked_by_safety_guard / ai_waiting_for_more_context`。
- **Rust 是边界，不是策略**：业务字段（`risk_level / knowledge_need / run_mode / autonomy_mode / needs_review / operation_state / consolidation_needed`）由 Reply Agent 输出；Rust 仅做"必填校验 + 枚举校验 + 安全门拦截"，不再替 Agent 兜底默认值。
- **决策与发送解耦**：决策走 review → outbox 持久化 → outbox dispatcher worker 二次安全门 → MCP 实际发送；任何路径异常都有 `agent_run_logs` envelope + `agent_events` trace 可追溯。
- **沿用现有枚举**：保留 `runMode = fast_chat / memory_candidate / knowledge_grounded / high_risk`、`knowledgeNeed = not_required / required / insufficient`、`risk_level = low / medium / high`、`OperationKnowledgeChunk.integrity_status`；只**新增** `autonomy_mode = auto / assisted / blocked` 与 `decision_phase = tool_calling / final`。
- **物理替换 + Sunset**：兼容路径（`MemoryFactRepr::Plain` / `autonomyProtocolEnabled` / `knowledgeRoutingMode=classic_router`）有明确移除时间点（D+14），不长期双轨。

## Architecture

> 中文小节标题：2. 架构总览

### 2.1 升级后的运行链路（高层）

```
入站消息 / 跟进任务
        │
        ▼
[R0] write_run_envelope (insert_one lifecycle="started")
        │
        ▼
[N2] RawAgentDecision (Option<T>) ── reply_with_tools_loop ──▶ MCP knowledge.* tools
        │                                  │
        │   (toolCalls 循环 0..N 轮)        │
        │                                  ▼
        │                            tool results 注入 prompt
        │
        ▼  decision_phase == "final"
[N2] RawAgentDecision::validate_and_promote → AgentDecision (含自治协议 9 字段 / 7 个必填)
        │
        ▼
enforce_decision_guards (R8 字典 candidate / R7 memory 接入)
        │
        ▼
review_decision (LLM full / light) ─┐
   或 local_decision_review          │
   (R3.7 / R3.8 二态)                │
                                     ▼
                       [N3] finalize_review_for_send
                              (R3.5 / R3.7 / R5.4 / R5.7 / R8.6 最终安全汇总)
                                     │
                ┌────────────────────┼────────────────────┐
                ▼                    ▼                    ▼
        approved + should_reply    needsRevision     shouldHold / blocked / hold
                │                    │                    │
                │              [R2] revision               │
                │              (单 run 一次)               │
                │                    │                    │
                │                    ▼                    │
                │             第二轮 finalize              │
                │                    │                    │
                ▼                    ▼                    ▼
[R0] write_agent_run_log (update_one — finalReviewStatus / autonomyMode / revisionApplied / pre+postRevisionSummary)
                │
                ▼ (仅当 finalReviewStatus ∈ {approved, revision_applied_approved} 且 should_reply=true)
[R13] outbox_dispatcher::enqueue (强幂等 idempotency_key = SHA256(source_event_id:contact_wxid:content_hash))
                │
                ▼ (异步 worker)
outbox_dispatcher tick:
   atomic findOneAndUpdate 抢占 → 二次安全门 → mcp::send_message → status=sent/failed_terminal/canceled
                │
                ▼
agent_events trace (outbox_created / outbox_sent / outbox_lease_expired / ...)
```

### 2.2 实施波次（强制顺序，与 N6 对齐）

| 波次 | 名称 | 主要内容 | 依赖前置 |
|----|----|----|----|
| **W0 基础设施波** | Infra Foundations | 新建 `agent_send_outbox / system_taxonomies / taxonomy_candidates` collection accessor + 索引；新增迁移脚手架（`2026_05_005_memory_facts_to_structured` / `2026_05_006_taxonomy_seed` / `2026_05_007_outbox_indexes`）；新增 runtime_parameters 字段（`autonomyProtocolEnabled / knowledgeRoutingMode / knowledgeMaxToolLoops / knowledgeMaxToolCalls / knowledgeOpenSliceMaxK / outboxPollIntervalSeconds / outboxLeaseSeconds`）；CI baseline 脚本 `scripts/check-baseline.ps1` | — |
| **W1 业务波 1** | 协议骨架 | N1 删除 `normalize_decision_runtime` 业务兜底；N2 引入 `RawAgentDecision` + `validate_and_promote`；R0 Run Envelope insert/update 双语义；新增 `autonomy_mode / decision_phase` 字段；旧 `Vec<String>` 反序列化 untagged enum | W0 |
| **W2 业务波 2** | 校验与最终安全门 | R3 必填 + 枚举严格；R5 verified knowledge fail-closed；N3 抽出 `finalize_review_for_send`；改造 `local_decision_review` 二态 | W1 |
| **W3 业务波 3** | 工具循环 + 字典 | R4 `reply_with_tools_loop` + MCP knowledge.* 三工具；R8 双层标签接入（system_taxonomies / agent_generated_signals / taxonomy_candidates） | W2 |
| **W4 业务波 4** | 发送闭环 | N4 outbox 解耦：gateway 不再直接调 send_outbound_message；新增 `outbox_dispatcher` worker（atomic claim + lease 崩溃恢复）；R13 强幂等 + 二次安全门 + 重试 + 取消 | W3 |
| **W5 业务波 5** | MemoryCard 边界替换 | N5 `OperatingMemory.memory_card: Document → MemoryCardTyped` 整层替换；R6 `MemoryFact` 强类型 + 稳定 id；R7 `deprecatedFacts / conflicts` 落库 | W2（可与 W3/W4 并行，但合并独立 PR） |
| **W6 业务波 6** | 自治回路监控 + PBT 收口 | R10 前端"自治回路监控"Tab；R12 `tests/autonomy_protocol_pbt.rs` 新文件 + `happy_path_run.rs` 扩展两个 case；R11 sunset 文档 | W1–W5 全部完成 |

### 2.3 关键模块新增/改造列表

| 模块 / 文件 | 现状 | 改造类型 | 关联需求 |
|----|----|----|----|
| `src/agent/types.rs` | `AgentDecision` 缺 9 字段 / autonomy_mode / decision_phase / toolCalls / agentGeneratedSignals；`MemoryCardTyped` 已有但仅内部用 | 新增字段 + 引入 `RawAgentDecision` | R1 / R3 / R4 / R6 / R8 |
| `src/agent/guards.rs` | `normalize_decision_runtime` 替 Agent 补默认值 | **删除** 默认值兜底，仅保留枚举校验；新增 `enforce_decision_guards` 接入 taxonomy candidate | N1 / R3 / R8 |
| `src/agent/review.rs` | `local_decision_review` 默认 `approved=true` | 改为二态（needs_review=true 走 blocked / needs_review=false 走 low_risk_approved）；新增 `finalize_review_for_send` 最终安全汇总层 | N3 / R3.7 / R5 / R8 |
| `src/agent/gateway.rs` | `run_user_operation_gateway` 串联 → 直接 `send_outbound_message` | 入口写 envelope；删直接发送，改为 `outbox_dispatcher::enqueue`；接入 `reply_with_tools_loop` 与 `finalize_review_for_send` | N4 / R0 / R2 / R13 |
| `src/agent/budget.rs` | `RunBudget` 仅含 token / llm_calls | 新增 `tool_calls_used` 计数 | R4 |
| `src/agent/memory.rs` | `compact_memory_card_with_previous` 等用 `Document` | 改用 `MemoryCardTyped`；处理 `deprecatedFacts / conflicts` | N5 / R6 / R7 |
| `src/agent/knowledge_router.rs` | `run_knowledge_router` 一次性拼 prompt | 保留 classic_router 路径作为灰度回退；新增 `reply_with_tools_loop` 与 MCP knowledge.* 三工具实现 | R4 / R11 |
| `src/agent/reaction.rs` | `record_user_reaction` 不通知 outbox | 新增 outbox 取消通道（user_reaction_stop_requested → 取消同 contact 所有 pending/in_flight 条目） | R13.6 |
| `src/agent/outbox.rs` **(新)** | — | 新模块：`OutboxEntry / OutboxStatus / OutboxDispatcher` + atomic claim + lease 崩溃恢复 + 重试调度 + 取消 | R13 |
| `src/agent/taxonomy.rs` **(新)** | — | 新模块：字典加载 + alias 命中判定 + candidate upsert + approve/reject API 后端逻辑 | R8 |
| `src/agent/run_envelope.rs` **(新)** | — | 新模块：`write_run_envelope_started / update_run_envelope_terminal` + panic catch | R0 |
| `src/models.rs` | `OperatingMemory.memory_card: Document`；`AgentRunLog` 缺 R9.1 字段 | 整层替换；新增 R9.1 字段 + finalReviewStatus 枚举校验 | N5 / R6 / R9 |
| `src/db/mod.rs` | 缺 outbox / system_taxonomies / taxonomy_candidates accessor | 新增 3 个 collection accessor | W0 / R8 / R13 |
| `src/db/indexes.rs` | 已有 `(run_id)` unique 等 | 新增 R8.1 / R13.1 索引；新增 `(account_id, finalReviewStatus, started_at) / (account_id, autonomyMode, started_at) / (account_id, lifecycle, started_at)` | R8.1 / R9.5 / R13.1 / R0.8 |
| `src/db/migrations.rs` | 已有迁移基础设施 | 新增 3 条幂等迁移 | W0 / R6.4 / R8.8 / R13 |
| `src/prompts.rs` | 现有 prompt 不输出 9 字段 / decision_phase / toolCalls | prompt 改造 + 自治协议输出契约段落 | R1 / R2 / R4 |
| `src/routes/*` | — | 新增 `routes/admin_taxonomies.rs / admin_taxonomy_candidates.rs / admin_outbox.rs / outcomes_autonomy.rs` | R8.7 / R10.4 / R13.6/9 |
| `src/mcp.rs` | `mcp::send_message` 直接发 | 仅供 outbox dispatcher worker 调用；语义不变 | N4 / R13.5 |
| `frontend/src/App.tsx` | 仅有运营成效中心，无自治 Tab | 新增"自治回路监控"Tab + horizon 选择器 + 7 指标卡 + revision 列表 + AI 暂缓分布图 + 发送链路状态 | R10 |
| `tests/autonomy_protocol_pbt.rs` **(新)** | — | P1–P7 七条性质 PBT，每条 ≥ 64 用例 | R12 |
| `tests/happy_path_run.rs` | 现有 happy path | 扩展 `autonomy_full_loop_with_revision / autonomy_tool_loop_happy_path` | R12.6 |
| `scripts/check-baseline.ps1` **(新)** | — | CI 基线核验：`cargo test --lib >= 78` + 4 PBT 文件 `>= 33` | N7 / R11.6 |
| `docs/sunset-plan.md` **(新)** | — | D+7 / D+14 / D+21 移除清单 | R11.5 |


## Data Models

> 中文小节标题：3. 数据模型变更

### 3.1 `agent_run_logs` 升级（R0 + R9）

新增字段（一次性 update_one 写入）：

```rust
// src/models.rs::AgentRunLog (扩展)
pub struct AgentRunLog {
    // ── 既有字段（保留）──
    pub run_id: String,
    pub account_id: ObjectId,
    pub contact_wxid: String,
    pub started_at: DateTime,
    pub gateway_status: String,
    pub planner: Option<Document>,
    pub context: Option<Document>,
    pub decision: Option<Document>,
    pub review: Option<Document>,
    pub gateway_result: Option<Document>,
    pub error: Option<Document>,
    pub token_budget: i64,
    pub tokens_used: i64,
    pub llm_calls_used: i32,
    pub degraded_reasons: Vec<String>,

    // ── R0 Run Envelope 新增 ──
    pub lifecycle: String,                    // started | running | completed | failed_before_decision | failed_after_decision | aborted_by_budget | aborted_by_external_signal
    pub source_event_id: String,              // 入站消息 id 或 task_id
    pub source_kind: String,                  // inbound_message | follow_up_task | manual_send
    pub error_summary: Option<String>,        // ≤ 1024 chars
    pub abort_reason: Option<String>,         // ≤ 256 chars

    // ── R9 自治审计字段新增 ──
    pub revision_applied: bool,
    pub revision_reason: String,              // ≤ 1024 chars
    pub pre_revision_summary: Option<String>, // ≤ 2048 chars
    pub post_revision_summary: Option<String>,// ≤ 2048 chars
    pub self_critique: Option<String>,        // ≤ 2048 chars
    pub autonomy_mode: String,                // auto | assisted | blocked
    pub final_review_status: String,          // 见 R9.2 严格枚举

    // ── R13 outbox 关联（trace 用）──
    pub outbox_status: Option<String>,        // pending | in_flight | sent | failed_terminal | canceled

    // ── R7 memory consolidator warnings ──
    pub memory_consolidator_warnings: Vec<String>,
}
```

`finalReviewStatus` 严格枚举（来自需求文档状态映射表）：`approved / revision_applied_approved / revision_failed / held_by_ai_policy / blocked_by_safety_guard / ai_waiting_for_more_context / blocked_by_required_field / blocked_by_budget / blocked_unverified_product_claim / legacy_mode_unchecked`。任何其他取值在写库前由 `assert_final_review_status_valid(&str)` 阻断（debug_assert + 生产 error log）。

新增索引（`src/db/indexes.rs`）：
- `(account_id, lifecycle, started_at)` — R0.8
- `(account_id, finalReviewStatus, started_at)` — R9.5
- `(account_id, autonomyMode, started_at)` — R9.5

**入口必须 insert_one + 后续 update_one 的协议约束**：`(run_id)` 已是 unique index（`src/db/indexes.rs:236`），同一 run 二次 insert_one 会返回 DuplicateKey；本期 `write_agent_run_log` 改写为 `update_one({run_id})` + `$set`；matched_count == 0 时走单次兜底 insert_one + `agent_events kind="run_envelope_recovered_via_insert"`。

### 3.2 `agent_send_outbox`（R13 新建）

```rust
// src/models.rs::OutboxEntry (新)
pub struct OutboxEntry {
    pub _id: ObjectId,
    pub account_id: ObjectId,
    pub contact_wxid: String,
    pub run_id: String,
    pub decision_id: ObjectId,
    pub source_event_id: String,
    pub source_kind: String,           // inbound_message | follow_up_task | manual_send
    pub content: String,               // 不可变
    pub content_hash: String,          // SHA-256 of content
    pub idempotency_key: String,       // SHA-256(source_event_id + ":" + contact_wxid + ":" + content_hash)
    pub attempt: i32,                  // default 0
    pub max_attempts: i32,             // default 3
    pub status: String,                // OutboxStatus
    pub cancel_reason: Option<String>,
    pub last_error: Option<String>,
    pub next_retry_at: Option<DateTime>,
    pub worker_id: Option<String>,     // hostname:pid:uuid
    pub locked_until: Option<DateTime>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
    pub sent_at: Option<DateTime>,
}

// 状态枚举
pub enum OutboxStatus { Pending, InFlight, Sent, FailedTerminal, Canceled }
//                       ^^^^^^^^^^^^^^^^ 注意：requirements R13.5 / R13.10 统一用 failed_terminal，禁止用 failed
```

索引（R13.1）：
- 复合 `(account_id, status, next_retry_at)` — worker 扫描
- **唯一** `(idempotency_key)` — 强幂等保证（写入冲突即 idempotent skip）
- 复合 `(status, locked_until)` — 崩溃恢复扫描
- 复合 `(source_event_id, contact_wxid)` — 按入站消息追溯

**强幂等公式**：`idempotency_key = SHA256(source_event_id + ":" + contact_wxid + ":" + content_hash)`。**不含** `run_id`，确保同一 `source_event_id` 触发的多次 run（每次 run_id 不同）只会发送一次。`source_event_id == ""` 兜底走 `SHA256("synthetic:" + run_id + ":" + contact_wxid + ":" + content_hash)` + 警告事件，正常路径不应触发。

### 3.3 `system_taxonomies`（R8 新建）

```rust
pub struct TaxonomyEntry {
    pub _id: ObjectId,
    pub scope: String,            // "global" | account_id_string
    pub kind: String,             // "customer_stage" | "intent_level" | "objection_type"
    pub value: TaxonomyValue,
    pub updated_at: DateTime,
}

pub struct TaxonomyValue {
    pub id: String,               // 字典 key（如 "first_contact"）
    pub display_name: String,
    pub description: String,
    pub aliases: Vec<String>,
    pub status: String,           // "active" | "deprecated"
}
```

唯一索引：`(scope, kind, value.id)`。

R8.8 数据迁移 seed `2026_05_006_taxonomy_seed`：基于现有 prompt 中硬编码运营术语（`customer_stage / intent_level / objection_type` 默认枚举集合）写入 `scope="global"` 默认字典；幂等。

### 3.4 `taxonomy_candidates`（R8 新建）

```rust
pub struct TaxonomyCandidate {
    pub _id: ObjectId,
    pub scope: String,
    pub kind: String,
    pub raw_value: String,
    pub evidence: Option<String>,
    pub confidence: i32,
    pub first_seen_at: DateTime,
    pub last_seen_at: DateTime,
    pub occurrences: i32,
    pub status: String,           // "pending" | "approved" | "rejected"
    pub reviewed_at: Option<DateTime>,
    pub reviewed_by: Option<String>,
}
```

索引：`(scope, kind, status)` + `(scope, kind, raw_value)` 唯一。

**关键约束（R8.4）**：候选写入 SHALL NOT 阻塞 Reply Agent；`review.risks` 仅追加 `taxonomy_candidate:<kind>:<value>`，`review.approved` 不被该字段强制 false。

### 3.5 `OperatingMemory.memory_card` 整层替换（N5 / R6）

```rust
// 旧（src/models.rs:300）
// pub memory_card: Document,

// 新
pub memory_card: MemoryCardTyped,

// src/agent/types.rs::MemoryCardTyped（提升为 OperatingMemory 字段类型）
pub struct MemoryCardTyped {
    pub core_facts: Vec<MemoryFact>,         // cap=6
    pub recent_facts: Vec<MemoryFact>,       // cap=10
    pub deprecated_facts: Vec<MemoryFact>,   // cap=20
    pub core_profile: CoreProfileTyped,
    pub relationship_state: RelationshipStateTyped,
    #[serde(default)]
    pub extra: Document,                     // 兜底承接未识别字段
}

pub struct MemoryFact {
    pub id: String,                          // UUIDv4 字符串
    pub text: String,                        // 1..=500 chars
    pub evidence: Option<String>,            // ≤ 1000 chars
    pub confidence: i32,                     // 0..=10
    pub importance: i32,                     // 0..=10
    pub may_expire: bool,
    pub deprecated_at: Option<DateTime>,
    pub deprecation_reason: Option<String>,  // ≤ 200 chars
    pub source_message_ids: Vec<ObjectId>,   // 最多 5 条
    pub source_run_id: Option<String>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}
```

**反序列化兼容（R6.3）**：

```rust
#[derive(Deserialize)]
#[serde(untagged)]
enum MemoryFactRepr {
    Plain(String),       // 老 Vec<String> 元素
    Structured(MemoryFact),
}

impl From<MemoryFactRepr> for MemoryFact {
    fn from(r: MemoryFactRepr) -> Self {
        match r {
            MemoryFactRepr::Plain(text) => MemoryFact {
                id: Uuid::new_v4().to_string(),    // 关键：fresh UUID
                text,
                evidence: None,
                confidence: 7,
                importance: 5,
                may_expire: false,
                deprecated_at: None,
                deprecation_reason: None,
                source_message_ids: vec![],
                source_run_id: None,
                created_at: now(),
                updated_at: now(),
            },
            MemoryFactRepr::Structured(f) => f,
        }
    }
}
```

**迁移 R6.4 `2026_05_005_memory_facts_to_structured`**：扫描所有 `operating_memories.memory_card.coreFacts / recentFacts` 中字符串元素 → 升级为结构化 + fresh UUID + `created_at = now`；幂等（用 `id` 字段是否存在判定）。

### 3.6 `runtime_parameters` 新增字段（W0 基础设施波）

```rust
// src/models.rs::RuntimeParameters (扩展)
pub struct RuntimeParameters {
    // ── 既有 ──
    // ...

    // ── W0 新增 ──
    pub autonomy_protocol_enabled: bool,           // default true，sunset D+14
    pub knowledge_routing_mode: String,            // "auto_tool_loop" | "classic_router"，sunset D+14
    pub knowledge_max_tool_loops: i32,             // default 3，范围 [1, 5]
    pub knowledge_max_tool_calls: i32,             // default 6，范围 [1, 16]
    pub knowledge_open_slice_max_k: i32,           // default 4，范围 [1, 16]
    pub knowledge_search_top_k: i32,               // default 8
    pub outbox_poll_interval_seconds: i32,         // default 5
    pub outbox_lease_seconds: i32,                 // default 60
}
```

### 3.7 `AgentDecision` / `RawAgentDecision` 双层结构（N2 / R1 / R3 / R4）

```rust
// src/agent/types.rs::RawAgentDecision (新，仅用于反序列化)
#[derive(Deserialize)]
pub struct RawAgentDecision {
    // 自治协议 9 字段（R1）
    pub user_understanding: Option<String>,
    pub relationship_read: Option<String>,
    pub operation_goal: Option<String>,
    pub knowledge_need_reason: Option<String>,
    pub memory_update_reason: Option<String>,
    pub self_critique: Option<String>,
    pub why_should_reply: Option<String>,
    pub why_skip_reply: Option<String>,
    pub risk_self_check: Option<String>,

    // 业务字段（R3 必填，但反序列化层用 Option 区分"未输出"与"输出 false/空"）
    pub risk_level: Option<String>,           // low | medium | high
    pub knowledge_need: Option<String>,       // not_required | required | insufficient
    pub run_mode: Option<String>,             // fast_chat | memory_candidate | knowledge_grounded | high_risk
    pub autonomy_mode: Option<String>,        // auto | assisted | blocked  (新增)
    pub needs_review: Option<bool>,
    pub operation_state: Option<String>,
    pub consolidation_needed: Option<bool>,

    // R4 工具循环
    pub decision_phase: Option<String>,       // tool_calling | final
    pub tool_calls: Option<Vec<ToolCallRequest>>,

    // R8 自由信号
    pub agent_generated_signals: Option<Vec<AgentSignal>>,

    // 既有字段（reply_text / should_reply / used_knowledge_ids / safe_claims_used / knowledge_route / 等）保留为 Option
    pub reply_text: Option<String>,
    pub should_reply: Option<bool>,
    pub used_knowledge_ids: Option<Vec<String>>,
    pub safe_claims_used: Option<Vec<String>>,
    pub knowledge_route: Option<KnowledgeRoute>,
    // ... 其他既有字段
}

// src/agent/types.rs::AgentDecision (扩展，业务结构)
pub struct AgentDecision {
    // 自治协议 9 字段（R1）
    pub user_understanding: String,
    pub relationship_read: String,
    pub operation_goal: String,
    pub knowledge_need_reason: String,
    pub memory_update_reason: String,
    pub self_critique: String,
    pub why_should_reply: String,
    pub why_skip_reply: String,
    pub risk_self_check: String,

    // 业务字段（R3 必填）
    pub risk_level: String,
    pub knowledge_need: String,
    pub run_mode: String,
    pub autonomy_mode: String,                // 新增
    pub needs_review: bool,
    pub operation_state: String,
    pub consolidation_needed: bool,

    // R4 工具循环
    pub decision_phase: String,               // 默认 "final"
    pub tool_calls: Vec<ToolCallRequest>,     // 默认空

    // R8 自由信号
    pub agent_generated_signals: Vec<AgentSignal>,

    // 既有字段
    pub reply_text: String,
    pub should_reply: bool,
    pub used_knowledge_ids: Vec<String>,
    pub safe_claims_used: Vec<String>,
    pub knowledge_route: KnowledgeRoute,      // 含 toolTrace
    // ...
}

pub struct ToolCallRequest {
    pub tool: String,                         // "knowledge.list_catalog" | "knowledge.search" | "knowledge.open_slice"
    pub arguments: Document,
}

pub struct AgentSignal {
    pub kind: String,                         // ≤ 40 chars
    pub value: String,                        // 1..=80 chars
    pub evidence: Option<String>,             // ≤ 500 chars
    pub confidence: i32,                      // 0..=10
}
```

### 3.8 `DecisionReviewResult` 扩展（R2）

```rust
pub struct DecisionReviewResult {
    // ── 既有字段保留 ──
    pub approved: bool,
    pub scores: ReviewScores,
    pub risks: Vec<String>,
    pub claim_analysis: Option<ClaimAnalysis>,

    // ── R2 新增 ──
    pub needs_revision: bool,                 // default false
    pub revision_direction: String,           // ≤ 1024 chars，default ""
    pub should_hold: bool,                    // default false
    pub hold_reason: String,                  // ≤ 512 chars，default ""
    pub hold_category: String,                // held_by_ai_policy | blocked_by_safety_guard | ai_waiting_for_more_context | ""
    pub self_critique_addressed: bool,        // default false

    // ── R9 同步落库（与 agent_run_logs 一致）──
    pub revision_applied: bool,
    pub final_review_status: String,
}
```

`hold_category` 取值 SHALL 严格在三选一内；`held_for_human / human_required / waiting_for_human` 等取值由 `assert_hold_category_valid` 强制阻断（违规改为 `held_by_ai_policy` + 写 `agent_events kind="autonomy_hold_category_invalid"`）。

### 3.9 `RunBudget` 扩展（R4.3）

```rust
// src/agent/budget.rs
pub struct RunBudget {
    // ── 既有 ──
    pub token_budget: i64,
    pub tokens_used: i64,
    pub llm_call_budget: i32,
    pub llm_calls_used: i32,

    // ── R4 新增 ──
    pub tool_call_budget: i32,                // 由 runtime_parameters.knowledge_max_tool_calls 注入
    pub tool_calls_used: i32,
}

impl RunBudget {
    pub fn record_tool_call(&mut self, tokens_consumed: i64) -> Result<(), BudgetError> {
        if self.tool_calls_used >= self.tool_call_budget { return Err(BudgetError::ToolCallsExceeded); }
        if self.tokens_used + tokens_consumed > self.token_budget { return Err(BudgetError::TokensExceeded); }
        self.tool_calls_used += 1;
        self.tokens_used += tokens_consumed;
        Ok(())
    }

    pub fn is_exceeded(&self) -> bool { /* 既有 + tool_calls */ }
}
```


## Components and Interfaces

> 中文小节标题：4. 组件级改造

### 4.1 `src/agent/run_envelope.rs`（新模块，R0）

```rust
pub fn write_run_envelope_started(
    db: &Database,
    run_id: &str,
    account_id: ObjectId,
    contact_wxid: &str,
    source_event_id: &str,
    source_kind: &str,
) -> Result<(), DbError> {
    // insert_one with lifecycle="started", gateway_status="pending", final_review_status=""
    // 必须发生在任何 LLM 调用之前；try/catch 之外
}

pub fn update_run_envelope_terminal(
    db: &Database,
    run_id: &str,
    update_fields: AgentRunLogTerminalFields,
) -> Result<(), DbError> {
    // update_one({run_id}) + $set 全部最终字段
    // 若 matched_count == 0 → tracing::error + 单次 insert_one 兜底 + agent_events kind="run_envelope_recovered_via_insert"
}

pub fn install_panic_hook_for_envelope(db: Arc<Database>) {
    // std::panic::catch_unwind 包裹 run pipeline
    // panic 时尽力 update lifecycle = failed_before_decision / failed_after_decision，写 error_summary="unhandled_panic: ..."
    // 二次失败：tracing::error! 不再尝试，避免 panic-in-panic
}
```

调用点改造（`src/agent/gateway.rs::run_user_operation_gateway` 入口）：

```rust
pub async fn run_user_operation_gateway(ctx: GatewayContext) -> Result<GatewayOutput, GatewayError> {
    // 1) R0：先写 envelope
    write_run_envelope_started(&ctx.db, &ctx.run_id, ctx.account_id, &ctx.contact_wxid,
                               &ctx.source_event_id, &ctx.source_kind).await?;

    // 2) 业务主流程（panic / 异常都被 envelope panic hook 捕获 → 终态推进）
    let result = inner_pipeline(ctx).await;

    // 3) update_run_envelope_terminal：写最终所有字段
    update_run_envelope_terminal(&ctx.db, &ctx.run_id, build_terminal_fields(&result)).await?;

    Ok(result?)
}
```

**关键约束**：`write_agent_run_log` 函数体改写为内部调用 `update_run_envelope_terminal`；外部调用方接口保持不变（向后兼容现有测试）。

### 4.2 `src/agent/guards.rs::normalize_decision_runtime`（N1 + R3）

**改造前（现状，~line 554）**：替 Agent 补 `risk_level / knowledge_need / run_mode / needs_review / consolidation_needed` 默认值。

**改造后**：

```rust
pub fn normalize_decision_runtime(
    decision: &mut AgentDecision,
    raw: &RawAgentDecision,
    risks: &mut Vec<String>,
) {
    // 不再补默认值；只做：
    // (1) 白名单枚举校验 — 非法值 → push "invalid_enum_value:<field>:<value>"
    // (2) planner 同步语义（既有）
    // (3) operation_state 必须在 default_user_operation_state_machine 中

    // 必填校验由 RawAgentDecision::validate_and_promote 在更早一步处理（见 4.3）
}
```

任何当前依赖此函数补默认值的下游代码（如 `gateway.rs` 调用点）SHALL 改为依赖 `validate_and_promote` 返回的 `Vec<RiskTag>`，failure 时直接走 R3.5 review_failed 路径。

### 4.3 `RawAgentDecision::validate_and_promote`（N2 + R1 + R3）

```rust
impl RawAgentDecision {
    pub fn validate_and_promote(
        self,
        runtime: &RuntimeParameters,
    ) -> (AgentDecision, Vec<String> /* risks */) {
        let mut risks = Vec::new();

        // ── decision_phase 解析 (R1.10) ──
        let phase = match self.decision_phase.as_deref() {
            Some("tool_calling") => "tool_calling".to_string(),
            Some("final") | None => "final".to_string(),
            Some(other) => {
                risks.push(format!("decision_phase_invalid:{}", other));
                "final".to_string()
            }
        };

        // ── tool_calling 中间轮：跳过 R1.3 / R1.4 / R1.5 / R3 校验 ──
        if phase == "tool_calling" {
            return (build_tool_calling_decision(self), risks);
        }

        // ── final 轮：执行完整校验 ──

        // R3.1 / R3.5 必填 + 枚举严格
        let risk_level = check_enum(self.risk_level, &["low", "medium", "high"], "risk_level", &mut risks);
        let knowledge_need = check_enum(self.knowledge_need, &["not_required", "required", "insufficient"], "knowledge_need", &mut risks);
        let run_mode = check_enum(self.run_mode, &["fast_chat", "memory_candidate", "knowledge_grounded", "high_risk"], "run_mode", &mut risks);
        let autonomy_mode = check_enum(self.autonomy_mode, &["auto", "assisted", "blocked"], "autonomy_mode", &mut risks);
        let needs_review = check_required_bool(self.needs_review, "needs_review", &mut risks);
        let consolidation_needed = check_required_bool(self.consolidation_needed, "consolidation_needed", &mut risks);
        let operation_state = check_required_string(self.operation_state, "operation_state", &mut risks);

        // R1.3 7 个始终必填字段
        let user_understanding = check_required_string_with_whitespace(self.user_understanding, "user_understanding", &mut risks);
        // ... 其余 6 个

        // R1.4 互斥必填 (whyShouldReply / whySkipReply 由 should_reply 决定)
        // ...

        // R1.5 / R1.6 条件长度（low_routine vs critical_turn）
        // ...

        // R3.6 任何 risks 非空 → autonomy_mode 强制 blocked（由 finalize_review_for_send 触发）
        // 这里仅返回 risks，由调用方决定如何 finalize

        let decision = AgentDecision {
            risk_level, knowledge_need, run_mode, autonomy_mode,
            needs_review, consolidation_needed, operation_state,
            decision_phase: phase,
            user_understanding,
            // ...
            tool_calls: self.tool_calls.unwrap_or_default(),
            agent_generated_signals: self.agent_generated_signals.unwrap_or_default(),
            // ...
        };

        (decision, risks)
    }
}
```

### 4.4 `src/agent/review.rs::local_decision_review`（R3.7 / R3.8）

```rust
pub fn local_decision_review(decision: &AgentDecision, budget: &RunBudget) -> DecisionReviewResult {
    if budget.is_exceeded() && decision.needs_review {
        // R3.7：blocked
        DecisionReviewResult {
            approved: false,
            risks: vec!["budget_exceeded_no_review".to_string()],
            scores: default_scores(),
            // 注意：autonomy_mode 由 finalize_review_for_send 在汇总时强制 "blocked"
            ..Default::default()
        }
    } else if budget.is_exceeded() && !decision.needs_review {
        // R3.8：低风险快速通道
        DecisionReviewResult {
            approved: true,
            risks: vec!["local_review_low_risk_only".to_string()],
            scores: default_scores(),
            ..Default::default()
        }
    } else {
        // 既有：默认 approved=true
        DecisionReviewResult { approved: true, ..Default::default() }
    }
}
```

### 4.5 `finalize_review_for_send`（N3，新函数 — R3 / R5 / R8 最终安全汇总层）

```rust
pub fn finalize_review_for_send(
    review: DecisionReviewResult,
    decision: &mut AgentDecision,
    runtime: &RuntimeParameters,
    contact: &Contact,
    knowledge_runtime: &KnowledgeRuntime,
    promote_risks: Vec<String>,    // 来自 validate_and_promote
) -> (DecisionReviewResult, GatewayStatusFinal) {
    let mut review = review;
    review.risks.extend(promote_risks);

    // ── R3.5 / R3.6 必填违规 → blocked_by_required_field ──
    if has_protocol_violation(&review.risks) {
        review.approved = false;
        decision.should_reply = false;
        decision.autonomy_mode = "blocked".to_string();
        return (review, GatewayStatusFinal::BlockedByRequiredField);
    }

    // ── R3.7 预算超额 + needs_review ──
    if review.risks.contains(&"budget_exceeded_no_review".to_string()) {
        decision.should_reply = false;
        decision.autonomy_mode = "blocked".to_string();
        return (review, GatewayStatusFinal::BlockedByBudget);
    }

    // ── R5.4 verified knowledge 强约束 ──
    if requires_product_knowledge(&review, decision) {
        let verified_chunks = compute_verified_chunks(&decision.used_knowledge_ids, knowledge_runtime);
        if verified_chunks.is_empty() {
            review.scores.fact_risk = review.scores.fact_risk.max(6);
            review.approved = false;
            review.risks.push("product_claim_without_verified_knowledge".to_string());
            decision.should_reply = false;
            decision.autonomy_mode = "blocked".to_string();
            return (review, GatewayStatusFinal::BlockedUnverifiedProductClaim);
        }
        // R5.7 safe_claims 反向门
        for claim in &decision.safe_claims_used {
            if !verified_chunks.iter().any(|c| c.safe_claims.contains(claim)) {
                push_capped_risk(&mut review.risks, format!("safe_claim_not_verified:{}", claim), 5);
            }
        }
    }

    // ── R5.3.a / R5.3.b claim_analysis 缺失 fail-closed ──
    if review.claim_analysis.is_none() {
        if is_product_claim_inferred(decision) {
            // 3.a fail-closed
            review.approved = false;
            review.risks.push("claim_analysis_malformed".to_string());
            decision.should_reply = false;
            decision.autonomy_mode = "blocked".to_string();
            return (review, GatewayStatusFinal::BlockedBySafetyGuard);
        } else {
            // 3.b 综合判断
            review.risks.push("claim_analysis_malformed".to_string());
        }
    }

    // ── R8 字典 candidate 标记（不阻塞）──
    apply_taxonomy_check(decision, &mut review.risks /* 仅 push tag，不强制 false */);

    // ── R2.6 shouldHold 路径 ──
    if review.should_hold {
        let cat = if is_valid_hold_category(&review.hold_category) {
            review.hold_category.clone()
        } else {
            // 非法 → 强制改为 held_by_ai_policy + 写事件
            "held_by_ai_policy".to_string()
        };
        decision.should_reply = false;
        return (review, GatewayStatusFinal::Held(cat));
    }

    // ── R2.3 needs_revision 路径（由调用方处理）──
    // 这里不直接发起 revision，仅返回 review；gateway 主流程根据 needs_revision 决定是否触发第二轮 Reply Agent

    if review.approved && decision.should_reply {
        (review, GatewayStatusFinal::Approved)
    } else {
        (review, GatewayStatusFinal::HeldByAiPolicy)
    }
}
```

`GatewayStatusFinal` 枚举严格映射到 R0/R9 的 `gateway_status / finalReviewStatus` 取值（参考需求文档状态映射表）。

**关键调用点（`src/agent/gateway.rs` 三分支接入）**：

```rust
// 改造前
let review = if budget.is_exceeded() { local_decision_review(...) }
             else if should_run_review() { review_decision(...).await? }
             else { local_decision_review(...) };

// 改造后
let raw_review = if budget.is_exceeded() { local_decision_review(&decision, &budget) }
                 else if should_run_review(&decision) { review_decision(...).await? }
                 else { local_decision_review(&decision, &budget) };

let (mut review, status_final) = finalize_review_for_send(
    raw_review, &mut decision, &runtime, &contact, &knowledge_runtime, promote_risks);

// R2 single-shot revision
if review.needs_revision && !review.should_hold && !budget.is_exceeded() && !revision_attempted {
    let revised = run_reply_agent_revision(&decision, &review.revision_direction, &mut budget).await?;
    let (revised_decision, revised_promote_risks) = revised.validate_and_promote(&runtime);
    let raw_review2 = review_decision(&revised_decision, ...).await?;
    let (review2, status_final2) = finalize_review_for_send(raw_review2, &mut revised_decision, ...);
    // ...
}
```

### 4.6 `reply_with_tools_loop`（R4，新函数 — `src/agent/knowledge_router.rs` 或新模块）

```rust
pub async fn reply_with_tools_loop(
    ctx: &mut ReplyContext,
    runtime: &RuntimeParameters,
    budget: &mut RunBudget,
) -> Result<(AgentDecision, Vec<String> /* risks */), GatewayError> {
    let max_loops = runtime.knowledge_max_tool_loops.clamp(1, 5);
    let loop_start = Instant::now();
    let mut accumulated_tool_results = Vec::new();
    let mut all_risks = Vec::new();
    let mut consecutive_errors: i32 = 0;

    for loop_count in 0..max_loops {
        if loop_start.elapsed() > Duration::from_secs(30) {
            // R4.8 总超时
            return Err(GatewayError::ToolLoopTimeout);
        }

        let prompt = build_reply_prompt_with_tool_results(ctx, &accumulated_tool_results);
        let raw = call_reply_agent(prompt, budget).await?;
        let (mut decision, risks) = raw.clone().validate_and_promote(runtime);
        all_risks.extend(risks);

        // R1.10 phase 解析
        if decision.decision_phase == "tool_calling" {
            // R4.1.a：tool_calling 中间轮
            if !decision.reply_text.is_empty() || decision.should_reply {
                all_risks.push("tool_calling_phase_with_reply_text".to_string());
                decision.reply_text = "".to_string();
                decision.should_reply = false;
            }
            // R4.7 每轮 toolCalls 上限
            if decision.tool_calls.len() > 4 {
                decision.tool_calls.truncate(4);
                all_risks.push("tool_calls_per_turn_truncated".to_string());
            }

            // R4.2 派发循环
            for call in &decision.tool_calls {
                match dispatch_tool_call(call, ctx, budget).await {
                    Ok(result) => {
                        accumulated_tool_results.push(ToolResult {
                            tool: call.tool.clone(),
                            arguments: call.arguments.clone(),
                            result,
                            latency_ms: ...,
                        });
                        consecutive_errors = 0;
                    }
                    Err(e) => {
                        accumulated_tool_results.push(ToolResult { error: Some(e), ... });
                        consecutive_errors += 1;
                        if consecutive_errors >= 3 {
                            // R4.8 失败连击
                            all_risks.push("tool_call_failure_streak".to_string());
                            return Err(GatewayError::ToolCallFailureStreak);
                        }
                    }
                }
            }
            // 注入 tool results 到下一轮 prompt（≤ 8000 chars 截断）
            ctx.inject_tool_results(&accumulated_tool_results, &mut all_risks);
            continue;
        }

        // R4.1.b：final 轮
        if !decision.tool_calls.is_empty() {
            decision.tool_calls.clear();
            all_risks.push("final_phase_extra_tool_calls_dropped".to_string());
        }
        // R4.11 声明而未使用
        if decision.knowledge_need_reason != "" && decision.knowledge_need_reason != "unchanged"
           && !has_successful_search_or_open(&accumulated_tool_results) {
            all_risks.push("knowledge_need_declared_but_not_consulted".to_string());
        }
        // R4.10 toolTrace 落 decision.knowledge_route
        decision.knowledge_route.tool_trace = accumulated_tool_results.into_iter()
            .map(|r| r.into_trace_entry()).take(32).collect();
        return Ok((decision, all_risks));
    }

    // R4.2 MAX_TOOL_LOOPS 耗尽
    all_risks.push("tool_loop_exhausted".to_string());
    // 强制下一次 final（不再带 toolCalls）
    let prompt = build_reply_prompt_with_tool_results(ctx, &accumulated_tool_results);
    let raw = call_reply_agent_force_final(prompt, budget).await?;
    let (mut decision, risks) = raw.validate_and_promote(runtime);
    decision.tool_calls.clear();
    all_risks.extend(risks);
    Ok((decision, all_risks))
}
```

**`dispatch_tool_call`** 路由到三个工具（`knowledge.list_catalog / search / open_slice`），每个 5s timeout（R4.8），integrity_status 过滤（R4.5/R4.6 — `body / snippet` 在非 verified 时被 `<redacted_unverified_chunk>` 占位但 `integrity_status` 字段保留），结果回写时同步计入 `RunBudget.record_tool_call(tokens)` 触发 R4.3 预算硬上限。

**fast lane 回退（R4.12 / R11.3）**：`runtime.knowledge_routing_mode == "classic_router"` 时 gateway 路径 SHALL 跳过 `reply_with_tools_loop`，回退到现 `run_knowledge_router` 一次性拼 prompt 路径；这保留作为 7 天验证窗口内的灰度回退，D+14 移除。

### 4.7 `outbox_dispatcher`（R13，新模块 `src/agent/outbox.rs`）

```rust
pub mod outbox {
    pub async fn enqueue(
        db: &Database,
        run_id: &str,
        decision_id: ObjectId,
        source_event_id: &str,
        source_kind: &str,
        contact_wxid: &str,
        account_id: ObjectId,
        content: &str,
    ) -> Result<EnqueueOutcome, OutboxError> {
        let content_hash = sha256_hex(content);
        let idempotency_key = if source_event_id.is_empty() {
            // 兜底 + 警告事件
            write_event(db, "outbox_synthetic_idempotency_key", ...).await?;
            sha256_hex(&format!("synthetic:{}:{}:{}", run_id, contact_wxid, content_hash))
        } else {
            sha256_hex(&format!("{}:{}:{}", source_event_id, contact_wxid, content_hash))
        };

        let entry = OutboxEntry {
            _id: ObjectId::new(),
            account_id, contact_wxid: contact_wxid.into(),
            run_id: run_id.into(), decision_id,
            source_event_id: source_event_id.into(), source_kind: source_kind.into(),
            content: content.into(), content_hash,
            idempotency_key,
            attempt: 0, max_attempts: 3,
            status: OutboxStatus::Pending.to_string(),
            cancel_reason: None, last_error: None, next_retry_at: None,
            worker_id: None, locked_until: None,
            created_at: now(), updated_at: now(), sent_at: None,
        };

        match db.collection::<OutboxEntry>("agent_send_outbox").insert_one(&entry).await {
            Ok(_) => {
                write_event(db, "outbox_created", ...).await?;
                Ok(EnqueueOutcome::Created(entry._id))
            }
            Err(e) if is_duplicate_key(&e) => {
                // R13.2 强幂等
                write_event(db, "outbox_idempotent_skip", ...).await?;
                Ok(EnqueueOutcome::IdempotentSkip)
            }
            Err(e) => Err(OutboxError::Db(e)),
        }
    }

    pub struct OutboxDispatcher {
        worker_id: String,                // hostname:pid:uuid
        poll_interval: Duration,
        lease: Duration,
    }

    impl OutboxDispatcher {
        pub async fn run(self, db: Arc<Database>) {
            loop {
                tokio::time::sleep(self.poll_interval).await;
                self.tick(&db).await.ok();
            }
        }

        async fn tick(&self, db: &Database) -> Result<(), OutboxError> {
            // (1) 崩溃恢复：扫 status="in_flight" AND locked_until < now → 抢回为 pending
            self.reclaim_expired_leases(db).await?;
            // (2) 抢占 pending 条目并处理
            while let Some(entry) = self.atomic_claim_pending(db).await? {
                self.process_entry(db, entry).await.ok();  // 单条失败不影响其它
            }
            Ok(())
        }

        async fn atomic_claim_pending(&self, db: &Database) -> Result<Option<OutboxEntry>, OutboxError> {
            // findOneAndUpdate({status:"pending", $or:[next_retry_at:null, next_retry_at:{$lte:now}]},
            //                  {$set:{status:"in_flight", worker_id:self.worker_id, locked_until:now+lease, updated_at:now}})
            // returnDocument: After
        }

        async fn reclaim_expired_leases(&self, db: &Database) -> Result<(), OutboxError> {
            // findOneAndUpdate({status:"in_flight", locked_until:{$lt:now}},
            //                  {$set:{status:"pending", worker_id:null, locked_until:null}})
            // 同时写 agent_events kind="outbox_lease_expired"
        }

        async fn process_entry(&self, db: &Database, mut entry: OutboxEntry) -> Result<(), OutboxError> {
            // (1) R13.4 二次安全门 — 4 类取消检查
            if let Some(reason) = self.second_safety_gate(&entry, db).await? {
                self.cancel(db, &mut entry, &reason).await?;
                return Ok(());
            }
            // (2) R13.5 调 mcp::send_message
            match mcp::send_message(&entry.contact_wxid, &entry.content).await {
                Ok(()) => self.mark_sent(db, entry).await,
                Err(e) => self.handle_send_failure(db, entry, e).await,
            }
        }

        async fn second_safety_gate(&self, entry: &OutboxEntry, db: &Database) -> Result<Option<String>, _> {
            // a) contact.cooldown_until > now → "contact_cooldown_active"
            // b) contact.last_inbound_at > decision.created_at && outcome=="user_replied_stop_requested" → "user_stop_requested_after_decision"
            // c) reaction outcome 含 stop_re* → 同上分类
            // d) created_at > 30min → "outbox_stale_30min"
        }

        async fn mark_sent(&self, db: &Database, mut entry: OutboxEntry) -> Result<(), _> {
            // status=Sent, sent_at=now, worker_id=null, locked_until=null
            // agent_events kind="outbox_sent"
        }

        async fn handle_send_failure(&self, db: &Database, mut entry: OutboxEntry, err: McpError) -> Result<(), _> {
            entry.attempt += 1;
            if entry.attempt < entry.max_attempts {
                // R13.5 backoff: now + (2^attempt)*5s + jitter
                entry.next_retry_at = Some(now() + backoff_with_jitter(entry.attempt));
                entry.status = OutboxStatus::Pending.to_string();
                entry.worker_id = None; entry.locked_until = None;
                entry.last_error = Some(err.to_string());
                update_entry(db, &entry).await?;
                write_event(db, "outbox_retry_scheduled", ...).await
            } else {
                entry.status = OutboxStatus::FailedTerminal.to_string();   // 注意：failed_terminal 统一枚举
                entry.last_error = Some(err.to_string());
                entry.worker_id = None; entry.locked_until = None;
                update_entry(db, &entry).await?;
                write_event(db, "outbox_failed_terminal", ...).await
            }
        }
    }

    pub async fn cancel_for_contact_on_user_reaction(
        db: &Database,
        account_id: ObjectId,
        contact_wxid: &str,
    ) -> Result<i64 /* canceled count */, OutboxError> {
        // 把同 contact 所有 status ∈ {pending, in_flight} 的 entry update 为 canceled + cancel_reason="user_reaction_stop_requested"
        // 同时清 worker_id / locked_until
        // 写 agent_events kind="outbox_canceled" 每条一条
    }
}
```

`record_user_reaction`（`src/agent/reaction.rs`）在检测到 stop_requested / cooldown 类 outcome 时新增调用 `outbox::cancel_for_contact_on_user_reaction`。

后台 API（W4 同步交付）：
- `POST /api/admin/outbox/:id/cancel` body `{ cancel_reason: String }`：仅允许取消 `pending / in_flight`；其它状态返回 409。
- `GET /api/admin/outbox?status=...&account_id=...&horizon=...`：列表查询。

### 4.8 `taxonomy.rs`（R8，新模块）

```rust
pub mod taxonomy {
    pub fn check_value(
        kind: &str,
        value: &str,
        scope: &str,
        cache: &TaxonomyCache,
    ) -> TaxonomyMatch {
        // 1) 命中 active 字典 → Match::Active
        // 2) 命中 deprecated 字典 → Match::Deprecated
        // 3) alias 命中 active → Match::AliasActive(canonical_id)
        // 4) 不命中 → Match::CandidateNew
    }

    pub async fn upsert_candidate(
        db: &Database,
        scope: &str, kind: &str, raw_value: &str,
        evidence: Option<&str>, confidence: i32,
    ) -> Result<(), DbError> {
        // upsert by (scope, kind, raw_value)
        // 已存在 status=rejected → 仅 last_seen_at 不递增 occurrences（避免 noise）
        // 已存在 status=pending → occurrences += 1
        // 已存在 status=approved → 不应到这（理论上字典里已有）
        // 不存在 → insert with status=pending, occurrences=1
    }

    pub async fn approve(db: &Database, candidate_id: ObjectId, by: &str) -> Result<(), _> {
        // 把 candidate 写入对应 system_taxonomies + 把 candidate.status=approved
    }

    pub async fn reject(db: &Database, candidate_id: ObjectId, by: &str) -> Result<(), _> {
        // candidate.status=rejected
    }
}
```

`enforce_decision_guards` 接入：

```rust
for (kind, value) in [("customer_stage", &decision.customer_stage),
                       ("intent_level", &decision.intent_level),
                       ("objection_type", &decision.objection_type)] {
    match taxonomy::check_value(kind, value, scope, cache) {
        TaxonomyMatch::Active => {},
        TaxonomyMatch::AliasActive(canonical) => decision.set_canonical(kind, canonical),  // 用 canonical id
        TaxonomyMatch::Deprecated => risks.push(format!("taxonomy_deprecated_value:{}:{}", kind, value)),
        TaxonomyMatch::CandidateNew => {
            risks.push(format!("taxonomy_candidate:{}:{}", kind, value));
            taxonomy::upsert_candidate(db, scope, kind, value, ..., decision.confidence_for(kind)).await.ok();
            // 关键：不 review.approved=false
        }
    }
}
```

### 4.9 Memory 模块改造（N5 / R6 / R7）

`compact_memory_card_with_previous / consolidate_contact_memory / default_memory_card / memory_card_from_contact / write_memory_to_db / read_memory_from_db` 全部签名改用 `MemoryCardTyped`：

```rust
// 改造前
pub fn compact_memory_card_with_previous(prev: &Document, next: &Document) -> Document;
// 改造后
pub fn compact_memory_card_with_previous(prev: &MemoryCardTyped, next: &MemoryCardTyped) -> MemoryCardTyped;
```

R7 `deprecatedFacts / conflicts` 处理：

```rust
pub fn apply_consolidator_deprecations(
    prev: &MemoryCardTyped,
    consolidator: &ConsolidatorOutput,
    warnings: &mut Vec<String>,
) -> MemoryCardTyped {
    let mut next = consolidator.into_memory_card();
    for dep in &consolidator.deprecated_facts {
        // R7.2 按 id 查找
        let found_in_prev = prev.find_fact_by_id(&dep.id);
        if found_in_prev.is_none() {
            warnings.push(format!("deprecated_fact_id_not_found:{}", dep.id));
            continue;
        }
        let mut deprecated_copy = found_in_prev.unwrap().clone();
        deprecated_copy.deprecated_at = parse_rfc3339_or_now(&dep.deprecated_at, &dep.id, warnings);
        deprecated_copy.deprecation_reason = Some(dep.reason.clone());
        deprecated_copy.updated_at = now();
        next.deprecated_facts.push(deprecated_copy);
        // 从 active 集合移除（如果 R7.2 同 id 误存活）
        next.core_facts.retain(|f| f.id != dep.id);
        next.recent_facts.retain(|f| f.id != dep.id);
        // R7.2 supersededBy 不存在警告
        if let Some(z) = &dep.superseded_by {
            if next.find_fact_by_id(z).is_none() {
                warnings.push(format!("superseded_by_id_not_found:{}:{}", dep.id, z));
            }
        }
    }
    // R7.4 cap 20 + R7.7 同 id 同时 active+deprecated 检查
    apply_deprecated_cap_and_dedup(&mut next, warnings);
    next
}
```

`conflicts[].winner != "none"` 写 `agent_events kind="memory_conflict_resolved"`；context 注入最近 K=5 条 `deprecatedFacts` 由 prompt builder 在 reply_context_pack 时实现。


## 5. 协议序列 / Sequence Protocols

### 5.1 工具循环协议（R4，编号步骤）

```
Step 0  入站消息 / 跟进任务触发，gateway 入口写 envelope (lifecycle="started")
Step 1  build_reply_prompt：注入 contact 画像 / memory_card / 最近 5 条 deprecatedFacts
Step 2  call_reply_agent → RawAgentDecision (loop 0)
Step 3  validate_and_promote → AgentDecision + risks
Step 4  IF decision_phase == "tool_calling":
        Step 4.1  IF reply_text != "" OR should_reply == true：丢弃这两个字段 + 追加 risk "tool_calling_phase_with_reply_text"
        Step 4.2  IF tool_calls.len > 4：截断到 4 + 追加 risk "tool_calls_per_turn_truncated"
        Step 4.3  FOR each call in tool_calls:
                  Step 4.3.a  budget.record_tool_call() 预算检查；超额返回 budget_exceeded 错误
                  Step 4.3.b  按 tool 名分发：list_catalog / search / open_slice，5s timeout
                  Step 4.3.c  非 verified chunk 的 body / snippet 替换为 redacted 占位（保 integrity_status）
                  Step 4.3.d  连续 3 次错误 → 强制结束循环 + risk "tool_call_failure_streak"
        Step 4.4  注入 [system tool result] 段（≤ 8000 chars 累计）→ 回到 Step 2 (loop_count += 1)
        Step 4.5  loop_count >= MAX_TOOL_LOOPS 且 tool_calls 仍非空 → force final 一次 + risk "tool_loop_exhausted"
        Step 4.6  loop 总耗时 > 30s → ToolLoopTimeout 错误，gateway_status = "tool_loop_timeout"
Step 5  ELSE (decision_phase == "final"):
        Step 5.1  IF tool_calls 非空：清空 + risk "final_phase_extra_tool_calls_dropped"
        Step 5.2  IF knowledge_need_reason 非空非 "unchanged" 且 toolTrace 中无成功的 search/open_slice：risk "knowledge_need_declared_but_not_consulted"
        Step 5.3  decision.knowledge_route.tool_trace = accumulated_results.take(32)（超出 risk "tool_trace_overflow"）
        Step 5.4  返回 (decision, all_risks) → 进入 R3 / R5 review 流程
```

### 5.2 review + revision + finalize 序列（R2 / R3 / R5）

```
Phase A  enforce_decision_guards (R8 字典 candidate / R7 memory 接入 / 既有 guards)
Phase B  分支选择：
         - budget.is_exceeded()  → local_decision_review (R3.7 / R3.8 二态)
         - should_run_review()    → review_decision (LLM full / light)
         - else                  → local_decision_review (默认 approved=true)
Phase C  finalize_review_for_send：
         C1  R3.5 必填违规 → blocked_by_required_field（autonomy_mode = blocked）
         C2  R3.7 budget_exceeded_no_review → blocked_by_budget
         C3  R5.4 verified knowledge 强约束 → blocked_unverified_product_claim
         C4  R5.3.a / R5.3.b claim_analysis 缺失 fail-closed → blocked_by_safety_guard
         C5  R8 字典 candidate 标记（不阻塞）
         C6  R2.6 should_hold → held_by_ai_policy / blocked_by_safety_guard / ai_waiting_for_more_context
         C7  approved + should_reply 通过
Phase D  IF needs_revision == true && !should_hold && !budget_exceeded && !revision_attempted:
         D1  call_reply_agent_revision(decision, revision_direction, budget) ← 30s 超时控制
         D2  validate_and_promote → 第二轮 decision
         D3  review_decision (第二轮)
         D4  finalize_review_for_send (第二轮) — 同 Phase C；revision_attempted = true
         D5  IF 第二轮仍 approved == false → gateway_status = "revision_failed"，should_reply = false
         D6  ELSE → finalReviewStatus = "revision_applied_approved"
Phase E  invalidate / branch:
         - revision LLM 30s 超时 / 不可解析 → "revision_failed" + agent_events "revision_llm_failure"
         - revisionDirection 空 → "revision_skipped_invalid_direction"
         - revision 之前 budget 超额 → "revision_skipped_budget_exceeded"
Phase F  write_agent_run_log (update_one) — finalReviewStatus / autonomyMode / revisionApplied / pre+postRevisionSummary 一次性落库
Phase G  IF finalReviewStatus ∈ {approved, revision_applied_approved} && should_reply:
         outbox::enqueue(...)（强幂等 idempotency_key 不含 run_id）
```

### 5.3 outbox dispatcher 状态机（R13）

```
        ┌───────────────────────────────────────────────────┐
        │                                                   │
        ▼                                                   │
   ┌─────────┐  atomic claim   ┌──────────┐  send ok    ┌──────┐
   │ pending │ ─────────────▶ │ in_flight │ ──────────▶ │ sent │
   └────┬────┘                └─────┬────┘             └──────┘
        │                           │
        │ send fail (attempt < 3)   │
        │ ◀─────────────────────────┘
        │ next_retry_at = now + 2^attempt*5s + jitter
        │
        │ send fail (attempt >= 3)
        ▼
   ┌──────────────────┐
   │ failed_terminal  │
   └──────────────────┘

        ▲    canceled by user_reaction / admin / cooldown / 30min stale
        │
   ┌──────────┐
   │ canceled │
   └──────────┘

   崩溃恢复：in_flight && locked_until < now → 抢回为 pending
            (worker_id=null, locked_until=null)
            agent_events kind="outbox_lease_expired"
```

**关键不变量**：
- 同 `idempotency_key` 唯一索引保证 enqueue 强幂等。
- `atomic claim` (`findOneAndUpdate`) + `locked_until` 双保险，防同一 entry 被两个 worker 同时处理。
- 崩溃恢复路径必须**先**于 `claim_pending`，避免新崩溃 entry 长期卡 in_flight。
- send 成功 / 失败 / 取消都清 `worker_id / locked_until`（避免后续 reclaim 误判）。

### 5.4 状态映射统一（gateway_status × finalReviewStatus）

设计层 `GatewayStatusFinal` 枚举到 `gateway_status / finalReviewStatus` 的映射表完全继承需求文档 Introduction 的状态映射表，**不重新定义**。`assert_gateway_status_valid / assert_final_review_status_valid` 在 `write_agent_run_log` 中强校验，超出枚举的取值 SHALL 阻断写库（R9.10.e）。

`autonomyMode` 落库取值规则（R9.3）：
- 正常路径：等于 `decision.autonomy_mode` 最终值（已经过 finalize_review_for_send 可能强制为 "blocked"）
- 缺失或非法：写 "blocked" + risk "autonomy_mode_invalid"

## Error Handling

> 中文小节标题：6. 错误处理

### 6.1 Run Envelope 错误处理（R0）

| 错误场景 | lifecycle 终态 | error_summary 模板 | 兜底 |
|---|---|---|---|
| LLM 超时（Reply / Review）发生在 decision 写出前 | `failed_before_decision` | `llm_timeout: <stage>` | update_one |
| JSON 解析失败 | `failed_before_decision` | `json_parse_error: <stage>` | update_one |
| Rust panic 在 decision 写出前 | `failed_before_decision` | `unhandled_panic: <message>` | catch_unwind + update_one |
| Rust panic 在 decision 写出后 | `failed_after_decision` | `unhandled_panic: <message>` | catch_unwind + update_one |
| MCP 错误（reply_with_tools_loop 内）| `failed_before_decision` | `mcp_error: <tool>: <message>` | update_one |
| 预算硬上限触发 run 取消 | `aborted_by_budget` | — | update_one + abort_reason 字段 |
| 用户拒绝 / cooldown 中途取消 | `aborted_by_external_signal` | — | abort_reason="..." |
| envelope insert 自身失败 | （无法落库） | tracing::error! | 单次重试，再失败放弃 |
| update_one matched_count == 0 | （兜底 insert）| — | 写 agent_events kind="run_envelope_recovered_via_insert" |
| panic-in-panic | （放弃）| tracing::error! | 不再写库 |

### 6.2 Tool Loop 错误处理（R4）

| 错误 | 处理 | 标记 |
|---|---|---|
| 单 tool call 5s 超时 | 该 call 返回 `{error: "timeout"}`，循环继续 | tool 错误计数 += 1 |
| 累计 ≥ 3 次 tool call 错误 | 强制结束循环，force tool_calls=[] | risk "tool_call_failure_streak" |
| 循环总耗时 > 30s | 抛 GatewayError::ToolLoopTimeout，gateway_status = "tool_loop_timeout"，fail-closed | finalReviewStatus = "blocked_by_safety_guard" |
| MAX_TOOL_LOOPS 耗尽且 tool_calls 仍非空 | force final 一次 + risk "tool_loop_exhausted" | 按 final 决策落最终状态 |
| 单轮 toolCalls > 4 | 截断到 4 | risk "tool_calls_per_turn_truncated" |
| 累计 tool result 注入 > 8000 chars | 丢弃最早 | risk "tool_result_context_truncated" |
| toolTrace > 32 条 | 截断 | risk "tool_trace_overflow" |
| budget 超额时调 tool | 立即返回 `{error: "budget_exceeded"}`，不实际执行 | risk "tool_budget_exhausted" |
| tool 名非法 / arguments schema 错 | 返回 `{error: "invalid_tool" / "invalid_input"}` | tool 错误计数 += 1 |
| open_slice 部分 chunk_id 命中、部分未命中 | 返回 `{error: "unknown_chunk_id", missing: [...]}` 全失败（避免 Agent 幻觉） | tool 错误计数 += 1 |

### 6.3 Revision 错误处理（R2）

| 触发条件 | gateway_status | finalReviewStatus | should_reply |
|---|---|---|---|
| revisionDirection 空或仅空白 | `revision_skipped_invalid_direction` | `revision_failed` | false |
| revision 之前 budget 超额 | `revision_skipped_budget_exceeded` | `revision_failed` | false |
| revision LLM 30s 超时 / 不可解析 | `revision_llm_failure` | `revision_failed` | false |
| revision 后第二轮 review 仍 fail | `revision_failed` | `revision_failed` | false |
| revision 后第二轮 review 通过 | `approved` | `revision_applied_approved` | true |

### 6.4 Outbox 错误处理（R13）

| 错误 | status 推进 | 事件 | 备注 |
|---|---|---|---|
| 同 idempotency_key 已存在 | (不写)| `outbox_idempotent_skip` | enqueue 阶段 |
| second safety gate 任一命中 | `canceled` | `outbox_canceled` | cancel_reason 写明 |
| MCP 调用失败、attempt < 3 | `pending` + next_retry_at | `outbox_retry_scheduled` | backoff 公式 (2^attempt)*5s + jitter |
| MCP 调用失败、attempt >= 3 | `failed_terminal` | `outbox_failed_terminal` | 不再重试 |
| worker 抢占后崩溃，lease 过期 | `pending`（重新抢占）| `outbox_lease_expired` | 由 reclaim_expired_leases 触发 |
| source_event_id 为空 | `pending` (synthetic key) | `outbox_synthetic_idempotency_key` | 警告事件，非阻塞 |
| 同 outbox_id 事件总数 > 20 | (不写) | — | 防 retry 风暴写爆 |

### 6.5 Memory Consolidator 错误处理（R6 / R7）

| 错误 | 处理 | 警告写入 |
|---|---|---|
| `evidence` 缺失或不足 | 写默认 + 警告 | `agent_run_logs.memory_consolidator_warnings: missing_evidence:<text>` |
| `confidence` 不在 0..=10 | 钳到 [0, 10] | `invalid_confidence:<text>:<value>` |
| `deprecatedFacts[].id` 在 prev 中查不到 | 不写入 deprecatedFacts | `deprecated_fact_id_not_found:<id>` |
| `supersededBy` 在新版中查不到 | deprecated 仍写入 | `superseded_by_id_not_found:<id>:<superseded_id>` |
| `deprecatedAt` 非 RFC3339 | 回退为 now | `invalid_deprecated_at:<id>:<raw>` |
| 同一 id 同时 active + deprecated | 仅在 deprecated 集合中保留 | `fact_simultaneously_active_and_deprecated:<id>` |

## 7. 前端设计 / Frontend Design

### 7.1 路由 + Tab 结构（R10.1）

`frontend/src/App.tsx` 已有 `运营成效中心` 顶级页面；新增 Tab：
- 路由：`/outcome/autonomy`
- Tab 标题：「自治回路监控」
- 复用现有 `tokens.css` 设计语言（R10.8）

### 7.2 指标卡布局（R10.2）

7 个指标卡 + 1 行发送链路状态：

```
┌──────────────────────────────────────────────────────────────────┐
│  Horizon: [24h ▼]  Account: [全部 ▼]                              │
├──────────────────────────────────────────────────────────────────┤
│  ┌────────┐ ┌────────┐ ┌────────────┐ ┌────────┐                │
│  │ 修订   │ │ 修订   │ │ AI 暂缓    │ │ 字典   │                │
│  │ 触发率 │ │ 通过率 │ │ 分类       │ │ 候选   │                │
│  │ 0.18   │ │ 0.71   │ │ 见下方饼图 │ │ 0.04   │                │
│  └────────┘ └────────┘ └────────────┘ └────────┘                │
│  ┌────────┐ ┌──────────┐ ┌────────────┐                         │
│  │ 未验证 │ │ 自我批判 │ │ 自治模式   │                         │
│  │ 拦截率 │ │ 解决率   │ │ 分布       │                         │
│  │ 0.02   │ │ 0.85     │ │ auto:0.6.. │                         │
│  └────────┘ └──────────┘ └────────────┘                         │
├──────────────────────────────────────────────────────────────────┤
│  AI 暂缓原因分布 (饼图 R10.7)：                                  │
│   ● AI 策略主动暂缓 (held_by_ai_policy)        12%               │
│   ● 安全门拦截 (blocked_by_safety_guard)        7%               │
│   ● AI 等待更多上下文 (ai_waiting_for_more_ctx) 4%               │
│  *严禁出现"人工接管"或"等待人工"分类*                            │
├──────────────────────────────────────────────────────────────────┤
│  发送链路状态：                                                   │
│   send_success: 0.92  canceled: 0.05  failed_terminal: 0.02     │
│   lease_expired: 0.01  mean_attempts_to_success: 1.08           │
├──────────────────────────────────────────────────────────────────┤
│  近 50 条 revision 记录                                          │
│  联系人 │ 修订前 │ 修订后 │ direction │ status │ holdCategory   │
│  ...                                                              │
└──────────────────────────────────────────────────────────────────┘
```

### 7.3 数据接口

新增后端：
- `GET /api/outcomes/autonomy?horizon=24h|7d|30d&account_id=...` — 7 指标 + 分子分母原始计数（`response_time ≤ 2s` 在 100k runs 规模下，R10.4）
- `GET /api/outcomes/autonomy/revisions?limit=50&horizon=...` — 近 N 条 revision 记录
- `GET /api/admin/outbox?status=...&account_id=...&horizon=...` — outbox 列表查询
- `GET /api/admin/taxonomies?kind=...` — 字典管理
- `POST /api/admin/taxonomies` / `PATCH /api/admin/taxonomies/:id` / `DELETE /api/admin/taxonomies/:id`（软删）
- `GET /api/admin/taxonomy-candidates?status=pending` — 候选审核
- `POST /api/admin/taxonomy-candidates/:id/approve` / `reject`
- `POST /api/admin/outbox/:id/cancel`

`total_runs == 0` 时所有比率返回 `null`，前端展示「暂无数据」（R10.5）。

### 7.4 严禁词检查（R2.7）

CI 文本 lint 规则（W6 落地）：扫描 `src/agent/ src/routes/ frontend/src/` 下**新增**字符串字面量（git diff 范围），禁止包含 `human / 人工 / 接管 / takeover / hand-off`。已有产品文案中的"人工抽查""人审"在波 D 之前保留兼容，本期新增内容 0 容忍。

### 7.5 历史脏数据隔离（R10.9.d）

`held_for_human / human_required` 等历史值 SHALL **不**被前端任何分类计数；接口聚合时 SHALL 用枚举严格 IN 语义（不是 prefix match），脏数据自然落空。


## Correctness Properties

> 中文小节标题：自治回路的可证伪性质（指向 §8.1 性质映射矩阵）

*A property is a characteristic or behavior that should hold true across all valid executions of a system — essentially, a formal statement about what the system should do. Properties serve as the bridge between human-readable specifications and machine-verifiable correctness guarantees.*

本节列出本期升级（agent-autonomy-loop）需要由属性测试（PBT）保证的核心不变量 P1–P7。每条性质的"测试文件 / 涉及组件 / 关联需求 / 用例数下限"详见下文 §8.1 性质映射矩阵（避免重复维护两份表格，故此处仅列**性质标题 + 一句中文摘要**，所有可执行细节以 §8.1 为权威来源）。

- **P1 自治字段必填**：对所有 `decision_phase == "final"` 的 RawAgentDecision，R1.3 / R3.5 中的必填字段任一缺失或枚举非法时，`finalize_review_for_send` SHALL 返回 `approved=false` 且 `autonomy_mode="blocked"`。
- **P2 Single-Shot Revision 上限**：对所有触发 `needsRevision=true` 的 run，单次 run 内 Reply Agent 调用次数 SHALL ≤ 2，且第二次仍 fail 时 `gateway_status="revision_failed"`、`should_reply=false`。
- **P3 预算超额不发送**：对所有 `RunBudget.is_exceeded() == true AND decision.needs_review == true` 的 run，最终 `should_reply` SHALL 为 false 且 `gateway_status="blocked_by_budget"`，`local_decision_review` 与 `finalize_review_for_send` 共同保证不存在"预算超额却仍然发送"的执行路径。
- **P4 产品声明强约束**：对所有需要 verified knowledge 支撑的产品声明回复（R5 触发条件命中），若 `used_knowledge_ids` 中无任一 `integrity_status == "verified"` 切片，`finalize_review_for_send` SHALL 返回 `approved=false` 且 `gateway_status="blocked_unverified_product_claim"`（fail-closed）。
- **P5 记忆冲突可追溯**：对所有 Memory Agent consolidator 输出的 `deprecatedFacts / conflicts`，`apply_consolidator_deprecations` SHALL 保留稳定 `id`，被弃用的旧 fact 仅出现于 `deprecated_facts` 集合中，且不会同时存活于 `core_facts / recent_facts` 中（互斥不变量）。
- **P6 字典 candidate 不阻塞**：对所有 Reply Agent 输出的 `customer_stage / intent_level / objection_type` 取值，若不在 `system_taxonomies` 字典内，`enforce_decision_guards` SHALL 仅写入 `taxonomy_candidates` 集合并在 `review.risks` 追加 `taxonomy_candidate:<kind>:<value>`，**不**强制 `review.approved=false`、**不**阻塞 Reply Agent 运行。
- **P7 工具循环不死锁 + 预算不被绕过**：对所有进入 `reply_with_tools_loop` 的 run，循环 SHALL 在 `MAX_TOOL_LOOPS` 轮内必然终止；每次 tool call SHALL 经过 `RunBudget::record_tool_call` 计数，预算耗尽后任何后续 tool call SHALL 返回 `{"error":"budget_exceeded"}` 而非实际执行；不存在"工具循环绕过 budget"的执行路径。

每条性质的具体测试文件、PBT 用例数下限（≥ 64）、关联组件与关联需求条目，由下方 §8.1 矩阵表权威定义。

### Property 1: 自治字段必填

*For any* `decision_phase == "final"` 的 RawAgentDecision，若 R1.3 / R3.5 中任一必填字段缺失或枚举非法，`finalize_review_for_send` SHALL 返回 `approved=false` 且 `autonomy_mode="blocked"`。

**Validates: Requirements 1.3, 3.5, 3.9**

### Property 2: Single-Shot Revision 上限

*For any* 触发 `needsRevision=true` 的 run，单次 run 内 Reply Agent 调用次数 SHALL ≤ 2；第二次仍 fail 时 `gateway_status="revision_failed"`、`should_reply=false`。

**Validates: Requirements 2.3, 2.4, 2.8**

### Property 3: 预算超额不发送

*For any* `RunBudget.is_exceeded() == true AND decision.needs_review == true` 的 run，最终 `should_reply` SHALL 为 false 且 `gateway_status="blocked_by_budget"`；不存在"预算超额却仍然发送"的执行路径。

**Validates: Requirements 3.7, 3.10**

### Property 4: 产品声明强约束

*For any* 需要 verified knowledge 支撑的产品声明回复（R5 触发条件命中），若 `used_knowledge_ids` 中无任一 `integrity_status == "verified"` 切片，`finalize_review_for_send` SHALL 返回 `approved=false` 且 `gateway_status="blocked_unverified_product_claim"`（fail-closed）。

**Validates: Requirements 5.4, 5.7**

### Property 5: 记忆冲突可追溯

*For any* Memory Agent consolidator 输出的 `deprecatedFacts / conflicts`，`apply_consolidator_deprecations` SHALL 保留稳定 `id`；被弃用的旧 fact 仅出现于 `deprecated_facts` 集合中，且不会同时存活于 `core_facts / recent_facts`（互斥不变量）。

**Validates: Requirements 6.3, 7.2, 7.4**

### Property 6: 字典 candidate 不阻塞

*For any* Reply Agent 输出的 `customer_stage / intent_level / objection_type` 取值，若不在 `system_taxonomies` 字典内，`enforce_decision_guards` SHALL 仅写入 `taxonomy_candidates` 集合并在 `review.risks` 追加 `taxonomy_candidate:<kind>:<value>`；不强制 `review.approved=false`、不阻塞 Reply Agent 运行。

**Validates: Requirements 8.3, 8.4**

### Property 7: 工具循环不死锁 + 预算不被绕过

*For any* 进入 `reply_with_tools_loop` 的 run，循环 SHALL 在 `MAX_TOOL_LOOPS` 轮内必然终止；每次 tool call SHALL 经过 `RunBudget::record_tool_call` 计数；预算耗尽后任何后续 tool call SHALL 返回 `{"error":"budget_exceeded"}` 而非实际执行。

**Validates: Requirements 4.2, 4.3, 4.8**

## Testing Strategy

> 中文小节标题：8. 测试策略

### 8.1 性质映射矩阵（R12 P1–P7）

| 性质 | 测试文件 | 涉及组件 | 关联需求 | 用例数下限 |
|---|---|---|---|---|
| **P1** 自治字段必填 | `tests/autonomy_protocol_pbt.rs` | `RawAgentDecision::validate_and_promote` + `finalize_review_for_send` | R1.3 / R3.5 / R3.9 | ≥ 64 |
| **P2** Single-Shot Revision 上限 | 同上 | `gateway::run_pipeline` (revision 循环) + `local_decision_review` | R2.3 / R2.4 / R2.8 | ≥ 64 |
| **P3** 预算超额不发送 | 同上 | `RunBudget::is_exceeded` + `local_decision_review` + `finalize_review_for_send` | R3.7 / R3.10 | ≥ 64 |
| **P4** 产品声明强约束 | 同上 | `finalize_review_for_send` (R5 分支) | R5.4 / R5.7 | ≥ 64 |
| **P5** 记忆冲突可追溯 | `tests/memory_card_invariants.rs`（扩展）+ 新 PBT | `apply_consolidator_deprecations` | R6.3 / R7.2 / R7.4 | ≥ 64 |
| **P6** 字典 candidate 不阻塞 | `tests/autonomy_protocol_pbt.rs` | `taxonomy::check_value` + `enforce_decision_guards` | R8.3 / R8.4 | ≥ 64 |
| **P7** 工具循环不死锁 + 预算不被绕过 | 同上 | `reply_with_tools_loop` + `RunBudget::record_tool_call` + `dispatch_tool_call` | R4.2 / R4.3 / R4.8 | ≥ 64 |

每条性质单条执行时间 SHALL ≤ 60 秒（防死循环）。失败时 SHALL 给出最小化反例（proptest 默认行为，无需额外配置）。

### 8.2 lib unit test 增量（R12.4）

| 区域 | 测试用例数下限 | 关联 |
|---|---|---|
| Run Envelope insert/update + panic + 超时 | ≥ 6 | R0.10 |
| 自治协议必填 + 条件长度 + decision_phase 门控 | ≥ 6 | R1.3 / R1.4 / R1.5 / R1.10 |
| revision 控制流 | ≥ 5 | R2.3 / R2.4 / R2.5 / R2.6 / R2.11 |
| local_decision_review 二态 + 枚举严格 | ≥ 3 | R3.7 / R3.8 / R3.11 |
| 工具循环 + 失败降级 + 预算计入 | ≥ 6 | R4.1 / R4.2 / R4.3 / R4.5 / R4.6 / R4.8 |
| verified knowledge block + claim_analysis fail-closed | ≥ 4 | R5.3 / R5.4 / R5.7 |
| MemoryCard 反序列化 + 整层 typed + stable id 迁移 | ≥ 4 | R6.1 / R6.2 / R6.3 / R6.4 |
| deprecatedFacts 落库 + conflict 事件 + warning | ≥ 4 | R7.2 / R7.5 / R7.7 / R7.8 |
| 字典 candidate 不阻塞 + 状态机分流 | ≥ 4 | R8.3 / R8.4 / R8.5 / R8.7 |
| 审计字段写入 + 严禁 held_for_human | ≥ 3 | R9.2 / R9.7 / R9.10 |
| 发送闭环 outbox + 强幂等 + 取消 + locked_until | ≥ 5 | R13.10 |

### 8.3 集成测试（testcontainers，R13.10）

- 决策通过 → outbox pending → worker 抢占 → MCP mock 成功 → status=`sent`
- MCP mock 失败 3 次 → status=`failed_terminal`（**枚举值统一**）
- `record_user_reaction` 检测 stop_requested → 同 contact 所有 pending outbox 被取消
- 30 分钟陈旧 outbox 自动取消
- 崩溃恢复：worker A 抢占后 kill，等 lease 过期，worker B 重新抢占并最终 `status=sent`
- 同 `source_event_id` 但 `run_id` 不同的多次决策 → 共享同一 idempotency_key → 只发送 1 次

### 8.4 happy_path 扩展（R12.6）

`tests/happy_path_run.rs` 新增：

1. **`autonomy_full_loop_with_revision`**：
   - 模拟 Reply Agent 输出引发 `needsRevision` → revision 后通过
   - 断言 Reply Agent 调用次数恰好 2
   - 断言 `agent_run_logs.revisionApplied == true`
   - 断言 `finalReviewStatus == "revision_applied_approved"`
   - 断言 `pre_revision_summary / post_revision_summary` 都非空

2. **`autonomy_tool_loop_happy_path`**：
   - 模拟 Reply Agent 调用 list_catalog → search → open_slice
   - 断言 `decision.knowledge_route.tool_trace.len() == 3`
   - 断言 `RunBudget.tool_calls_used == 3`
   - 断言 `finalReviewStatus == "approved"`

测试 helper 新增 `wait_for_outbox_processed(run_id, timeout=10s)` —— 由于 outbox 解耦，老测试断言"已发送"的 case 必须等 worker tick 后才能 assert（N4 提示）。

### 8.5 基线核验脚本（N7 / R11.6）

`scripts/check-baseline.ps1`（PowerShell）+ `scripts/check-baseline.sh`（Linux/CI 复用）：

```powershell
# scripts/check-baseline.ps1
$lib = cargo test --lib 2>&1 | Select-String "test result: ok\." | Measure-Object | Select-Object -ExpandProperty Count
$pbt_tests = @("state_transition_pbt", "memory_card_invariants", "string_fact_risk_guard", "llm_retry_jitter")
$pbt_total = 0
foreach ($t in $pbt_tests) {
    $out = cargo test --test $t 2>&1
    $count = ($out | Select-String "(\d+) passed").Matches | ForEach-Object { [int]$_.Groups[1].Value } | Measure-Object -Sum | Select-Object -ExpandProperty Sum
    $pbt_total += $count
}

if ($lib -lt 78) { Write-Error "lib baseline failed: $lib < 78"; exit 1 }
if ($pbt_total -lt 33) { Write-Error "pbt baseline failed: $pbt_total < 33"; exit 1 }
Write-Host "baseline OK: lib=$lib, pbt=$pbt_total"
```

CI workflow（`.github/workflows/baseline.yml` 或等价）SHALL 在 `git push` 时自动运行；任何 PR 让任一数字下跌 SHALL 阻断合并（CI exit code 0 才放行）。

### 8.6 PBT mock 边界（R12.7）

性质测试 SHALL NOT 依赖真实 MongoDB / 真实 LLM / 真实 MCP / 真实网络：
- LLM mock：直接 stub `RawAgentDecision` 输出（避免 prompt 序列化往返）
- MongoDB mock：用纯函数 helper（参考现有 `compact_memory_card_with_previous` 风格），或 in-memory 字典
- MCP mock：trait + mock 实现（已有惯例）
- 时间 mock：`Clock` trait 注入（避免 backoff jitter 不可重现）

## 9. 灰度与 Sunset / Sunset Plan

`docs/sunset-plan.md`（W6 交付，R11.5）的核心时间表：

| 时点 | 动作 | 触发条件 / 验证 |
|---|---|---|
| **D** | 合并 W0–W6 全部代码 | CI baseline ≥ 78 / 33；R12 全部新性质通过 |
| **D + 7 天** | 跑"升级度量"脚本 | 剩余非结构化 facts 数 / 字典外 stage 数 / `legacy_mode_unchecked` run 占比 / `classic_router` run 占比；任一 > 0.1% 则推迟 |
| **D + 14 天** | 物理移除 `MemoryFactRepr::Plain` 反序列化兼容 + 移除 `autonomyProtocolEnabled / knowledgeRoutingMode` 字段 | 度量全部 ≤ 0.1% 持续 7 天；移除后 simulation 老 `Vec<String>` 输入路径 SHALL 改返回 400（R6.7） |
| **D + 21 天** | 物理移除 R6.4 / R8.8 数据迁移脚本 | 同一 migration_id 重启不再扫描 |

灰度开关在 sunset 前的行为：

| 开关 | true / 默认 | false / 回退 | sunset 后 |
|---|---|---|---|
| `autonomyProtocolEnabled` | 完整 R1 / R3 校验 → blocked_by_required_field | R1.3 / R1.4 / R3.5 不强制 review.approved=false；finalReviewStatus 改写 `legacy_mode_unchecked` | 物理移除，等价 always-on |
| `knowledgeRoutingMode` | `auto_tool_loop` → 走 reply_with_tools_loop | `classic_router` → 走旧 `run_knowledge_router` 一次性拼 prompt | 物理移除，等价 always auto_tool_loop |

**协议违规**（R11.9）：本期升级 SHALL NOT 引入 R11.5 之外的额外"双轨长期维护"代码；任何 PR 引入新的"灰度开关"但没写进 R11.5 SHALL 视为协议违规并阻断合并（设计层 PR review checklist 强制项）。

## 10. 实施顺序总结 / Implementation Wave Summary

| 波次 | 主要 PR | 输出 | 验证 |
|---|---|---|---|
| **W0** Infra | 1 个 PR | 3 collection accessor + 索引 + 迁移脚手架 + runtime_parameters 字段 + check-baseline.ps1 | 基线脚本通过；3 collection 启动期建表 OK |
| **W1** 协议骨架 | 1 个 PR | 删 normalize_decision_runtime 默认值；引入 RawAgentDecision + validate_and_promote；R0 envelope；新增 autonomy_mode / decision_phase 字段 | lib + 新单元测试 R0/R1 通过；老 PBT 全过 |
| **W2** 校验 + 安全门 | 1 个 PR | finalize_review_for_send；local_decision_review 二态；R5 verified knowledge 强约束；claim_analysis fail-closed | R3 / R5 单元测试 + P1/P3/P4 PBT 通过 |
| **W3** 工具 + 字典 | 1 个 PR | reply_with_tools_loop + 3 个 knowledge.* 工具；taxonomy 模块 + admin API；agentGeneratedSignals 字段 | R4 / R8 单元测试 + P6/P7 PBT 通过 |
| **W4** Outbox | 1 个 PR | outbox 模块 + dispatcher worker + atomic claim + lease + 重试 + 取消 + 二次安全门；admin /api/admin/outbox；reaction 接入 | R13 单元测试 + 集成测试（含崩溃恢复）通过 |
| **W5** Memory 替换 | 1 个 PR | OperatingMemory.memory_card 整层替换；R6 强类型 + 稳定 id；R7 deprecatedFacts 落库；MemoryFactRepr::Plain 兼容 | R6 / R7 单元测试 + P5 PBT + memory_card_invariants 扩展通过 |
| **W6** 监控 + 收口 | 1 个 PR | 前端 /outcome/autonomy Tab；/api/outcomes/autonomy；happy_path 扩展两 case；docs/sunset-plan.md；CI 严禁词 lint | E2E + happy_path + baseline 全过 |

W1–W4 串行；W5 可与 W3/W4 并行（不互相依赖）；W6 依赖 W1–W5 全部完成。

## 11. 关键设计决策 / Design Decisions

### 11.1 为什么用 RawAgentDecision + validate_and_promote 双层结构

**Rationale**：现状 `AgentDecision` 用非 Option 字段，serde 反序列化在缺字段时取 default value（`bool=false / String=""`），无法区分"模型没输出"与"模型输出 false / 空字符串"。N2 强制要求 raw 层用 `Option<T>` 表达"模型是否输出"，validate 层把"未输出"与"输出非法值"映射到不同 risks（`missing_required_field` vs `invalid_enum_value`）。这让 R3.5 / R12.P1 能精确测试。

**Alternative considered**：直接给 `AgentDecision` 字段加 `#[serde(default)]` 然后在 review 阶段判空。**Rejected**：在 `String=""` 与 Agent 真的输出 `""` 之间无法区分；R1.3 需要分别报告"未填"与"填了非法值"。

### 11.2 为什么 idempotency_key 不含 run_id

**Rationale**：同一条入站消息可能因为重试 run（envelope 已写但中途崩溃 → 新一次 run_id 重生成）触发多次决策；如果 idempotency_key 含 run_id，同 source_event_id 会写出 N 条 outbox（每次 run 一条），各自发送，导致用户收到重复消息。R13.2 公式 `SHA256(source_event_id + ":" + contact_wxid + ":" + content_hash)` 保证：

- 同一 `source_event_id` 触发的所有 run，只要最终 `content` 相同，idempotency_key 相同 → outbox 唯一索引保证只插入一次（第二次写入返回 DuplicateKey → idempotent skip）。
- 不同 `content`（例如 revision 改写后） → idempotency_key 不同 → 视为新消息（合理，因为修订后的内容确实是新发送意图）。

**Alternative considered**：用 `run_id + content_hash`。**Rejected**：违反需求文档明确约束。

### 11.3 为什么 finalize_review_for_send 是单一最终安全门

**Rationale**：N3 的核心问题是 `local_decision_review` 默认 `approved=true`，且 R5 / R3.7 / R8 的硬安全门散落在不同分支，容易漏覆盖。集中到 `finalize_review_for_send` 一个函数，所有 review 路径（LLM full / light / local）都接入这里，保证 R3.7 / R5.4 / R8.6 等硬约束**不能被任何上游 approved=true 路径绕过**（这是 P1 / P3 / P4 PBT 能成立的前提）。

### 11.4 为什么 outbox dispatcher 用 atomic claim + lease 而不是 distributed lock

**Rationale**：MongoDB 标准部署不开启事务（避免增加运维复杂度）；`findOneAndUpdate` 的 atomic 语义已经够用；`locked_until` 提供"租约自动过期"的崩溃恢复，比独立分布式锁服务（Redis / ZooKeeper）更轻量。

**关键不变量**（P7 PBT 保证）：在任意 worker 抢占 / 崩溃 / 续租序列下，永远不会出现 2 个 worker 同时持有同一 entry。

### 11.5 为什么 OperatingMemory.memory_card 整层替换而非局部 helper

**Rationale**：N5 的核心风险是"两套表示并行存在"会导致 helper A 转了但 helper B 没转 → 数据飘散。物理替换 `Document → MemoryCardTyped` 让 Rust 类型系统在编译期就阻断这种不一致。`extra: Document` 字段兜底承接老数据中未识别字段，保留向后兼容窗口；`MemoryFactRepr::Plain` 一次性兼容 `Vec<String>` 反序列化，sunset 后移除。

### 11.6 为什么 decision_phase 是 prompt 协议的一部分而非 Rust 推断

**Rationale**：R1.10 显式要求 Reply Agent 自己声明 `tool_calling` 还是 `final`。如果 Rust 通过 `tool_calls.is_empty()` 推断 phase，会导致两个误判：
- Agent 既输出 toolCalls 又输出 reply_text 时，Rust 不知道 Agent 的真实意图
- final 轮 Agent 误填 toolCalls 时，Rust 会错误地继续循环

让 Agent 显式声明 phase 让协议违规可在 R1.10 / R4.1 处精确捕获并标记 risks（`tool_calling_phase_with_reply_text / final_phase_extra_tool_calls_dropped`）。

### 11.7 为什么 system_taxonomies 不阻塞但 operation_state 严格阻塞

**Rationale**：
- `customer_stage / intent_level / objection_type` 是**业务理解**维度，Agent 对真实用户的判断会随时间演化（新行业 / 新场景），强制阻塞会让运行链路僵化。R8.3 / R8.4 用 `taxonomy_candidate` 候选审解决"既要严格聚合又不能死板"的矛盾。
- `operation_state` 是**状态机骨架**，乱填会导致后续状态迁移逻辑错乱（`allowedFrom` 校验失败、跟进任务调度失效）。R8.5 / R3.5 严格阻塞，与 R8 path 互斥。

## 12. 与需求文档的可追溯性 / Traceability

每个组件改造都明确标注了其满足的需求条目（章节 4 与章节 8）。本设计文档不引入需求文档之外的新功能或新枚举值；如实施过程中发现需求条目无法落地或冲突，SHALL 回到需求文档修改而非在设计层"打补丁"。

需求 ↔ 设计章节交叉索引：

| 需求 | 设计章节 |
|---|---|
| R0 Run Envelope | §3.1, §4.1, §5.1, §6.1 |
| R1 自治协议 9 字段 + decision_phase | §3.7, §4.3, §5.1 |
| R2 SelfCritique + Single-Shot Revision | §3.8, §4.5, §5.2, §6.3 |
| R3 必填校验 + autonomy_mode | §3.7, §4.2, §4.3, §4.5 |
| R4 工具循环 | §3.6, §3.9, §4.6, §5.1, §6.2 |
| R5 verified knowledge 强约束 | §4.5, §5.2 |
| R6 MemoryCard 整层强类型 | §3.5, §4.9 |
| R7 deprecatedFacts / conflicts | §4.9, §6.5 |
| R8 双层标签 | §3.3, §3.4, §4.8 |
| R9 审计字段 | §3.1, §5.4 |
| R10 前端监控 | §7 |
| R11 灰度 + sunset | §9 |
| R12 PBT + 回归 | §8 |
| R13 outbox | §3.2, §4.7, §5.3, §6.4 |
| N1–N7 实现注记 | §2.2, §2.3, §11 |
