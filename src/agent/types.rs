//! 用户运营 Agent 内部使用的数据结构与 serde 反序列化辅助。
//!
//! 该模块汇集了 Reply Agent / Review Agent / Knowledge Router /
//! Send Gateway 之间通讯的 JSON shape：[`AgentDecision`]、
//! [`DecisionReviewResult`]、[`KnowledgeRouteResult`]、[`RunPlannerResult`]
//! 等以及伴随的 `string_or_vec` / `number_i32` / `optional_i32` /
//! `document_vec` 等宽容的反序列化器，都在这里。
//!
//! 所有类型仅做"数据契约 + 兜底解析"，不放任何业务行为，便于子模块
//! （decision / review / knowledge_router / gateway / simulation 等）
//! 共享而不形成循环依赖。

use mongodb::bson::{doc, to_document, Document};
use serde::{Deserialize, Deserializer, Serialize};

use crate::models::{
    AgentProfile, AgentTask, ConversationMessage, OperationKnowledgeChunk,
    OperationKnowledgeDocument,
};

/// outbox 执行客户发送时的失败边界。业务决策已由模型完成；这里仅表达执行器能否
/// 证明“客户投递请求尚未造成不可逆副作用”。
#[derive(Debug, thiserror::Error)]
pub(crate) enum OutboundSendError {
    #[error("safe to retry: {0}")]
    SafeToRetry(String),
    #[error("delivery uncertain: {0}")]
    DeliveryUncertain(String),
}

/// 对已经越过 outbox 最后可取消点的发送做事后核验时的三态结果。
/// `NotDelivered` 只能由权威查询明确未命中，或由发送前置条件证明客户投递尚未发生；
/// 缺少证据不是 `NotDelivered`，而是 `Inconclusive`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeliveryVerification {
    Delivered,
    NotDelivered,
    Inconclusive,
}

impl From<crate::error::AppError> for OutboundSendError {
    fn from(value: crate::error::AppError) -> Self {
        Self::SafeToRetry(value.to_string())
    }
}

impl From<mongodb::error::Error> for OutboundSendError {
    fn from(value: mongodb::error::Error) -> Self {
        Self::SafeToRetry(value.to_string())
    }
}

impl From<crate::mcp::McpSendError> for OutboundSendError {
    fn from(value: crate::mcp::McpSendError) -> Self {
        match value {
            crate::mcp::McpSendError::SafeToRetry(reason) => Self::SafeToRetry(reason),
            crate::mcp::McpSendError::DeliveryUncertain(reason) => Self::DeliveryUncertain(reason),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedOperationProfile {
    pub agent_profile: AgentProfile,
    #[serde(default, deserialize_with = "string_or_vec")]
    pub tags: Vec<String>,
    pub customer_stage: Option<String>,
    pub intent_level: Option<String>,
    pub last_commitment: Option<String>,
    pub follow_up_policy: Option<String>,
    #[serde(default)]
    pub profile_attributes: Document,
}

/// Reply Agent → MCP knowledge.* 工具调用请求。
///
/// agent-autonomy-loop W1 / Task 2.2 引入：在 [`AgentDecision::tool_calls`] /
/// [`RawAgentDecision::tool_calls`] 中承载 `tool_calling` 中间轮 Agent 想调用的工具。
/// `tool` 取值约束在 R4 工具循环中校验（`knowledge.list_catalog` /
/// `knowledge.search` / `knowledge.open_slice` 三选一），本结构本身仅做
/// 反序列化容器，不做语义校验。
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallRequest {
    #[serde(default)]
    pub tool: String,
    #[serde(default)]
    pub arguments: Document,
}

/// Reply Agent 自由输出的"对真实用户的理解"自由信号（R8 自由维度）。
///
/// agent-autonomy-loop W1 / Task 2.2 引入：与 `customer_stage / intent_level /
/// objection_type` 等严格字典字段正交，本结构 SHALL NOT 参与统计聚合，仅供
/// Agent 后续自我引用与人审审计。
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSignal {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub evidence: Option<String>,
    #[serde(default, deserialize_with = "number_i32")]
    pub confidence: i32,
}

/// agent-autonomy-loop W1 / Task 2.2：[`AgentDecision::decision_phase`] 的默认值。
/// 缺失或解析失败时按 R1.10 / R4.1 视为最终轮（保守 + 触发完整 review 校验）。
fn default_decision_phase() -> String {
    "final".to_string()
}

/// 缺失时按"寒暄关系"作为最保守模式（不会触发产品话术 + 5 闸宽松）。
fn default_conversation_mode() -> String {
    "casual_relationship".to_string()
}

/// Model-owned control step for the next harness iteration. The protocol is closed so the
/// runtime can route capabilities, while the semantic choice of step remains entirely with AI.
fn infer_next_step(decision_phase: &str, should_reply: bool, escalation_needed: bool) -> String {
    if decision_phase == "tool_calling" {
        "retrieve".to_string()
    } else if escalation_needed {
        "ask_principal".to_string()
    } else if should_reply {
        "respond".to_string()
    } else {
        "stay_silent".to_string()
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DraftClaim {
    /// Stable model-local identifier used only to correlate a draft across repair iterations.
    #[serde(default)]
    pub claim_id: String,
    /// Atomic meaning asserted by the candidate. This is advisory; independent ClaimGate owns
    /// send authorization and re-extracts claims from the final reply.
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub requires_evidence: bool,
    #[serde(default, deserialize_with = "string_or_vec")]
    pub proposed_source_ids: Vec<String>,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VerificationDecision {
    #[serde(default)]
    pub needed: bool,
    #[serde(default)]
    pub reason: String,
}

/// Typed visit request emitted by the Reply Agent. This represents a customer request only;
/// confirmation is a separate lifecycle transition requiring trusted external provenance.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppointmentRequestDecision {
    #[serde(default)]
    pub requested: bool,
    #[serde(default)]
    pub request_text: String,
    #[serde(default)]
    pub preferred_start: String,
    #[serde(default)]
    pub preferred_end: String,
    #[serde(default)]
    pub location_preference: String,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AgentDecision {
    #[serde(default)]
    pub run_mode: String,
    #[serde(default)]
    pub risk_level: String,
    #[serde(default)]
    pub knowledge_need: String,
    #[serde(default)]
    pub needs_review: bool,
    #[serde(default)]
    pub should_reply: bool,
    #[serde(default)]
    pub reply_text: String,
    #[serde(default)]
    pub profile_update: Option<AgentProfile>,
    #[serde(default, deserialize_with = "string_or_vec")]
    pub tags: Vec<String>,
    /// 子计划2：LLM 指认的标签证据——窗口内消息序位（0-based）。代码侧映射回 _id 并 fail-closed 校验。
    #[serde(default)]
    pub tag_evidence_turns: Vec<i32>,
    /// customer_stage 判断的证据序位。
    #[serde(default)]
    pub stage_evidence_turns: Vec<i32>,
    /// LLM 标注：customer_stage 是否基于客户明示意图（非 AI 语境推断）。
    #[serde(default)]
    pub stage_explicit_intent: bool,
    /// 子计划4：贝叶斯评估旁路——LLM 自由发现的客户维度观察（最多 6 个，开放维度）。
    /// 纯观测侧路，**永不驱动**任何决策/筛选/状态机；gateway 发送后用代码侧证据强度
    /// 统计 + apply_bayesian_update 增量更新写回 `Contact.bayesian_signals`。
    #[serde(default)]
    pub bayesian_observations: Vec<BayesianObservationRaw>,
    pub customer_stage: Option<String>,
    pub intent_level: Option<String>,
    /// universal-domain-adaptation H1：对维度名零假设的开放画像信号容器。
    ///
    /// 销售域只有 `customer_stage` / `intent_level` 两维（仍保留为上面的 typed
    /// 字段，删了会破 lib 基线 + state_transition_pbt）；陪伴/同行等非销售域可携带
    /// `relationship_closeness` / `emotional_state` 等任意维度。
    /// [`super::domain_signals::normalize_domain_signals`] 在 typed 字段与本容器
    /// 之间做双向同步，落库经由统一写入内核。DEFAULT 销售域里 LLM 只输出 typed、
    /// 不输出 `domainSignals`，故本容器由 normalize 从 typed 镜像得来——行为不变。
    #[serde(default)]
    pub domain_signals: Document,
    /// 维度值 → 中文显示名。LLM 仅在为某维度填了「字典外自造新值」时，在此为该
    /// 维度配一个简洁中文名（如 `{"customer_stage": "焦虑观望"}`）。字典已有的标准
    /// 值不必填（已有 canonical label）。gateway / decision_taxonomy 产 taxonomy
    /// 候选时按 kind 查此表取中文名作 `suggested_display_name`（收件箱命名卡预填）。
    /// 绝大多数轮次不出现（无自造值即无名字）——故 `#[serde(default)]` 是输出容错，
    /// 非兼容 shim；改必填会使 LLM 漏填的轮次 decision 反序列化失败、决策链路崩。
    #[serde(default)]
    pub dimension_display_names: Document,
    pub last_commitment: Option<String>,
    /// PR-D：结构化承诺（带可选 dueAt）。promote 时从 RawAgentDecision.commitment 透传。
    pub commitment: Option<CommitmentDecision>,
    /// Model-selected lifecycle transitions for already-active commitments. The runtime only
    /// validates referenced ids/statuses and applies authorized transitions; it never infers an
    /// action from customer words.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commitment_updates: Vec<CommitmentLifecycleDecision>,
    pub follow_up_policy: Option<String>,
    #[serde(default)]
    pub profile_attributes: Document,
    #[serde(default)]
    pub intent_analysis: Document,
    #[serde(default)]
    pub next_best_action: Document,
    pub operation_state: Option<String>,
    pub operation_state_reason: Option<String>,
    #[serde(default, deserialize_with = "optional_i32")]
    pub operation_state_confidence: Option<i32>,
    pub cooldown_until: Option<String>,
    #[serde(default, deserialize_with = "optional_i32")]
    pub product_fit_score: Option<i32>,
    #[serde(default, deserialize_with = "string_or_vec")]
    pub matched_knowledge_ids: Vec<String>,
    #[serde(default, deserialize_with = "string_or_vec")]
    pub safe_claims_used: Vec<String>,
    #[serde(default, deserialize_with = "optional_i32")]
    pub forbidden_claim_risk: Option<i32>,
    #[serde(default, deserialize_with = "string_or_vec")]
    pub objections_detected: Vec<String>,
    #[serde(default, deserialize_with = "string_or_vec")]
    pub recommended_resource_ids: Vec<String>,
    #[serde(default)]
    pub operating_memory_update: Document,
    #[serde(default, deserialize_with = "document_vec")]
    pub memory_candidates: Vec<Document>,
    #[serde(default, deserialize_with = "number_i32")]
    pub memory_write_score: i32,
    #[serde(default)]
    pub consolidation_needed: bool,
    #[serde(default, deserialize_with = "string_or_vec")]
    pub used_knowledge_ids: Vec<String>,
    /// 历史 Reply 协议字段：模型自报其引用的产品 ID。
    ///
    /// 仅为持久化/提示兼容保留，不能作为目录背书或发送授权证据。R5.4 的
    /// `priced_from_catalog` 只由独立 ClaimGate 对最终正文提取、精确 quote 锚定并由
    /// 服务端逐字段核对 active catalog 后产生。
    #[serde(default, deserialize_with = "string_or_vec")]
    pub quoted_product_ids: Vec<String>,
    #[serde(default)]
    pub memory_update: String,
    pub context_pack_version: Option<i32>,
    #[serde(default)]
    pub follow_up: Option<FollowUpDecision>,

    // ── agent-autonomy-loop W1 / Task 2.2：自治协议 9 字段（R1.1） ──
    //
    // 9 个字段全部以 `String` 落入 `agent_run_logs.decision`，便于审计端原文读取；
    // 长度上限与必填规则在 W1 task 2.3 的 `RawAgentDecision::validate_and_promote`
    // 中校验，本结构仅承担数据容器角色。
    #[serde(default)]
    pub user_understanding: String,
    #[serde(default)]
    pub relationship_read: String,
    #[serde(default)]
    pub operation_goal: String,
    #[serde(default)]
    pub knowledge_need_reason: String,
    #[serde(default)]
    pub memory_update_reason: String,
    #[serde(default)]
    pub self_critique: String,
    #[serde(default)]
    pub why_should_reply: String,
    #[serde(default)]
    pub why_skip_reply: String,
    #[serde(default)]
    pub risk_self_check: String,

    // ── agent-autonomy-loop W1 / Task 2.2：自治控制位 + tool-loop 协议字段 ──
    //
    // `autonomy_mode`：与 `run_mode` 正交，描述本轮 Agent 自主权范围
    // （`auto / assisted / blocked`，详见 R3.3）。
    // `decision_phase`：tool-loop 中间轮 / 最终轮区分（`tool_calling / final`，
    // 详见 R1.10、R4.1）；JSON 缺失时由 `default_decision_phase` 回退为 "final"，
    // Rust 侧 `Default::default()` 同样回退为 "final"（保守 + 触发完整 review 校验）。
    // `tool_calls`：Reply Agent 在 `decision_phase=="tool_calling"` 时声明
    // 的 MCP knowledge.* 工具调用请求（详见 R4.1）。
    // `agent_generated_signals`：R8 自由维度信号（不参与聚合统计）。
    #[serde(default)]
    pub autonomy_mode: String,
    #[serde(default = "default_decision_phase")]
    pub decision_phase: String,
    #[serde(default)]
    pub tool_calls: Vec<ToolCallRequest>,
    #[serde(default)]
    pub agent_generated_signals: Vec<AgentSignal>,

    /// Model-selected harness action. Missing legacy values are inferred from the structural
    /// phase, escalation request, and shouldReply flag; code never infers it from message words.
    #[serde(default)]
    pub next_step: String,
    /// Draft-side claim inventory for self-critique and repair. It is never an authorization
    /// source; the independent ClaimGate produces the authoritative manifest.
    #[serde(default)]
    pub claim_manifest: Vec<DraftClaim>,
    #[serde(default)]
    pub verification: VerificationDecision,
    #[serde(default)]
    pub appointment_request: Option<AppointmentRequestDecision>,

    // ── conversation_mode：四模式人格切换（R-prompt-v3） ──
    //
    // 取代以前"统一人格 + LLM 自由判断 shouldReply"的单层结构。每轮 Reply Agent
    // 必须输出 conversation_mode（严格枚举），决定本轮的语气、信息密度、
    // 5 闸阈值偏好（详见 docs/conversation-mode-design.md）。
    //
    // 取值：
    //   - casual_relationship：寒暄关系，维系熟悉度，不推销
    //   - value_exchange     ：价值互换，分享内容，不强推产品
    //   - consultative       ：顾问/销售模式，明确处理产品/价格/方案/异议
    //   - boundary_protection：边界保护，客户已表达不需要 / 仅服务老客户
    //
    // 模式由 Reply Agent 根据完整语境输出；服务端只校验协议枚举，不扫描消息关键词。
    #[serde(default = "default_conversation_mode")]
    pub conversation_mode: String,
    #[serde(default)]
    pub conversation_mode_reason: Option<String>,

    /// decision Agent emit 的请示意图；None=本轮无需请示真人。
    #[serde(default)]
    pub escalation_request: Option<crate::models::EscalationRequest>,

    /// 销售素材文件发送（media-asset Task 8）：本轮 Reply Agent 决定发给客户的
    /// 素材清单（每项 = 候选「可发送素材」里的 assetId + 选材理由）。LLM 不选时为
    /// 空 Vec（默认）。gateway 在文本回复 enqueue 之后，把每项转成一条独立媒体
    /// outbox 条目（先文字后文件），并做 approved+sendable 二次准入校验。
    #[serde(default)]
    pub assets_to_send: Vec<AssetSendDirective>,

    /// 专属顾问名片引荐：AI 识别高价值客户（签约/到店）后，本轮决定推给客户的
    /// 真人专属顾问名片（候选清单里的 cardId + 选择理由）。LLM 不推时为 None
    /// （默认）。gateway 在文本回复 enqueue 之后，转成一条 message_send_namecard
    /// outbox 条目，并做 approved + 候选校验防幻觉；推完 AI 退居辅助。
    #[serde(default)]
    pub namecard_to_send: Option<NamecardDirective>,

    /// 渐进式三档 + 充分性自评（2026-06-23）：Reply Agent 自评本轮信息是否充分。
    /// - sufficiency: "enough" | "need_more_context" | "need_clarification"
    /// - missing_tier: "none" | "relational" | "full"
    /// - clarification_intent: 若 need_clarification，给澄清方向
    #[serde(default)]
    pub sufficiency: String,
    #[serde(default)]
    pub missing_tier: String,
    #[serde(default)]
    pub clarification_intent: String,
}

/// 单条素材发送指令：LLM 从注入的「可发送素材」候选清单里选出的一项。
/// `asset_id` 必须是候选清单里列出的 ContentAsset `_id`（gateway 二次校验防幻觉）。
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AssetSendDirective {
    #[serde(default)]
    pub asset_id: String,
    #[serde(default)]
    pub reason: Option<String>,
}

/// 单条名片引荐指令（专属顾问名片引荐）：AI 识别到高价值客户（签约/到店）后，
/// 从注入的「可发送名片」候选清单里选出的一项。`card_id` 必须是候选清单里列出的
/// 名片标识（gateway 二次校验防幻觉）。AI 推完名片后退居辅助，客户始终只跟 AI 对话。
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NamecardDirective {
    #[serde(default)]
    pub card_id: String,
    #[serde(default)]
    pub reason: Option<String>,
}

/// 子计划4：贝叶斯评估旁路的单条维度观察（LLM 输出形态）。
/// `confidence` 是 LLM 自报的观察值；**强证据数不由 LLM 自报**——代码侧用
/// `evidence_turns` + 消息方向（客户入站消息）在 gateway 计算。纯观测，永不驱动决策。
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BayesianObservationRaw {
    #[serde(default)]
    pub dimension: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub confidence: f64,
    #[serde(default)]
    pub evidence_turns: Vec<i32>,
}

impl Default for AgentDecision {
    fn default() -> Self {
        Self {
            run_mode: String::new(),
            risk_level: String::new(),
            knowledge_need: String::new(),
            needs_review: false,
            should_reply: false,
            reply_text: String::new(),
            profile_update: None,
            tags: Vec::new(),
            tag_evidence_turns: Vec::new(),
            stage_evidence_turns: Vec::new(),
            stage_explicit_intent: false,
            bayesian_observations: Vec::new(),
            customer_stage: None,
            intent_level: None,
            domain_signals: Document::new(),
            dimension_display_names: Document::new(),
            last_commitment: None,
            commitment: None,
            commitment_updates: Vec::new(),
            follow_up_policy: None,
            profile_attributes: Document::new(),
            intent_analysis: Document::new(),
            next_best_action: Document::new(),
            operation_state: None,
            operation_state_reason: None,
            operation_state_confidence: None,
            cooldown_until: None,
            product_fit_score: None,
            matched_knowledge_ids: Vec::new(),
            safe_claims_used: Vec::new(),
            forbidden_claim_risk: None,
            objections_detected: Vec::new(),
            recommended_resource_ids: Vec::new(),
            operating_memory_update: Document::new(),
            memory_candidates: Vec::new(),
            memory_write_score: 0,
            consolidation_needed: false,
            used_knowledge_ids: Vec::new(),
            quoted_product_ids: Vec::new(),
            memory_update: String::new(),
            context_pack_version: None,
            follow_up: None,
            // 自治协议 9 字段：默认空串（W1 task 2.3 中由
            // `RawAgentDecision::validate_and_promote` 在 final 轮触发必填校验）
            user_understanding: String::new(),
            relationship_read: String::new(),
            operation_goal: String::new(),
            knowledge_need_reason: String::new(),
            memory_update_reason: String::new(),
            self_critique: String::new(),
            why_should_reply: String::new(),
            why_skip_reply: String::new(),
            risk_self_check: String::new(),
            // 自治控制位：默认空（task 2.3 中由 validate_and_promote 校验枚举）
            autonomy_mode: String::new(),
            // tool-loop：默认 "final"（保守 + 触发完整 review 校验）
            decision_phase: default_decision_phase(),
            tool_calls: Vec::new(),
            agent_generated_signals: Vec::new(),
            next_step: String::new(),
            claim_manifest: Vec::new(),
            verification: VerificationDecision::default(),
            appointment_request: None,
            // conversation_mode：默认寒暄模式（最保守）
            conversation_mode: default_conversation_mode(),
            conversation_mode_reason: None,
            // 请示意图：默认无（本轮不向幕后真人请示）
            escalation_request: None,
            // 素材发送：默认空（LLM 不选材时不发任何文件）
            assets_to_send: Vec::new(),
            // 名片引荐：默认 None（LLM 不推时不发任何名片）
            namecard_to_send: None,
            // 渐进式三档 + 充分性自评（2026-06-23）：默认空（LLM 未输出时不触发档位提升）
            sufficiency: String::new(),
            missing_tier: String::new(),
            clarification_intent: String::new(),
        }
    }
}

/// Reply Agent → Rust 边界的"原始反序列化"结构（agent-autonomy-loop W1 / Task 2.2 / N2）。
///
/// 与业务结构 [`AgentDecision`] 的差异：本结构所有字段均为 `Option<T>`，用于
/// **区分"未输出"与"输出 false / 空字符串"** 这两个语义。task 2.3 的
/// `RawAgentDecision::validate_and_promote(self, runtime) -> (AgentDecision,
/// Vec<String>)` 会把这里的 `Option<T>` 映射为 `AgentDecision` 的非 Option
/// 字段，并在 `final` 轮按 R1.3 / R1.4 / R1.5 / R3.1 / R3.5 等聚合协议违规标签。
///
/// **注意**：本结构本身只是反序列化容器，不做枚举校验、必填校验、长度校验。
/// 任何看到非法值的报错路径都在 task 2.3 的 promote 函数中收口。
///
/// W1 task 2.3 已实现 `validate_and_promote`；后续 W2/W3 在 gateway / reply
/// 解析路径接入即可消费它。
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RawAgentDecision {
    // ── 自治协议 9 字段（R1.1 / R1.3 / R1.4 / R1.5 / R1.6） ──
    pub user_understanding: Option<String>,
    pub relationship_read: Option<String>,
    pub operation_goal: Option<String>,
    pub knowledge_need_reason: Option<String>,
    pub memory_update_reason: Option<String>,
    pub self_critique: Option<String>,
    pub why_should_reply: Option<String>,
    pub why_skip_reply: Option<String>,
    pub risk_self_check: Option<String>,

    // ── 业务必填字段（R3.1 / R3.2 / R3.3） ──
    pub risk_level: Option<String>,     // low | medium | high
    pub knowledge_need: Option<String>, // not_required | required | insufficient
    pub run_mode: Option<String>, // fast_chat | memory_candidate | knowledge_grounded | high_risk
    pub autonomy_mode: Option<String>, // auto | assisted | blocked
    pub needs_review: Option<bool>,
    pub operation_state: Option<String>,
    pub consolidation_needed: Option<bool>,

    // ── R4 工具循环协议 ──
    pub decision_phase: Option<String>, // tool_calling | final
    pub tool_calls: Option<Vec<ToolCallRequest>>,

    // ── R8 自由信号 ──
    pub agent_generated_signals: Option<Vec<AgentSignal>>,

    // ── Harness control and typed side-effect intents ──
    pub next_step: Option<String>,
    pub claim_manifest: Option<Vec<DraftClaim>>,
    pub verification: Option<VerificationDecision>,
    pub appointment_request: Option<AppointmentRequestDecision>,

    // ── R-prompt-v3 conversation_mode：四模式人格切换 ──
    pub conversation_mode: Option<String>,
    pub conversation_mode_reason: Option<String>,

    // ── 既有回复 / 知识 / 记忆 / 信号字段（保留为 Option，由 promote 落地为非 Option）──
    pub reply_text: Option<String>,
    pub should_reply: Option<bool>,
    pub used_knowledge_ids: Option<Vec<String>>,
    /// 历史兼容字段；模型自报值不参与 `priced_from_catalog` 授权判定。
    pub quoted_product_ids: Option<Vec<String>>,
    pub safe_claims_used: Option<Vec<String>>,
    pub knowledge_route: Option<KnowledgeRouteResult>,
    pub profile_update: Option<AgentProfile>,
    pub tags: Option<Vec<String>>,
    /// 子计划2：LLM 指认的标签证据窗口序位（promote 后透传到 AgentDecision.tag_evidence_turns）。
    #[serde(default)]
    pub tag_evidence_turns: Option<Vec<i32>>,
    /// customer_stage 判断的证据序位。
    #[serde(default)]
    pub stage_evidence_turns: Option<Vec<i32>>,
    /// LLM 标注：customer_stage 是否基于客户明示意图（非 AI 语境推断）。
    #[serde(default)]
    pub stage_explicit_intent: Option<bool>,
    /// 子计划4：贝叶斯评估旁路——LLM 自由发现的客户维度观察（promote 后透传到
    /// AgentDecision.bayesian_observations）。纯观测，永不驱动决策。
    #[serde(default)]
    pub bayesian_observations: Option<Vec<BayesianObservationRaw>>,
    #[serde(alias = "customer_stage")]
    pub customer_stage: Option<String>,
    #[serde(alias = "intent_level")]
    pub intent_level: Option<String>,
    /// universal-domain-adaptation G1：对维度名零假设的开放画像信号容器。非销售
    /// 行业（陪伴/同行等）的「参与决策」维度（如 `purchase_lifecycle` /
    /// `relationship_closeness`）由 LLM 写进这里。销售域 LLM 只输出 typed
    /// `customerStage`/`intentLevel`，本字段缺省 → promote 后容器空、由
    /// `normalize_domain_signals` 从 typed 镜像，行为与改造前逐字等价。
    #[serde(default)]
    pub domain_signals: Option<Document>,
    /// 维度值→中文名映射（promote 后经 carry_through 透传到
    /// `AgentDecision.dimension_display_names`）。LLM 缺省 → None → 容器空。
    #[serde(default)]
    pub dimension_display_names: Option<Document>,
    pub last_commitment: Option<String>,
    /// PR-D：结构化承诺（带可选 dueAt）。缺失时回落 last_commitment。
    pub commitment: Option<CommitmentDecision>,
    #[serde(default)]
    pub commitment_updates: Option<Vec<CommitmentLifecycleDecision>>,
    pub follow_up_policy: Option<String>,
    pub profile_attributes: Option<Document>,
    pub intent_analysis: Option<Document>,
    pub next_best_action: Option<Document>,
    pub operation_state_reason: Option<String>,
    pub operation_state_confidence: Option<i32>,
    pub cooldown_until: Option<String>,
    pub product_fit_score: Option<i32>,
    pub matched_knowledge_ids: Option<Vec<String>>,
    pub forbidden_claim_risk: Option<i32>,
    pub objections_detected: Option<Vec<String>>,
    pub recommended_resource_ids: Option<Vec<String>>,
    pub operating_memory_update: Option<Document>,
    pub memory_candidates: Option<Vec<Document>>,
    pub memory_write_score: Option<i32>,
    pub memory_update: Option<String>,
    pub context_pack_version: Option<i32>,
    pub follow_up: Option<FollowUpDecision>,
    #[serde(default)]
    pub escalation_request: Option<crate::models::EscalationRequest>,
    /// media-asset Task 8：LLM 输出的素材发送清单（先落 Option 容器，再由
    /// `carry_through_fields` 透传到 `AgentDecision.assets_to_send`）。
    pub assets_to_send: Option<Vec<AssetSendDirective>>,
    /// 专属顾问名片引荐：LLM 输出的名片引荐指令（先落 Option 容器，再由
    /// carry-through 透传到 `AgentDecision.namecard_to_send`）。
    pub namecard_to_send: Option<NamecardDirective>,

    /// 渐进式三档 + 充分性自评（2026-06-23）：Reply Agent 自评本轮信息是否充分。
    #[serde(default)]
    pub sufficiency: Option<String>,
    #[serde(default)]
    pub missing_tier: Option<String>,
    #[serde(default)]
    pub clarification_intent: Option<String>,
}

/// Post-send projection output. This contract deliberately cannot represent reply text,
/// authorization, escalation, media, commitments, or follow-up scheduling.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DeferredProjectionDecision {
    pub profile_update: Option<AgentProfile>,
    #[serde(deserialize_with = "string_or_vec")]
    pub tags: Vec<String>,
    pub tag_evidence_turns: Vec<i32>,
    pub stage_evidence_turns: Vec<i32>,
    pub stage_explicit_intent: bool,
    pub bayesian_observations: Vec<BayesianObservationRaw>,
    pub customer_stage: Option<String>,
    pub intent_level: Option<String>,
    pub domain_signals: Document,
    pub dimension_display_names: Document,
    pub follow_up_policy: Option<String>,
    pub profile_attributes: Document,
    pub next_best_action: Document,
    #[serde(deserialize_with = "string_or_vec")]
    pub objections_detected: Vec<String>,
    pub operating_memory_update: Document,
    #[serde(deserialize_with = "document_vec")]
    pub memory_candidates: Vec<Document>,
    #[serde(deserialize_with = "number_i32")]
    pub memory_write_score: i32,
    pub consolidation_needed: bool,
    pub memory_update: String,
    pub agent_generated_signals: Vec<AgentSignal>,
}

impl DeferredProjectionDecision {
    /// Parse a projection while fail-closing only fields that could affect customer delivery.
    /// Unknown analytical fields are ignored for forward compatibility and counted by callers.
    pub fn from_value(value: serde_json::Value) -> Result<Self, String> {
        const FORBIDDEN: &[&str] = &[
            "replyText",
            "shouldReply",
            "needsReview",
            "review",
            "assetsToSend",
            "namecardToSend",
            "escalationRequest",
            "lastCommitment",
            "commitment",
            "commitmentUpdates",
            "followUp",
            "toolCalls",
            "operationState",
            "operationStateReason",
            "operationStateConfidence",
            "cooldownUntil",
            "nextStep",
            "appointmentRequest",
        ];
        let object = value
            .as_object()
            .ok_or_else(|| "projection output must be a JSON object".to_string())?;
        if let Some(field) = FORBIDDEN.iter().find(|field| object.contains_key(**field)) {
            return Err(format!(
                "projection output contains forbidden send-control field: {field}"
            ));
        }
        serde_json::from_value(value).map_err(|error| error.to_string())
    }

    /// Return unknown top-level analytical keys for observability. They do not fail parsing.
    pub fn unknown_fields(value: &serde_json::Value) -> Vec<String> {
        const KNOWN: &[&str] = &[
            "profileUpdate",
            "tags",
            "tagEvidenceTurns",
            "stageEvidenceTurns",
            "stageExplicitIntent",
            "bayesianObservations",
            "customerStage",
            "intentLevel",
            "domainSignals",
            "dimensionDisplayNames",
            "followUpPolicy",
            "profileAttributes",
            "nextBestAction",
            "objectionsDetected",
            "operatingMemoryUpdate",
            "memoryCandidates",
            "memoryWriteScore",
            "consolidationNeeded",
            "memoryUpdate",
            "agentGeneratedSignals",
        ];
        value.as_object().map_or_else(Vec::new, |object| {
            object
                .keys()
                .filter(|key| !KNOWN.contains(&key.as_str()))
                .cloned()
                .collect()
        })
    }

    /// Convert only projection-owned fields into the legacy persistence shape.
    pub fn into_agent_decision(mut self) -> AgentDecision {
        self.tags.truncate(24);
        self.bayesian_observations.truncate(6);
        self.memory_candidates.truncate(6);
        self.agent_generated_signals.truncate(12);
        AgentDecision {
            profile_update: self.profile_update,
            tags: self.tags,
            tag_evidence_turns: self.tag_evidence_turns,
            stage_evidence_turns: self.stage_evidence_turns,
            stage_explicit_intent: self.stage_explicit_intent,
            bayesian_observations: self.bayesian_observations,
            customer_stage: self.customer_stage,
            intent_level: self.intent_level,
            domain_signals: self.domain_signals,
            dimension_display_names: self.dimension_display_names,
            follow_up_policy: self.follow_up_policy,
            profile_attributes: self.profile_attributes,
            next_best_action: self.next_best_action,
            objections_detected: self.objections_detected,
            operating_memory_update: self.operating_memory_update,
            memory_candidates: self.memory_candidates,
            memory_write_score: self.memory_write_score.clamp(0, 10),
            consolidation_needed: self.consolidation_needed,
            memory_update: self.memory_update,
            agent_generated_signals: self.agent_generated_signals,
            ..AgentDecision::default()
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// agent-autonomy-loop W1 / Task 2.3：`validate_and_promote`
//
// Reply Agent JSON → `RawAgentDecision` → 本函数 → `(AgentDecision, risks)`
// 的边界校验层。语义对齐 design.md §4.3 的伪代码：
//
// 1. 解析 `decision_phase`：`tool_calling | final`，未填或非法走默认 `final`
//    并追加 `decision_phase_invalid:<v>` 风险标签（R1.10）。
// 2. `tool_calling` 中间轮：跳过 R1.3 / R1.4 / R1.5 / R1.6 / R3 全部校验，
//    仅校验 toolCalls 的 tool 名是否在 `knowledge.list_catalog /
//    knowledge.search / knowledge.open_slice` 三选一（R4.1）。
// 3. `final` 轮：执行 R3.1/R3.2/R3.3 必填+严格枚举、R1.3 7 字段必填、
//    R1.4 互斥必填（whyShouldReply / whySkipReply）、R1.5 条件长度
//    （low_routine `unchanged` 短形式 vs critical_turn ≥ 20 字符）、
//    R1.6 回复理由长度延伸。
// 4. `runtime.autonomy_protocol_enabled == false` 时（灰度 / sunset 路径）
//    跳过全部校验，构造最小 `AgentDecision` 并返回空 risks（R11 sunset
//    路径预留）。
//
// 违规聚合为 `Vec<String>`：
//   - `missing_required_field:<f>`        — 字段未填或仅含空白
//   - `invalid_enum_value:<f>:<v>`        — 枚举非法
//   - `invalid_type:<f>`                  — bool 字段类型违规
//   - `decision_phase_invalid:<v>`        — decision_phase 取值非法
//   - `insufficient_detail_in_critical_turn:<f>` — R1.5 / R1.6 长度违规
//   - `invalid_tool_call:<tool>`          — tool_calling 阶段 tool 名非法
// ─────────────────────────────────────────────────────────────────────────

const RAW_TOOL_CALLING: &str = "tool_calling";
const RAW_FINAL: &str = "final";

const RISK_LEVEL_VALUES: &[&str] = &["low", "medium", "high"];
const KNOWLEDGE_NEED_VALUES: &[&str] = &["not_required", "required", "insufficient"];
const RUN_MODE_VALUES: &[&str] = &[
    "fast_chat",
    "memory_candidate",
    "knowledge_grounded",
    "high_risk",
];
const AUTONOMY_MODE_VALUES: &[&str] = &["auto", "assisted", "blocked"];
const NEXT_STEP_VALUES: &[&str] = &[
    "respond",
    "stay_silent",
    "retrieve",
    "verify",
    "repair",
    "clarify",
    "ask_principal",
    "defer",
];
const CONVERSATION_MODE_VALUES: &[&str] = &[
    "casual_relationship",
    "value_exchange",
    "consultative",
    "boundary_protection",
];
pub(crate) const SEMANTIC_SPEECH_ACT_VALUES: &[&str] = &[
    "greeting",
    "question",
    "request",
    "statement",
    "wish",
    "hypothetical",
    "quoted",
    "negated",
    "empathy",
    "uncertain",
];
pub(crate) const SEMANTIC_SUBJECT_VALUES: &[&str] =
    &["customer", "business", "third_party", "general", "none"];
pub(crate) const SEMANTIC_ASSERTION_STATUS_VALUES: &[&str] = &[
    "asserted",
    "interrogative",
    "requested",
    "hypothetical",
    "quoted",
    "negated",
    "uncertain",
    "not_applicable",
];
pub(crate) const SEMANTIC_KNOWLEDGE_NEED_VALUES: &[&str] =
    &["not_required", "required", "uncertain"];
pub(crate) const SEMANTIC_RESPONSE_DISPOSITION_VALUES: &[&str] = &[
    "reply",
    "acknowledgement",
    "clarify",
    "defer",
    "silent",
    "cooldown",
];
pub(crate) const SEMANTIC_RISK_VALUES: &[&str] = &["low", "medium", "high"];
const ALLOWED_TOOL_NAMES: &[&str] = &[
    "knowledge.list_catalog",
    "knowledge.search",
    "knowledge.open_slice",
];

/// Whether a promoted Reply decision risk means the model failed the structured wire contract.
///
/// These tags describe protocol shape, not customer semantics. Keeping the classifier here gives
/// the Harness and final send gate one shared definition and avoids status/text heuristics.
pub(crate) fn is_reply_protocol_violation(risk: &str) -> bool {
    risk.starts_with("missing_required_field:")
        || risk.starts_with("invalid_enum_value:")
        || risk.starts_with("invalid_type:")
        || risk.starts_with("decision_phase_invalid:")
}

pub(crate) fn reply_protocol_violations(risks: &[String]) -> Vec<String> {
    risks
        .iter()
        .filter(|risk| is_reply_protocol_violation(risk))
        .cloned()
        .collect()
}

/// 计 Unicode 字符数（按 char 计，与 R1 / R3 中需求文本一致）。
fn count_unicode_chars(s: &str) -> usize {
    s.chars().count()
}

/// 计汉字数量（Unicode 范围 U+4E00..=U+9FFF，中日韩统一表意文字基本区）。
fn count_hanzi(s: &str) -> usize {
    s.chars()
        .filter(|c| matches!(*c, '\u{4E00}'..='\u{9FFF}'))
        .count()
}

/// R1.3 / R3.5：必填字符串（trim 后非空）；空或仅空白 SHALL 追加
/// `missing_required_field:<name>` 并返回空字符串（落入 AgentDecision 默认）。
fn check_required_string(field: Option<String>, name: &str, risks: &mut Vec<String>) -> String {
    match field {
        Some(value) if !value.trim().is_empty() => value,
        _ => {
            risks.push(format!("missing_required_field:{}", name));
            String::new()
        }
    }
}

/// R3.1/R3.2/R3.3：必填 + 严格枚举校验。`None` 或空 → `missing_required_field`；
/// 非法值 → `invalid_enum_value:<name>:<value>`。
fn check_required_enum(
    field: Option<String>,
    name: &str,
    allowed: &[&str],
    risks: &mut Vec<String>,
) -> String {
    match field {
        Some(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                risks.push(format!("missing_required_field:{}", name));
                String::new()
            } else if allowed.iter().any(|a| *a == trimmed) {
                trimmed.to_string()
            } else {
                risks.push(format!("invalid_enum_value:{}:{}", name, trimmed));
                String::new()
            }
        }
        None => {
            risks.push(format!("missing_required_field:{}", name));
            String::new()
        }
    }
}

/// R3.1：必填 bool。`None` 视为未输出 → `missing_required_field:<name>`；
/// 类型非法（在 RawAgentDecision 反序列化层直接报错）此处 None 兜底归为 missing。
fn check_required_bool(field: Option<bool>, name: &str, risks: &mut Vec<String>) -> bool {
    match field {
        Some(v) => v,
        None => {
            risks.push(format!("missing_required_field:{}", name));
            false
        }
    }
}

impl RawAgentDecision {
    /// Validate the compact send-critical contract. Projection and diagnostic fields are
    /// explicitly discarded even if an old/custom prompt still emits them.
    pub fn validate_reply_critical(
        self,
        runtime: &super::runtime::UserRuntimeParameters,
    ) -> (AgentDecision, Vec<String>) {
        let mut risks = Vec::new();
        let phase = match self.decision_phase.as_deref().map(str::trim) {
            Some(RAW_TOOL_CALLING) => RAW_TOOL_CALLING.to_string(),
            Some(RAW_FINAL) | None | Some("") => RAW_FINAL.to_string(),
            Some(other) => {
                risks.push(format!("decision_phase_invalid:{other}"));
                RAW_FINAL.to_string()
            }
        };
        if phase == RAW_TOOL_CALLING {
            let tool_calls = self.tool_calls.clone().unwrap_or_default();
            if tool_calls.is_empty() {
                risks.push("missing_required_field:tool_calls".to_string());
            }
            for call in &tool_calls {
                let trimmed = call.tool.trim();
                if trimmed.is_empty()
                    || !ALLOWED_TOOL_NAMES.iter().any(|allowed| *allowed == trimmed)
                {
                    risks.push(format!("invalid_tool_call:{}", call.tool));
                }
            }
            let mut decision = build_tool_calling_decision(self, phase);
            normalize_harness_control(&mut decision, &mut risks);
            return (decision, risks);
        }
        let allowed_modes: Vec<&str> = if runtime.allowed_conversation_modes.is_empty() {
            CONVERSATION_MODE_VALUES.to_vec()
        } else {
            runtime
                .allowed_conversation_modes
                .iter()
                .map(String::as_str)
                .collect()
        };
        let risk_level = check_required_enum(
            self.risk_level.clone(),
            "risk_level",
            RISK_LEVEL_VALUES,
            &mut risks,
        );
        let knowledge_need = check_required_enum(
            self.knowledge_need.clone(),
            "knowledge_need",
            KNOWLEDGE_NEED_VALUES,
            &mut risks,
        );
        let run_mode = check_required_enum(
            self.run_mode.clone(),
            "run_mode",
            RUN_MODE_VALUES,
            &mut risks,
        );
        let autonomy_mode = check_required_enum(
            self.autonomy_mode.clone(),
            "autonomy_mode",
            AUTONOMY_MODE_VALUES,
            &mut risks,
        );
        let conversation_mode = check_required_enum(
            self.conversation_mode.clone(),
            "conversation_mode",
            &allowed_modes,
            &mut risks,
        );
        let operation_state =
            check_required_string(self.operation_state.clone(), "operation_state", &mut risks);
        let needs_review = check_required_bool(self.needs_review, "needs_review", &mut risks);
        let should_reply = check_required_bool(self.should_reply, "should_reply", &mut risks);
        let risk_self_check =
            check_required_string(self.risk_self_check.clone(), "risk_self_check", &mut risks);
        let reply_text = self.reply_text.clone().unwrap_or_default();
        if should_reply && reply_text.trim().is_empty() {
            risks.push("missing_required_field:reply_text".to_string());
        }
        if !should_reply
            && self
                .why_skip_reply
                .as_deref()
                .map(str::trim)
                .unwrap_or_default()
                .is_empty()
        {
            risks.push("missing_required_field:why_skip_reply".to_string());
        }

        let mut decision = build_minimal_decision(self);
        decision.decision_phase = phase;
        decision.risk_level = risk_level;
        decision.knowledge_need = knowledge_need;
        decision.run_mode = run_mode;
        decision.autonomy_mode = autonomy_mode;
        decision.conversation_mode = if conversation_mode.is_empty() {
            default_conversation_mode()
        } else {
            conversation_mode
        };
        decision.operation_state = (!operation_state.is_empty()).then_some(operation_state);
        decision.needs_review = needs_review;
        decision.should_reply = should_reply;
        decision.reply_text = reply_text;
        decision.risk_self_check = risk_self_check;
        clear_deferred_fields(&mut decision);
        normalize_harness_control(&mut decision, &mut risks);
        sanitize_semantic_assessment(&mut decision, &mut risks);
        (decision, risks)
    }

    /// 把 `RawAgentDecision`（Reply Agent JSON 边界结构）映射到业务结构
    /// [`AgentDecision`]，同时聚合协议违规标签到 `Vec<String>`。详见模块顶部
    /// 长 doc-comment（W1 task 2.3 / N2 / R1 / R3 / R4）。
    pub fn validate_and_promote(
        self,
        runtime: &super::runtime::UserRuntimeParameters,
    ) -> (AgentDecision, Vec<String>) {
        // ── R11 sunset：autonomyProtocolEnabled = false 时跳过全部校验 ──
        // 灰度回退路径仅构造最小 AgentDecision、返回空 risks，由调用方按
        // legacy_mode_unchecked 走老链路（finalReviewStatus 在 W2/W3 落定）。
        if !runtime.autonomy_protocol_enabled {
            return (build_minimal_decision(self), Vec::new());
        }

        let mut risks: Vec<String> = Vec::new();

        // ── R1.10 解析 decision_phase ──
        let phase = match self.decision_phase.as_deref().map(str::trim) {
            Some(RAW_TOOL_CALLING) => RAW_TOOL_CALLING.to_string(),
            Some(RAW_FINAL) | None | Some("") => RAW_FINAL.to_string(),
            Some(other) => {
                risks.push(format!("decision_phase_invalid:{}", other));
                RAW_FINAL.to_string()
            }
        };

        // ── tool_calling 中间轮（R1.10 / R4.1）：仅做 toolCalls schema 检查 ──
        if phase == RAW_TOOL_CALLING {
            let tool_calls = self.tool_calls.clone().unwrap_or_default();
            if tool_calls.is_empty() {
                risks.push("missing_required_field:tool_calls".to_string());
            }
            for call in &tool_calls {
                let trimmed = call.tool.trim();
                if trimmed.is_empty() || !ALLOWED_TOOL_NAMES.iter().any(|a| *a == trimmed) {
                    risks.push(format!("invalid_tool_call:{}", call.tool));
                }
            }
            let mut decision = build_tool_calling_decision(self, phase);
            normalize_harness_control(&mut decision, &mut risks);
            return (decision, risks);
        }

        // ── final 轮：执行完整校验 ──

        // R3.1 / R3.2 / R3.3 必填 + 严格枚举
        let risk_level = check_required_enum(
            self.risk_level.clone(),
            "risk_level",
            RISK_LEVEL_VALUES,
            &mut risks,
        );
        let knowledge_need = check_required_enum(
            self.knowledge_need.clone(),
            "knowledge_need",
            KNOWLEDGE_NEED_VALUES,
            &mut risks,
        );
        let run_mode = check_required_enum(
            self.run_mode.clone(),
            "run_mode",
            RUN_MODE_VALUES,
            &mut risks,
        );
        let autonomy_mode = check_required_enum(
            self.autonomy_mode.clone(),
            "autonomy_mode",
            AUTONOMY_MODE_VALUES,
            &mut risks,
        );
        // H9 universal-domain-adaptation：conversationMode 允许集合从 runtime
        // 注入（active DomainProfile.conversation_modes，由 gateway 在加载 profile
        // 后写入 runtime.allowed_conversation_modes）。空时回落到内置销售域四模式
        // `CONVERSATION_MODE_VALUES`（DEFAULT 逐字等价 + 兼容 PBT/无 profile 入口）。
        let conversation_mode_values: Vec<&str> = if runtime.allowed_conversation_modes.is_empty() {
            CONVERSATION_MODE_VALUES.to_vec()
        } else {
            runtime
                .allowed_conversation_modes
                .iter()
                .map(String::as_str)
                .collect()
        };
        let conversation_mode = check_required_enum(
            self.conversation_mode.clone(),
            "conversation_mode",
            &conversation_mode_values,
            &mut risks,
        );
        let needs_review = check_required_bool(self.needs_review, "needs_review", &mut risks);
        let consolidation_needed = check_required_bool(
            self.consolidation_needed,
            "consolidation_needed",
            &mut risks,
        );
        let operation_state =
            check_required_string(self.operation_state.clone(), "operation_state", &mut risks);

        // R1.3 7 字段始终必填（trim 后非空）
        let user_understanding = check_required_string(
            self.user_understanding.clone(),
            "user_understanding",
            &mut risks,
        );
        let relationship_read = check_required_string(
            self.relationship_read.clone(),
            "relationship_read",
            &mut risks,
        );
        let operation_goal =
            check_required_string(self.operation_goal.clone(), "operation_goal", &mut risks);
        let knowledge_need_reason = check_required_string(
            self.knowledge_need_reason.clone(),
            "knowledge_need_reason",
            &mut risks,
        );
        let memory_update_reason = check_required_string(
            self.memory_update_reason.clone(),
            "memory_update_reason",
            &mut risks,
        );
        let self_critique =
            check_required_string(self.self_critique.clone(), "self_critique", &mut risks);
        let risk_self_check =
            check_required_string(self.risk_self_check.clone(), "risk_self_check", &mut risks);

        // R1.4 互斥必填（whyShouldReply / whySkipReply 由 should_reply 决定）
        let should_reply = self.should_reply.unwrap_or(false);
        let why_should_reply = self.why_should_reply.clone().unwrap_or_default();
        let why_skip_reply = self.why_skip_reply.clone().unwrap_or_default();

        if should_reply {
            if !is_valid_reply_reason(&why_should_reply, 10, 6) {
                risks.push("missing_required_field:why_should_reply".to_string());
            }
        } else if !is_valid_reply_reason(&why_skip_reply, 10, 6) {
            risks.push("missing_required_field:why_skip_reply".to_string());
        }

        // R1.5 / R1.6 条件长度判定
        let is_low_routine =
            risk_level == "low" && knowledge_need == "not_required" && !consolidation_needed;
        let is_critical_turn = risk_level == "high"
            || run_mode == "high_risk"
            || knowledge_need == "required"
            || knowledge_need == "insufficient"
            || consolidation_needed;

        if is_critical_turn {
            // 关键变化轮：所有 7 个 R1.3 字段不得使用 `"unchanged"` 且每个 ≥ 20 chars
            let strict_pairs: &[(&str, &str)] = &[
                ("user_understanding", &user_understanding),
                ("relationship_read", &relationship_read),
                ("operation_goal", &operation_goal),
                ("knowledge_need_reason", &knowledge_need_reason),
                ("memory_update_reason", &memory_update_reason),
                ("self_critique", &self_critique),
                ("risk_self_check", &risk_self_check),
            ];
            for (name, value) in strict_pairs {
                if value.is_empty() {
                    // 已被 R1.3 missing_required_field 标记，不重复
                    continue;
                }
                if value.trim() == "unchanged" || count_unicode_chars(value) < 20 {
                    risks.push(format!("insufficient_detail_in_critical_turn:{}", name));
                }
            }

            // R1.6：回复理由（命中那一个）≥ 30 unicode chars 含 ≥ 12 hanzi
            if should_reply {
                if !why_should_reply.is_empty() && !is_valid_reply_reason(&why_should_reply, 30, 12)
                {
                    risks.push("insufficient_detail_in_critical_turn:why_should_reply".to_string());
                }
            } else if !why_skip_reply.is_empty() && !is_valid_reply_reason(&why_skip_reply, 30, 12)
            {
                risks.push("insufficient_detail_in_critical_turn:why_skip_reply".to_string());
            }
        } else if is_low_routine {
            // 低风险常规轮：5 字段（user_understanding / relationship_read /
            // operation_goal / memory_update_reason / risk_self_check）允许
            // `unchanged` 短形式或任意长度的简短陈述（已通过 R1.3 非空校验即可）；
            // 2 字段（knowledge_need_reason / self_critique）需 ≥ 6 unicode chars。
            let strict_pairs: &[(&str, &str)] = &[
                ("knowledge_need_reason", &knowledge_need_reason),
                ("self_critique", &self_critique),
            ];
            for (name, value) in strict_pairs {
                if value.is_empty() {
                    continue;
                }
                if count_unicode_chars(value) < 6 {
                    risks.push(format!("insufficient_detail_in_critical_turn:{}", name));
                }
            }
        }
        // 其它情形（medium 风险等）：R1.3 已保证非空即可，无额外长度要求。

        // ── 构造 AgentDecision ──
        let mut decision = AgentDecision {
            risk_level,
            knowledge_need,
            run_mode,
            autonomy_mode,
            needs_review,
            consolidation_needed,
            operation_state: if operation_state.is_empty() {
                None
            } else {
                Some(operation_state)
            },
            decision_phase: phase,
            user_understanding,
            relationship_read,
            operation_goal,
            knowledge_need_reason,
            memory_update_reason,
            self_critique,
            why_should_reply,
            why_skip_reply,
            risk_self_check,
            should_reply,
            reply_text: self.reply_text.clone().unwrap_or_default(),
            tool_calls: self.tool_calls.clone().unwrap_or_default(),
            agent_generated_signals: self.agent_generated_signals.clone().unwrap_or_default(),
            conversation_mode: if conversation_mode.is_empty() {
                default_conversation_mode()
            } else {
                conversation_mode
            },
            conversation_mode_reason: self
                .conversation_mode_reason
                .clone()
                .filter(|s| !s.trim().is_empty()),
            ..AgentDecision::default()
        };

        // 把既有 carry-through 字段从 raw 拷过去（避免 promote 把它们丢失）。
        carry_through_fields(self, &mut decision);
        normalize_harness_control(&mut decision, &mut risks);
        sanitize_semantic_assessment(&mut decision, &mut risks);

        (decision, risks)
    }
}

fn clear_deferred_fields(decision: &mut AgentDecision) {
    decision.profile_update = None;
    decision.tags.clear();
    decision.tag_evidence_turns.clear();
    decision.stage_evidence_turns.clear();
    decision.stage_explicit_intent = false;
    decision.bayesian_observations.clear();
    decision.customer_stage = None;
    decision.intent_level = None;
    decision.domain_signals.clear();
    decision.dimension_display_names.clear();
    decision.follow_up_policy = None;
    decision.profile_attributes.clear();
    // `intent_analysis` carries the Reply Agent's semantic contract.  It is part of the
    // send-time review input, not a deferred projection field, so keep it across promotion.
    decision.next_best_action.clear();
    decision.cooldown_until = None;
    decision.product_fit_score = None;
    decision.objections_detected.clear();
    decision.recommended_resource_ids.clear();
    decision.operating_memory_update.clear();
    decision.memory_candidates.clear();
    decision.memory_write_score = 0;
    decision.consolidation_needed = false;
    decision.memory_update.clear();
    decision.agent_generated_signals.clear();
    decision.user_understanding.clear();
    decision.relationship_read.clear();
    decision.operation_goal.clear();
    decision.knowledge_need_reason.clear();
    decision.memory_update_reason.clear();
    decision.self_critique.clear();
}

fn normalize_harness_control(decision: &mut AgentDecision, risks: &mut Vec<String>) {
    let escalation_needed = decision
        .escalation_request
        .as_ref()
        .is_some_and(|request| request.needed);
    let inferred = || {
        infer_next_step(
            &decision.decision_phase,
            decision.should_reply,
            escalation_needed,
        )
    };
    let normalized = decision.next_step.trim();
    if normalized.is_empty() {
        decision.next_step = inferred();
    } else if !NEXT_STEP_VALUES.contains(&normalized) {
        risks.push(format!("invalid_enum_value:next_step:{normalized}"));
        decision.next_step = inferred();
    } else {
        decision.next_step = normalized.to_string();
    }

    if escalation_needed && decision.next_step != "ask_principal" {
        risks.push("next_step_inconsistent:escalation_request".to_string());
        decision.next_step = "ask_principal".to_string();
    }

    if decision.decision_phase == RAW_TOOL_CALLING
        && !matches!(decision.next_step.as_str(), "retrieve" | "verify")
    {
        risks.push("next_step_inconsistent:tool_calling".to_string());
        decision.next_step = if decision.verification.needed {
            "verify".to_string()
        } else {
            "retrieve".to_string()
        };
    }
    if decision.decision_phase == RAW_FINAL
        && matches!(
            decision.next_step.as_str(),
            "retrieve" | "verify" | "repair"
        )
    {
        risks.push("next_step_inconsistent:final".to_string());
        decision.next_step = inferred();
    }

    decision.claim_manifest.retain(|claim| {
        let valid = !claim.claim_id.trim().is_empty() && !claim.text.trim().is_empty();
        if !valid {
            risks.push("draft_claim_manifest_entry_invalid".to_string());
        }
        valid
    });
    if decision.verification.needed && decision.verification.reason.trim().is_empty() {
        risks.push("verification_reason_missing".to_string());
    }
    if decision.commitment_updates.len() > 8 {
        risks.push("commitment_updates_over_limit".to_string());
    }
    let mut commitment_ids = Vec::with_capacity(decision.commitment_updates.len());
    for update in &mut decision.commitment_updates {
        update.commitment_id = update.commitment_id.trim().to_string();
        update.reason = update.reason.trim().to_string();
        if update.commitment_id.is_empty() {
            risks.push("commitment_update_id_missing".to_string());
        }
        if update.reason.is_empty() {
            risks.push("commitment_update_reason_missing".to_string());
        } else if update.reason.chars().count() > 500 {
            risks.push("commitment_update_reason_too_long".to_string());
        }
        if update.action == CommitmentLifecycleAction::Unknown {
            risks.push("commitment_update_action_invalid".to_string());
        }
        if commitment_ids.contains(&update.commitment_id) {
            risks.push("commitment_update_duplicate_id".to_string());
        } else {
            commitment_ids.push(update.commitment_id.clone());
        }
    }
    if let Some(request) = decision.appointment_request.as_ref() {
        if !request.requested {
            decision.appointment_request = None;
        } else if request.request_text.trim().is_empty() {
            risks.push("appointment_request_text_missing".to_string());
            decision.appointment_request = None;
        }
    }
}

/// Validate the Reply Agent's optional semantic contract without classifying natural-language
/// content.  The contract is advisory input for the independent reviewers: malformed self-report
/// is removed and audited, while the candidate still proceeds through the full Reviewer and Claim
/// Gate path.  Missing contracts remain compatible with older prompt packs and are handled by
/// those independent gates rather than becoming a hard protocol block.
fn sanitize_semantic_assessment(decision: &mut AgentDecision, risks: &mut Vec<String>) {
    let Some(assessment) = decision
        .intent_analysis
        .get_document("semanticAssessment")
        .ok()
        .cloned()
    else {
        return;
    };

    let invalid = validate_semantic_assessment_shape(
        &assessment,
        &decision.reply_text,
        decision.should_reply,
        &decision.knowledge_need,
    )
    .err();
    if let Some(field) = invalid {
        decision.intent_analysis.remove("semanticAssessment");
        risks.push(format!("semantic_contract_invalid:{field}"));
    }
}

fn validate_semantic_assessment_shape(
    assessment: &Document,
    _reply_text: &str,
    should_reply: bool,
    top_level_knowledge_need: &str,
) -> Result<(), &'static str> {
    fn required_text<'a>(
        document: &'a Document,
        key: &'static str,
    ) -> Result<&'a str, &'static str> {
        document
            .get_str(key)
            .ok()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or(key)
    }
    fn enum_value(
        document: &Document,
        key: &'static str,
        allowed: &[&str],
    ) -> Result<(), &'static str> {
        let value = required_text(document, key)?;
        if allowed.iter().any(|candidate| *candidate == value) {
            Ok(())
        } else {
            Err(key)
        }
    }

    required_text(assessment, "intent")?;
    required_text(assessment, "reason")?;
    enum_value(assessment, "speechAct", SEMANTIC_SPEECH_ACT_VALUES)?;
    enum_value(assessment, "subject", SEMANTIC_SUBJECT_VALUES)?;
    enum_value(
        assessment,
        "assertionStatus",
        SEMANTIC_ASSERTION_STATUS_VALUES,
    )?;
    let semantic_knowledge_need = required_text(assessment, "knowledgeNeed")?;
    if !SEMANTIC_KNOWLEDGE_NEED_VALUES.contains(&semantic_knowledge_need) {
        return Err("knowledgeNeed");
    }
    let response_disposition = required_text(assessment, "responseDisposition")?;
    if !SEMANTIC_RESPONSE_DISPOSITION_VALUES.contains(&response_disposition) {
        return Err("responseDisposition");
    }
    if should_reply == matches!(response_disposition, "silent" | "cooldown") {
        return Err("responseDispositionConsistency");
    }
    let knowledge_consistent = match (top_level_knowledge_need, semantic_knowledge_need) {
        ("not_required", "not_required")
        | ("required", "required")
        | ("insufficient", "uncertain" | "required") => true,
        // Legacy/custom prompts may omit or conservatively widen the top-level route.  Do not
        // reject a semantically valid assessment solely because the route is more conservative.
        ("required", "uncertain") | ("insufficient", "not_required") => true,
        _ => false,
    };
    if !knowledge_consistent {
        return Err("knowledgeNeedConsistency");
    }

    let risk = assessment
        .get_document("semanticRisk")
        .map_err(|_| "semanticRisk")?;
    for key in ["content", "pressure", "boundary", "privacy"] {
        enum_value(risk, key, SEMANTIC_RISK_VALUES)?;
    }
    let confidence = risk
        .get_f64("confidence")
        .map_err(|_| "semanticRisk.confidence")?;
    if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
        return Err("semanticRisk.confidence");
    }

    let claims = assessment.get_array("claims").map_err(|_| "claims")?;
    for claim in claims {
        let claim = claim.as_document().ok_or("claims[]")?;
        required_text(claim, "text")?;
        required_text(claim, "reason")?;
        claim
            .get_bool("requiresEvidence")
            .map_err(|_| "claims[].requiresEvidence")?;
    }
    Ok(())
}

/// 检查"该回复 / 不回复理由"长度与汉字数量是否达标（R1.4 / R1.6）。
fn is_valid_reply_reason(value: &str, min_chars: usize, min_hanzi: usize) -> bool {
    if value.trim().is_empty() {
        return false;
    }
    count_unicode_chars(value) >= min_chars && count_hanzi(value) >= min_hanzi
}

/// `decision_phase == "tool_calling"` 中间轮：构造最小 AgentDecision，仅保留
/// tool_calls + carry-through，不落 9 字段（R1.10 / R4.1）。
fn build_tool_calling_decision(raw: RawAgentDecision, phase: String) -> AgentDecision {
    let mut decision = AgentDecision {
        decision_phase: phase,
        tool_calls: raw.tool_calls.clone().unwrap_or_default(),
        agent_generated_signals: raw.agent_generated_signals.clone().unwrap_or_default(),
        ..AgentDecision::default()
    };
    carry_through_fields(raw, &mut decision);
    // tool_calling 中间轮强制丢弃 reply_text / should_reply（与 R4.1.b 协议一致：
    // 中间轮意外填了 reply_text 时本函数只保证默认安全，清空它）
    decision.reply_text = String::new();
    decision.should_reply = false;
    decision
}

/// `runtime.autonomy_protocol_enabled == false` 时构造最小 AgentDecision，
/// 跳过全部校验（R11 sunset 灰度路径）。
fn build_minimal_decision(raw: RawAgentDecision) -> AgentDecision {
    let phase = match raw.decision_phase.as_deref().map(str::trim) {
        Some(RAW_TOOL_CALLING) => RAW_TOOL_CALLING.to_string(),
        _ => RAW_FINAL.to_string(),
    };
    let mut decision = AgentDecision {
        decision_phase: phase,
        risk_level: raw.risk_level.clone().unwrap_or_default(),
        knowledge_need: raw.knowledge_need.clone().unwrap_or_default(),
        run_mode: raw.run_mode.clone().unwrap_or_default(),
        autonomy_mode: raw.autonomy_mode.clone().unwrap_or_default(),
        needs_review: raw.needs_review.unwrap_or(false),
        consolidation_needed: raw.consolidation_needed.unwrap_or(false),
        operation_state: raw.operation_state.clone(),
        user_understanding: raw.user_understanding.clone().unwrap_or_default(),
        relationship_read: raw.relationship_read.clone().unwrap_or_default(),
        operation_goal: raw.operation_goal.clone().unwrap_or_default(),
        knowledge_need_reason: raw.knowledge_need_reason.clone().unwrap_or_default(),
        memory_update_reason: raw.memory_update_reason.clone().unwrap_or_default(),
        self_critique: raw.self_critique.clone().unwrap_or_default(),
        why_should_reply: raw.why_should_reply.clone().unwrap_or_default(),
        why_skip_reply: raw.why_skip_reply.clone().unwrap_or_default(),
        risk_self_check: raw.risk_self_check.clone().unwrap_or_default(),
        should_reply: raw.should_reply.unwrap_or(false),
        reply_text: raw.reply_text.clone().unwrap_or_default(),
        tool_calls: raw.tool_calls.clone().unwrap_or_default(),
        agent_generated_signals: raw.agent_generated_signals.clone().unwrap_or_default(),
        conversation_mode: raw
            .conversation_mode
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(default_conversation_mode),
        conversation_mode_reason: raw
            .conversation_mode_reason
            .clone()
            .filter(|s| !s.trim().is_empty()),
        ..AgentDecision::default()
    };
    carry_through_fields(raw, &mut decision);
    decision
}

/// 把既有非 9 自治协议字段（profile / tags / memory / signals 等）从 Raw 透传到
/// `AgentDecision`，避免 promote 把它们丢失。
fn carry_through_fields(raw: RawAgentDecision, decision: &mut AgentDecision) {
    if let Some(v) = raw.next_step {
        decision.next_step = v;
    }
    if let Some(v) = raw.claim_manifest {
        decision.claim_manifest = v;
    }
    if let Some(v) = raw.verification {
        decision.verification = v;
    }
    if raw.appointment_request.is_some() {
        decision.appointment_request = raw.appointment_request;
    }
    if let Some(v) = raw.used_knowledge_ids {
        decision.used_knowledge_ids = v;
    }
    if let Some(v) = raw.quoted_product_ids {
        decision.quoted_product_ids = v;
    }
    if let Some(v) = raw.safe_claims_used {
        decision.safe_claims_used = v;
    }
    if let Some(v) = raw.knowledge_route {
        // KnowledgeRouteResult 用作 AgentDecision.knowledge_route 的承载在后续 wave
        // 引入；本期 AgentDecision 暂未持有该字段，故先吞掉，避免 promote 过程
        // 把它丢失也不报 dead-store 警告。
        let _ = v;
    }
    if let Some(v) = raw.profile_update {
        decision.profile_update = Some(v);
    }
    if let Some(v) = raw.tags {
        decision.tags = v;
    }
    // 子计划2：标签/stage 证据序位 + 明示意图标志透传。只在 Some 时覆盖，None 保持默认空。
    if let Some(v) = raw.tag_evidence_turns {
        decision.tag_evidence_turns = v;
    }
    if let Some(v) = raw.stage_evidence_turns {
        decision.stage_evidence_turns = v;
    }
    if let Some(v) = raw.stage_explicit_intent {
        decision.stage_explicit_intent = v;
    }
    // 子计划4：贝叶斯维度观察透传（纯观测，永不驱动决策）。
    if let Some(v) = raw.bayesian_observations {
        decision.bayesian_observations = v;
    }
    if raw.customer_stage.is_some() {
        decision.customer_stage = raw.customer_stage;
    }
    if raw.intent_level.is_some() {
        decision.intent_level = raw.intent_level;
    }
    if let Some(v) = raw.domain_signals {
        // G1：非销售维度的开放容器从 LLM JSON `domainSignals` 透传。销售域 LLM
        // 不输出该键 → None → 不触；典型行业由 normalize_domain_signals 再镜像 typed。
        if !v.is_empty() {
            decision.domain_signals = v;
        }
    }
    if let Some(v) = raw.dimension_display_names {
        // 维度中文名 carry-through（同 namecard/assets 老坑：不透传则 promote 后
        // 永远空、LLM 产的中文名被静默丢弃，收件箱又回落英文）。仅非空覆盖。
        if !v.is_empty() {
            decision.dimension_display_names = v;
        }
    }
    if raw.last_commitment.is_some() {
        decision.last_commitment = raw.last_commitment;
    }
    if let Some(c) = raw.commitment {
        // 只在 text 非空时透传，避免 LLM 输出空壳 commitment 对象覆盖 last_commitment 路径。
        if !c.text.trim().is_empty() {
            // gateway 落库入口判断 last_commitment 是否非空；LLM 只给结构化 commitment
            // 而没给 last_commitment 时，用 commitment.text 回填，保证承诺不丢、且 due_at
            // 从 commitment 取。
            if decision
                .last_commitment
                .as_deref()
                .map(|s| s.trim().is_empty())
                .unwrap_or(true)
            {
                decision.last_commitment = Some(c.text.clone());
            }
            decision.commitment = Some(c);
        }
    }
    if let Some(v) = raw.commitment_updates {
        decision.commitment_updates = v;
    }
    if raw.follow_up_policy.is_some() {
        decision.follow_up_policy = raw.follow_up_policy;
    }
    if let Some(v) = raw.profile_attributes {
        decision.profile_attributes = v;
    }
    if let Some(v) = raw.intent_analysis {
        decision.intent_analysis = v;
    }
    if let Some(v) = raw.next_best_action {
        decision.next_best_action = v;
    }
    if raw.operation_state_reason.is_some() {
        decision.operation_state_reason = raw.operation_state_reason;
    }
    if raw.operation_state_confidence.is_some() {
        decision.operation_state_confidence = raw.operation_state_confidence;
    }
    if raw.cooldown_until.is_some() {
        decision.cooldown_until = raw.cooldown_until;
    }
    if raw.product_fit_score.is_some() {
        decision.product_fit_score = raw.product_fit_score;
    }
    if let Some(v) = raw.matched_knowledge_ids {
        decision.matched_knowledge_ids = v;
    }
    if raw.forbidden_claim_risk.is_some() {
        decision.forbidden_claim_risk = raw.forbidden_claim_risk;
    }
    if let Some(v) = raw.objections_detected {
        decision.objections_detected = v;
    }
    if let Some(v) = raw.recommended_resource_ids {
        decision.recommended_resource_ids = v;
    }
    if let Some(v) = raw.operating_memory_update {
        decision.operating_memory_update = v;
    }
    if let Some(v) = raw.memory_candidates {
        decision.memory_candidates = v;
    }
    if let Some(v) = raw.memory_write_score {
        decision.memory_write_score = v;
    }
    if let Some(v) = raw.memory_update {
        decision.memory_update = v;
    }
    if raw.context_pack_version.is_some() {
        decision.context_pack_version = raw.context_pack_version;
    }
    if raw.follow_up.is_some() {
        decision.follow_up = raw.follow_up;
    }
    if raw.escalation_request.is_some() {
        decision.escalation_request = raw.escalation_request;
    }
    // media-asset Task 8（硬伤① carry-through）：只保留带有效结构化 id 的动作。部分
    // provider 会为可选对象输出空壳 `{}`；它表达的是 no-op，不能升级成真实发送副作用。
    if let Some(v) = raw.assets_to_send {
        decision.assets_to_send = v
            .into_iter()
            .filter(|directive| !directive.asset_id.trim().is_empty())
            .collect();
    }
    // 名片引荐 carry-through：LLM 选的名片若不在此透传，promote 后 namecard_to_send
    // 永远为 None、名片被静默丢弃。空 cardId 是 no-op，不构成可执行引荐。
    if let Some(v) = raw
        .namecard_to_send
        .filter(|directive| !directive.card_id.trim().is_empty())
    {
        decision.namecard_to_send = Some(v);
    }
    // 渐进式三档 + 充分性自评（2026-06-23）：LLM 输出的自评字段若不在此透传，
    // promote 后永远为空字符串、自评结果被静默丢弃。只在 Some 时覆盖，None 保持默认空。
    if let Some(v) = raw.sufficiency {
        decision.sufficiency = v;
    }
    if let Some(v) = raw.missing_tier {
        decision.missing_tier = v;
    }
    if let Some(v) = raw.clarification_intent {
        decision.clarification_intent = v;
    }
    // 自治协议 9 字段已在 promote 主路径填好（或在 minimal/tool_calling 分支处理），
    // 此处不再覆盖，避免 final 轮的 trim 后值被原始 Some(空白) 覆盖。
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FollowUpDecision {
    #[serde(default)]
    pub needed: bool,
    #[serde(default)]
    pub run_at: String,
    #[serde(default)]
    pub content: String,
}

/// LLM 输出的结构化承诺（PR-D）：在 `lastCommitment` 字符串之外可选携带 `dueAt`。
/// 让 Planner 直接拿到承诺到期时间，而非全部走 from_plain_text（due_at=None）兜底。
/// 向后兼容：LLM 不输出 `commitment` 时该字段为 None，回落旧的 last_commitment 路径。
#[derive(Debug, Serialize, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CommitmentDecision {
    #[serde(default)]
    pub text: String,
    /// RFC3339 到期时间；空串 / 非法格式时落库为 due_at=None，由 planner 的
    /// created_at 兜底接住（见 [`super::super::planner`] commitment fallback）。
    #[serde(default)]
    pub due_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommitmentLifecycleAction {
    Fulfilled,
    Cancelled,
    Superseded,
    Expired,
    #[serde(other)]
    Unknown,
}

impl Default for CommitmentLifecycleAction {
    fn default() -> Self {
        Self::Unknown
    }
}

/// A typed lifecycle decision about one existing active commitment.
///
/// `reason` is audit/reviewer context, not a customer-facing sentence. A `superseded` action is
/// linked by the runtime to the new commitment created by the same authorized reply.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CommitmentLifecycleDecision {
    #[serde(default)]
    pub commitment_id: String,
    #[serde(default)]
    pub action: CommitmentLifecycleAction,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ReviewScores {
    #[serde(default, deserialize_with = "number_i32")]
    pub human_like: i32,
    #[serde(default, deserialize_with = "number_i32")]
    pub emotional_value: i32,
    /// 反序列化兼容：reviewer prompt 历史上以 `factRisk` 命名该评分键，
    /// 接受 alias 以免 LLM 输出 / 旧持久化文档静默落 0（5→3 闸方法论塌缩遗留）。
    #[serde(default, deserialize_with = "number_i32", alias = "factRisk")]
    pub hallucination_score: i32,
    /// 反序列化兼容：reviewer prompt 历史上以 `productAccuracy` 命名该评分键。
    #[serde(default, deserialize_with = "number_i32", alias = "productAccuracy")]
    pub knowledge_grounding_score: i32,
    /// Phase B / B1：恢复 `pressure_risk` 软闸评分（0-10，越高压迫感越强）。Reviewer 输出，
    /// `review_passed` 与 single-shot revision 通道判定时使用（与 `pressure_risk_block_at`
    /// 等个位数阈值同档比较）。R11 兼容：缺省 `0`，旧 review JSON 反序列化不破坏。
    #[serde(default, deserialize_with = "number_i32")]
    pub pressure_risk: i32,
    /// 渐进式三档+隐私维度(2026-06-23)：边界/隐私安全评分(0-10,越高越安全)。
    /// 判断候选回复是否:(a)泄露对客户的内部画像/评判;(b)暴露AI身份;
    /// (c)暴露幕后决策源(领导)或内部系统信息。≤3视为失败→触发改写。
    /// 向后兼容:缺省0(最保守)。
    #[serde(default, deserialize_with = "number_i32")]
    pub boundary_privacy_safety: i32,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DecisionReviewResult {
    #[serde(default)]
    pub approved: bool,
    #[serde(default)]
    pub scores: ReviewScores,
    #[serde(default)]
    pub formula_breakdown: Document,
    #[serde(default)]
    pub claim_analysis: Document,
    #[serde(default, deserialize_with = "string_or_vec")]
    pub risks: Vec<String>,
    #[serde(default)]
    pub rewrite_instruction: String,
    #[serde(default)]
    pub review_summary: String,

    // ─────────────────────────────────────────────────────────────────
    // agent-autonomy-loop W2 / Task 3.3：R2 / R9 自治回路扩字段。
    //
    // 全部字段均带 `#[serde(default)]`，确保 W2 task 3.1 的 review.rs
    // 二态改造 / 既有 review JSON / Mongo 老数据反序列化时不会因为缺
    // 字段而失败（向后兼容）；写入路径上由 task 3.4 finalize 阶段填充。
    //
    // - `needs_revision / revision_direction`：R2.1 — Review Agent 输出
    //   "需要重写吗 + 重写方向"，由 task 3.4 的 single-shot revision 控
    //   制流消费。
    // - `should_hold / hold_reason / hold_category`：R2.1 / R2.6 — AI 策
    //   略性暂缓，类别仅允许 `held_by_ai_policy / blocked_by_safety_guard
    //   / ai_waiting_for_more_context` 三选一（详见
    //   [`assert_hold_category_valid`]）。
    // - `self_critique_addressed`：R2.10 — 第二轮 review 显式表明 Reply
    //   Agent 是否解决了上一轮的 selfCritique。
    // - `revision_applied / final_review_status`：R9.1 / R9.8 — 与
    //   `agent_run_logs` 同步落库，便于前端 horizon 聚合（详见
    //   `src/agent/run_envelope.rs::FINAL_REVIEW_STATUS_VALUES`）。
    // ─────────────────────────────────────────────────────────────────
    #[serde(default)]
    pub needs_revision: bool,
    #[serde(default)]
    pub revision_direction: String,
    #[serde(default)]
    pub should_hold: bool,
    #[serde(default)]
    pub hold_reason: String,
    #[serde(default)]
    pub hold_category: String,
    #[serde(default)]
    pub self_critique_addressed: bool,
    #[serde(default)]
    pub revision_applied: bool,
    #[serde(default)]
    pub final_review_status: String,
}

/// agent-autonomy-loop W2 / Task 3.3：`hold_category` 允许枚举（R2.2 / R9.8）。
///
/// 严格三选一，禁止 `held_for_human / human_required / waiting_for_human` 等
/// 暗示人工接管的取值（违反全 AI 自治流程的产品定位）。
pub const HOLD_CATEGORY_HELD_BY_AI_POLICY: &str = "held_by_ai_policy";
pub const HOLD_CATEGORY_BLOCKED_BY_SAFETY_GUARD: &str = "blocked_by_safety_guard";
pub const HOLD_CATEGORY_AI_WAITING_FOR_MORE_CONTEXT: &str = "ai_waiting_for_more_context";

/// `hold_category` 允许取值集合。
pub const HOLD_CATEGORY_VALUES: &[&str] = &[
    HOLD_CATEGORY_HELD_BY_AI_POLICY,
    HOLD_CATEGORY_BLOCKED_BY_SAFETY_GUARD,
    HOLD_CATEGORY_AI_WAITING_FOR_MORE_CONTEXT,
];

/// `hold_category` 严禁取值（R2.7 业务语义保护 + R9.8）。
#[allow(dead_code)]
const HOLD_CATEGORY_FORBIDDEN_VALUES: &[&str] = &[
    "held_for_human",
    "human_required",
    "waiting_for_human",
    "handoff_to_human",
    "manual_takeover",
];

/// agent-autonomy-loop W2 / Task 3.3：`autonomy_hold_category_invalid` 事件 kind 常量。
///
/// `assert_hold_category_valid` 在原值非法时把 [`DecisionReviewResult::hold_category`]
/// 强制改写为 `held_by_ai_policy` 并指示调用方写一条 `agent_events` 记录；事件 kind 由
/// 调用方使用此常量持有，避免散落字面量（详见 R2.6 / R9.8）。
pub const EVENT_AUTONOMY_HOLD_CATEGORY_INVALID: &str = "autonomy_hold_category_invalid";

/// 描述 `assert_hold_category_valid` 是否对原值进行了改写。
///
/// `Unchanged` 表示原值合法、未改写；`Coerced { original }` 表示原值被强制改为
/// [`HOLD_CATEGORY_HELD_BY_AI_POLICY`]，调用方 SHALL 在 `agent_events` 写一条
/// `kind="autonomy_hold_category_invalid"` 事件，detail 含原始值。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HoldCategoryAssertion {
    Unchanged,
    Coerced { original: String },
}

/// agent-autonomy-loop W2 / Task 3.3：校验并矫正 [`DecisionReviewResult::hold_category`]。
///
/// 行为（对应 R2.6 / R9.8）：
/// * `should_hold == false`：
///   - 空字符串 → 视为合法（`Unchanged`），不改写；
///   - 非空但不在 [`HOLD_CATEGORY_VALUES`] 内 → 强制改为
///     `held_by_ai_policy` 并返回 `Coerced { original }`；
///   - 含禁用 `held_for_human / human_required / ...` 等取值 → 同上 `Coerced`。
/// * `should_hold == true`：
///   - 空字符串 / 仅含空白 → 默认填 `held_by_ai_policy`，返回 `Coerced { original }`；
///   - 合法枚举（三选一）→ `Unchanged`；
///   - 其它脏值 → 强制改为 `held_by_ai_policy`，返回 `Coerced { original }`。
///
/// 调用方 SHALL 在返回 `Coerced { original }` 时往 `agent_events` 写一条 kind =
/// [`EVENT_AUTONOMY_HOLD_CATEGORY_INVALID`] 的事件，details 含 `original` 原值，
/// 便于运维追溯哪些 Review Agent 输出违反了业务语义保护约束。
///
/// 该函数是纯函数 + 单一可变引用，不直接写库（避免在 review/types 模块引入
/// `db.events()` 依赖反向耦合），事件埋点由 W2 task 3.2 / task 3.4 的 finalize
/// 路径完成。
pub(crate) fn assert_hold_category_valid(
    review: &mut DecisionReviewResult,
) -> HoldCategoryAssertion {
    let original = review.hold_category.clone();
    let trimmed = original.trim();

    // should_hold=false 时空字符串视为合法占位（review 未触发 hold 路径）
    if !review.should_hold && trimmed.is_empty() {
        // 同步把字段裁剪为标准空串，避免遗留 "  " 等空白脏值
        review.hold_category = String::new();
        return HoldCategoryAssertion::Unchanged;
    }

    // 合法枚举（三选一）→ 同步把字段标准化为去 trim 后的字面量
    if HOLD_CATEGORY_VALUES.iter().any(|v| *v == trimmed) {
        if review.hold_category != trimmed {
            review.hold_category = trimmed.to_string();
        }
        return HoldCategoryAssertion::Unchanged;
    }

    // 其它情形（禁用值 / 未知字符串 / should_hold=true 但空）→ 强制改写
    review.hold_category = HOLD_CATEGORY_HELD_BY_AI_POLICY.to_string();
    HoldCategoryAssertion::Coerced { original }
}

/// 调用方便利函数：判断给定 hold_category 取值是否属于禁用的 human-handoff 语义。
///
/// 用于事件埋点 / lint 报警等场景区分"正常未填"与"违反业务语义保护"两类
/// 异常源（详见 R2.7）。
#[allow(dead_code)]
pub(crate) fn is_forbidden_hold_category(value: &str) -> bool {
    HOLD_CATEGORY_FORBIDDEN_VALUES.contains(&value.trim())
}

/// Knowledge Agent's semantic assessment of whether the opened evidence can answer the turn.
///
/// The runtime consumes this closed protocol only for capability routing. It never infers the
/// value from customer text, phrases, or a keyword list.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeAnswerability {
    Supported,
    PartiallySupported,
    Unsupported,
    NotRequired,
    #[serde(other)]
    Unknown,
}

impl Default for KnowledgeAnswerability {
    fn default() -> Self {
        Self::Unknown
    }
}

/// The authority class the Knowledge Agent says is needed to close an evidence gap.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeRequiredAuthority {
    None,
    Customer,
    AuthorizedOperator,
    LicensedProfessional,
    ExternalSystem,
    #[serde(other)]
    Unknown,
}

impl Default for KnowledgeRequiredAuthority {
    fn default() -> Self {
        Self::Unknown
    }
}

/// Model-selected next capability after knowledge research.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeNextStep {
    Respond,
    ClarifyCustomer,
    AskPrincipal,
    Defer,
    #[serde(other)]
    Unknown,
}

impl Default for KnowledgeNextStep {
    fn default() -> Self {
        Self::Unknown
    }
}

/// Typed semantic hand-off from the Knowledge Agent to the shared turn Harness.
///
/// `authority_question` is internal control data for the configured principal channel. The Reply
/// Agent must render its own customer-facing language and must not expose this structure.
#[derive(Debug, Serialize, Deserialize, Default, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeResolution {
    #[serde(default)]
    pub answerability: KnowledgeAnswerability,
    #[serde(default)]
    pub required_authority: KnowledgeRequiredAuthority,
    #[serde(default)]
    pub recommended_next_step: KnowledgeNextStep,
    #[serde(default, deserialize_with = "string_or_vec")]
    pub missing_information: Vec<String>,
    #[serde(default)]
    pub authority_question: String,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeRouteResult {
    #[serde(default, deserialize_with = "string_or_vec")]
    pub needed_categories: Vec<String>,
    #[serde(default, deserialize_with = "string_or_vec")]
    pub selected_knowledge_ids: Vec<String>,
    #[serde(default, deserialize_with = "string_or_vec")]
    pub selected_document_ids: Vec<String>,
    #[serde(default, deserialize_with = "string_or_vec")]
    pub selected_chunk_ids: Vec<String>,
    #[serde(default, deserialize_with = "string_or_vec")]
    pub selected_slice_reasons: Vec<String>,
    #[serde(default)]
    pub risk_level: String,
    #[serde(default)]
    pub requires_evidence: bool,
    #[serde(default)]
    pub knowledge_coverage: String,
    #[serde(default, deserialize_with = "string_or_vec")]
    pub missing_knowledge: Vec<String>,
    #[serde(default)]
    pub reason: String,
    /// AI-owned semantic research result. Deterministic code may route capabilities from these
    /// enums, but never derives them by scanning natural-language customer content.
    #[serde(default)]
    pub resolution: KnowledgeResolution,
    #[serde(default, deserialize_with = "document_vec")]
    pub tool_trace: Vec<Document>,
    #[serde(default, deserialize_with = "string_or_vec")]
    pub evidence_excerpts: Vec<String>,
    /// 自学习采集管道 S4：召回倾向占位（recall propensity）。
    ///
    /// 记录本次检索每条被选 chunk 的排名 / 排序分 / 候选池大小，为未来 IPW
    /// （inverse-propensity weighting）纠偏召回偏置留位——没有 propensity 就无法
    /// 区分"chunk 真的好"与"chunk 只是恰好排前面被高频选中"。本阶段只采集落库，
    /// 不参与任何加权。随 `knowledge_router` 既有 `to_document(route)` 自动持久化
    /// 到 `knowledge_usage_logs.route_result`。缺字段时反序列化为空 Vec（R11 安全）。
    #[serde(default)]
    pub selected_chunk_rankings: Vec<SelectedChunkRanking>,
    /// B2：本轮 `selected_chunk_ids` 是否来自 `fallback_rank` 弱回填，而非 Knowledge
    /// Agent 的 citation。
    ///
    /// **为什么必须单独一个字段**：`selected_chunk_ids` 同时承担两种语义——「导航候选」
    /// （喂 prompt 当参考材料）与「可授权证据」（喂 `used_knowledge_ids` → 产品背书硬闸
    /// `compute_verified_chunks`）。回填候选只满足前者：它由静态排序取 top-N 得来，
    /// **无最低相关度门槛**、未经 citation/quote/anchor 校验，与本轮候选回复里的产品
    /// claim 没有任何绑定关系。`verified` 只证明该 chunk 自身经过管理员审核，不证明它与
    /// 当前 query 相关。若让它进入 `used_knowledge_ids`，`used ∩ verified` 非空即放行，
    /// 会从结构上架空 `blocked_unverified_product_claim`。
    ///
    /// 该字段只由服务端在 `route_operation_knowledge_inner` 内赋值（`KnowledgeRouteResult`
    /// 无 LLM 反序列化路径），LLM 无法伪造。缺字段反序列化为 `false`——历史 route 文档
    /// 按「非回填」处理，与本改动前的行为一致。
    #[serde(default)]
    pub selected_chunks_are_fallback: bool,
    /// B5（知识窗口错位修复）：agent 引用并经 DB 直查复核的**窗外** chunk 完整文档。
    ///
    /// 为什么需要携带：运行时静态窗口（`load_operation_knowledge`）只装 top-200
    /// （priority/updated_at 倒排），而 knowledge_agent 的 `open_chunk` 按 `_id`
    /// 直查、不受窗口限制——agent 完全可能合法引用第 201 名的 verified chunk。
    /// 此前 router 把 cited 与窗口求交，窗外引用被当成"不在 corpus"降格成
    /// fallback 弱回填；修复后由本字段携带窗外文档，
    /// `select_operation_knowledge_chunks` 在窗内查不到该 id 时从这里补齐，使
    /// prompt 注入与 `compute_verified_chunks`（R5.4 产品背书）拿到同一批文档。
    /// 装入前已按与窗口逐字同口径的过滤（workspace + domain + status=active +
    /// integrity_status=verified + account 归属）复核，verified-only 语义只增真。
    ///
    /// `#[serde(skip)]`：纯运行时载体，不进任何序列化面（`to_document` →
    /// knowledge_usage_logs.route_result / run_envelope.knowledge_route /
    /// simulation 报告 / AgentDecision.knowledge_route），持久化形状零变化；
    /// 反序列化时恒为默认空 Vec（R11 安全）。
    #[serde(skip)]
    pub cited_verified_chunks: Vec<OperationKnowledgeChunk>,
}

/// 自学习采集管道 S4：单条被选 chunk 的召回倾向快照。
///
/// 只承载"该 chunk 在本次检索里如何被排到"的客观量，不含任何质量判断——质量
/// 判断（reviewer 是否采纳）由 `knowledge_usage_logs.review_approved` 另行承载，
/// 两层刻意分离（Law ③ 观察/解释分层）。
#[derive(Debug, Serialize, Deserialize, Default, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SelectedChunkRanking {
    /// chunk 的 hex id。
    #[serde(default)]
    pub chunk_id: String,
    /// 0-based 排名（0 = 排序后第一名）。
    #[serde(default)]
    pub rank: usize,
    /// 排序分（既有 `wiki_type_priority × dynamic_confidence` 等综合分）。
    #[serde(default)]
    pub score: f64,
    /// 本次检索的候选池大小（计算 propensity 的分母基数）。
    #[serde(default)]
    pub pool_size: usize,
    /// 排序来源标记（如 `"fallback_rank"` / `"tool_loop"`），便于区分召回路径。
    #[serde(default)]
    pub source: String,
    /// P4 探索注入：该 chunk 在本次抽样下**被选中的概率**（propensity）。
    /// 确定性 top-k 模式下为 `None`（等价 1.0，无探索）；探索模式（softmax/ε）
    /// 下记录抽样概率。**本阶段只记录不消费**——为路线图的 IPS/DR off-policy
    /// 纠偏留数据（确定性日志 propensity 非 0 即 1，不补探索则一切 off-policy 非法）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection_prob: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RunPlannerResult {
    #[serde(default)]
    pub risk_level: String,
    #[serde(default)]
    pub context_needs_refresh: bool,
    #[serde(default, deserialize_with = "number_i32")]
    pub memory_change_importance: i32,
    #[serde(default)]
    pub knowledge_required: bool,
    #[serde(default)]
    pub review_mode: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub confidence_override_triggered: bool,
    #[serde(default)]
    pub confidence_override_reason: String,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct KnowledgeRuntime {
    pub documents: Vec<OperationKnowledgeDocument>,
    pub chunks: Vec<OperationKnowledgeChunk>,
}

pub(crate) fn string_or_vec<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
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

pub(crate) fn number_i32<'de, D>(deserializer: D) -> Result<i32, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value_to_i32(&value).unwrap_or_default())
}

pub(crate) fn optional_i32<'de, D>(deserializer: D) -> Result<Option<i32>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value_to_i32(&value))
}

pub(crate) fn document_vec<'de, D>(deserializer: D) -> Result<Vec<Document>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    if let Some(items) = value.as_array() {
        return Ok(items
            .iter()
            .filter_map(|item| to_document(item).ok())
            .collect());
    }
    if let Some(text) = value.as_str() {
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }
        return Ok(vec![doc! {
            "tool": "knowledge.search",
            "reason": text.trim()
        }]);
    }
    Ok(Vec::new())
}

pub(crate) fn value_to_i32(value: &serde_json::Value) -> Option<i32> {
    if value.is_null() {
        return None;
    }
    if let Some(number) = value.as_i64() {
        return Some(number.clamp(i32::MIN as i64, i32::MAX as i64) as i32);
    }
    if let Some(number) = value.as_f64() {
        if number.is_finite() {
            return Some(number.round().clamp(i32::MIN as f64, i32::MAX as f64) as i32);
        }
    }
    value
        .as_str()
        .and_then(|text| text.trim().parse::<f64>().ok())
        .map(|number| number.round().clamp(i32::MIN as f64, i32::MAX as f64) as i32)
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SendGatewayResult {
    #[serde(default)]
    pub allowed: bool,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub policy_blocks: Vec<String>,
    #[serde(default)]
    pub run_mode: String,
    pub message_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserOperationSimulationTurn {
    pub turn: usize,
    pub inbound_text: String,
    pub should_reply: bool,
    pub reply_text: String,
    pub status: String,
    pub decision: Document,
    pub review: Document,
    pub gateway_result: Document,
    pub knowledge_route: Document,
    pub context_pack: Document,
    pub commit_receipt: Document,
    pub memory_preview: Document,
    pub state_transition: Document,
    /// Per-turn shadow timings. These are diagnostic only and never authorize a side effect.
    #[serde(default)]
    pub performance: Document,
}

#[derive(Debug, Clone, Default)]
pub struct ManualContactSend {
    pub content: String,
    pub source: Document,
    pub original_content_locked: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactSendResult {
    pub sent_content: String,
    pub message_id: Option<String>,
    pub review_approved: bool,
    pub gateway_status: String,
    pub gateway_reason: String,
    pub decision_review_id: Option<String>,
}

pub(crate) enum AgentTrigger<'a> {
    Inbound(&'a ConversationMessage),
    FollowUp(&'a AgentTask),
}

impl AgentTrigger<'_> {
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            AgentTrigger::Inbound(_) => "inbound",
            AgentTrigger::FollowUp(_) => "follow_up",
        }
    }
}

/// 从 `Document` 取 i64，缺失时返回默认值。
pub(crate) fn doc_i64(params: Option<&Document>, key: &str, default: i64) -> i64 {
    params
        .and_then(|doc| {
            doc.get_i64(key)
                .ok()
                .or_else(|| doc.get_i32(key).ok().map(i64::from))
        })
        .unwrap_or(default)
}

/// 从 `Document` 取 i32，缺失时返回默认值。
pub(crate) fn doc_i32(params: Option<&Document>, key: &str, default: i32) -> i32 {
    params
        .and_then(|doc| {
            doc.get_i32(key).ok().or_else(|| {
                doc.get_i64(key)
                    .ok()
                    .and_then(|value| i32::try_from(value).ok())
            })
        })
        .unwrap_or(default)
}

/// 从 `Document` 取 bool，缺失时视为 false。
pub(crate) fn doc_bool(doc: &Document, key: &str) -> bool {
    doc.get_bool(key).unwrap_or(false)
}

/// 从 `Document` 取 trim 后非空字符串。
pub(crate) fn doc_string(doc: &Document, key: &str) -> Option<String> {
    doc.get_str(key)
        .ok()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

pub(crate) fn string_array(value: &serde_json::Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(|item| item.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::trim))
                .filter(|item| !item.is_empty())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn optional_string(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToString::to_string)
}

pub(crate) fn non_empty_option(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToString::to_string)
}

pub(crate) fn parse_rfc3339_to_bson(value: &str) -> Option<mongodb::bson::DateTime> {
    mongodb::bson::DateTime::parse_rfc3339_str(value).ok()
}

/// Parse an LLM-proposed follow-up time. Invalid or missing times are rejected:
/// guessing a fallback can turn a future reminder into an immediate duplicate.
pub(crate) fn parse_follow_up_run_at(raw: &str) -> Option<mongodb::bson::DateTime> {
    parse_rfc3339_to_bson(raw.trim())
}

#[cfg(test)]
mod follow_up_run_at_tests {
    use super::parse_follow_up_run_at;

    #[test]
    fn valid_rfc3339_is_accepted() {
        let raw = "2026-06-12T00:00:00Z";
        let dt = parse_follow_up_run_at(raw).expect("valid RFC3339");
        assert_eq!(
            dt.timestamp_millis(),
            mongodb::bson::DateTime::parse_rfc3339_str(raw)
                .unwrap()
                .timestamp_millis(),
        );
    }

    #[test]
    fn empty_string_is_rejected_instead_of_becoming_immediate() {
        assert!(parse_follow_up_run_at("").is_none());
        assert!(parse_follow_up_run_at("   ").is_none());
    }

    #[test]
    fn natural_language_time_is_rejected_instead_of_becoming_immediate() {
        assert!(parse_follow_up_run_at("明天下午").is_none());
    }
}

#[cfg(test)]
mod validate_and_promote_tests {
    //! agent-autonomy-loop W1 / Task 2.3：核心校验路径的内联单元测试。
    //!
    //! 完整覆盖（含 PBT）由 W3 task 2.6 + W6 task 7.* 落地；这里只做最小
    //! sanity check，确保 `validate_and_promote` 在编译通过的同时，
    //! tool_calling / final / sunset / 必填违规 / 枚举非法 / critical_turn
    //! 五条主路径行为符合 design.md §4.3 的伪代码。

    use super::*;
    use crate::agent::runtime::UserRuntimeParameters;

    pub(super) fn runtime_default(autonomy_protocol_enabled: bool) -> UserRuntimeParameters {
        UserRuntimeParameters {
            recent_message_limit: 12,
            min_reply_interval_seconds: 20,
            max_daily_touches: 3,
            max_pending_follow_ups: 3,
            follow_up_expires_hours: 48,
            cooldown_after_no_reply_hours: 24,
            fact_risk_block_at: 6,
            pressure_risk_block_at: 7,
            human_like_rewrite_below: 6,
            emotional_value_rewrite_below: 6,
            product_accuracy_block_below: 7,
            operation_state_confidence_full_review_below: 4,
            run_token_budget: 300000,
            run_token_budget_escalated: 600000,
            run_max_llm_calls: 10,
            simulation_token_budget: 300000,
            reaction_token_budget: 8000,
            reaction_max_llm_calls: 2,
            autonomy_protocol_enabled,
            knowledge_max_tool_calls: 6,
            knowledge_open_slice_max_k: 4,
            knowledge_search_top_k: 8,
            outbox_poll_interval_seconds: 5,
            outbox_lease_seconds: 60,
            quiet_hours_enabled: true,
            quiet_hours_start: 22,
            quiet_hours_end: 8,
            quiet_hours_tz_offset_hours: 8,
            allowed_conversation_modes: crate::agent::runtime::default_conversation_modes(),
            grounding_gate_bypass_without_claim: false,
            distrust_self_reported_low_risk: false,
            consolidation_window_char_budget: 6000,
            consolidation_window_max_messages: 60,
            bayesian_slot_min_hits: 3,
            bayesian_slot_min_strong: 2,
        }
    }

    /// 一个能通过 final 轮全部 R1.3/R3.1/R3.2/R3.3 校验的 raw（low_routine）。
    pub(super) fn make_valid_low_routine_raw() -> RawAgentDecision {
        RawAgentDecision {
            decision_phase: Some("final".to_string()),
            risk_level: Some("low".to_string()),
            knowledge_need: Some("not_required".to_string()),
            run_mode: Some("fast_chat".to_string()),
            autonomy_mode: Some("auto".to_string()),
            needs_review: Some(false),
            consolidation_needed: Some(false),
            operation_state: Some("idle".to_string()),
            user_understanding: Some("unchanged".to_string()),
            relationship_read: Some("unchanged".to_string()),
            operation_goal: Some("unchanged".to_string()),
            // R1.5 low_routine 严格 2 字段：≥ 6 unicode chars
            knowledge_need_reason: Some("无须查询知识库即可回应".to_string()),
            memory_update_reason: Some("unchanged".to_string()),
            self_critique: Some("回复内容平和，无误导".to_string()),
            risk_self_check: Some("unchanged".to_string()),
            // R1.4：should_reply=true 时 why_should_reply 必填
            why_should_reply: Some("用户主动打招呼，及时寒暄维持关系".to_string()),
            why_skip_reply: None,
            should_reply: Some(true),
            reply_text: Some("好的，谢谢你的问候。".to_string()),
            conversation_mode: Some("casual_relationship".to_string()),
            conversation_mode_reason: Some("当前是普通问候，按轻量关系模式回应。".to_string()),
            intent_analysis: Some(mongodb::bson::doc! {
                "semanticAssessment": {
                    "intent": "回应客户的轻量问候",
                    "speechAct": "greeting",
                    "subject": "customer",
                    "assertionStatus": "not_applicable",
                    "knowledgeNeed": "not_required",
                    "responseDisposition": "reply",
                    "semanticRisk": {
                        "content": "low",
                        "pressure": "low",
                        "boundary": "low",
                        "privacy": "low",
                        "confidence": 0.98,
                    },
                    "claims": [],
                    "reason": "普通会话寒暄，不代表任何现实业务事实。",
                }
            }),
            ..RawAgentDecision::default()
        }
    }

    #[test]
    fn commitment_carry_through_backfills_last_commitment_when_only_structured() {
        // PR-D：LLM 只给结构化 commitment（带 dueAt）、没给 lastCommitment 字符串时，
        // promote 应回填 last_commitment（gateway 落库入口判断它），并保留 commitment。
        let mut raw = make_valid_low_routine_raw();
        raw.commitment = Some(CommitmentDecision {
            text: "周五前发方案".to_string(),
            due_at: "2026-06-12T09:00:00+08:00".to_string(),
        });
        let runtime = runtime_default(true);
        let (decision, _risks) = raw.validate_and_promote(&runtime);
        assert_eq!(decision.last_commitment.as_deref(), Some("周五前发方案"));
        assert_eq!(
            decision.commitment.as_ref().map(|c| c.due_at.as_str()),
            Some("2026-06-12T09:00:00+08:00")
        );
    }

    #[test]
    fn commitment_carry_through_empty_text_does_not_override() {
        // 空壳 commitment（text 空）不应覆盖 last_commitment 路径，commitment 保持 None。
        let mut raw = make_valid_low_routine_raw();
        raw.last_commitment = Some("旧字符串承诺".to_string());
        raw.commitment = Some(CommitmentDecision {
            text: "  ".to_string(),
            due_at: "".to_string(),
        });
        let runtime = runtime_default(true);
        let (decision, _risks) = raw.validate_and_promote(&runtime);
        assert_eq!(decision.last_commitment.as_deref(), Some("旧字符串承诺"));
        assert!(decision.commitment.is_none(), "空壳 commitment 不透传");
    }

    #[test]
    fn tool_calling_phase_skips_r1_validation_even_with_empty_fields() {
        let raw = RawAgentDecision {
            decision_phase: Some("tool_calling".to_string()),
            tool_calls: Some(vec![ToolCallRequest {
                tool: "knowledge.search".to_string(),
                arguments: Document::new(),
            }]),
            // 故意把 R1.3 / R3.1 全部留空
            ..RawAgentDecision::default()
        };

        let runtime = runtime_default(true);
        let (decision, risks) = raw.validate_and_promote(&runtime);

        assert_eq!(decision.decision_phase, "tool_calling");
        assert_eq!(decision.tool_calls.len(), 1);
        assert_eq!(decision.tool_calls[0].tool, "knowledge.search");
        // 中间轮：R1.3 missing_required_field / R3.1 invalid_enum_value 均不应触发
        assert!(
            risks.is_empty(),
            "tool_calling 中间轮 SHALL 跳过 R1.3/R1.4/R1.5/R3 校验，但实际 risks={:?}",
            risks
        );
    }

    #[test]
    fn tool_calling_phase_flags_invalid_tool_name() {
        let raw = RawAgentDecision {
            decision_phase: Some("tool_calling".to_string()),
            tool_calls: Some(vec![ToolCallRequest {
                tool: "knowledge.unknown".to_string(),
                arguments: Document::new(),
            }]),
            ..RawAgentDecision::default()
        };

        let runtime = runtime_default(true);
        let (_decision, risks) = raw.validate_and_promote(&runtime);

        assert!(
            risks
                .iter()
                .any(|r| r == "invalid_tool_call:knowledge.unknown"),
            "应追加 invalid_tool_call risk，实际 risks={:?}",
            risks
        );
    }

    #[test]
    fn legacy_final_without_next_step_is_inferred_structurally() {
        let raw = make_valid_low_routine_raw();
        let (decision, risks) = raw.validate_reply_critical(&runtime_default(true));

        assert_eq!(decision.next_step, "respond");
        assert!(!risks
            .iter()
            .any(|risk| risk.contains("next_step") || risk.contains("nextStep")));
    }

    #[test]
    fn escalation_request_normalizes_control_step_to_ask_principal() {
        let mut raw = make_valid_low_routine_raw();
        raw.next_step = Some("respond".to_string());
        raw.escalation_request = Some(crate::models::EscalationRequest {
            needed: true,
            category: Some(crate::models::ESCALATION_CATEGORY_OUT_OF_SCOPE.to_string()),
            reason: Some("需要当前授权口径".to_string()),
            question_for_principal: Some("当前应按什么口径处理？".to_string()),
            self_serviceable_part: None,
            is_generalizable: false,
        });

        let (decision, risks) = raw.validate_reply_critical(&runtime_default(true));

        assert_eq!(decision.next_step, "ask_principal");
        assert!(risks
            .iter()
            .any(|risk| risk == "next_step_inconsistent:escalation_request"));
    }

    #[test]
    fn final_defer_step_is_preserved_as_a_closed_harness_action() {
        let mut raw = make_valid_low_routine_raw();
        raw.next_step = Some("defer".to_string());

        let (decision, risks) = raw.validate_reply_critical(&runtime_default(true));

        assert_eq!(decision.next_step, "defer");
        assert!(!risks
            .iter()
            .any(|risk| risk.contains("next_step") || risk.contains("nextStep")));
    }

    #[test]
    fn compact_reply_validation_accepts_valid_tool_calling_phase() {
        let raw = RawAgentDecision {
            decision_phase: Some("tool_calling".to_string()),
            next_step: Some("verify".to_string()),
            verification: Some(VerificationDecision {
                needed: true,
                reason: "需要核对当前已审核知识切片".to_string(),
            }),
            tool_calls: Some(vec![ToolCallRequest {
                tool: "knowledge.search".to_string(),
                arguments: mongodb::bson::doc! { "query": "用户当前问题" },
            }]),
            ..RawAgentDecision::default()
        };

        let (decision, risks) = raw.validate_reply_critical(&runtime_default(true));

        assert_eq!(decision.decision_phase, "tool_calling");
        assert_eq!(decision.next_step, "verify");
        assert_eq!(decision.tool_calls.len(), 1);
        assert!(!decision.should_reply);
        assert!(decision.reply_text.is_empty());
        assert!(risks.is_empty(), "unexpected tool phase risks: {risks:?}");
    }

    #[test]
    fn appointment_request_never_materializes_without_request_text() {
        let mut raw = make_valid_low_routine_raw();
        raw.appointment_request = Some(AppointmentRequestDecision {
            requested: true,
            request_text: "   ".to_string(),
            preferred_start: "2026-08-20T10:00:00+08:00".to_string(),
            ..AppointmentRequestDecision::default()
        });

        let (decision, risks) = raw.validate_reply_critical(&runtime_default(true));

        assert!(decision.appointment_request.is_none());
        assert!(risks
            .iter()
            .any(|risk| risk == "appointment_request_text_missing"));
    }

    #[test]
    fn draft_claim_manifest_is_preserved_only_as_advisory_metadata() {
        let mut raw = make_valid_low_routine_raw();
        raw.claim_manifest = Some(vec![DraftClaim {
            claim_id: "draft-1".to_string(),
            text: "候选回复中的待核验断言".to_string(),
            subject: "business".to_string(),
            requires_evidence: true,
            proposed_source_ids: vec!["model-invented-source".to_string()],
            reason: "草稿自检".to_string(),
        }]);

        let (decision, risks) = raw.validate_reply_critical(&runtime_default(true));

        assert!(risks.is_empty(), "unexpected risks: {risks:?}");
        assert_eq!(decision.claim_manifest.len(), 1);
        assert_eq!(
            decision.claim_manifest[0].proposed_source_ids,
            vec!["model-invented-source"]
        );
        assert!(decision.used_knowledge_ids.is_empty());
        assert!(decision.safe_claims_used.is_empty());
    }

    #[test]
    fn final_phase_with_empty_user_understanding_pushes_missing_required_field() {
        let mut raw = make_valid_low_routine_raw();
        raw.user_understanding = Some("   ".to_string()); // 仅空白 → 视为 missing

        let runtime = runtime_default(true);
        let (_decision, risks) = raw.validate_and_promote(&runtime);

        assert!(
            risks.contains(&"missing_required_field:user_understanding".to_string()),
            "risks={:?}",
            risks
        );
    }

    #[test]
    fn final_phase_with_invalid_risk_level_critical_pushes_invalid_enum_value() {
        let mut raw = make_valid_low_routine_raw();
        raw.risk_level = Some("critical".to_string()); // 本期不引入 critical

        let runtime = runtime_default(true);
        let (_decision, risks) = raw.validate_and_promote(&runtime);

        assert!(
            risks
                .iter()
                .any(|r| r == "invalid_enum_value:risk_level:critical"),
            "应触发 invalid_enum_value:risk_level:critical, risks={:?}",
            risks
        );
    }

    #[test]
    fn low_routine_with_unchanged_short_form_does_not_trigger_critical_turn_risk() {
        let raw = make_valid_low_routine_raw();
        let runtime = runtime_default(true);
        let (decision, risks) = raw.validate_and_promote(&runtime);

        // 不应触发任何 insufficient_detail_in_critical_turn:* 风险
        for r in &risks {
            assert!(
                !r.starts_with("insufficient_detail_in_critical_turn:"),
                "low_routine SHALL NOT 触发 critical_turn 长度风险, risks={:?}",
                risks
            );
        }
        assert_eq!(decision.user_understanding, "unchanged");
        assert_eq!(decision.knowledge_need_reason, "无须查询知识库即可回应");
    }

    #[test]
    fn critical_turn_with_unchanged_pushes_insufficient_detail() {
        // critical_turn 触发条件：risk_level=high 即可
        let raw = RawAgentDecision {
            decision_phase: Some("final".to_string()),
            risk_level: Some("high".to_string()),
            knowledge_need: Some("required".to_string()),
            run_mode: Some("knowledge_grounded".to_string()),
            autonomy_mode: Some("assisted".to_string()),
            needs_review: Some(true),
            consolidation_needed: Some(false),
            operation_state: Some("active".to_string()),
            // 故意给 user_understanding=unchanged，应被关键变化轮拒绝
            user_understanding: Some("unchanged".to_string()),
            relationship_read: Some("用户对产品功能与价格表达明显的关注与试探".to_string()),
            operation_goal: Some("建立信任并引导对方进入下一阶段的产品评估对话".to_string()),
            knowledge_need_reason: Some("需要核实产品定价细节避免给出错误的报价信息".to_string()),
            memory_update_reason: Some(
                "用户提及具体预算区间，需写入 recent_facts 以便后续跟进".to_string(),
            ),
            self_critique: Some("上一轮回复略显急切，本轮放慢节奏并增加问题确认环节".to_string()),
            risk_self_check: Some(
                "需避免对未验证产品功能做承诺，仅引用 verified 知识切片".to_string(),
            ),
            why_should_reply: Some(
                "用户主动询问产品差异，及时回应有助于推进决策且不显得冷淡".to_string(),
            ),
            should_reply: Some(true),
            reply_text: Some("您好，关于这款产品...".to_string()),
            ..RawAgentDecision::default()
        };

        let runtime = runtime_default(true);
        let (_decision, risks) = raw.validate_and_promote(&runtime);

        assert!(
            risks
                .iter()
                .any(|r| r == "insufficient_detail_in_critical_turn:user_understanding"),
            "critical_turn 拒绝 unchanged 短形式, risks={:?}",
            risks
        );
    }

    #[test]
    fn autonomy_protocol_disabled_returns_empty_risks_regardless_of_empty_fields() {
        let raw = RawAgentDecision {
            // 故意全空，预期在 sunset 路径被忽略
            ..RawAgentDecision::default()
        };

        let runtime = runtime_default(false);
        let (decision, risks) = raw.validate_and_promote(&runtime);

        assert!(
            risks.is_empty(),
            "autonomy_protocol_enabled=false SHALL 跳过校验, risks={:?}",
            risks
        );
        // 默认空字符串落入 final
        assert_eq!(decision.decision_phase, "final");
    }

    #[test]
    fn invalid_decision_phase_falls_back_to_final_with_risk() {
        let mut raw = make_valid_low_routine_raw();
        raw.decision_phase = Some("planner".to_string());

        let runtime = runtime_default(true);
        let (decision, risks) = raw.validate_and_promote(&runtime);

        assert_eq!(decision.decision_phase, "final");
        assert!(
            risks.iter().any(|r| r == "decision_phase_invalid:planner"),
            "risks={:?}",
            risks
        );
    }

    #[test]
    fn final_phase_should_reply_false_requires_why_skip_reply() {
        let mut raw = make_valid_low_routine_raw();
        raw.should_reply = Some(false);
        raw.why_should_reply = None; // R1.4 此时允许空
        raw.why_skip_reply = None; // 但 why_skip_reply 必填，缺失 → 违规

        let runtime = runtime_default(true);
        let (_decision, risks) = raw.validate_and_promote(&runtime);

        assert!(
            risks.contains(&"missing_required_field:why_skip_reply".to_string()),
            "risks={:?}",
            risks
        );
    }

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
        let raw: RawAgentDecision = serde_json::from_str(r#"{}"#).expect("parse empty");
        assert!(raw.escalation_request.is_none());
    }

    /// D-01：LLM 若顶层输出 snake_case customer_stage / intent_level（而非 schema
    /// 要求的 camelCase），须经 #[serde(alias)] 正确吸收为 Some，不再静默 miss→None
    /// 致标签丢失。与初始画像路径 decision.rs 的 camel→snake 双形兜底对齐。
    /// 回退（去掉 alias）即变红——rename_all=camelCase 下顶层 snake_case 恒 miss。
    #[test]
    fn raw_decision_accepts_snake_case_stage_and_intent() {
        // 顶层用 snake_case（LLM 偶发形态）。
        let snake = r#"{"customer_stage":"decision","intent_level":"high"}"#;
        let raw: RawAgentDecision = serde_json::from_str(snake).expect("parse snake");
        assert_eq!(
            raw.customer_stage.as_deref(),
            Some("decision"),
            "顶层 snake_case customer_stage 须经 alias 吸收"
        );
        assert_eq!(
            raw.intent_level.as_deref(),
            Some("high"),
            "顶层 snake_case intent_level 须经 alias 吸收"
        );

        // camelCase 主名仍照常工作（rename_all 主形态不受 alias 影响）。
        let camel = r#"{"customerStage":"evaluation","intentLevel":"medium"}"#;
        let raw2: RawAgentDecision = serde_json::from_str(camel).expect("parse camel");
        assert_eq!(
            raw2.customer_stage.as_deref(),
            Some("evaluation"),
            "camelCase 主名仍须正常解析"
        );
        assert_eq!(raw2.intent_level.as_deref(), Some("medium"));
    }

    // ─────────────────────────────────────────────────────────────────────
    // universal-domain-adaptation H9：conversationMode 枚举从 runtime 注入。
    // 锁死：① DEFAULT 销售域四模式逐字等价（反过拟合护栏）；② runtime 注入的
    // 行业模式集合生效（情感陪伴可声明 intimate_companion）；③ 注入集合外的值被
    // 严格拒绝；④ runtime 空集合 fallback 到内置四模式。
    // 这些是确定性单测，替代此前仅有的概率性 real-LLM 兜底。
    // ─────────────────────────────────────────────────────────────────────

    /// H9 辅助：构造一个能通过 final 校验、且显式带某个 conversation_mode 的 raw。
    fn raw_with_conversation_mode(mode: &str) -> RawAgentDecision {
        let mut raw = make_valid_low_routine_raw();
        raw.conversation_mode = Some(mode.to_string());
        raw
    }

    /// 提取与 conversation_mode 相关的违规标签（missing / invalid_enum）。
    fn conversation_mode_risks(risks: &[String]) -> Vec<&String> {
        risks
            .iter()
            .filter(|r| r.contains("conversation_mode"))
            .collect()
    }

    #[test]
    fn h9_default_runtime_locks_four_sales_modes_verbatim() {
        // runtime_default(true) 走 UserRuntimeParameters::default → 内置四模式。
        // 四个销售域模式逐一通过，无 conversation_mode 相关 risk。
        let runtime = runtime_default(true);
        for mode in [
            "casual_relationship",
            "value_exchange",
            "consultative",
            "boundary_protection",
        ] {
            let raw = raw_with_conversation_mode(mode);
            let (decision, risks) = raw.validate_and_promote(&runtime);
            assert_eq!(decision.conversation_mode, mode, "mode {mode} 应原样保留");
            assert!(
                conversation_mode_risks(&risks).is_empty(),
                "销售域四模式 {mode} 不应产生 conversation_mode risk，实际：{risks:?}"
            );
        }
    }

    #[test]
    fn h9_default_runtime_rejects_non_sales_mode() {
        // 默认四模式集合下，情感陪伴模式 intimate_companion 不在集合内 → 被严格拒绝。
        let runtime = runtime_default(true);
        let raw = raw_with_conversation_mode("intimate_companion");
        let (_decision, risks) = raw.validate_and_promote(&runtime);
        assert!(
            risks
                .iter()
                .any(|r| r == "invalid_enum_value:conversation_mode:intimate_companion"),
            "默认四模式集合应拒绝 intimate_companion，实际 risks：{risks:?}"
        );
    }

    #[test]
    fn h9_profile_injected_modes_accept_industry_specific_value() {
        // 模拟 gateway 用 active DomainProfile.conversation_modes 覆盖 runtime：
        // 情感陪伴行业声明 intimate_companion + 仍保留 boundary_protection（边界保护红线）。
        let mut runtime = runtime_default(true);
        runtime.allowed_conversation_modes = vec![
            "intimate_companion".to_string(),
            "boundary_protection".to_string(),
        ];
        let raw = raw_with_conversation_mode("intimate_companion");
        let (decision, risks) = raw.validate_and_promote(&runtime);
        assert_eq!(decision.conversation_mode, "intimate_companion");
        assert!(
            conversation_mode_risks(&risks).is_empty(),
            "profile 已声明 intimate_companion，不应产生 risk，实际：{risks:?}"
        );
        // 注入集合外的销售模式 value_exchange 现在反而被拒绝（集合已切换为情感行业）。
        let raw2 = raw_with_conversation_mode("value_exchange");
        let (_d2, risks2) = raw2.validate_and_promote(&runtime);
        assert!(
            risks2
                .iter()
                .any(|r| r == "invalid_enum_value:conversation_mode:value_exchange"),
            "情感行业集合应拒绝销售模式 value_exchange，实际：{risks2:?}"
        );
    }

    #[test]
    fn h9_empty_runtime_modes_fall_back_to_four_const() {
        // runtime.allowed_conversation_modes 为空（防御性：理论上 from_config/Default
        // 都给了四模式，但显式清空模拟边界）→ fallback 到 const 四模式。
        let mut runtime = runtime_default(true);
        runtime.allowed_conversation_modes = Vec::new();
        // 销售模式通过。
        let raw = raw_with_conversation_mode("consultative");
        let (decision, risks) = raw.validate_and_promote(&runtime);
        assert_eq!(decision.conversation_mode, "consultative");
        assert!(conversation_mode_risks(&risks).is_empty());
        // 非销售模式仍被拒（fallback 集合 = 四模式）。
        let raw2 = raw_with_conversation_mode("intimate_companion");
        let (_d2, risks2) = raw2.validate_and_promote(&runtime);
        assert!(risks2
            .iter()
            .any(|r| r == "invalid_enum_value:conversation_mode:intimate_companion"));
    }

    // ── media-asset Task 8：assets_to_send 反序列化 + carry-through ──

    #[test]
    fn decision_without_assets_field_defaults_empty() {
        // 旧 LLM 输出（无 assetsToSend）必须仍能反序列化、字段默认空。
        let json = r#"{"replyText":"你好","shouldReply":true}"#;
        let d: AgentDecision = serde_json::from_str(json).expect("must deserialize");
        assert!(d.assets_to_send.is_empty());
    }

    #[test]
    fn decision_parses_assets_to_send() {
        let json =
            r#"{"replyText":"这是报价单","assetsToSend":[{"assetId":"a1","reason":"客户问价"}]}"#;
        let d: AgentDecision = serde_json::from_str(json).unwrap();
        assert_eq!(d.assets_to_send.len(), 1);
        assert_eq!(d.assets_to_send[0].asset_id, "a1");
        assert_eq!(d.assets_to_send[0].reason.as_deref(), Some("客户问价"));
    }

    /// 硬伤① 回归：LLM 输出走 RawAgentDecision → validate_and_promote 后，
    /// assets_to_send 必须被 carry-through 到最终 AgentDecision，不能被静默丢弃。
    #[test]
    fn raw_decision_carries_assets_to_send_through_promote() {
        let mut raw = make_valid_low_routine_raw();
        raw.assets_to_send = Some(vec![AssetSendDirective {
            asset_id: "a1".to_string(),
            reason: Some("问价".to_string()),
        }]);
        let runtime = runtime_default(true);
        let (decision, _risks) = raw.validate_and_promote(&runtime);
        assert_eq!(decision.assets_to_send.len(), 1);
        assert_eq!(decision.assets_to_send[0].asset_id, "a1");
    }

    #[test]
    fn raw_decision_discards_empty_asset_directives_as_noops() {
        let mut raw = make_valid_low_routine_raw();
        raw.assets_to_send = Some(vec![
            AssetSendDirective::default(),
            AssetSendDirective {
                asset_id: "  ".to_string(),
                reason: Some("empty provider placeholder".to_string()),
            },
        ]);
        let runtime = runtime_default(true);
        let (decision, _risks) = raw.validate_and_promote(&runtime);
        assert!(decision.assets_to_send.is_empty());
    }

    /// LLM 没给 assetsToSend（None）时，promote 后保持默认空——不误造素材。
    #[test]
    fn raw_decision_without_assets_promotes_empty() {
        let raw = make_valid_low_routine_raw();
        let runtime = runtime_default(true);
        let (decision, _risks) = raw.validate_and_promote(&runtime);
        assert!(decision.assets_to_send.is_empty());
    }

    #[test]
    fn compact_reply_validation_discards_projection_fields_but_keeps_send_directives() {
        let mut raw = make_valid_low_routine_raw();
        raw.conversation_mode = Some("casual_relationship".to_string());
        raw.profile_update = Some(AgentProfile {
            summary: "must not enter the send decision".to_string(),
            ..AgentProfile::default()
        });
        raw.tags = Some(vec!["projection-only".to_string()]);
        raw.customer_stage = Some("invented-stage".to_string());
        raw.memory_update = Some("projection-only memory".to_string());
        raw.agent_generated_signals = Some(vec![AgentSignal {
            kind: "relationship_type".to_string(),
            value: "peer".to_string(),
            ..AgentSignal::default()
        }]);
        raw.assets_to_send = Some(vec![AssetSendDirective {
            asset_id: "asset-1".to_string(),
            reason: Some("send-critical".to_string()),
        }]);
        raw.commitment = Some(CommitmentDecision {
            text: "明天给答复".to_string(),
            due_at: "2026-06-12T09:00:00+08:00".to_string(),
        });

        let (decision, risks) = raw.validate_reply_critical(&runtime_default(true));

        assert!(
            risks.is_empty(),
            "unexpected compact-contract risks: {risks:?}"
        );
        assert!(decision.profile_update.is_none());
        assert!(decision.tags.is_empty());
        assert!(decision.customer_stage.is_none());
        assert!(decision.memory_update.is_empty());
        assert!(decision.agent_generated_signals.is_empty());
        assert_eq!(decision.assets_to_send.len(), 1);
        assert_eq!(decision.last_commitment.as_deref(), Some("明天给答复"));
    }

    #[test]
    fn compact_reply_validation_keeps_valid_semantic_contract() {
        let raw = make_valid_low_routine_raw();
        let (decision, risks) = raw.validate_reply_critical(&runtime_default(true));

        assert!(risks.is_empty(), "unexpected risks: {risks:?}");
        assert!(decision
            .intent_analysis
            .get_document("semanticAssessment")
            .is_ok());
    }

    #[test]
    fn compact_reply_validation_drops_invalid_semantic_enum_without_hard_block_tag() {
        let mut raw = make_valid_low_routine_raw();
        raw.intent_analysis
            .as_mut()
            .expect("intent analysis")
            .get_document_mut("semanticAssessment")
            .expect("semantic assessment")
            .insert("speechAct", "price_magic_word");

        let (decision, risks) = raw.validate_reply_critical(&runtime_default(true));

        assert!(decision
            .intent_analysis
            .get_document("semanticAssessment")
            .is_err());
        assert_eq!(
            risks,
            vec!["semantic_contract_invalid:speechAct".to_string()]
        );
        assert!(!risks.iter().any(|risk| {
            risk.starts_with("missing_required_field:") || risk.starts_with("invalid_enum_value:")
        }));
    }

    #[test]
    fn compact_reply_validation_rejects_silent_disposition_for_sendable_body() {
        let mut raw = make_valid_low_routine_raw();
        raw.intent_analysis
            .as_mut()
            .expect("intent analysis")
            .get_document_mut("semanticAssessment")
            .expect("semantic assessment")
            .insert("responseDisposition", "silent");

        let (decision, risks) = raw.validate_reply_critical(&runtime_default(true));

        assert!(decision
            .intent_analysis
            .get_document("semanticAssessment")
            .is_err());
        assert!(risks
            .iter()
            .any(|risk| risk == "semantic_contract_invalid:responseDispositionConsistency"));
    }

    #[test]
    fn compact_reply_validation_allows_legacy_missing_semantic_contract() {
        let mut raw = make_valid_low_routine_raw();
        raw.intent_analysis = None;

        let (_decision, risks) = raw.validate_reply_critical(&runtime_default(true));

        assert!(
            risks.is_empty(),
            "旧 Prompt 缺少辅助语义合同应退回独立双闸，而不是结构性硬拦: {risks:?}"
        );
    }

    #[test]
    fn deferred_projection_schema_rejects_send_control_fields() {
        for field in [
            "replyText",
            "operationState",
            "cooldownUntil",
            "appointmentRequest",
        ] {
            let mut value = serde_json::json!({ "tags": [] });
            value[field] = serde_json::json!("must never be accepted");
            let error = DeferredProjectionDecision::from_value(value)
                .expect_err("projection schema must deny send-control fields");
            assert!(error.contains("forbidden send-control field"));
        }
    }

    #[test]
    fn deferred_projection_conversion_cannot_authorize_delivery() {
        let projected: DeferredProjectionDecision = serde_json::from_value(serde_json::json!({
            "tags": ["stable"],
            "memoryWriteScore": 99
        }))
        .expect("valid sparse projection");
        let decision = projected.into_agent_decision();
        assert_eq!(decision.tags, vec!["stable"]);
        assert_eq!(decision.memory_write_score, 10);
        assert!(!decision.should_reply);
        assert!(decision.reply_text.is_empty());
        assert!(decision.assets_to_send.is_empty());
        assert!(decision.namecard_to_send.is_none());
        assert!(decision.escalation_request.is_none());
        assert!(decision.follow_up.is_none());
        assert!(decision.commitment.is_none());
    }

    /// 子计划2 Task2：carry_through_fields 须把 LLM 指认的标签/stage 证据序位
    /// + 明示意图标志透传到最终 AgentDecision，不能静默丢失。
    #[test]
    fn carry_through_propagates_evidence_fields() {
        let mut raw = RawAgentDecision::default();
        raw.tag_evidence_turns = Some(vec![1, 2]);
        raw.stage_evidence_turns = Some(vec![3]);
        raw.stage_explicit_intent = Some(true);
        let mut decision = AgentDecision::default();
        carry_through_fields(raw, &mut decision);
        assert_eq!(decision.tag_evidence_turns, vec![1, 2]);
        assert_eq!(decision.stage_evidence_turns, vec![3]);
        assert!(decision.stage_explicit_intent);
    }
}

#[cfg(test)]
mod namecard_directive_tests {
    //! 专属顾问名片引荐 Task 4：`namecard_to_send` 三处接线回归。
    //! 旧 LLM 输出（无 namecardToSend）必须仍能反序列化、字段默认 None；
    //! LLM 给出 namecardToSend 时必须被 carry-through 到最终 AgentDecision，
    //! 不能在 validate_and_promote 后被静默丢弃（防丢字段硬伤）。
    use super::validate_and_promote_tests::{make_valid_low_routine_raw, runtime_default};
    use super::*;

    #[test]
    fn decision_without_namecard_field_defaults_none() {
        // 旧 LLM 输出（无 namecardToSend）必须仍能反序列化、字段默认 None。
        let json = r#"{"replyText":"你好","shouldReply":true}"#;
        let d: AgentDecision = serde_json::from_str(json).expect("must deserialize");
        assert!(d.namecard_to_send.is_none());
    }

    /// carry-through 回归：LLM 输出走 RawAgentDecision → validate_and_promote 后，
    /// namecard_to_send 必须被透传到最终 AgentDecision，不能被静默丢弃。
    #[test]
    fn raw_decision_carries_namecard_to_send_through_promote() {
        let mut raw = make_valid_low_routine_raw();
        raw.namecard_to_send = Some(NamecardDirective {
            card_id: "c1".to_string(),
            reason: Some("已签约转专属顾问".to_string()),
        });
        let runtime = runtime_default(true);
        let (decision, _risks) = raw.validate_and_promote(&runtime);
        let card = decision
            .namecard_to_send
            .expect("namecard must carry through");
        assert_eq!(card.card_id, "c1");
    }

    #[test]
    fn raw_decision_discards_empty_namecard_directive_as_noop() {
        let mut raw = make_valid_low_routine_raw();
        raw.namecard_to_send = Some(NamecardDirective {
            card_id: "  ".to_string(),
            reason: Some("empty provider placeholder".to_string()),
        });
        let runtime = runtime_default(true);
        let (decision, _risks) = raw.validate_and_promote(&runtime);
        assert!(decision.namecard_to_send.is_none());
    }
}

#[cfg(test)]
mod dimension_display_names_tests {
    //! dimensionDisplayNames carry-through 回归：LLM 输出的维度中文名映射
    //! 必须经 RawAgentDecision → validate_and_promote 透传到 AgentDecision，
    //! 不能被静默丢弃（防丢字段硬伤，同 namecard/assets 老坑）。
    use super::validate_and_promote_tests::{make_valid_low_routine_raw, runtime_default};
    use super::*;
    use mongodb::bson::doc;

    #[test]
    fn decision_without_display_names_defaults_empty() {
        // 旧/常规 LLM 输出（无 dimensionDisplayNames）仍能反序列化，字段默认空 doc。
        let json = r#"{"replyText":"你好","shouldReply":true}"#;
        let d: AgentDecision = serde_json::from_str(json).expect("must deserialize");
        assert!(d.dimension_display_names.is_empty());
    }

    #[test]
    fn raw_decision_carries_display_names_through_promote() {
        let mut raw = make_valid_low_routine_raw();
        raw.dimension_display_names = Some(doc! { "customer_stage": "焦虑观望" });
        let runtime = runtime_default(true);
        let (decision, _risks) = raw.validate_and_promote(&runtime);
        assert_eq!(
            decision
                .dimension_display_names
                .get_str("customer_stage")
                .ok(),
            Some("焦虑观望"),
            "dimensionDisplayNames 必须 carry-through，实际 {:?}",
            decision.dimension_display_names
        );
    }

    #[test]
    fn raw_decision_none_display_names_stays_empty_after_promote() {
        // Raw 未给该字段 → promote 后保持空 doc（不 panic、不误填）。
        let raw = make_valid_low_routine_raw();
        let runtime = runtime_default(true);
        let (decision, _risks) = raw.validate_and_promote(&runtime);
        assert!(decision.dimension_display_names.is_empty());
    }
}

#[cfg(test)]
mod decision_review_result_tests {
    //! agent-autonomy-loop W2 / Task 3.3：[`DecisionReviewResult`] 扩字段
    //! 与 [`assert_hold_category_valid`] 行为单元测试。
    //!
    //! 覆盖 R2.1 / R2.2 / R2.6 / R9.8 的关键路径：
    //! * 老 review JSON（无新字段）反序列化时新字段全部走 `Default`
    //!   （向后兼容，避免合并顺序导致的解析失败）；
    //! * `hold_category="held_for_human"` 强制改写为 `held_by_ai_policy`
    //!   并返回 `Coerced { original }`；
    //! * 三个合法枚举（`held_by_ai_policy / blocked_by_safety_guard /
    //!   ai_waiting_for_more_context`）均视为 `Unchanged`；
    //! * `should_hold=true` 但 `hold_category=""` 也走 Coerced 默认填补。

    use super::*;

    fn legacy_review_json() -> &'static str {
        r#"{
            "approved": true,
            "scores": {
                "humanLike": 8,
                "emotionalValue": 7,
                "productAccuracy": 9,
                "relationshipProgress": 6,
                "conversionReadiness": 5,
                "pressureRisk": 1,
                "factRisk": 0
            },
            "risks": [],
            "reviewSummary": "ok"
        }"#
    }

    #[test]
    fn legacy_review_json_deserializes_with_default_new_fields() {
        // 不含 needsRevision / shouldHold / holdCategory 等扩字段的老格式
        // SHALL 反序列化成功，所有新字段走 Default（向后兼容，避免合并
        // 顺序导致的解析失败 — task 3.1 review.rs 改造与本任务并行）。
        let review: DecisionReviewResult =
            serde_json::from_str(legacy_review_json()).expect("legacy review parses");

        assert!(review.approved);
        assert_eq!(review.review_summary, "ok");

        // task 3.3 新字段全部走默认值
        assert!(!review.needs_revision);
        assert_eq!(review.revision_direction, "");
        assert!(!review.should_hold);
        assert_eq!(review.hold_reason, "");
        assert_eq!(review.hold_category, "");
        assert!(!review.self_critique_addressed);
        assert!(!review.revision_applied);
        assert_eq!(review.final_review_status, "");
    }

    #[test]
    fn structured_review_with_camel_case_new_fields_round_trips() {
        // Review Agent 输出 camelCase（与 prompt schema 一致），反序列化 SHALL
        // 把 needsRevision / revisionDirection / shouldHold / holdReason /
        // holdCategory / selfCritiqueAddressed / revisionApplied /
        // finalReviewStatus 全部正确映射到 snake_case 字段。
        let json = r#"{
            "approved": false,
            "scores": {
                "humanLike": 6,
                "emotionalValue": 5,
                "productAccuracy": 8,
                "relationshipProgress": 5,
                "conversionReadiness": 4,
                "pressureRisk": 2,
                "factRisk": 1
            },
            "risks": ["needs_polish"],
            "needsRevision": true,
            "revisionDirection": "把第二句改得更口语化一些",
            "shouldHold": false,
            "holdReason": "",
            "holdCategory": "",
            "selfCritiqueAddressed": false,
            "revisionApplied": false,
            "finalReviewStatus": ""
        }"#;
        let review: DecisionReviewResult =
            serde_json::from_str(json).expect("structured review parses");

        assert!(!review.approved);
        assert!(review.needs_revision);
        assert_eq!(review.revision_direction, "把第二句改得更口语化一些");
        assert!(!review.should_hold);
        assert_eq!(review.hold_category, "");
    }

    #[test]
    fn assert_hold_category_valid_accepts_three_canonical_values() {
        for canonical in HOLD_CATEGORY_VALUES {
            let mut review = DecisionReviewResult {
                should_hold: true,
                hold_category: (*canonical).to_string(),
                ..Default::default()
            };
            let outcome = assert_hold_category_valid(&mut review);
            assert_eq!(
                outcome,
                HoldCategoryAssertion::Unchanged,
                "canonical={canonical}",
            );
            assert_eq!(review.hold_category, *canonical);
        }
    }

    #[test]
    fn assert_hold_category_valid_coerces_held_for_human_to_held_by_ai_policy() {
        // R2.6 / R9.8：`held_for_human` 是被 R2.7 业务语义保护明确禁用的
        // 取值，SHALL 被强制改写为 `held_by_ai_policy` 并返回 `Coerced`，
        // 调用方据此往 agent_events 写 kind="autonomy_hold_category_invalid"。
        let mut review = DecisionReviewResult {
            should_hold: true,
            hold_category: "held_for_human".to_string(),
            hold_reason: "user explicitly asked to wait".to_string(),
            ..Default::default()
        };
        let outcome = assert_hold_category_valid(&mut review);

        assert_eq!(
            outcome,
            HoldCategoryAssertion::Coerced {
                original: "held_for_human".to_string()
            }
        );
        assert_eq!(review.hold_category, HOLD_CATEGORY_HELD_BY_AI_POLICY);
        // hold_reason 不应被改写
        assert_eq!(review.hold_reason, "user explicitly asked to wait");
    }

    #[test]
    fn assert_hold_category_valid_coerces_arbitrary_unknown_value() {
        // 非禁用名单内的任意未知字符串也 SHALL 被矫正为合法默认值
        let mut review = DecisionReviewResult {
            should_hold: true,
            hold_category: "foo_bar_baz".to_string(),
            ..Default::default()
        };
        let outcome = assert_hold_category_valid(&mut review);

        assert!(matches!(outcome, HoldCategoryAssertion::Coerced { .. }));
        assert_eq!(review.hold_category, HOLD_CATEGORY_HELD_BY_AI_POLICY);
    }

    #[test]
    fn assert_hold_category_valid_should_hold_true_empty_is_coerced() {
        // R2.6：should_hold=true 但 hold_category 为空（含仅空白） SHALL
        // 默认填 `held_by_ai_policy` 并返回 Coerced。
        let mut review = DecisionReviewResult {
            should_hold: true,
            hold_category: "   ".to_string(),
            ..Default::default()
        };
        let outcome = assert_hold_category_valid(&mut review);

        assert_eq!(
            outcome,
            HoldCategoryAssertion::Coerced {
                original: "   ".to_string()
            }
        );
        assert_eq!(review.hold_category, HOLD_CATEGORY_HELD_BY_AI_POLICY);
    }

    #[test]
    fn assert_hold_category_valid_should_hold_false_empty_is_unchanged() {
        // should_hold=false 时 hold_category 留空是合法占位（review 未触发
        // hold 路径）；不需要写违规事件。
        let mut review = DecisionReviewResult {
            should_hold: false,
            hold_category: String::new(),
            ..Default::default()
        };
        let outcome = assert_hold_category_valid(&mut review);

        assert_eq!(outcome, HoldCategoryAssertion::Unchanged);
        assert_eq!(review.hold_category, "");
    }

    #[test]
    fn assert_hold_category_valid_should_hold_false_with_dirty_value_is_coerced() {
        // 但 should_hold=false 时若 hold_category 仍取了禁用值（脏数据 / 上
        // 游逻辑错误），仍 SHALL 被矫正为合法默认 + 触发事件埋点。
        let mut review = DecisionReviewResult {
            should_hold: false,
            hold_category: "human_required".to_string(),
            ..Default::default()
        };
        let outcome = assert_hold_category_valid(&mut review);

        assert_eq!(
            outcome,
            HoldCategoryAssertion::Coerced {
                original: "human_required".to_string()
            }
        );
        assert_eq!(review.hold_category, HOLD_CATEGORY_HELD_BY_AI_POLICY);
    }

    #[test]
    fn assert_hold_category_valid_trims_canonical_value_with_whitespace() {
        // 容错：合法枚举值前后有空白 SHALL 被 trim 后视为合法（Unchanged）
        // 并把字段标准化为去 trim 形态，避免脏数据混入下游聚合查询。
        let mut review = DecisionReviewResult {
            should_hold: true,
            hold_category: "  blocked_by_safety_guard  ".to_string(),
            ..Default::default()
        };
        let outcome = assert_hold_category_valid(&mut review);

        assert_eq!(outcome, HoldCategoryAssertion::Unchanged);
        assert_eq!(review.hold_category, HOLD_CATEGORY_BLOCKED_BY_SAFETY_GUARD);
    }

    #[test]
    fn is_forbidden_hold_category_recognizes_human_handoff_aliases() {
        for forbidden in [
            "held_for_human",
            "human_required",
            "waiting_for_human",
            "handoff_to_human",
            "manual_takeover",
        ] {
            assert!(
                is_forbidden_hold_category(forbidden),
                "forbidden={forbidden}",
            );
        }
        // 合法值不应被识别为禁用
        for canonical in HOLD_CATEGORY_VALUES {
            assert!(
                !is_forbidden_hold_category(canonical),
                "canonical={canonical}"
            );
        }
        // 任意未知字符串也不算禁用（仅在 hold_category 校验环节被矫正）
        assert!(!is_forbidden_hold_category("ai_thinking_more"));
    }

    #[test]
    fn review_scores_map_factrisk_and_productaccuracy_aliases() {
        // 回归守门：reviewer prompt 至今以 `factRisk` / `productAccuracy` 命名
        // 这两个评分键（review.rs prompt schema），而结构体字段是
        // hallucination_score / knowledge_grounding_score。若 alias 缺失，
        // number_i32 会让 missing key 静默落 0 —— 导致 fact-risk 闸（block）
        // 永远不触发、product-accuracy 闸恒判为 0 < block_below 而误拦。
        // 本用例锁死 alias 行为，保证评分真正落到判定字段。
        let json = r#"{
            "humanLike": 6,
            "emotionalValue": 5,
            "productAccuracy": 9,
            "pressureRisk": 2,
            "factRisk": 8
        }"#;
        let scores: ReviewScores = serde_json::from_str(json).expect("scores parse");

        assert_eq!(scores.human_like, 6);
        assert_eq!(scores.emotional_value, 5);
        // factRisk → hallucination_score（≥6 触发 fact-risk block）
        assert_eq!(scores.hallucination_score, 8);
        // productAccuracy → knowledge_grounding_score（<7 触发 product-claim block）
        assert_eq!(scores.knowledge_grounding_score, 9);
        assert_eq!(scores.pressure_risk, 2);
    }

    #[test]
    fn review_scores_accept_canonical_snake_to_camel_keys() {
        // 新 prompt 若改用规范键（hallucinationScore / knowledgeGroundingScore）
        // 也 SHALL 正确反序列化 —— alias 是“额外接受”，不替换规范键。
        let json = r#"{
            "humanLike": 7,
            "emotionalValue": 6,
            "hallucinationScore": 3,
            "knowledgeGroundingScore": 8,
            "pressureRisk": 1
        }"#;
        let scores: ReviewScores = serde_json::from_str(json).expect("scores parse");

        assert_eq!(scores.hallucination_score, 3);
        assert_eq!(scores.knowledge_grounding_score, 8);
    }

    #[test]
    fn test_review_scores_boundary_privacy_dimension_backward_compat() {
        // 老 JSON 缺 boundaryPrivacySafety 应成功反序列化取默认 0
        let old_json = r#"{"humanLike":7,"emotionalValue":6}"#;
        let scores: ReviewScores = serde_json::from_str(old_json).unwrap();
        assert_eq!(scores.boundary_privacy_safety, 0);
    }

    #[test]
    fn test_agent_decision_sufficiency_fields_backward_compat() {
        // 老 JSON（无新字段）应成功反序列化，新字段取默认
        let old_json = r#"{"reply_text":"test","conversation_mode":"casual_relationship"}"#;
        let decision: AgentDecision = serde_json::from_str(old_json).unwrap();
        assert_eq!(decision.sufficiency, "");
        assert_eq!(decision.missing_tier, "");
        assert_eq!(decision.clarification_intent, "");
    }
}
