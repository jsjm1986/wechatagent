use mongodb::bson::{oid::ObjectId, DateTime, Document};
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    Normal,
    Managed,
}

impl Default for AgentStatus {
    fn default() -> Self {
        Self::Normal
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfile {
    #[serde(default)]
    pub summary: String,
    #[serde(default, deserialize_with = "string_or_vec")]
    pub interests: Vec<String>,
    #[serde(default)]
    pub communication_style: String,
    #[serde(default)]
    pub operation_goal: String,
}

fn string_or_vec<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    if let Some(items) = value.as_array() {
        return Ok(items
            .iter()
            .filter_map(|item| item.as_str())
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(ToString::to_string)
            .collect());
    }
    if let Some(text) = value.as_str() {
        return Ok(text
            .split([',', '，', '\n', ';', '；'])
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(ToString::to_string)
            .collect());
    }
    Ok(Vec::new())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WechatAccount {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub alias: String,
    pub display_name: String,
    pub app_id: Option<String>,
    pub wxid: Option<String>,
    pub nick_name: Option<String>,
    pub mcp_base_url: Option<String>,
    pub mcp_api_key: Option<String>,
    pub online: bool,
    pub last_sync_at: DateTime,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contact {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub wxid: String,
    pub nickname: Option<String>,
    pub remark: Option<String>,
    pub alias: Option<String>,
    #[serde(default)]
    pub agent_status: AgentStatus,
    pub human_profile_note: Option<String>,
    pub agent_profile: Option<AgentProfile>,
    pub memory_summary: Option<String>,
    pub playbook_id: Option<ObjectId>,
    pub playbook_version: Option<i32>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub customer_stage: Option<String>,
    /// agent-autonomy-loop M2：`customer_stage` 上次发生变化的时间。
    /// 写入约束：仅当 `customer_stage` 实际变更时同步刷新（见
    /// `routes/shared.rs::set_customer_stage_with_ts`）。Planner
    /// `scan_stage_stagnation` 据此判定阶段长期停滞。
    #[serde(default)]
    pub customer_stage_updated_at: Option<DateTime>,
    pub intent_level: Option<String>,
    /// agent-autonomy-loop M2：结构化承诺列表（cap 8）。
    ///
    /// 取代原 `last_commitment: Option<String>` 自由文本字段，让 Planner
    /// `scan_commitments` 能按 `due_at` 判定 overdue/imminent。
    /// `serde(default)` 保证旧文档反序列化为空 Vec；迁移
    /// `2026_05_008_contact_commitments_reshape` 把旧 `last_commitment`
    /// 一次性转为单元素 `Vec<CommitmentRepr::Structured>` 并 `$unset` 旧字段。
    #[serde(default)]
    pub commitments: Vec<CommitmentRepr>,
    pub follow_up_policy: Option<String>,
    pub operation_state: Option<String>,
    pub operation_state_reason: Option<String>,
    pub operation_state_confidence: Option<i32>,
    pub operation_state_updated_at: Option<DateTime>,
    pub cooldown_until: Option<DateTime>,
    #[serde(default)]
    pub operation_policy: Document,
    #[serde(default)]
    pub profile_attributes: Document,
    pub profile_updated_at: Option<DateTime>,
    pub last_message_at: Option<DateTime>,
    pub last_inbound_at: Option<DateTime>,
    pub last_outbound_at: Option<DateTime>,
    pub last_agent_run_at: Option<DateTime>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MessageDirection {
    Inbound,
    Outbound,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMessage {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub contact_wxid: String,
    pub message_id: Option<String>,
    #[serde(default)]
    pub dedupe_key: Option<String>,
    pub direction: MessageDirection,
    pub content: String,
    pub raw: Option<Document>,
    pub created_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTask {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub contact_wxid: String,
    pub kind: String,
    pub run_at: DateTime,
    pub expires_at: Option<DateTime>,
    pub content: String,
    pub status: String,
    pub source_decision_id: Option<ObjectId>,
    #[serde(default)]
    pub review_required: bool,
    #[serde(default)]
    pub attempt_count: i32,
    #[serde(default)]
    pub max_attempts: i32,
    pub next_retry_at: Option<DateTime>,
    pub gateway_status: Option<String>,
    pub cancel_reason: Option<String>,
    pub error: Option<String>,
    #[serde(default)]
    pub claimed_at: Option<DateTime>,
    #[serde(default)]
    pub claim_recovery_count: i32,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEvent {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub contact_wxid: Option<String>,
    pub kind: String,
    pub status: String,
    pub summary: String,
    pub details: Option<Document>,
    pub created_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationRecord {
    #[serde(rename = "_id")]
    pub id: String,
    pub applied_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpCallLog {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub tool_name: String,
    pub request: Document,
    pub response: Option<Document>,
    pub error: Option<String>,
    pub created_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentAsset {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: Option<String>,
    pub kind: String,
    pub title: String,
    pub body: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub url: Option<String>,
    pub media_id: Option<String>,
    pub usage_scene: Option<String>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSoul {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub agent_kind: String,
    pub name: String,
    pub content: String,
    pub status: String,
    pub version: i32,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptTemplate {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub prompt_key: String,
    pub agent_kind: String,
    pub layer: String,
    pub title: String,
    pub description: Option<String>,
    pub content: String,
    pub status: String,
    pub version: i32,
    pub prompt_pack_version: String,
    pub created_by: String,
    pub created_at: DateTime,
    pub updated_at: DateTime,
    /// agent-self-evolution M4 / W0 Task 1.5：多版本支持。`true` 表示当前生效版本。
    /// 同 `(workspace_id, prompt_key)` 下应至多有一条 `current_version=true`，由
    /// `release_prompt` 在 mongo 事务内保证。缺字段时反序列化为 `false`，
    /// `2026_05_M4_001_prompt_template_versioned` 迁移把历史唯一一条置 `true`。
    #[serde(default)]
    pub current_version: bool,
    /// agent-self-evolution M4：被替换的上一版本号（rollback 时取回它）。
    /// `release_prompt` 写入新条时设为旧条的 `version`；首次 seed 为 `None`。
    pub previous_version: Option<i32>,
    /// agent-self-evolution M4：写入来源（`"system"` / `"legacy_migration"` /
    /// `"evolution_release"` 等），方便排查谁改的。
    pub seeded_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationPlaybook {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub name: String,
    pub description: Option<String>,
    pub method_prompt: String,
    pub profile_method: Option<String>,
    pub tag_method: Option<String>,
    pub stage_method: Option<String>,
    pub intent_method: Option<String>,
    pub follow_up_method: Option<String>,
    pub reply_style: Option<String>,
    pub forbidden_rules: Option<String>,
    pub success_criteria: Option<String>,
    pub created_by: String,
    pub is_default: bool,
    pub version: i32,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationDomainConfig {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub domain: String,
    pub name: String,
    pub goal: String,
    pub methodology: String,
    pub workflow: String,
    pub tool_policy: String,
    pub automation_policy: String,
    pub review_policy: String,
    #[serde(default)]
    pub runtime_parameters: Document,
    #[serde(default)]
    pub state_machine: Document,
    pub status: String,
    pub updated_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatingMemory {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub contact_wxid: String,
    #[serde(default)]
    pub user_understanding: Document,
    #[serde(default)]
    pub relationship_state: Document,
    #[serde(default)]
    pub product_fit: Document,
    #[serde(default)]
    pub next_action: Document,
    #[serde(default)]
    pub context_pack: Document,
    #[serde(default)]
    pub context_pack_version: i32,
    pub context_pack_updated_at: Option<DateTime>,
    #[serde(default)]
    pub memory_card: MemoryCardTyped,
    #[serde(default)]
    pub memory_card_version: i32,
    pub memory_card_updated_at: Option<DateTime>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryCandidate {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub contact_wxid: String,
    pub run_id: Option<String>,
    pub source: String,
    #[serde(default)]
    pub candidates: Vec<Document>,
    #[serde(default)]
    pub memory_write_score: i32,
    pub status: String,
    pub reason: Option<String>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationKnowledgeItem {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: Option<String>,
    pub domain: String,
    pub category: String,
    pub business_type: String,
    pub knowledge_type: Option<String>,
    pub business_context: Option<String>,
    pub title: String,
    pub summary: Option<String>,
    pub body: Option<String>,
    pub routing_card: Option<String>,
    #[serde(default)]
    pub applicable_scenes: Vec<String>,
    #[serde(default)]
    pub not_applicable_scenes: Vec<String>,
    #[serde(default)]
    pub suitable_for: Vec<String>,
    #[serde(default)]
    pub not_suitable_for: Vec<String>,
    #[serde(default)]
    pub customer_stages: Vec<String>,
    #[serde(default)]
    pub operation_states: Vec<String>,
    #[serde(default)]
    pub intent_levels: Vec<String>,
    #[serde(default)]
    pub safe_claims: Vec<String>,
    #[serde(default)]
    pub forbidden_claims: Vec<String>,
    #[serde(default)]
    pub common_questions: Vec<String>,
    #[serde(default)]
    pub common_objections: Vec<String>,
    #[serde(default)]
    pub evidence_items: Vec<String>,
    pub source_type: String,
    pub source_name: Option<String>,
    pub status: String,
    pub priority: i32,
    pub version: i32,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationKnowledgeDocument {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: Option<String>,
    pub domain: String,
    pub source_type: String,
    pub source_name: Option<String>,
    pub title: String,
    pub summary: Option<String>,
    pub catalog_summary: Option<String>,
    #[serde(default)]
    pub routing_map: Vec<String>,
    #[serde(default)]
    pub risk_notes: Vec<String>,
    pub raw_content: Option<String>,
    pub content_hash: Option<String>,
    #[serde(default)]
    pub line_index: Vec<Document>,
    #[serde(default)]
    pub section_index: Vec<Document>,
    pub status: String,
    pub version: i32,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationKnowledgeChunk {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: Option<String>,
    pub document_id: Option<ObjectId>,
    pub item_id: Option<ObjectId>,
    pub domain: String,
    pub knowledge_type: Option<String>,
    pub business_context: Option<String>,
    pub title: String,
    pub summary: Option<String>,
    pub body: Option<String>,
    pub routing_card: Option<String>,
    #[serde(default)]
    pub applicable_scenes: Vec<String>,
    #[serde(default)]
    pub not_applicable_scenes: Vec<String>,
    #[serde(default)]
    pub safe_claims: Vec<String>,
    #[serde(default)]
    pub forbidden_claims: Vec<String>,
    #[serde(default)]
    pub evidence_items: Vec<String>,
    pub source_quote: Option<String>,
    #[serde(default)]
    pub source_anchors: Vec<Document>,
    pub integrity_status: Option<String>,
    pub confidence_score: Option<i32>,
    #[serde(default)]
    pub distortion_risks: Vec<String>,
    #[serde(default)]
    pub unsupported_claims: Vec<String>,
    #[serde(default)]
    pub verified_claims: Vec<String>,
    pub status: String,
    pub priority: i32,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeUsageLog {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub contact_wxid: Option<String>,
    pub run_id: String,
    #[serde(default)]
    pub knowledge_ids: Vec<ObjectId>,
    #[serde(default)]
    pub route_result: Document,
    pub reply_text: Option<String>,
    pub review_approved: bool,
    pub blocked_reason: Option<String>,
    #[serde(default)]
    pub tool_trace: Vec<Document>,
    pub created_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDecisionReview {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub contact_wxid: Option<String>,
    pub run_id: Option<String>,
    pub inbound_message_id: Option<String>,
    pub reply_text: Option<String>,
    pub approved: bool,
    #[serde(default)]
    pub scores: Document,
    #[serde(default)]
    pub formula_breakdown: Document,
    #[serde(default)]
    pub risks: Vec<String>,
    pub rewrite_instruction: Option<String>,
    pub review_summary: Option<String>,
    pub playbook_id: Option<ObjectId>,
    pub playbook_version: Option<i32>,
    #[serde(default)]
    pub used_knowledge_ids: Vec<ObjectId>,
    #[serde(default)]
    pub prompt_versions: Document,
    pub operation_state: Option<String>,
    #[serde(default)]
    pub next_best_action: Document,
    #[serde(default)]
    pub context_pack_snapshot: Document,
    #[serde(default)]
    pub domain_config_snapshot: Document,
    #[serde(default)]
    pub runtime_parameters_snapshot: Document,
    #[serde(default)]
    pub send_gateway_result: Document,
    pub outcome_status: Option<String>,
    #[serde(default)]
    pub reaction_analysis: Document,
    #[serde(default)]
    pub reaction_claimed_at: Option<DateTime>,
    pub status: String,
    pub created_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRunLog {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub contact_wxid: Option<String>,
    pub run_id: String,
    pub trigger_kind: String,
    pub status: String,
    #[serde(default)]
    pub planner: Document,
    #[serde(default)]
    pub context: Document,
    #[serde(default)]
    pub knowledge_route: Document,
    #[serde(default)]
    pub decision: Document,
    #[serde(default)]
    pub review: Document,
    #[serde(default)]
    pub gateway_result: Document,
    pub error: Option<String>,
    /// MP-5 / Task 15：单次 run 的 token 预算。
    #[serde(default)]
    pub token_budget: i64,
    /// MP-5 / Task 15：本次 run 累计消耗 token。
    #[serde(default)]
    pub tokens_used: i64,
    /// MP-5 / Task 15：本次 run 实际 LLM 调用次数。
    #[serde(default)]
    pub llm_calls_used: i32,
    /// MP-5 / Task 15：本次 run 触发的降级原因列表（"review_skipped_budget_exceeded" 等）。
    #[serde(default)]
    pub degraded_reasons: Vec<String>,
    // ── agent-autonomy-loop W1 (Task 2.4) ──
    //
    // R0 Run Envelope + R9 自治审计字段。所有新增字段使用 `#[serde(default)]`
    // 以保持向后兼容：升级前已经写入的 `agent_run_logs` 文档不含这些字段，
    // 反序列化时取默认值（空字符串 / None / 空 Vec / false），不会破坏现有
    // 测试 / 历史回放。
    //
    // BSON 字段名沿用结构体的 snake_case（与 W0 task 1.2 在 `src/db/indexes.rs`
    // 创建、W6 task 7.1 修订后的 `(account_id, lifecycle, created_at)` /
    // `(account_id, final_review_status, created_at)` /
    // `(account_id, autonomy_mode, created_at)` 复合索引一致）。
    /// R0 Run Envelope：lifecycle ∈ `started / running / completed /
    /// failed_before_decision / failed_after_decision / aborted_by_budget /
    /// aborted_by_external_signal`。新建 envelope 时由
    /// [`crate::agent::run_envelope::write_run_envelope_started`] 显式置为
    /// `"started"`。
    #[serde(default)]
    pub lifecycle: String,
    /// R0 Run Envelope：触发本次 run 的入站消息或跟进任务 ID（`inbound_message.message_id`
    /// 或 `task_id`），是 R13 outbox `idempotency_key` 的核心成分。
    #[serde(default)]
    pub source_event_id: String,
    /// R0 Run Envelope：触发来源类别。允许枚举 `inbound_message / follow_up_task /
    /// manual_send`（具体校验在 W2 finalize 阶段）。
    #[serde(default)]
    pub source_kind: String,
    /// R0 Run Envelope：错误简要（≤ 1024 chars），由 envelope panic / failure
    /// 路径在 `lifecycle ∈ {failed_before_decision, failed_after_decision}` 时填写。
    #[serde(default)]
    pub error_summary: Option<String>,
    /// R0 Run Envelope：取消原因（≤ 256 chars），lifecycle =
    /// `aborted_by_external_signal` 时填写（如 `user_reaction_stop_requested`）。
    #[serde(default)]
    pub abort_reason: Option<String>,
    /// R9：是否触发了 single-shot revision。
    #[serde(default)]
    pub revision_applied: bool,
    /// R9：触发 revision 的原因摘要（≤ 1024 chars）。
    #[serde(default)]
    pub revision_reason: String,
    /// R9：revision 之前的 decision 文案摘要（≤ 2048 chars）。
    #[serde(default)]
    pub pre_revision_summary: Option<String>,
    /// R9：revision 之后的 decision 文案摘要（≤ 2048 chars）。
    #[serde(default)]
    pub post_revision_summary: Option<String>,
    /// R9：Reply Agent 自我批判（≤ 2048 chars）。
    #[serde(default)]
    pub self_critique: Option<String>,
    /// R3 / R9：自治控制位 ∈ `auto / assisted / blocked`。
    #[serde(default)]
    pub autonomy_mode: String,
    /// R9：最终归档状态（前端 horizon 聚合用）。允许枚举详见
    /// requirements.md "状态枚举映射表" finalReviewStatus 列。
    #[serde(default)]
    pub final_review_status: String,
    /// R13：与 outbox 的反向关联状态（dispatcher worker 在状态推进时
    /// `update_one(agent_run_logs, $set)` 反写）。允许 `pending / in_flight /
    /// sent / failed_terminal / canceled`。
    #[serde(default)]
    pub outbox_status: Option<String>,
    /// R7：memory consolidator 在 `apply_consolidator_deprecations` 中产出的
    /// warnings（如 `deprecated_fact_id_not_found:<id>` /
    /// `superseded_by_id_not_found:<id>:<sup>` 等）。
    #[serde(default)]
    pub memory_consolidator_warnings: Vec<String>,
    pub created_at: DateTime,
}

// ── agent-autonomy-loop W0 (Task 1.1) ──
//
// 三个新增 collection 的占位 struct（`agent_send_outbox / system_taxonomies /
// taxonomy_candidates`）。字段定义参照 design.md §3.2 / §3.3 / §3.4，最终字段
// 与 serde 命名约定将在 W3 / W4 波次的具体业务 task 中落定（含 idempotency_key
// 计算、字典 alias 命中、worker lease 字段语义等）。
//
// 设计注记（保持与既有代码一致的写法）：
// - `account_id` 在本仓既有数据模型中均为 `String`（不是 `ObjectId`），故占位
//   struct 沿用 `String`；如 W4 outbox dispatcher 需要 `ObjectId` 可届时调整。
// - 主键统一沿用 `pub id: Option<ObjectId>` + `#[serde(rename = "_id", skip_serializing_if = "Option::is_none")]`。
// - 不强制 camelCase rename；与现有大多数 collection（如 `AgentRunLog`）保持一致。

/// agent-autonomy-loop W0：`agent_send_outbox` 集合占位结构。
///
/// 用于 Reply Agent → review → MCP 发送之间的可靠链路（持久化 / 幂等 / 重试 /
/// 取消）。具体语义详见 design.md §3.2 与 requirements.md R13；本期 W0 仅用作
/// `Database::collection_agent_send_outbox` 的类型绑定，最终字段约束（如
/// `idempotency_key` 唯一索引、`status` 枚举值 `pending|in_flight|sent|failed_terminal|canceled`）
/// 在 W4 task 5.x 中落地。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboxEntry {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub contact_wxid: String,
    pub run_id: String,
    pub decision_id: Option<ObjectId>,
    pub source_event_id: String,
    pub source_kind: String,
    pub content: String,
    pub content_hash: String,
    pub idempotency_key: String,
    #[serde(default)]
    pub attempt: i32,
    #[serde(default)]
    pub max_attempts: i32,
    pub status: String,
    pub cancel_reason: Option<String>,
    pub last_error: Option<String>,
    pub next_retry_at: Option<DateTime>,
    pub worker_id: Option<String>,
    pub locked_until: Option<DateTime>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
    pub sent_at: Option<DateTime>,
}

/// agent-autonomy-loop W0：`system_taxonomies` 集合占位结构。
///
/// 双层标签的"严格字典"层：`customer_stage / intent_level / objection_type` 等
/// 维度由 `(scope, kind, value.id)` 唯一标识。详见 design.md §3.3 与
/// requirements.md R8；W3 task 4.6 / 4.7 / 4.9 落实加载 / alias 命中 / seed 迁移。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxonomyEntry {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    /// `"global"` 或 `account_id` 字符串。
    pub scope: String,
    /// 维度名（如 `"customer_stage"` / `"intent_level"` / `"objection_type"`）。
    pub kind: String,
    pub value: TaxonomyValue,
    pub updated_at: DateTime,
}

/// agent-autonomy-loop W0：`TaxonomyEntry.value` 内嵌结构。字段名采用 camelCase
/// 以便与 design.md 中"value.id"等索引路径一致（详见 R8.1 唯一索引）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaxonomyValue {
    /// 字典 key（如 `"first_contact"`），不是 BSON `_id`。
    pub id: String,
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    /// `"active"` | `"deprecated"`。
    pub status: String,
}

/// agent-autonomy-loop W0：`taxonomy_candidates` 集合占位结构。
///
/// 双层标签的"候选"层：Reply Agent 输出但不在 `system_taxonomies` 中的取值落入
/// 此集合，由后台审核后并入正式字典；候选状态 SHALL NOT 阻塞 Reply Agent 运行。
/// 详见 design.md §3.4 与 requirements.md R8.4；W3 task 4.6 / 4.7 / 4.8 落实
/// upsert / approve / reject 业务逻辑。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxonomyCandidate {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub scope: String,
    pub kind: String,
    pub raw_value: String,
    pub evidence: Option<String>,
    #[serde(default)]
    pub confidence: i32,
    pub first_seen_at: DateTime,
    pub last_seen_at: DateTime,
    #[serde(default)]
    pub occurrences: i32,
    /// `"pending"` | `"approved"` | `"rejected"`。
    pub status: String,
    pub reviewed_at: Option<DateTime>,
    pub reviewed_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmCallLog {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: Option<String>,
    pub contact_wxid: Option<String>,
    pub run_id: Option<String>,
    pub prompt_key: String,
    pub model: String,
    pub status: String,
    pub latency_ms: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub prompt_cache_hit_tokens: i64,
    pub prompt_cache_miss_tokens: i64,
    pub error: Option<String>,
    /// HP-4 / Task 11：本次调用前发生的重试次数（0 表示一次成功，max_retries-1 表示走完所有重试才成功/失败）。
    #[serde(default)]
    pub retry_count: i32,
    /// HP-4 / Task 11：调用最终状态。`success | failed | json_error | cache_hit`。
    #[serde(default)]
    pub final_status: Option<String>,
    pub created_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserOperationGuidePreview {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub contact_id: ObjectId,
    pub contact_wxid: String,
    pub instruction: String,
    pub mode: String,
    pub status: String,
    pub summary: String,
    #[serde(default)]
    pub impact_scope: String,
    #[serde(default)]
    pub scope_reason: String,
    #[serde(default)]
    pub readable_changes: Vec<String>,
    #[serde(default)]
    pub health_scores: Document,
    #[serde(default)]
    pub suggested_changes: Document,
    #[serde(default)]
    pub risk_warnings: Vec<String>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagementAgentSession {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub title: String,
    /// S-20 / Task 19：dry-run 模式默认值。`true` 时所有非 read 工具调用
    /// 只回放计划不实际执行；可由单条消息请求覆盖。
    #[serde(default)]
    pub dry_run: bool,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagementAgentMessage {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub session_id: ObjectId,
    pub role: String,
    pub content: String,
    pub created_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCommandRun {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub session_id: ObjectId,
    pub operator_message: String,
    pub status: String,
    pub plan: Option<Document>,
    pub summary: String,
    pub error: Option<String>,
    #[serde(default)]
    pub prompt_versions: Document,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentToolCall {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub command_run_id: ObjectId,
    pub tool_name: String,
    pub arguments: Document,
    pub status: String,
    pub response: Option<Document>,
    pub error: Option<String>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

/// S-19 / Task 17：长 horizon 用户运营 outcome 指标。
///
/// `_id` 形如 `"{account_id}:{horizon}:{date}"`，便于幂等聚合（重跑覆盖同 _id）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentOutcomeMetric {
    #[serde(rename = "_id")]
    pub id: String,
    pub workspace_id: String,
    pub account_id: String,
    pub horizon: String,
    pub date: String,
    /// 波 A2：outbound 总数为 0 时为 `None`（"无数据"），不再静默写 0。
    #[serde(default)]
    pub reply_rate: Option<f64>,
    /// 波 A2：managed contact 数为 0 时为 `None`。
    #[serde(default)]
    pub conversation_depth: Option<f64>,
    /// 波 A2：AI 自暂缓后由 AI 自身澄清恢复继续的比例。当前 events 中尚无
    /// 对应事件源，统一写 `None` 表示"指标暂不可用"，避免前端误判为"零成
    /// 功率"。后续接入事件源后改为实际比例。
    /// 注：旧字段 `human_handoff_success_rate` 已退役（违反全自治产品定位），
    /// 使用 `serde(alias)` 兼容历史 BSON 文档读取，写入用新字段名。
    #[serde(default, alias = "human_handoff_success_rate")]
    pub ai_hold_cleared_rate: Option<f64>,
    /// 波 A2：review 总数为 0 时为 `None`。
    #[serde(default)]
    pub agent_block_rate: Option<f64>,
    #[serde(default)]
    pub daily_run_count: i64,
    #[serde(default)]
    pub daily_run_token_total: i64,
    pub created_at: DateTime,
}

/// S-18 / Task 18：公式遵守度评测场景。
///
/// 用于对比模型 `formula_breakdown` / `review.scores` 与人工标注 ground_truth
/// 的偏差，跟踪四大公式（Trust/ConversionReadiness/EmotionalValue/NextBestActionScore）
/// 的遵守程度。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluationScenario {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub scenario_id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub account_id: Option<String>,
    #[serde(default)]
    pub contact_seed: Document,
    #[serde(default)]
    pub inbound_messages: Vec<String>,
    #[serde(default)]
    pub ground_truth: Document,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "default_evaluation_scenario_status")]
    pub status: String,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

fn default_evaluation_scenario_status() -> String {
    "active".to_string()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnableAgentRequest {
    pub human_profile_note: String,
    pub playbook_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileNoteRequest {
    pub human_profile_note: String,
}

#[derive(Debug, Deserialize)]
pub struct SearchImportRequest {
    pub query: String,
    #[serde(rename = "accountId")]
    pub account_id: Option<String>,
}

/// 波 A3：把 `search` 返回的候选导入本地 contacts 集合。两种入参互斥：
/// - `query`：等价于先 search 再 import（兼容老调用方）。
/// - `candidates`：直接消费前端拿到的候选数组。
#[derive(Debug, Deserialize)]
pub struct ImportContactsRequest {
    /// 沿用旧"搜索-导入"语义，方便客户端一步完成。
    #[serde(default)]
    pub query: Option<String>,
    #[serde(rename = "accountId")]
    pub account_id: Option<String>,
    /// 来自前端 search 的候选项原样数组；每项若含 `.contact` 子对象会被解开。
    #[serde(default)]
    pub candidates: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct ContactQuery {
    pub status: Option<String>,
    pub q: Option<String>,
    #[serde(rename = "accountId")]
    pub account_id: Option<String>,
    pub limit: Option<i64>,
    pub skip: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiCommitment {
    pub id: String,
    pub text: String,
    pub due_at: Option<String>,
    pub created_at: Option<String>,
}

impl From<&CommitmentRepr> for ApiCommitment {
    fn from(repr: &CommitmentRepr) -> Self {
        match repr {
            CommitmentRepr::Plain(text) => Self {
                id: String::new(),
                text: text.clone(),
                due_at: None,
                created_at: None,
            },
            CommitmentRepr::Structured(entry) => Self {
                id: entry.id.clone(),
                text: entry.text.clone(),
                due_at: dt_to_string(entry.due_at.unwrap_or(DateTime::from_millis(0)))
                    .filter(|_| entry.due_at.is_some()),
                created_at: dt_to_string(entry.created_at),
            },
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiContact {
    pub id: String,
    pub workspace_id: String,
    pub account_id: String,
    pub wxid: String,
    pub nickname: Option<String>,
    pub remark: Option<String>,
    pub alias: Option<String>,
    pub agent_status: AgentStatus,
    pub human_profile_note: Option<String>,
    pub agent_profile: Option<AgentProfile>,
    pub memory_summary: Option<String>,
    pub playbook_id: Option<String>,
    pub playbook_version: Option<i32>,
    pub tags: Vec<String>,
    pub customer_stage: Option<String>,
    pub customer_stage_updated_at: Option<String>,
    pub intent_level: Option<String>,
    pub commitments: Vec<ApiCommitment>,
    pub follow_up_policy: Option<String>,
    pub operation_state: Option<String>,
    pub operation_state_reason: Option<String>,
    pub operation_state_confidence: Option<i32>,
    pub operation_state_updated_at: Option<String>,
    pub cooldown_until: Option<String>,
    pub operation_policy: Document,
    pub profile_attributes: Document,
    pub profile_updated_at: Option<String>,
    pub last_message_at: Option<String>,
    pub last_inbound_at: Option<String>,
    pub last_outbound_at: Option<String>,
    pub last_agent_run_at: Option<String>,
    pub updated_at: String,
}

impl From<Contact> for ApiContact {
    fn from(contact: Contact) -> Self {
        Self {
            id: contact.id.map(|id| id.to_hex()).unwrap_or_default(),
            workspace_id: contact.workspace_id,
            account_id: contact.account_id,
            wxid: contact.wxid,
            nickname: contact.nickname,
            remark: contact.remark,
            alias: contact.alias,
            agent_status: contact.agent_status,
            human_profile_note: contact.human_profile_note,
            agent_profile: contact.agent_profile,
            memory_summary: contact.memory_summary,
            playbook_id: contact.playbook_id.map(|id| id.to_hex()),
            playbook_version: contact.playbook_version,
            tags: contact.tags,
            customer_stage: contact.customer_stage,
            customer_stage_updated_at: contact.customer_stage_updated_at.and_then(dt_to_string),
            intent_level: contact.intent_level,
            commitments: contact.commitments.iter().map(ApiCommitment::from).collect(),
            follow_up_policy: contact.follow_up_policy,
            operation_state: contact.operation_state,
            operation_state_reason: contact.operation_state_reason,
            operation_state_confidence: contact.operation_state_confidence,
            operation_state_updated_at: contact.operation_state_updated_at.and_then(dt_to_string),
            cooldown_until: contact.cooldown_until.and_then(dt_to_string),
            operation_policy: contact.operation_policy,
            profile_attributes: contact.profile_attributes,
            profile_updated_at: contact.profile_updated_at.and_then(dt_to_string),
            last_message_at: contact.last_message_at.and_then(dt_to_string),
            last_inbound_at: contact.last_inbound_at.and_then(dt_to_string),
            last_outbound_at: contact.last_outbound_at.and_then(dt_to_string),
            last_agent_run_at: contact.last_agent_run_at.and_then(dt_to_string),
            updated_at: dt_to_string(contact.updated_at).unwrap_or_default(),
        }
    }
}

pub fn dt_to_string(dt: DateTime) -> Option<String> {
    dt.try_to_rfc3339_string().ok()
}

// LP-12 / Task 21：核心 Document 字段的强类型版本。
//
// **设计决策**：本次迭代仅引入 *新增* 强类型 struct 与转换辅助，**不**强制替换
// 既有 Document 调用点（避免触发大面积改动）。后续小迭代里业务代码逐步
// 迁移到强类型上，最终再彻底删除 Document 字段。
//
// 所有 struct 都用 `#[serde(rename_all = "camelCase")]` + `#[serde(default)]`，
// 既保留 wire 格式又能向后兼容老数据缺字段。

mod typed {
    use super::*;
    use mongodb::bson;

    /// LP-12 / Task 21：`OperationDomainConfig.runtime_parameters` 的强类型版本。
    ///
    /// 字段与 [`crate::agent::UserRuntimeParameters`] 通过 doc_i32/doc_i64 读取的
    /// key 完全对齐；运行时仍走 Document，但后台编辑/校验/迁移可以借助这个 struct
    /// 获得 compiler 帮助。
    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct RuntimeParametersTyped {
        #[serde(default = "defaults::recent_message_limit")]
        pub recent_message_limit: i64,
        #[serde(default = "defaults::min_reply_interval_seconds")]
        pub min_reply_interval_seconds: i64,
        #[serde(default = "defaults::max_daily_touches")]
        pub max_daily_touches: i64,
        #[serde(default = "defaults::max_pending_follow_ups")]
        pub max_pending_follow_ups: i64,
        #[serde(default = "defaults::follow_up_expires_hours")]
        pub follow_up_expires_hours: i64,
        #[serde(default = "defaults::cooldown_after_no_reply_hours")]
        pub cooldown_after_no_reply_hours: i64,
        #[serde(default = "defaults::fact_risk_block_at")]
        pub fact_risk_block_at: i32,
        #[serde(default = "defaults::pressure_risk_block_at")]
        pub pressure_risk_block_at: i32,
        #[serde(default = "defaults::human_like_rewrite_below")]
        pub human_like_rewrite_below: i32,
        #[serde(default = "defaults::emotional_value_rewrite_below")]
        pub emotional_value_rewrite_below: i32,
        #[serde(default = "defaults::product_accuracy_block_below")]
        pub product_accuracy_block_below: i32,
        #[serde(default = "defaults::operation_state_confidence_full_review_below")]
        pub operation_state_confidence_full_review_below: i32,
        #[serde(default = "defaults::run_token_budget")]
        pub run_token_budget: i64,
        #[serde(default = "defaults::run_max_llm_calls")]
        pub run_max_llm_calls: i32,
        #[serde(default = "defaults::simulation_token_budget")]
        pub simulation_token_budget: i64,
        /// 波 A1：reaction 路径单 run token 上限。
        #[serde(default = "defaults::reaction_token_budget")]
        pub reaction_token_budget: i64,
        /// 波 A1：reaction 路径单 run 最多 LLM 调用次数。
        #[serde(default = "defaults::reaction_max_llm_calls")]
        pub reaction_max_llm_calls: i32,
        /// agent-autonomy-loop W0 / Task 1.3：是否启用自治协议字段校验路径。
        /// sunset D+14；老 runtime 文档缺该字段时走 `true`（启用）。
        #[serde(default = "defaults::autonomy_protocol_enabled")]
        pub autonomy_protocol_enabled: bool,
        /// agent-autonomy-loop W0 / Task 1.3：知识路由模式。
        /// 取值 `auto_tool_loop`（默认）或 `classic_router`（灰度回退，sunset D+14）。
        #[serde(default = "defaults::knowledge_routing_mode")]
        pub knowledge_routing_mode: String,
        /// agent-autonomy-loop W0 / Task 1.3：`reply_with_tools_loop` 的最大轮数。
        /// 默认 3，loader 中 clamp 到 [1, 5]。
        #[serde(default = "defaults::knowledge_max_tool_loops")]
        pub knowledge_max_tool_loops: i32,
        /// agent-autonomy-loop W0 / Task 1.3：单 run 内 tool call 总次数上限。
        /// 默认 6，loader 中 clamp 到 [1, 16]。
        #[serde(default = "defaults::knowledge_max_tool_calls")]
        pub knowledge_max_tool_calls: i32,
        /// agent-autonomy-loop W0 / Task 1.3：`knowledge.open_slice` 单次入参 K 上限。
        /// 默认 4，loader 中 clamp 到 [1, 16]。
        #[serde(default = "defaults::knowledge_open_slice_max_k")]
        pub knowledge_open_slice_max_k: i32,
        /// agent-autonomy-loop W0 / Task 1.3：`knowledge.search` 默认 top_k。
        /// 默认 8，loader 中 clamp 到 [1, 32]。
        #[serde(default = "defaults::knowledge_search_top_k")]
        pub knowledge_search_top_k: i32,
        /// agent-autonomy-loop W0 / Task 1.3：outbox dispatcher 轮询间隔（秒）。
        /// 默认 5，loader 中 clamp 到 [1, 60]。
        #[serde(default = "defaults::outbox_poll_interval_seconds")]
        pub outbox_poll_interval_seconds: i32,
        /// agent-autonomy-loop W0 / Task 1.3：outbox dispatcher claim lease 时长（秒）。
        /// 默认 60，loader 中 clamp 到 [10, 600]。
        #[serde(default = "defaults::outbox_lease_seconds")]
        pub outbox_lease_seconds: i32,
    }

    impl Default for RuntimeParametersTyped {
        fn default() -> Self {
            Self {
                recent_message_limit: defaults::recent_message_limit(),
                min_reply_interval_seconds: defaults::min_reply_interval_seconds(),
                max_daily_touches: defaults::max_daily_touches(),
                max_pending_follow_ups: defaults::max_pending_follow_ups(),
                follow_up_expires_hours: defaults::follow_up_expires_hours(),
                cooldown_after_no_reply_hours: defaults::cooldown_after_no_reply_hours(),
                fact_risk_block_at: defaults::fact_risk_block_at(),
                pressure_risk_block_at: defaults::pressure_risk_block_at(),
                human_like_rewrite_below: defaults::human_like_rewrite_below(),
                emotional_value_rewrite_below: defaults::emotional_value_rewrite_below(),
                product_accuracy_block_below: defaults::product_accuracy_block_below(),
                operation_state_confidence_full_review_below:
                    defaults::operation_state_confidence_full_review_below(),
                run_token_budget: defaults::run_token_budget(),
                run_max_llm_calls: defaults::run_max_llm_calls(),
                simulation_token_budget: defaults::simulation_token_budget(),
                reaction_token_budget: defaults::reaction_token_budget(),
                reaction_max_llm_calls: defaults::reaction_max_llm_calls(),
                autonomy_protocol_enabled: defaults::autonomy_protocol_enabled(),
                knowledge_routing_mode: defaults::knowledge_routing_mode(),
                knowledge_max_tool_loops: defaults::knowledge_max_tool_loops(),
                knowledge_max_tool_calls: defaults::knowledge_max_tool_calls(),
                knowledge_open_slice_max_k: defaults::knowledge_open_slice_max_k(),
                knowledge_search_top_k: defaults::knowledge_search_top_k(),
                outbox_poll_interval_seconds: defaults::outbox_poll_interval_seconds(),
                outbox_lease_seconds: defaults::outbox_lease_seconds(),
            }
        }
    }

    impl From<RuntimeParametersTyped> for Document {
        fn from(p: RuntimeParametersTyped) -> Self {
            bson::to_document(&p).expect("RuntimeParametersTyped serializable")
        }
    }

    pub mod defaults {
        pub fn recent_message_limit() -> i64 {
            12
        }
        pub fn min_reply_interval_seconds() -> i64 {
            20
        }
        pub fn max_daily_touches() -> i64 {
            3
        }
        pub fn max_pending_follow_ups() -> i64 {
            3
        }
        pub fn follow_up_expires_hours() -> i64 {
            48
        }
        pub fn cooldown_after_no_reply_hours() -> i64 {
            24
        }
        pub fn fact_risk_block_at() -> i32 {
            6
        }
        pub fn pressure_risk_block_at() -> i32 {
            7
        }
        pub fn human_like_rewrite_below() -> i32 {
            6
        }
        pub fn emotional_value_rewrite_below() -> i32 {
            5
        }
        pub fn product_accuracy_block_below() -> i32 {
            7
        }
        pub fn operation_state_confidence_full_review_below() -> i32 {
            4
        }
        pub fn run_token_budget() -> i64 {
            30000
        }
        pub fn run_max_llm_calls() -> i32 {
            6
        }
        pub fn simulation_token_budget() -> i64 {
            60000
        }
        pub fn reaction_token_budget() -> i64 {
            8000
        }
        pub fn reaction_max_llm_calls() -> i32 {
            2
        }
        // ── agent-autonomy-loop W0 / Task 1.3 新增默认值 ──
        pub fn autonomy_protocol_enabled() -> bool {
            true
        }
        pub fn knowledge_routing_mode() -> String {
            "auto_tool_loop".to_string()
        }
        pub fn knowledge_max_tool_loops() -> i32 {
            3
        }
        pub fn knowledge_max_tool_calls() -> i32 {
            6
        }
        pub fn knowledge_open_slice_max_k() -> i32 {
            4
        }
        pub fn knowledge_search_top_k() -> i32 {
            8
        }
        pub fn outbox_poll_interval_seconds() -> i32 {
            5
        }
        pub fn outbox_lease_seconds() -> i32 {
            60
        }
    }

    /// LP-12 / Task 21 / agent-autonomy-loop W5 task 6.1：MemoryCard 的强类型版本。
    ///
    /// 本结构是 [`super::OperatingMemory::memory_card`] 的 wire schema，作为整层
    /// 替换 `Document` 的"边界类型"。设计要点（task 6.1）：
    ///
    /// * `core_facts` / `recent_facts` / `deprecated_facts` 三类事实集合，
    ///   各自有写入侧 cap（6 / 10 / 20，由 `compact_memory_card_typed`
    ///   或 consolidator 在写入前强制）。事实条目类型 [`MemoryFactRepr`] 是
    ///   `#[serde(untagged)]` 包装：当前历史数据形态以 `String`（`Plain`）为主，
    ///   task 6.2 引入完整 [`MemoryFact`] 结构后会通过 `Structured(MemoryFact)`
    ///   分支落地，保证升级后的 round-trip 兼容性。
    /// * `core_profile` / `relationship_state` 仍为 `Document`，承接历史
    ///   "free-form 子文档"形态；这两块的强类型化属于后续优化范畴
    ///   （task 6 之外），本任务保留 Document 以最小化 blast radius。
    /// * `extra: Document` 通过 `#[serde(flatten)]` 兜底，承接所有未在本结构
    ///   显式声明的顶层字段（`preferences / doNotDo / commitments /
    ///   objections / openLoops / recentEpisodeSummary / conflicts /
    ///   source / version / coreFacts 旧字符串数组` 等），以保证：
    ///   1) 历史 BSON 文档反序列化不丢字段；
    ///   2) Document 版 helper（如 `compact_memory_card_with_previous`）
    ///      仍可在 `to_document()` 之后无缝消费整张卡。
    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    #[serde(rename_all = "camelCase")]
    pub struct MemoryCardTyped {
        #[serde(default)]
        pub core_profile: Document,
        #[serde(default)]
        pub relationship_state: Document,
        #[serde(default)]
        pub core_facts: Vec<MemoryFactRepr>,
        #[serde(default)]
        pub recent_facts: Vec<MemoryFactRepr>,
        #[serde(default)]
        pub deprecated_facts: Vec<MemoryFactRepr>,
        /// `#[serde(flatten)]` catch-all：承接所有未在上述字段显式声明的顶层
        /// 字段，避免历史数据丢失（如 `preferences / doNotDo / commitments /
        /// objections / openLoops / recentEpisodeSummary / conflicts /
        /// source / version`），并允许 task 6 之外的字段持续以 free-form
        /// 形态共存。
        #[serde(flatten, default)]
        pub extra: Document,
    }

    /// agent-autonomy-loop W5 task 6.1：事实条目的反序列化容器。
    ///
    /// 当前历史数据中 `coreFacts / recentFacts` 几乎全部是 `Vec<String>`
    /// 形态；W5 task 6.2 会引入完整 [`MemoryFact`] 结构（含 id / text /
    /// confidence / importance / ...）。`#[serde(untagged)]` 让两种形态都能
    /// 反序列化进同一字段，避免现网数据迁移期间被 schema 锁死。
    ///
    /// task 6.1 仅需保证：
    /// * `Plain(String)` 分支可承接老数据；
    /// * `Structured(MemoryFact)` 分支以"占位 shell"（仅 `text` + 可选 `id`）
    ///   形式存在，task 6.2 再补齐字段。
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    #[serde(untagged)]
    pub enum MemoryFactRepr {
        /// 历史 / 简单形态：单个字符串即 fact text。
        Plain(String),
        /// 结构化形态：task 6.2 完整化（id / confidence / importance / ...）。
        /// 当前只保留 `text` 与可选 `id`，作为 forward-compat 占位。
        Structured(MemoryFact),
    }

    impl MemoryFactRepr {
        /// 取出 fact 的文本表示（无论 Plain / Structured 都能工作）。
        pub fn as_text(&self) -> &str {
            match self {
                MemoryFactRepr::Plain(s) => s.as_str(),
                MemoryFactRepr::Structured(fact) => fact.text.as_str(),
            }
        }
    }

    impl From<String> for MemoryFactRepr {
        fn from(s: String) -> Self {
            MemoryFactRepr::Plain(s)
        }
    }

    impl<'a> From<&'a str> for MemoryFactRepr {
        fn from(s: &'a str) -> Self {
            MemoryFactRepr::Plain(s.to_string())
        }
    }

    /// task 6.2：`MemoryFact` 的 `created_at / updated_at` 字段在反序列化期
    /// 缺失时的兜底 default。`bson::DateTime` 没有原生 `Default` 实现，因此
    /// 这里固定回退 Unix epoch；正常写入路径请用 `DateTime::now()`。
    fn default_epoch_dt() -> DateTime {
        DateTime::from_millis(0)
    }

    /// agent-autonomy-loop W5 task 6.2：完整 [`MemoryFact`] 强类型结构。
    ///
    /// 字段对照 design.md §3.5 的契约：
    ///
    /// | 字段                  | 类型                | 约束                                                        |
    /// |-----------------------|---------------------|-------------------------------------------------------------|
    /// | `id`                  | `String`            | UUIDv4，必填，作为 consolidator 合并 / 弃用 / 冲突的稳定锚 |
    /// | `text`                | `String`            | 1..=500 chars                                               |
    /// | `evidence`            | `Option<String>`    | ≤ 1000 chars                                                |
    /// | `confidence`          | `i32`               | 0..=10（Plain → 默认 7）                                    |
    /// | `importance`          | `i32`               | 0..=10（Plain → 默认 5）                                    |
    /// | `may_expire`          | `bool`              | —                                                           |
    /// | `deprecated_at`       | `Option<DateTime>`  | task 6.4 `apply_consolidator_deprecations` 写入             |
    /// | `deprecation_reason`  | `Option<String>`    | ≤ 200 chars                                                 |
    /// | `source_message_ids`  | `Vec<ObjectId>`     | ≤ 5（写入侧 cap）                                           |
    /// | `source_run_id`       | `Option<String>`    | 关联触发本条 fact 的 run_id（用于 trace）                    |
    /// | `created_at`          | `DateTime`          | first-seen 时间戳                                            |
    /// | `updated_at`          | `DateTime`          | 末次写入时间戳                                              |
    ///
    /// **稳定 id（task 6.2 / 6.4）**：所有迁移 / 反序列化 / consolidator
    /// 合并路径 SHALL 通过 `id` 字段（UUIDv4 字符串）锚定 fact，禁止用 `text`
    /// 作为身份 key（同义改写场景下 text 会变化）。Plain 升级路径生成 fresh
    /// UUID（见 [`From<MemoryFactRepr>`]），避免历史 `Vec<String>` 数据被多次
    /// 升级到不同 UUID 而无法匹配。
    ///
    /// **bounds 校验**：[`MemoryFact::validate`] 提供运行时长度 / 范围检查；
    /// task 6.4 的 `apply_consolidator_deprecations` 与 W2 校验链调用前会
    /// 执行该函数，违规 fact 将被 drop + warning。
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    #[serde(rename_all = "camelCase")]
    pub struct MemoryFact {
        /// UUIDv4 字符串。空字符串视为"老数据未升级"，由 task 6.6 迁移补 id；
        /// `From<MemoryFactRepr::Plain>` 路径直接生成 fresh UUID。
        #[serde(default)]
        pub id: String,
        /// fact 文本。1..=500 chars（[`MemoryFact::validate`] 校验）。
        #[serde(default)]
        pub text: String,
        /// 证据 / quote。≤ 1000 chars。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub evidence: Option<String>,
        /// 置信度 0..=10。Plain → 默认 7（design.md §3.5）。
        #[serde(default)]
        pub confidence: i32,
        /// 重要性 0..=10。Plain → 默认 5（design.md §3.5）。
        #[serde(default)]
        pub importance: i32,
        /// 是否易过期（提示 consolidator 优先抽查）。
        #[serde(default)]
        pub may_expire: bool,
        /// 弃用时间。`Some` 表示该 fact 已被移到 `deprecated_facts` 集合。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub deprecated_at: Option<DateTime>,
        /// 弃用理由。≤ 200 chars。仅在 `deprecated_at == Some` 时有意义。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub deprecation_reason: Option<String>,
        /// 触发本条 fact 的入站消息 id 列表。≤ 5（写入侧 cap）。
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub source_message_ids: Vec<ObjectId>,
        /// 触发本条 fact 的 run_id。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub source_run_id: Option<String>,
        /// 创建时间。
        #[serde(default = "default_epoch_dt")]
        pub created_at: DateTime,
        /// 末次更新时间。
        #[serde(default = "default_epoch_dt")]
        pub updated_at: DateTime,
        /// 兜底 `extra` 承接未识别的额外字段（前向兼容老数据 / 未来扩展）。
        #[serde(flatten, default)]
        pub extra: Document,
    }

    impl Default for MemoryFact {
        /// `bson::DateTime` 没有 `Default` 实现，因此 `MemoryFact` 需要手写
        /// `Default`：所有时间戳字段统一回退到 `DateTime::from_millis(0)`
        /// （Unix epoch），其余字段沿用零值 / 空集合。这个 default 仅用于
        /// 极少数"占位"场景（测试 / `Document` 兜底反序列化失败时），生产
        /// 路径请用 [`MemoryFact::from_plain_text`] 直接构造。
        fn default() -> Self {
            Self {
                id: String::new(),
                text: String::new(),
                evidence: None,
                confidence: 0,
                importance: 0,
                may_expire: false,
                deprecated_at: None,
                deprecation_reason: None,
                source_message_ids: Vec::new(),
                source_run_id: None,
                created_at: DateTime::from_millis(0),
                updated_at: DateTime::from_millis(0),
                extra: Document::new(),
            }
        }
    }

    impl MemoryFact {
        /// task 6.2：从 `Plain(text)` 升级时调用的工厂；生成 fresh UUIDv4 + 默认
        /// confidence=7 / importance=5（design.md §3.5）。
        pub fn from_plain_text(text: String) -> Self {
            let now = DateTime::now();
            Self {
                id: uuid::Uuid::new_v4().to_string(),
                text,
                evidence: None,
                confidence: 7,
                importance: 5,
                may_expire: false,
                deprecated_at: None,
                deprecation_reason: None,
                source_message_ids: Vec::new(),
                source_run_id: None,
                created_at: now,
                updated_at: now,
                extra: Document::new(),
            }
        }

        /// task 6.2：bounds 校验。返回违规列表（空表示 OK）；调用方按违规
        /// 严重程度决定 drop fact 或追加 warning。具体 bound 见 struct
        /// doc-comment 表格。
        ///
        /// 字符长度统一按 Unicode `char` 计（与 R1 / R3 文本约束一致）。
        pub fn validate(&self) -> Vec<String> {
            let mut errors = Vec::new();
            let text_len = self.text.chars().count();
            if text_len == 0 {
                errors.push("memory_fact_text_empty".to_string());
            } else if text_len > 500 {
                errors.push(format!("memory_fact_text_over_500:{}", text_len));
            }
            if let Some(ev) = &self.evidence {
                let ev_len = ev.chars().count();
                if ev_len > 1000 {
                    errors.push(format!("memory_fact_evidence_over_1000:{}", ev_len));
                }
            }
            if !(0..=10).contains(&self.confidence) {
                errors.push(format!("memory_fact_confidence_out_of_range:{}", self.confidence));
            }
            if !(0..=10).contains(&self.importance) {
                errors.push(format!("memory_fact_importance_out_of_range:{}", self.importance));
            }
            if let Some(reason) = &self.deprecation_reason {
                let reason_len = reason.chars().count();
                if reason_len > 200 {
                    errors.push(format!(
                        "memory_fact_deprecation_reason_over_200:{}",
                        reason_len
                    ));
                }
            }
            if self.source_message_ids.len() > 5 {
                errors.push(format!(
                    "memory_fact_source_message_ids_over_5:{}",
                    self.source_message_ids.len()
                ));
            }
            errors
        }
    }

    /// task 6.2：`MemoryFactRepr → MemoryFact` 的反序列化兼容转换。
    ///
    /// * `Plain(text)`：生成 **fresh UUIDv4**（关键约束）+ 默认 confidence=7 /
    ///   importance=5 / created_at=updated_at=now。fresh UUID 是为了避免
    ///   老 `Vec<String>` 数据每次升级被分配不同 id 而导致 consolidator 合并失真。
    /// * `Structured(fact)`：透传（id 由迁移 task 6.6 或 consolidator 直接写入）。
    impl From<MemoryFactRepr> for MemoryFact {
        fn from(repr: MemoryFactRepr) -> Self {
            match repr {
                MemoryFactRepr::Plain(text) => MemoryFact::from_plain_text(text),
                MemoryFactRepr::Structured(fact) => fact,
            }
        }
    }

    impl MemoryCardTyped {
        /// 整层 typed 后，Document 版 helper 仍是合法消费者。本方法把 typed
        /// 结构序列化成 BSON `Document`，便于在迁移期把"读路径 typed +
        /// 写路径 Document helper"两边粘合。失败（极端情况下不可序列化）
        /// 时返回空 `Document`，避免 panic 把整个 run 拖垮。
        pub fn to_document(&self) -> Document {
            bson::to_document(self).unwrap_or_default()
        }

        /// 把任意 `Document`（含历史老数据）反序列化为 typed；不可识别字段
        /// 落入 `extra`，整体 round-trip 不丢字段。失败回退 `Default`。
        pub fn from_document(doc: &Document) -> Self {
            bson::from_document::<MemoryCardTyped>(doc.clone()).unwrap_or_default()
        }

        /// 与 `Document::is_empty` 语义对齐：当且仅当所有 typed 字段都空
        /// 且 `extra` 也空时返回 true，便于既有 `if !memory.memory_card.is_empty()`
        /// 调用方零改动迁移。
        pub fn is_empty(&self) -> bool {
            self.core_profile.is_empty()
                && self.relationship_state.is_empty()
                && self.core_facts.is_empty()
                && self.recent_facts.is_empty()
                && self.deprecated_facts.is_empty()
                && self.extra.is_empty()
        }

        /// agent-autonomy-loop W5 / Task 6.7：检查 typed 三组事实数组中是否
        /// 存在 [`MemoryFactRepr::Plain`] 形态。返回 true 表示 caller（多为
        /// consolidator / simulation 种子）传入了老 `Vec<String>` 输入，需要
        /// 触发 `memory_facts_auto_upgraded` 警告并升级为结构化形态。
        pub fn has_plain_facts(&self) -> bool {
            self.core_facts
                .iter()
                .chain(self.recent_facts.iter())
                .chain(self.deprecated_facts.iter())
                .any(|fact| matches!(fact, MemoryFactRepr::Plain(_)))
        }

        /// agent-autonomy-loop W5 / Task 6.7：把 typed 三组事实数组中所有
        /// [`MemoryFactRepr::Plain`] 升级为 [`MemoryFactRepr::Structured`]，
        /// 字段走 [`MemoryFact::from_plain_text`] 默认值（fresh UUIDv4 +
        /// confidence=7 / importance=5）。返回升级条数；调用方据此决定是否
        /// 在审计 / 响应 body 中追加 `memory_facts_auto_upgraded` warning。
        pub fn auto_upgrade_plain_facts(&mut self) -> usize {
            let mut upgraded = 0usize;
            for repr in self
                .core_facts
                .iter_mut()
                .chain(self.recent_facts.iter_mut())
                .chain(self.deprecated_facts.iter_mut())
            {
                if let MemoryFactRepr::Plain(text) = repr {
                    let promoted = MemoryFact::from_plain_text(std::mem::take(text));
                    *repr = MemoryFactRepr::Structured(promoted);
                    upgraded += 1;
                }
            }
            upgraded
        }
    }

    /// agent-autonomy-loop M2：`Contact.commitments` 元素的反序列化容器。
    ///
    /// 与 [`MemoryFactRepr`] 相同的 `#[serde(untagged)]` 兼容套路：历史数据
    /// （`extra.commitments: Vec<String>` 或 M1 期之前 `Contact.last_commitment:
    /// Option<String>` 经迁移成的单元素 Vec）落入 `Plain`，新写入数据落入
    /// `Structured(CommitmentEntry)`，承诺到期扫描器 (`planner::scan_commitments`)
    /// 对前者跳过、对后者按 `due_at` 判定 overdue/imminent。
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    #[serde(untagged)]
    pub enum CommitmentRepr {
        /// 旧字符串形态（无 due_at）。Planner 不会对其 emit follow_up，等下次
        /// memoryCard rewrite 由 Reply Agent 重塑为 `Structured` 形态。
        Plain(String),
        /// 新结构化形态：含稳定 id / due_at / created_at。
        Structured(CommitmentEntry),
    }

    impl CommitmentRepr {
        /// 取出承诺文本，无论 Plain / Structured。
        pub fn text(&self) -> &str {
            match self {
                CommitmentRepr::Plain(s) => s.as_str(),
                CommitmentRepr::Structured(entry) => entry.text.as_str(),
            }
        }

        /// 取 `due_at`；`Plain` 无该信息，固定返回 `None`。
        pub fn due_at(&self) -> Option<DateTime> {
            match self {
                CommitmentRepr::Plain(_) => None,
                CommitmentRepr::Structured(entry) => entry.due_at,
            }
        }

        /// 取 commitment 的稳定 id；`Plain` 无 id，固定返回空串（调用方据此跳过）。
        pub fn id(&self) -> &str {
            match self {
                CommitmentRepr::Plain(_) => "",
                CommitmentRepr::Structured(entry) => entry.id.as_str(),
            }
        }
    }

    impl From<String> for CommitmentRepr {
        fn from(s: String) -> Self {
            CommitmentRepr::Plain(s)
        }
    }

    impl<'a> From<&'a str> for CommitmentRepr {
        fn from(s: &'a str) -> Self {
            CommitmentRepr::Plain(s.to_string())
        }
    }

    /// agent-autonomy-loop M2：结构化承诺条目。
    ///
    /// 字段对照：
    /// - `id`: UUIDv4 字符串，作为 Planner emit 幂等键（`agent_events.details.commitment_id`）。
    /// - `text`: 承诺文本；1..=500 chars（[`CommitmentEntry::validate`] 校验）。
    /// - `due_at`: 截止时间；可选（旧 `last_commitment` 字符串无此信息）。
    /// - `created_at`: 创建时间，迁移时回退到 `Contact.updated_at`。
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    #[serde(rename_all = "camelCase")]
    pub struct CommitmentEntry {
        #[serde(default)]
        pub id: String,
        #[serde(default)]
        pub text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub due_at: Option<DateTime>,
        #[serde(default = "default_epoch_dt")]
        pub created_at: DateTime,
        #[serde(flatten, default)]
        pub extra: Document,
    }

    impl CommitmentEntry {
        /// 从纯文本构造一条新承诺：fresh UUIDv4 + `created_at = now()`。
        pub fn from_plain_text(text: String) -> Self {
            Self {
                id: uuid::Uuid::new_v4().to_string(),
                text,
                due_at: None,
                created_at: DateTime::now(),
                extra: Document::new(),
            }
        }

        /// bounds 校验，返回违规列表（空表示 OK）。
        pub fn validate(&self) -> Vec<String> {
            let mut errors = Vec::new();
            let text_len = self.text.chars().count();
            if text_len == 0 {
                errors.push("commitment_text_empty".to_string());
            } else if text_len > 500 {
                errors.push(format!("commitment_text_over_500:{}", text_len));
            }
            if self.id.is_empty() {
                errors.push("commitment_id_empty".to_string());
            }
            errors
        }
    }

    impl From<CommitmentRepr> for CommitmentEntry {
        fn from(repr: CommitmentRepr) -> Self {
            match repr {
                CommitmentRepr::Plain(text) => CommitmentEntry::from_plain_text(text),
                CommitmentRepr::Structured(entry) => entry,
            }
        }
    }

    /// 波 D1：`OperationDomainConfig.state_machine` 的强类型版本。
    ///
    /// 与运行时 `crate::agent::guards::check_state_transition` 消费的字段对齐。
    /// 字段全部 `#[serde(default)]`，老文档缺字段时走默认值（避免破坏既有数据）。
    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    #[serde(rename_all = "camelCase")]
    pub struct OperationStateMachineTyped {
        #[serde(default)]
        pub states: Vec<OperationStateTyped>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    #[serde(rename_all = "camelCase")]
    pub struct OperationStateTyped {
        #[serde(default)]
        pub key: String,
        #[serde(default)]
        pub name: String,
        #[serde(default)]
        pub goal: String,
        #[serde(default)]
        pub allowed_actions: Vec<String>,
        #[serde(default)]
        pub allowed_from: Vec<String>,
        #[serde(default)]
        pub allow_from_any: bool,
        #[serde(default)]
        pub advance_signals: Vec<String>,
        #[serde(default)]
        pub cooldown_signals: Vec<String>,
        #[serde(default)]
        pub risk_rules: Vec<String>,
        #[serde(default)]
        pub success_criteria: Vec<String>,
    }

    impl From<OperationStateMachineTyped> for Document {
        fn from(machine: OperationStateMachineTyped) -> Self {
            bson::to_document(&machine).expect("OperationStateMachineTyped serializable")
        }
    }
}

pub use typed::{
    CommitmentEntry, CommitmentRepr, MemoryCardTyped, MemoryFact, MemoryFactRepr,
    OperationStateMachineTyped, OperationStateTyped, RuntimeParametersTyped,
};

impl OperationDomainConfig {
    /// LP-12 / Task 21：把 `runtime_parameters` Document 转成强类型；
    /// 缺失字段走 default。出错（极端情况下不可序列化）时返回默认值。
    pub fn runtime_parameters_typed(&self) -> RuntimeParametersTyped {
        let bson = mongodb::bson::Bson::Document(self.runtime_parameters.clone());
        mongodb::bson::from_bson(bson).unwrap_or_default()
    }

    /// 波 D1：把 `state_machine` Document 转成强类型，便于运行时 guard 与
    /// 后台校验复用同一份 schema。缺失/损坏字段走 default。
    pub fn state_machine_typed(&self) -> OperationStateMachineTyped {
        let bson = mongodb::bson::Bson::Document(self.state_machine.clone());
        mongodb::bson::from_bson(bson).unwrap_or_default()
    }
}

// ──────────────────────────────────────────────────────────────────────────
// agent-self-evolution W0 (Task 1.1)：5 个新 collection 的占位结构。
//
// 业务字段在 W2/W3/W4 落地（threshold/prompt 候选、shadow replay、release
// 路径），这里仅给出 BSON 往返所需的最小字段集合，保证 `Database::experiments()`
// 等 typed accessor 编译通过且 `models_smoke` 单测可往返。
// ──────────────────────────────────────────────────────────────────────────

/// agent-self-evolution W0：`experiments` 信封（一次 tick 一条）。
/// Requirements 1.3 / 8.1。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Experiment {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub experiment_id: String,
    pub workspace_id: String,
    pub account_id: String,
    /// `"collecting"` | `"evaluating"` | `"awaiting_admin"` | `"released"` | `"aborted"`。
    pub status: String,
    pub window_hours: i32,
    pub started_at: DateTime,
    pub updated_at: DateTime,
    pub finished_at: Option<DateTime>,
    #[serde(default)]
    pub cohort_threshold_run_ids: Vec<ObjectId>,
    #[serde(default)]
    pub cohort_prompt_run_ids: Vec<ObjectId>,
    #[serde(default)]
    pub budget_used_tokens: i64,
    #[serde(default)]
    pub budget_used_calls: i32,
    #[serde(default)]
    pub proposals_count: i32,
    #[serde(default)]
    pub proposals_eligible_count: i32,
}

/// agent-self-evolution W0：`proposals`（threshold 或 prompt 候选）。
/// Requirements 3.x / 4.x / 5.x / 6.x / 8.1。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proposal {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub experiment_id: String,
    pub workspace_id: String,
    pub account_id: String,
    /// `"threshold"` | `"prompt"`。
    pub proposal_kind: String,
    /// `"pending_eval"` | `"evaluating"` | `"eligible_for_release"`
    /// | `"rejected_below_threshold"` | `"released"` | `"rolled_back"`。
    pub status: String,
    /// threshold 类专用（如 `"fact_risk_block"` / `"planner_block_rate_threshold"`）。
    pub gate_key: Option<String>,
    /// threshold 类：当前生效值（写入时记快照）。
    pub current_value: Option<f64>,
    /// threshold 类：候选值。
    pub proposed_value: Option<f64>,
    /// threshold 候选生成的命中率统计 / cohort 备注（自由 Document）。
    #[serde(default)]
    pub cohort_notes: Document,
    /// prompt 类：模板 key（如 `"reply_agent_main"`）。
    pub proposed_template_key: Option<String>,
    /// prompt 类：要修订的 section（`"soul" | "system_contract" | "policy" | "operator_instruction"`）。
    pub proposed_section: Option<String>,
    /// prompt 类：Critic LLM 输出的 diff 摘要 / 摘要文本。
    pub diff_summary: Option<String>,
    pub diff_snippet: Option<String>,
    pub critic_reasoning: Option<String>,
    #[serde(default)]
    pub expected_improvement_on: Vec<String>,
    pub risk_note: Option<String>,
    /// release 路径回滚专用：被替换的旧 prompt 版本号（rollback 取回它）。
    pub previous_prompt_version: Option<String>,
    /// 显著性测试结果（design.md §4.6）。
    #[serde(default)]
    pub eval_metrics: Document,
    #[serde(default)]
    pub eval_replays_completed: i32,
    #[serde(default)]
    pub eval_replays_failed: i32,
    pub significance_passed: Option<bool>,
    pub failure_reason: Option<String>,
    pub released_at: Option<DateTime>,
    pub released_by: Option<String>,
    pub rolled_back_at: Option<DateTime>,
    pub rolled_back_by: Option<String>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

/// agent-self-evolution W0：`shadow_replays`（每条 proposal × source_run_id 一条）。
/// Requirements 5.x / 8.1。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowReplay {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub proposal_id: ObjectId,
    pub experiment_id: String,
    pub workspace_id: String,
    pub account_id: String,
    pub source_run_id: ObjectId,
    /// `"completed"` | `"failed"`。
    pub status: String,
    pub failure_reason: Option<String>,
    pub original_final_review_status: Option<String>,
    pub new_final_review_status: Option<String>,
    #[serde(default)]
    pub new_review_risks: Vec<String>,
    pub new_token_cost: Option<i64>,
    /// 5 闸命中布尔向量（fact_risk / pressure_risk / human_like / emotional / product_accuracy / planner_block_rate）。
    #[serde(default)]
    pub new_5gate_hit: Document,
    pub new_self_critique_addressed: Option<bool>,
    /// 与原 reply 的近似度（0.0~1.0）；W3 task 4.1 写 0.0 占位。
    #[serde(default)]
    pub similarity_to_original_text: f64,
    pub started_at: DateTime,
    pub finished_at: Option<DateTime>,
}

/// agent-self-evolution W0：`threshold_overrides`（按 gate_key 维度的发布覆盖层）。
/// Requirements 6.x / 8.1。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThresholdOverride {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub gate_key: String,
    pub value: f64,
    pub source_proposal_id: ObjectId,
    pub released_at: DateTime,
    pub released_by: String,
    pub rolled_back_at: Option<DateTime>,
    pub rolled_back_by: Option<String>,
}

/// agent-self-evolution W0：`post_release_reviews`（W4 Task 5.6 +24h 对比窗口评测）。
/// Requirements 9.7。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostReleaseReview {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub proposal_id: ObjectId,
    pub workspace_id: String,
    pub account_id: String,
    /// `"threshold"` | `"prompt"`。
    pub proposal_kind: String,
    pub released_at: DateTime,
    pub scheduled_at: DateTime,
    #[serde(default)]
    pub completed: bool,
    pub actual_send_success_rate_delta: Option<f64>,
    #[serde(default)]
    pub actual_5gate_hit_delta: Document,
    pub completed_at: Option<DateTime>,
}

#[cfg(test)]
mod typed_tests {
    use super::*;
    use mongodb::bson::doc;

    #[test]
    fn runtime_parameters_typed_defaults_apply_when_missing() {
        let p: RuntimeParametersTyped =
            mongodb::bson::from_document(doc! {}).expect("default deserialize");
        assert_eq!(p.recent_message_limit, 12);
        assert_eq!(p.fact_risk_block_at, 6);
        assert_eq!(p.run_token_budget, 30000);
        assert_eq!(p.run_max_llm_calls, 6);
    }

    #[test]
    fn runtime_parameters_typed_reads_existing_values() {
        let doc = doc! {
            "recentMessageLimit": 24,
            "factRiskBlockAt": 8,
            "runTokenBudget": 50000_i64
        };
        let p: RuntimeParametersTyped = mongodb::bson::from_document(doc).expect("deserialize");
        assert_eq!(p.recent_message_limit, 24);
        assert_eq!(p.fact_risk_block_at, 8);
        assert_eq!(p.run_token_budget, 50000);
        // 其它字段 fallback 默认值。
        assert_eq!(p.pressure_risk_block_at, 7);
    }

    #[test]
    fn runtime_parameters_typed_into_document_roundtrip() {
        let p = RuntimeParametersTyped::default();
        let doc: Document = p.into();
        assert_eq!(doc.get_i64("recentMessageLimit").unwrap(), 12);
        assert_eq!(doc.get_i32("factRiskBlockAt").unwrap(), 6);
    }

    #[test]
    fn memory_card_typed_default_is_empty_document_relationship() {
        // task 6.1：relationship_state 现在是 Document（非 typed 子结构），
        // default 应是空 Document。任何"unknown" 默认值由写入侧（如
        // `memory_card_from_contact`）显式注入。
        let card: MemoryCardTyped = mongodb::bson::from_document(doc! {}).expect("default");
        assert!(card.relationship_state.is_empty());
        assert!(card.core_profile.is_empty());
        assert!(card.core_facts.is_empty());
        assert!(card.recent_facts.is_empty());
        assert!(card.deprecated_facts.is_empty());
        assert!(card.extra.is_empty());
    }

    #[test]
    fn memory_card_typed_round_trips_legacy_string_facts() {
        // task 6.1：历史 `Vec<String>` 形态的 coreFacts/recentFacts 必须能
        // 反序列化进 typed.core_facts / recent_facts（走 MemoryFactRepr::Plain
        // 分支），写回 Document 后字段顺序与原值一致。
        let legacy = doc! {
            "coreFacts": ["fact_a", "fact_b"],
            "recentFacts": ["recent_1"],
            "preferences": ["likes 中文"],
            "version": 3_i32,
        };
        let card: MemoryCardTyped =
            mongodb::bson::from_document(legacy.clone()).expect("legacy deserializes");
        assert_eq!(card.core_facts.len(), 2);
        assert_eq!(card.core_facts[0].as_text(), "fact_a");
        assert_eq!(card.recent_facts[0].as_text(), "recent_1");
        // preferences / version 落入 extra 兜底字段。
        assert!(card.extra.contains_key("preferences"));
        assert_eq!(card.extra.get_i32("version").unwrap(), 3);
        let round_tripped = card.to_document();
        assert_eq!(
            round_tripped.get_array("coreFacts").unwrap().len(),
            2,
            "core_facts round-trip 不丢条目"
        );
    }

    /// task 6.2：`MemoryFact::from_plain_text` 给老 `Vec<String>` 元素
    /// 升级时 SHALL 生成 fresh UUIDv4 + confidence=7 + importance=5。
    /// fresh UUID 是关键：避免同一 Plain 文本两次升级被分配同一 id 而
    /// 在 consolidator 合并时被错误判定为"未变 fact"。
    #[test]
    fn memory_fact_from_plain_text_uses_fresh_uuid_and_default_scores() {
        let f = MemoryFact::from_plain_text("用户偏好微信沟通".to_string());
        assert_eq!(f.text, "用户偏好微信沟通");
        assert_eq!(f.confidence, 7, "design.md §3.5：Plain 默认 confidence=7");
        assert_eq!(f.importance, 5, "design.md §3.5：Plain 默认 importance=5");
        assert!(!f.may_expire);
        assert!(f.evidence.is_none());
        assert!(f.deprecated_at.is_none());
        assert!(f.deprecation_reason.is_none());
        assert!(f.source_message_ids.is_empty());
        assert!(f.source_run_id.is_none());
        // UUID 形态：恰好 36 chars，含 4 个 '-'，且能被 uuid::Uuid::parse_str 接受。
        assert_eq!(f.id.len(), 36, "UUIDv4 字符串长度恒为 36");
        assert_eq!(f.id.matches('-').count(), 4);
        let _ = uuid::Uuid::parse_str(&f.id).expect("id 必须是合法 UUID");

        // fresh：再升级一次同样文本应得到不同 UUID。
        let f2 = MemoryFact::from_plain_text("用户偏好微信沟通".to_string());
        assert_ne!(f.id, f2.id, "同文本两次升级 SHALL 生成不同 UUID");
    }

    /// task 6.2：`From<MemoryFactRepr>` 桥接两种反序列化形态：
    /// `Plain` → fresh-UUID shell，`Structured` → 原样透传。
    #[test]
    fn memory_fact_repr_into_memory_fact_bridges_plain_and_structured() {
        let plain = MemoryFactRepr::Plain("简短事实".to_string());
        let upgraded: MemoryFact = plain.into();
        assert_eq!(upgraded.text, "简短事实");
        assert_eq!(upgraded.confidence, 7);
        assert!(!upgraded.id.is_empty());

        // Structured 透传：同 id / 同 confidence / 同 importance 不被覆盖。
        let preset_id = uuid::Uuid::new_v4().to_string();
        let original = MemoryFact {
            id: preset_id.clone(),
            text: "已结构化的事实".to_string(),
            confidence: 9,
            importance: 8,
            ..MemoryFact::default()
        };
        let structured = MemoryFactRepr::Structured(original.clone());
        let passthrough: MemoryFact = structured.into();
        assert_eq!(passthrough.id, preset_id);
        assert_eq!(passthrough.confidence, 9);
        assert_eq!(passthrough.importance, 8);
        assert_eq!(passthrough.text, "已结构化的事实");
    }

    /// task 6.2：`MemoryFact::validate` 长度 / 范围校验。
    #[test]
    fn memory_fact_validate_enforces_bounds() {
        // 合法 fact：无违规。
        let ok = MemoryFact::from_plain_text("ok".to_string());
        assert!(ok.validate().is_empty());

        // text 空：违规。
        let empty_text = MemoryFact {
            text: String::new(),
            ..MemoryFact::from_plain_text("placeholder".to_string())
        };
        assert!(empty_text
            .validate()
            .iter()
            .any(|e| e == "memory_fact_text_empty"));

        // text 超 500：违规。
        let too_long = MemoryFact::from_plain_text("a".repeat(501));
        assert!(too_long
            .validate()
            .iter()
            .any(|e| e.starts_with("memory_fact_text_over_500:")));

        // confidence 超 10：违规。
        let bad_conf = MemoryFact {
            confidence: 11,
            ..MemoryFact::from_plain_text("ok".to_string())
        };
        assert!(bad_conf
            .validate()
            .iter()
            .any(|e| e.starts_with("memory_fact_confidence_out_of_range:")));

        // importance 负值：违规。
        let bad_imp = MemoryFact {
            importance: -1,
            ..MemoryFact::from_plain_text("ok".to_string())
        };
        assert!(bad_imp
            .validate()
            .iter()
            .any(|e| e.starts_with("memory_fact_importance_out_of_range:")));

        // evidence 超 1000 / deprecation_reason 超 200 / source_message_ids 超 5。
        let bad_evidence = MemoryFact {
            evidence: Some("e".repeat(1001)),
            ..MemoryFact::from_plain_text("ok".to_string())
        };
        assert!(bad_evidence
            .validate()
            .iter()
            .any(|e| e.starts_with("memory_fact_evidence_over_1000:")));

        let bad_reason = MemoryFact {
            deprecation_reason: Some("r".repeat(201)),
            ..MemoryFact::from_plain_text("ok".to_string())
        };
        assert!(bad_reason
            .validate()
            .iter()
            .any(|e| e.starts_with("memory_fact_deprecation_reason_over_200:")));

        let too_many_sources = MemoryFact {
            source_message_ids: (0..6).map(|_| ObjectId::new()).collect(),
            ..MemoryFact::from_plain_text("ok".to_string())
        };
        assert!(too_many_sources
            .validate()
            .iter()
            .any(|e| e.starts_with("memory_fact_source_message_ids_over_5:")));
    }

    /// task 6.2：`MemoryFact` 完整结构 BSON round-trip 不丢字段；
    /// 含 evidence / confidence / importance / deprecated_at / source_*。
    #[test]
    fn memory_fact_bson_round_trip_preserves_all_fields() {
        let fact = MemoryFact {
            id: uuid::Uuid::new_v4().to_string(),
            text: "用户期望本周末签约".to_string(),
            evidence: Some("用户原话：周六签".to_string()),
            confidence: 9,
            importance: 8,
            may_expire: true,
            deprecated_at: None,
            deprecation_reason: None,
            source_message_ids: vec![ObjectId::new(), ObjectId::new()],
            source_run_id: Some("run-abc".to_string()),
            created_at: DateTime::from_millis(1_700_000_000_000),
            updated_at: DateTime::from_millis(1_700_000_010_000),
            extra: Document::new(),
        };
        let doc = mongodb::bson::to_document(&fact).expect("serialize");
        // camelCase wire shape：sourceMessageIds / sourceRunId / mayExpire / createdAt / updatedAt
        assert_eq!(doc.get_str("text").unwrap(), "用户期望本周末签约");
        assert_eq!(doc.get_str("evidence").unwrap(), "用户原话：周六签");
        assert_eq!(doc.get_i32("confidence").unwrap(), 9);
        assert_eq!(doc.get_i32("importance").unwrap(), 8);
        assert!(doc.get_bool("mayExpire").unwrap());
        assert_eq!(doc.get_array("sourceMessageIds").unwrap().len(), 2);
        assert_eq!(doc.get_str("sourceRunId").unwrap(), "run-abc");
        assert!(doc.contains_key("createdAt"));
        assert!(doc.contains_key("updatedAt"));
        // 反序列化回来全字段一致。
        let parsed: MemoryFact = mongodb::bson::from_document(doc).expect("deserialize");
        assert_eq!(parsed, fact);
    }

    /// task 6.2：`MemoryCardTyped` 中 `Vec<MemoryFactRepr>` 既能承接老
    /// `Vec<String>`，也能承接新结构化形态；untagged enum 在两种 wire shape
    /// 下都不丢字段。
    #[test]
    fn memory_card_typed_accepts_mixed_plain_and_structured_facts() {
        let mixed = doc! {
            "coreFacts": [
                "纯字符串老 fact",
                {
                    "id": "11111111-2222-3333-4444-555555555555",
                    "text": "结构化事实",
                    "confidence": 9_i32,
                    "importance": 7_i32,
                    "mayExpire": false,
                }
            ]
        };
        let card: MemoryCardTyped =
            mongodb::bson::from_document(mixed).expect("mixed coreFacts deserializes");
        assert_eq!(card.core_facts.len(), 2);
        // Plain 分支：as_text 取出原字符串，但 id 仍然空（迁移 task 6.6 会补）。
        match &card.core_facts[0] {
            MemoryFactRepr::Plain(s) => assert_eq!(s, "纯字符串老 fact"),
            MemoryFactRepr::Structured(_) => panic!("first fact 应走 Plain 分支"),
        }
        // Structured 分支：id / confidence / importance 完整保留。
        match &card.core_facts[1] {
            MemoryFactRepr::Structured(f) => {
                assert_eq!(f.id, "11111111-2222-3333-4444-555555555555");
                assert_eq!(f.text, "结构化事实");
                assert_eq!(f.confidence, 9);
                assert_eq!(f.importance, 7);
            }
            MemoryFactRepr::Plain(_) => panic!("second fact 应走 Structured 分支"),
        }
    }

    /// agent-autonomy-loop W5 / Task 6.7：`auto_upgrade_plain_facts` 把所有
    /// Plain 形态升级为 Structured，并返回升级条数；已经是 Structured 的不计。
    #[test]
    fn auto_upgrade_plain_facts_promotes_only_plain_branches() {
        let mut card: MemoryCardTyped = mongodb::bson::from_document(doc! {
            "coreFacts": [
                "纯字符串 A",
                {
                    "id": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
                    "text": "已结构化",
                    "confidence": 8_i32,
                    "importance": 6_i32,
                    "mayExpire": false,
                }
            ],
            "recentFacts": ["纯字符串 R1", "纯字符串 R2"],
        })
        .expect("mixed deserialize");

        assert!(card.has_plain_facts(), "升级前应有 Plain 形态");
        let upgraded = card.auto_upgrade_plain_facts();
        assert_eq!(upgraded, 3, "三个 Plain 应当被全部升级");
        assert!(!card.has_plain_facts(), "升级后不应残留 Plain");

        // 升级后 Plain 入口的字段获得默认 confidence=7 / importance=5 + fresh UUID。
        match &card.core_facts[0] {
            MemoryFactRepr::Structured(f) => {
                assert_eq!(f.text, "纯字符串 A");
                assert_eq!(f.confidence, 7);
                assert_eq!(f.importance, 5);
                assert!(!f.id.is_empty(), "Plain 升级须分配 fresh UUIDv4");
            }
            MemoryFactRepr::Plain(_) => panic!("应已升级为 Structured"),
        }
        // 已经 Structured 的条目原样保留：id / confidence / importance 不变。
        match &card.core_facts[1] {
            MemoryFactRepr::Structured(f) => {
                assert_eq!(f.id, "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee");
                assert_eq!(f.confidence, 8);
                assert_eq!(f.importance, 6);
            }
            MemoryFactRepr::Plain(_) => panic!("Structured 应原样保留"),
        }
        // 二次调用应返回 0（幂等）。
        assert_eq!(card.auto_upgrade_plain_facts(), 0, "幂等：第二次升级应当 0 条");
    }

    #[test]
    fn operation_domain_config_runtime_parameters_typed_helper() {
        let cfg = OperationDomainConfig {
            id: None,
            workspace_id: "default".to_string(),
            domain: "user_operations".to_string(),
            name: "test".to_string(),
            goal: String::new(),
            methodology: String::new(),
            workflow: String::new(),
            tool_policy: String::new(),
            automation_policy: String::new(),
            review_policy: String::new(),
            runtime_parameters: doc! {
                "recentMessageLimit": 16,
                "runMaxLlmCalls": 4
            },
            state_machine: Document::new(),
            status: "active".to_string(),
            updated_at: DateTime::now(),
        };
        let typed = cfg.runtime_parameters_typed();
        assert_eq!(typed.recent_message_limit, 16);
        assert_eq!(typed.run_max_llm_calls, 4);
    }

    /// 波 A2：AgentOutcomeMetric 在缺数据时字段是 None；前端能区分"暂无数据"
    /// 和"零成功率"。
    #[test]
    fn agent_outcome_metric_round_trips_null_fields() {
        let metric = AgentOutcomeMetric {
            id: "default:default:7d:2026-05-18".to_string(),
            workspace_id: "default".to_string(),
            account_id: "default".to_string(),
            horizon: "7d".to_string(),
            date: "2026-05-18".to_string(),
            reply_rate: None,
            conversation_depth: None,
            ai_hold_cleared_rate: None,
            agent_block_rate: None,
            daily_run_count: 0,
            daily_run_token_total: 0,
            created_at: DateTime::now(),
        };
        // BSON round-trip 保持 None。
        let doc = mongodb::bson::to_document(&metric).expect("serialize metric");
        // 字段使用 snake_case（与原始 struct 字段保持一致；前端 JSON 由
        // `outcome_metric_json` 单独 camelCase 化）。
        assert!(matches!(
            doc.get("reply_rate"),
            Some(mongodb::bson::Bson::Null)
        ));
        assert!(matches!(
            doc.get("ai_hold_cleared_rate"),
            Some(mongodb::bson::Bson::Null)
        ));
        let parsed: AgentOutcomeMetric =
            mongodb::bson::from_document(doc).expect("deserialize metric");
        assert!(parsed.reply_rate.is_none());
        assert!(parsed.ai_hold_cleared_rate.is_none());
        assert!(parsed.agent_block_rate.is_none());
        assert!(parsed.conversation_depth.is_none());
    }

    /// 波 A2：旧 BSON 文档（`replyRate: 0.0` 与 `human_handoff_success_rate`
    /// 字段名）反序列化保留为 `Some(0.0)`，验证 `serde(alias)` 兼容性。
    #[test]
    fn agent_outcome_metric_reads_legacy_zero_value() {
        let legacy = doc! {
            "_id": "default:default:7d:2026-05-17",
            "workspace_id": "default",
            "account_id": "default",
            "horizon": "7d",
            "date": "2026-05-17",
            "reply_rate": 0.0,
            "conversation_depth": 0.0,
            "human_handoff_success_rate": 0.0,
            "agent_block_rate": 0.0,
            "daily_run_count": 0_i64,
            "daily_run_token_total": 0_i64,
            "created_at": DateTime::now(),
        };
        let parsed: AgentOutcomeMetric =
            mongodb::bson::from_document(legacy).expect("deserialize legacy");
        assert_eq!(parsed.reply_rate, Some(0.0));
        assert_eq!(parsed.ai_hold_cleared_rate, Some(0.0));
    }

    /// 波 D1：`OperationStateMachineTyped` 能从默认状态机正确反序列化 +
    /// `state_machine_typed` 辅助方法可用。
    #[test]
    fn state_machine_typed_round_trips_default() {
        use crate::prompts::default_user_operation_state_machine;
        let cfg = OperationDomainConfig {
            id: None,
            workspace_id: "default".to_string(),
            domain: "user_operations".to_string(),
            name: "x".to_string(),
            goal: String::new(),
            methodology: String::new(),
            workflow: String::new(),
            tool_policy: String::new(),
            automation_policy: String::new(),
            review_policy: String::new(),
            runtime_parameters: Document::new(),
            state_machine: default_user_operation_state_machine(),
            status: "active".to_string(),
            updated_at: DateTime::now(),
        };
        let typed = cfg.state_machine_typed();
        assert!(!typed.states.is_empty());
        // cooldown state 必须 allowFromAny=true。
        let cooldown = typed
            .states
            .iter()
            .find(|s| s.key == "cooldown")
            .expect("cooldown state");
        assert!(cooldown.allow_from_any);
        // new_contact 在 allowedFrom 里包含自身（自循环显式写入）。
        let new_contact = typed
            .states
            .iter()
            .find(|s| s.key == "new_contact")
            .expect("new_contact state");
        assert!(new_contact.allowed_from.iter().any(|s| s == "new_contact"));
    }

    // ── agent-autonomy-loop W0 (Task 1.6) ──
    //
    // 为 W0 新增的三个 collection 占位 struct（`OutboxEntry / TaxonomyEntry /
    // TaxonomyCandidate`）写最小 BSON 往返 smoke 单元测试。仅验证序列化/反序列化
    // 不丢字段，不依赖 MongoDB 连接。
    //
    // NOTE: `ensure_indexes()` 的真实启动期返回 OK 路径（Requirements 8.1 / 13.1）
    // 由 W0 集成测试在 `tests/` 中通过 testcontainers 覆盖（需要运行中的 Mongo），
    // 此处不在 lib unit test 范围内。

    /// W0 / Task 1.6：`OutboxEntry` BSON 往返保字段不丢（Requirements 13.1）。
    #[test]
    fn outbox_entry_bson_round_trip() {
        let now = DateTime::now();
        let decision_id = ObjectId::new();
        let entry = OutboxEntry {
            id: None,
            workspace_id: "default".to_string(),
            account_id: "acct-1".to_string(),
            contact_wxid: "wxid_test_001".to_string(),
            run_id: "run-uuid-001".to_string(),
            decision_id: Some(decision_id),
            source_event_id: "evt-source-001".to_string(),
            source_kind: "inbound_message".to_string(),
            content: "你好，欢迎咨询".to_string(),
            content_hash: "sha256:abcdef".to_string(),
            idempotency_key: "evt-source-001:wxid_test_001:sha256:abcdef".to_string(),
            attempt: 0,
            max_attempts: 3,
            status: "pending".to_string(),
            cancel_reason: None,
            last_error: None,
            next_retry_at: None,
            worker_id: None,
            locked_until: None,
            created_at: now,
            updated_at: now,
            sent_at: None,
        };

        let doc = mongodb::bson::to_document(&entry).expect("serialize OutboxEntry");
        // `_id` 在 None 时被 skip_serializing_if 忽略。
        assert!(!doc.contains_key("_id"));
        // 关键字段透传不丢。
        assert_eq!(doc.get_str("workspace_id").unwrap(), "default");
        assert_eq!(doc.get_str("account_id").unwrap(), "acct-1");
        assert_eq!(doc.get_str("contact_wxid").unwrap(), "wxid_test_001");
        assert_eq!(doc.get_str("run_id").unwrap(), "run-uuid-001");
        assert_eq!(
            doc.get_str("idempotency_key").unwrap(),
            "evt-source-001:wxid_test_001:sha256:abcdef"
        );
        assert_eq!(doc.get_str("status").unwrap(), "pending");

        let parsed: OutboxEntry =
            mongodb::bson::from_document(doc).expect("deserialize OutboxEntry");
        assert_eq!(parsed.workspace_id, entry.workspace_id);
        assert_eq!(parsed.account_id, entry.account_id);
        assert_eq!(parsed.contact_wxid, entry.contact_wxid);
        assert_eq!(parsed.run_id, entry.run_id);
        assert_eq!(parsed.decision_id, Some(decision_id));
        assert_eq!(parsed.source_event_id, entry.source_event_id);
        assert_eq!(parsed.source_kind, entry.source_kind);
        assert_eq!(parsed.content, entry.content);
        assert_eq!(parsed.idempotency_key, entry.idempotency_key);
        assert_eq!(parsed.status, "pending");
        assert_eq!(parsed.attempt, 0);
        assert_eq!(parsed.max_attempts, 3);
        assert!(parsed.cancel_reason.is_none());
        assert!(parsed.sent_at.is_none());
    }

    /// W0 / Task 1.6：`TaxonomyEntry` BSON 往返；同时验证 `TaxonomyValue` 的
    /// camelCase rename（`display_name` → `displayName`），保证 `(scope, kind,
    /// value.id)` 唯一索引路径与序列化字段一致（Requirements 8.1）。
    #[test]
    fn taxonomy_entry_bson_round_trip() {
        let now = DateTime::now();
        let entry = TaxonomyEntry {
            id: None,
            scope: "global".to_string(),
            kind: "customer_stage".to_string(),
            value: TaxonomyValue {
                id: "first_contact".to_string(),
                display_name: "首次接触".to_string(),
                description: "首次建立联系，尚未深入沟通".to_string(),
                aliases: vec!["新客".to_string(), "first-contact".to_string()],
                status: "active".to_string(),
            },
            updated_at: now,
        };

        let doc = mongodb::bson::to_document(&entry).expect("serialize TaxonomyEntry");
        assert_eq!(doc.get_str("scope").unwrap(), "global");
        assert_eq!(doc.get_str("kind").unwrap(), "customer_stage");
        // value 是嵌套 Document；其 `id` / `displayName` 字段正确地以 camelCase
        // 写出（`displayName` 而非 `display_name`，与 R8.1 索引路径一致）。
        let value_doc = doc.get_document("value").expect("value document");
        assert_eq!(value_doc.get_str("id").unwrap(), "first_contact");
        assert_eq!(value_doc.get_str("displayName").unwrap(), "首次接触");
        // 反向：snake_case 字段不应出现。
        assert!(!value_doc.contains_key("display_name"));
        // 反向：唯一索引路径用的是 `value.id`，绝不是 `value.value_id`。
        assert!(!value_doc.contains_key("value_id"));
        let aliases = value_doc.get_array("aliases").expect("aliases array");
        assert_eq!(aliases.len(), 2);
        assert_eq!(value_doc.get_str("status").unwrap(), "active");

        let parsed: TaxonomyEntry =
            mongodb::bson::from_document(doc).expect("deserialize TaxonomyEntry");
        assert_eq!(parsed.scope, "global");
        assert_eq!(parsed.kind, "customer_stage");
        assert_eq!(parsed.value.id, "first_contact");
        assert_eq!(parsed.value.display_name, "首次接触");
        assert_eq!(parsed.value.description, "首次建立联系，尚未深入沟通");
        assert_eq!(parsed.value.aliases.len(), 2);
        assert_eq!(parsed.value.aliases[0], "新客");
        assert_eq!(parsed.value.status, "active");
    }

    /// W0 / Task 1.6：`TaxonomyCandidate` BSON 往返保字段不丢（Requirements 8.1）。
    #[test]
    fn taxonomy_candidate_bson_round_trip() {
        let now = DateTime::now();
        let candidate = TaxonomyCandidate {
            id: None,
            scope: "default".to_string(),
            kind: "objection_type".to_string(),
            raw_value: "价格敏感_新词".to_string(),
            evidence: Some("用户多次询问折扣并对比同类产品".to_string()),
            confidence: 7,
            first_seen_at: now,
            last_seen_at: now,
            occurrences: 1,
            status: "pending".to_string(),
            reviewed_at: None,
            reviewed_by: None,
        };

        let doc = mongodb::bson::to_document(&candidate).expect("serialize TaxonomyCandidate");
        assert_eq!(doc.get_str("scope").unwrap(), "default");
        assert_eq!(doc.get_str("kind").unwrap(), "objection_type");
        assert_eq!(doc.get_str("raw_value").unwrap(), "价格敏感_新词");
        assert_eq!(doc.get_str("status").unwrap(), "pending");
        assert_eq!(doc.get_i32("confidence").unwrap(), 7);
        assert_eq!(doc.get_i32("occurrences").unwrap(), 1);

        let parsed: TaxonomyCandidate =
            mongodb::bson::from_document(doc).expect("deserialize TaxonomyCandidate");
        assert_eq!(parsed.scope, candidate.scope);
        assert_eq!(parsed.kind, candidate.kind);
        assert_eq!(parsed.raw_value, candidate.raw_value);
        assert_eq!(
            parsed.evidence.as_deref(),
            Some("用户多次询问折扣并对比同类产品")
        );
        assert_eq!(parsed.confidence, 7);
        assert_eq!(parsed.occurrences, 1);
        assert_eq!(parsed.status, "pending");
        assert!(parsed.reviewed_at.is_none());
        assert!(parsed.reviewed_by.is_none());
    }

    // ── agent-self-evolution W0 (Task 1.6) ──
    //
    // 5 个新 collection 的 BSON 往返 smoke：保证字段命名 / Option / 嵌套 Document
    // 在序列化后能还原；W2/W3/W4 落地业务字段后再补显著性 / release 路径专项测试。

    #[test]
    fn experiment_bson_round_trip() {
        let now = mongodb::bson::DateTime::now();
        let exp = Experiment {
            id: None,
            experiment_id: "exp_2026_05_001".to_string(),
            workspace_id: "default".to_string(),
            account_id: "default".to_string(),
            status: "collecting".to_string(),
            window_hours: 72,
            started_at: now,
            updated_at: now,
            finished_at: None,
            cohort_threshold_run_ids: vec![mongodb::bson::oid::ObjectId::new()],
            cohort_prompt_run_ids: vec![],
            budget_used_tokens: 0,
            budget_used_calls: 0,
            proposals_count: 0,
            proposals_eligible_count: 0,
        };
        let doc = mongodb::bson::to_document(&exp).expect("serialize Experiment");
        assert_eq!(doc.get_str("experiment_id").unwrap(), "exp_2026_05_001");
        assert_eq!(doc.get_str("status").unwrap(), "collecting");
        assert_eq!(doc.get_i32("window_hours").unwrap(), 72);
        let parsed: Experiment =
            mongodb::bson::from_document(doc).expect("deserialize Experiment");
        assert_eq!(parsed.experiment_id, exp.experiment_id);
        assert_eq!(parsed.cohort_threshold_run_ids.len(), 1);
        assert!(parsed.cohort_prompt_run_ids.is_empty());
        assert!(parsed.finished_at.is_none());
    }

    #[test]
    fn proposal_bson_round_trip_threshold_kind() {
        let now = mongodb::bson::DateTime::now();
        let p = Proposal {
            id: None,
            experiment_id: "exp_001".to_string(),
            workspace_id: "default".to_string(),
            account_id: "default".to_string(),
            proposal_kind: "threshold".to_string(),
            status: "pending_eval".to_string(),
            gate_key: Some("fact_risk_block".to_string()),
            current_value: Some(6.0),
            proposed_value: Some(7.0),
            cohort_notes: doc! { "hit_rate_observed": 0.42 },
            proposed_template_key: None,
            proposed_section: None,
            diff_summary: None,
            diff_snippet: None,
            critic_reasoning: None,
            expected_improvement_on: vec![],
            risk_note: None,
            previous_prompt_version: None,
            eval_metrics: doc! {},
            eval_replays_completed: 0,
            eval_replays_failed: 0,
            significance_passed: None,
            failure_reason: None,
            released_at: None,
            released_by: None,
            rolled_back_at: None,
            rolled_back_by: None,
            created_at: now,
            updated_at: now,
        };
        let doc = mongodb::bson::to_document(&p).expect("serialize Proposal");
        assert_eq!(doc.get_str("proposal_kind").unwrap(), "threshold");
        assert_eq!(doc.get_str("gate_key").unwrap(), "fact_risk_block");
        assert_eq!(doc.get_f64("current_value").unwrap(), 6.0);
        let cohort = doc.get_document("cohort_notes").unwrap();
        assert_eq!(cohort.get_f64("hit_rate_observed").unwrap(), 0.42);
        let parsed: Proposal =
            mongodb::bson::from_document(doc).expect("deserialize Proposal");
        assert_eq!(parsed.proposed_value, Some(7.0));
        assert!(parsed.proposed_template_key.is_none());
    }

    #[test]
    fn proposal_bson_round_trip_prompt_kind() {
        let now = mongodb::bson::DateTime::now();
        let p = Proposal {
            id: None,
            experiment_id: "exp_002".to_string(),
            workspace_id: "default".to_string(),
            account_id: "default".to_string(),
            proposal_kind: "prompt".to_string(),
            status: "pending_eval".to_string(),
            gate_key: None,
            current_value: None,
            proposed_value: None,
            cohort_notes: doc! {},
            proposed_template_key: Some("reply_agent_main".to_string()),
            proposed_section: Some("policy".to_string()),
            diff_summary: Some("强化 product fact-check 兜底语句".to_string()),
            diff_snippet: Some("…在引用产品参数前必须确认 knowledge chunk…".to_string()),
            critic_reasoning: Some("过去 30 条失败 cohort 中 12 条触发 fact_risk_block".to_string()),
            expected_improvement_on: vec!["blocked_unverified_product_claim".to_string()],
            risk_note: None,
            previous_prompt_version: Some("v3".to_string()),
            eval_metrics: doc! {},
            eval_replays_completed: 0,
            eval_replays_failed: 0,
            significance_passed: None,
            failure_reason: None,
            released_at: None,
            released_by: None,
            rolled_back_at: None,
            rolled_back_by: None,
            created_at: now,
            updated_at: now,
        };
        let doc = mongodb::bson::to_document(&p).expect("serialize Proposal");
        assert_eq!(doc.get_str("proposal_kind").unwrap(), "prompt");
        assert_eq!(doc.get_str("proposed_section").unwrap(), "policy");
        let parsed: Proposal =
            mongodb::bson::from_document(doc).expect("deserialize Proposal");
        assert_eq!(parsed.proposed_template_key.as_deref(), Some("reply_agent_main"));
        assert_eq!(parsed.expected_improvement_on.len(), 1);
        assert_eq!(parsed.previous_prompt_version.as_deref(), Some("v3"));
    }

    #[test]
    fn shadow_replay_bson_round_trip() {
        let now = mongodb::bson::DateTime::now();
        let proposal_id = mongodb::bson::oid::ObjectId::new();
        let source_run_id = mongodb::bson::oid::ObjectId::new();
        let r = ShadowReplay {
            id: None,
            proposal_id,
            experiment_id: "exp_003".to_string(),
            workspace_id: "default".to_string(),
            account_id: "default".to_string(),
            source_run_id,
            status: "completed".to_string(),
            failure_reason: None,
            original_final_review_status: Some("blocked_unverified_product_claim".to_string()),
            new_final_review_status: Some("approved".to_string()),
            new_review_risks: vec!["pressure_risk:hard_close".to_string()],
            new_token_cost: Some(1234),
            new_5gate_hit: doc! { "fact_risk": false, "pressure_risk": true },
            new_self_critique_addressed: Some(true),
            similarity_to_original_text: 0.0,
            started_at: now,
            finished_at: Some(now),
        };
        let doc = mongodb::bson::to_document(&r).expect("serialize ShadowReplay");
        assert_eq!(doc.get_str("status").unwrap(), "completed");
        assert_eq!(doc.get_f64("similarity_to_original_text").unwrap(), 0.0);
        let parsed: ShadowReplay =
            mongodb::bson::from_document(doc).expect("deserialize ShadowReplay");
        assert_eq!(parsed.proposal_id, proposal_id);
        assert_eq!(parsed.source_run_id, source_run_id);
        assert_eq!(parsed.new_token_cost, Some(1234));
        assert!(parsed.new_self_critique_addressed.unwrap());
    }

    #[test]
    fn threshold_override_bson_round_trip() {
        let now = mongodb::bson::DateTime::now();
        let proposal_id = mongodb::bson::oid::ObjectId::new();
        let o = ThresholdOverride {
            id: None,
            workspace_id: "default".to_string(),
            account_id: "default".to_string(),
            gate_key: "human_like_score_rewrite".to_string(),
            value: 6.5,
            source_proposal_id: proposal_id,
            released_at: now,
            released_by: "admin@local".to_string(),
            rolled_back_at: None,
            rolled_back_by: None,
        };
        let doc = mongodb::bson::to_document(&o).expect("serialize ThresholdOverride");
        assert_eq!(doc.get_str("gate_key").unwrap(), "human_like_score_rewrite");
        assert_eq!(doc.get_f64("value").unwrap(), 6.5);
        let parsed: ThresholdOverride =
            mongodb::bson::from_document(doc).expect("deserialize ThresholdOverride");
        assert_eq!(parsed.gate_key, "human_like_score_rewrite");
        assert_eq!(parsed.value, 6.5);
        assert_eq!(parsed.source_proposal_id, proposal_id);
        assert!(parsed.rolled_back_at.is_none());
    }

    #[test]
    fn post_release_review_bson_round_trip() {
        let now = mongodb::bson::DateTime::now();
        let proposal_id = mongodb::bson::oid::ObjectId::new();
        let r = PostReleaseReview {
            id: None,
            proposal_id,
            workspace_id: "default".to_string(),
            account_id: "default".to_string(),
            proposal_kind: "threshold".to_string(),
            released_at: now,
            scheduled_at: now,
            completed: false,
            actual_send_success_rate_delta: None,
            actual_5gate_hit_delta: doc! {},
            completed_at: None,
        };
        let doc = mongodb::bson::to_document(&r).expect("serialize PostReleaseReview");
        assert_eq!(doc.get_str("proposal_kind").unwrap(), "threshold");
        assert_eq!(doc.get_bool("completed").unwrap(), false);
        let parsed: PostReleaseReview =
            mongodb::bson::from_document(doc).expect("deserialize PostReleaseReview");
        assert_eq!(parsed.proposal_id, proposal_id);
        assert!(!parsed.completed);
        assert!(parsed.actual_send_success_rate_delta.is_none());
    }

    /// W0 / Task 1.6：M4 演化器迁移 ID 必须按字符串字典序排在 M3 末条之后，
    /// 否则 `migration_ids_are_chronologically_ordered` 测试将失败。
    /// 这里独立断言一次，给后续追加 M4_002 / M5 时一个明确锚点。
    #[test]
    fn m4_migration_id_is_after_2026_05_009() {
        // `0` (0x30) < `M` (0x4D)：`2026_05_009...` < `2026_05_M4_001...`。
        assert!("2026_05_009_contact_customer_stage_updated_at_backfill"
            < "2026_05_M4_001_prompt_template_versioned");
    }
}
