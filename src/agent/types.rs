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
    pub customer_stage: Option<String>,
    pub intent_level: Option<String>,
    pub last_commitment: Option<String>,
    /// PR-D：结构化承诺（带可选 dueAt）。promote 时从 RawAgentDecision.commitment 透传。
    pub commitment: Option<CommitmentDecision>,
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
    // gateway 在 keyword fastpath 命中时会强制覆盖为 consultative。
    #[serde(default = "default_conversation_mode")]
    pub conversation_mode: String,
    #[serde(default)]
    pub conversation_mode_reason: Option<String>,

    /// decision Agent emit 的请示意图；None=本轮无需请示真人。
    #[serde(default)]
    pub escalation_request: Option<crate::models::EscalationRequest>,
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
            customer_stage: None,
            intent_level: None,
            last_commitment: None,
            commitment: None,
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
            // conversation_mode：默认寒暄模式（最保守）
            conversation_mode: default_conversation_mode(),
            conversation_mode_reason: None,
            // 请示意图：默认无（本轮不向幕后真人请示）
            escalation_request: None,
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
    pub risk_level: Option<String>,        // low | medium | high
    pub knowledge_need: Option<String>,    // not_required | required | insufficient
    pub run_mode: Option<String>,          // fast_chat | memory_candidate | knowledge_grounded | high_risk
    pub autonomy_mode: Option<String>,     // auto | assisted | blocked
    pub needs_review: Option<bool>,
    pub operation_state: Option<String>,
    pub consolidation_needed: Option<bool>,

    // ── R4 工具循环协议 ──
    pub decision_phase: Option<String>,    // tool_calling | final
    pub tool_calls: Option<Vec<ToolCallRequest>>,

    // ── R8 自由信号 ──
    pub agent_generated_signals: Option<Vec<AgentSignal>>,

    // ── R-prompt-v3 conversation_mode：四模式人格切换 ──
    pub conversation_mode: Option<String>,
    pub conversation_mode_reason: Option<String>,

    // ── 既有回复 / 知识 / 记忆 / 信号字段（保留为 Option，由 promote 落地为非 Option）──
    pub reply_text: Option<String>,
    pub should_reply: Option<bool>,
    pub used_knowledge_ids: Option<Vec<String>>,
    pub safe_claims_used: Option<Vec<String>>,
    pub knowledge_route: Option<KnowledgeRouteResult>,
    pub profile_update: Option<AgentProfile>,
    pub tags: Option<Vec<String>>,
    pub customer_stage: Option<String>,
    pub intent_level: Option<String>,
    pub last_commitment: Option<String>,
    /// PR-D：结构化承诺（带可选 dueAt）。缺失时回落 last_commitment。
    pub commitment: Option<CommitmentDecision>,
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
const RUN_MODE_VALUES: &[&str] =
    &["fast_chat", "memory_candidate", "knowledge_grounded", "high_risk"];
const AUTONOMY_MODE_VALUES: &[&str] = &["auto", "assisted", "blocked"];
const CONVERSATION_MODE_VALUES: &[&str] = &[
    "casual_relationship",
    "value_exchange",
    "consultative",
    "boundary_protection",
];
const ALLOWED_TOOL_NAMES: &[&str] = &[
    "knowledge.list_catalog",
    "knowledge.search",
    "knowledge.open_slice",
];

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
fn check_required_string(
    field: Option<String>,
    name: &str,
    risks: &mut Vec<String>,
) -> String {
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
fn check_required_bool(
    field: Option<bool>,
    name: &str,
    risks: &mut Vec<String>,
) -> bool {
    match field {
        Some(v) => v,
        None => {
            risks.push(format!("missing_required_field:{}", name));
            false
        }
    }
}

impl RawAgentDecision {
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
            for call in &tool_calls {
                let trimmed = call.tool.trim();
                if trimmed.is_empty() || !ALLOWED_TOOL_NAMES.iter().any(|a| *a == trimmed) {
                    risks.push(format!("invalid_tool_call:{}", call.tool));
                }
            }
            return (build_tool_calling_decision(self, phase), risks);
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
        let conversation_mode = check_required_enum(
            self.conversation_mode.clone(),
            "conversation_mode",
            CONVERSATION_MODE_VALUES,
            &mut risks,
        );
        let needs_review = check_required_bool(self.needs_review, "needs_review", &mut risks);
        let consolidation_needed = check_required_bool(
            self.consolidation_needed,
            "consolidation_needed",
            &mut risks,
        );
        let operation_state = check_required_string(
            self.operation_state.clone(),
            "operation_state",
            &mut risks,
        );

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
        let operation_goal = check_required_string(
            self.operation_goal.clone(),
            "operation_goal",
            &mut risks,
        );
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
        let self_critique = check_required_string(
            self.self_critique.clone(),
            "self_critique",
            &mut risks,
        );
        let risk_self_check = check_required_string(
            self.risk_self_check.clone(),
            "risk_self_check",
            &mut risks,
        );

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
        let is_low_routine = risk_level == "low"
            && knowledge_need == "not_required"
            && !consolidation_needed;
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
                    risks.push(format!(
                        "insufficient_detail_in_critical_turn:{}",
                        name
                    ));
                }
            }

            // R1.6：回复理由（命中那一个）≥ 30 unicode chars 含 ≥ 12 hanzi
            if should_reply {
                if !why_should_reply.is_empty()
                    && !is_valid_reply_reason(&why_should_reply, 30, 12)
                {
                    risks.push(
                        "insufficient_detail_in_critical_turn:why_should_reply".to_string(),
                    );
                }
            } else if !why_skip_reply.is_empty()
                && !is_valid_reply_reason(&why_skip_reply, 30, 12)
            {
                risks
                    .push("insufficient_detail_in_critical_turn:why_skip_reply".to_string());
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
                    risks.push(format!(
                        "insufficient_detail_in_critical_turn:{}",
                        name
                    ));
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

        (decision, risks)
    }
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
    // 若 Agent 在中间轮意外填了，由 W3 task 4.3 的 reply_with_tools_loop 在丢弃
    // 时追加 `tool_calling_phase_with_reply_text` 标签；本函数只保证默认安全）
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
    if let Some(v) = raw.used_knowledge_ids {
        decision.used_knowledge_ids = v;
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
    if raw.customer_stage.is_some() {
        decision.customer_stage = raw.customer_stage;
    }
    if raw.intent_level.is_some() {
        decision.intent_level = raw.intent_level;
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
    /// Phase B / B1：恢复 `pressure_risk` 软闸评分（0-100）。Reviewer 输出，
    /// `review_passed` 与 single-shot revision 通道判定时使用。R11 兼容：
    /// 缺省 `0`，旧 review JSON 反序列化不破坏。
    #[serde(default, deserialize_with = "number_i32")]
    pub pressure_risk: i32,
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
    pub memory_preview: Document,
    pub state_transition: Document,
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

pub(crate) fn to_bson_array(values: &[String]) -> Vec<mongodb::bson::Bson> {
    values
        .iter()
        .cloned()
        .map(mongodb::bson::Bson::String)
        .collect()
}

pub(crate) fn parse_rfc3339_to_bson(value: &str) -> Option<mongodb::bson::DateTime> {
    mongodb::bson::DateTime::parse_rfc3339_str(value).ok()
}

/// 解析 LLM 给出的 follow_up.run_at（RFC3339）；解析失败时**降级**到
/// `now + degrade_offset_ms` 而非静默丢弃整条跟进任务。
///
/// #66：此前 gateway 用 `if let Some(run_at) = parse_rfc3339_to_bson(..)` 无 else
/// 分支——LLM 给空串 / 非法格式（prompt 模板 runAt 默认空串、无格式约束）时整条
/// follow_up 无声蒸发、无日志无事件。降级到"现在 + 偏移"后任务仍会入队，由
/// precheck 的 context_changed / expired 正常守门；返回的 bool 标记是否走了降级，
/// 供调用方写审计事件。
pub(crate) fn resolve_run_at_or_degrade(
    raw: &str,
    now_ms: i64,
    degrade_offset_ms: i64,
) -> (mongodb::bson::DateTime, bool) {
    match parse_rfc3339_to_bson(raw) {
        Some(dt) => (dt, false),
        None => (
            mongodb::bson::DateTime::from_millis(now_ms.saturating_add(degrade_offset_ms)),
            true,
        ),
    }
}

#[cfg(test)]
mod run_at_degrade_tests {
    use super::resolve_run_at_or_degrade;

    #[test]
    fn valid_rfc3339_parses_without_degrade() {
        // 用 UTC 整点避免时区换算的魔数；与 bson 自身解析对照，不硬编码毫秒。
        let raw = "2026-06-12T00:00:00Z";
        let (dt, degraded) = resolve_run_at_or_degrade(raw, 0, 999);
        assert!(!degraded, "合法 RFC3339 不应降级");
        assert_eq!(
            dt.timestamp_millis(),
            mongodb::bson::DateTime::parse_rfc3339_str(raw)
                .unwrap()
                .timestamp_millis(),
        );
    }

    #[test]
    fn empty_string_degrades_to_now_plus_offset() {
        let now_ms = 1_000_000;
        let (dt, degraded) = resolve_run_at_or_degrade("", now_ms, 3_600_000);
        assert!(degraded, "空串应降级");
        assert_eq!(dt.timestamp_millis(), now_ms + 3_600_000);
    }

    #[test]
    fn garbage_degrades_to_now_when_offset_zero() {
        let now_ms = 5_000;
        let (dt, degraded) = resolve_run_at_or_degrade("明天下午", now_ms, 0);
        assert!(degraded, "非法格式应降级");
        assert_eq!(dt.timestamp_millis(), now_ms, "offset=0 时降级到 now");
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

    fn runtime_default(autonomy_protocol_enabled: bool) -> UserRuntimeParameters {
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
            run_token_budget: 30000,
            run_max_llm_calls: 6,
            simulation_token_budget: 60000,
            reaction_token_budget: 8000,
            reaction_max_llm_calls: 2,
            autonomy_protocol_enabled,
            knowledge_routing_mode: "auto_tool_loop".to_string(),
            knowledge_max_tool_loops: 3,
            knowledge_max_tool_calls: 6,
            knowledge_open_slice_max_k: 4,
            knowledge_search_top_k: 8,
            outbox_poll_interval_seconds: 5,
            outbox_lease_seconds: 60,
            quiet_hours_enabled: true,
            quiet_hours_start: 22,
            quiet_hours_end: 8,
        }
    }

    /// 一个能通过 final 轮全部 R1.3/R3.1/R3.2/R3.3 校验的 raw（low_routine）。
    fn make_valid_low_routine_raw() -> RawAgentDecision {
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
            risks.iter().any(|r| r == "invalid_tool_call:knowledge.unknown"),
            "应追加 invalid_tool_call risk，实际 risks={:?}",
            risks
        );
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
            relationship_read: Some(
                "用户对产品功能与价格表达明显的关注与试探".to_string(),
            ),
            operation_goal: Some(
                "建立信任并引导对方进入下一阶段的产品评估对话".to_string(),
            ),
            knowledge_need_reason: Some(
                "需要核实产品定价细节避免给出错误的报价信息".to_string(),
            ),
            memory_update_reason: Some(
                "用户提及具体预算区间，需写入 recent_facts 以便后续跟进".to_string(),
            ),
            self_critique: Some(
                "上一轮回复略显急切，本轮放慢节奏并增加问题确认环节".to_string(),
            ),
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
            assert!(!is_forbidden_hold_category(canonical), "canonical={canonical}");
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
}
