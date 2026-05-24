//! Review Agent 与本地兜底评审。
//!
//! 该模块负责：
//! - `review_decision`：调用 `user.review.system` / `user.review.light.system`
//!   prompt，对候选回复做评审；调用结束后串行执行
//!   [`super::guards::enforce_decision_guards`] 的所有守卫并最终
//!   `review_passed` 收敛 `approved` 标志；
//! - `local_decision_review`：当预算超额或 review 不需要 LLM 介入时，
//!   返回一个保守通过的本地评审结果（避免阻塞主流程）；
//! - `effective_review_mode` / `should_run_review`：根据 planner、decision
//!   置信度等决定本轮使用 light 还是 full review；
//! - `review_passed`：把多个评分阈值收敛成一个布尔，是其它子模块（gateway、
//!   simulation 等）判断是否可发送的统一入口。

use mongodb::bson::Document;

use crate::error::AppResult;
use crate::models::{
    Contact, ConversationMessage, OperatingMemory, OperationDomainConfig, OperationKnowledgeChunk,
    OperationKnowledgeItem, OperationPlaybook,
};
use crate::prompts;
use crate::routes::AppState;

use super::budget::RunBudget;
use super::decision::{format_operation_domain_config_for_prompt, format_playbook_for_prompt};
use super::generate_agent_json;
use super::guards::{
    append_unverified_safe_claim_risks, claim_analysis_is_malformed,
    claim_requires_product_knowledge, compute_unverified_safe_claims, compute_verified_chunks,
    enforce_decision_guards_with_markers, infer_product_claim_trigger, load_product_claim_markers,
    ProductClaimMarkers,
};
use super::knowledge_router::format_operation_knowledge_for_prompt;
use super::runtime::UserRuntimeParameters;
use super::types::{
    assert_hold_category_valid, AgentDecision, DecisionReviewResult, HoldCategoryAssertion,
    KnowledgeRouteResult, ReviewScores, RunPlannerResult, EVENT_AUTONOMY_HOLD_CATEGORY_INVALID,
    HOLD_CATEGORY_AI_WAITING_FOR_MORE_CONTEXT, HOLD_CATEGORY_BLOCKED_BY_SAFETY_GUARD,
    HOLD_CATEGORY_HELD_BY_AI_POLICY,
};

pub(crate) fn effective_review_mode(
    planner: &RunPlannerResult,
    decision: &AgentDecision,
    runtime: &UserRuntimeParameters,
    force_full: bool,
) -> &'static str {
    if force_full || planner.risk_level == "high" || planner.knowledge_required {
        return "full";
    }
    // MP-10 / Task 14：低 confidence 强制 full review。
    let confidence = decision.operation_state_confidence.unwrap_or(10);
    if confidence < runtime.operation_state_confidence_full_review_below {
        return "full";
    }
    if planner.review_mode == "light" {
        "light"
    } else {
        "full"
    }
}

pub(crate) fn should_run_review(
    decision: &AgentDecision,
    planner: &RunPlannerResult,
    runtime: &UserRuntimeParameters,
) -> bool {
    let confidence = decision.operation_state_confidence.unwrap_or(10);
    decision.should_reply
        && (decision.needs_review
            || decision.risk_level == "high"
            || planner.risk_level == "high"
            || planner.knowledge_required
            || confidence < runtime.operation_state_confidence_full_review_below)
}

/// agent-autonomy-loop W2 / Task 3.1：`local_decision_review` 二态语义。
///
/// 旧语义：无论 budget 是否超额、`needs_review` 取值如何，本函数都返回
/// `approved=true` + 一组保守评分；导致预算超额仍可能放过高风险回复。
/// 新语义按 R3.7 / R3.8 / R3.10 拆成三种确定性路径：
///
/// * `budget.is_exceeded() && decision.needs_review == true`：返回
///   `approved=false` + `risks=["budget_exceeded_no_review"]`；调用方
///   （`finalize_review_for_send`）后续 SHALL 把 `autonomy_mode` 强制改写
///   为 `"blocked"`，本函数本身不直接改写 decision；
/// * `budget.is_exceeded() && decision.needs_review == false`：返回
///   `approved=true` + `risks` 追加 `"local_review_low_risk_only"`，
///   `autonomy_mode` 保持原值（低风险快速通道）；
/// * 默认（未超额）：保留与旧实现一致的 `approved=true` + 保守评分。
///
/// 注意：本函数不依赖 task-local `RUN_BUDGET`，调用方必须显式传入
/// `&RunBudget`，便于 `simulation` 等持有自己 `Arc<RunBudget>` 的入口
/// 复用同一份判定逻辑。
///
/// agent-autonomy-loop W3 / Task 4.13：本函数同时作为 P3 性质测试的公开入
/// 口（`tests/autonomy_protocol_pbt.rs`），故可见性提升为 `pub`；语义不变。
pub fn local_decision_review(
    decision: &AgentDecision,
    budget: &RunBudget,
) -> DecisionReviewResult {
    if budget.is_exceeded() {
        if decision.needs_review {
            // R3.7：高风险路径 — 不放行，由 finalize 阶段补 autonomy_mode=blocked。
            return DecisionReviewResult {
                approved: false,
                scores: ReviewScores {
                    human_like: 0,
                    emotional_value: 0,
                    product_accuracy: 0,
                    relationship_progress: 0,
                    conversion_readiness: 0,
                    pressure_risk: 0,
                    fact_risk: 0,
                },
                risks: vec!["budget_exceeded_no_review".to_string()],
                review_summary: "预算超额且 needs_review=true：本地兜底拒绝放行，等待 finalize 强制 blocked".to_string(),
                ..Default::default()
            };
        }

        // R3.8：低风险快速通道 — 仍然 approved，但显式标注本路径未走 LLM review。
        return DecisionReviewResult {
            approved: true,
            scores: ReviewScores {
                human_like: 8,
                emotional_value: 7,
                product_accuracy: if decision.knowledge_need == "required" {
                    7
                } else {
                    10
                },
                relationship_progress: 6,
                conversion_readiness: 5,
                pressure_risk: 1,
                fact_risk: 0,
            },
            risks: vec!["local_review_low_risk_only".to_string()],
            review_summary: "预算超额但 needs_review=false：本地低风险快速通道放行".to_string(),
            ..Default::default()
        };
    }

    // 默认路径（未超额）：与旧实现一致的保守 approved 结果。
    DecisionReviewResult {
        approved: true,
        scores: ReviewScores {
            human_like: 8,
            emotional_value: 7,
            product_accuracy: if decision.knowledge_need == "required" {
                7
            } else {
                10
            },
            relationship_progress: 6,
            conversion_readiness: 5,
            pressure_risk: 1,
            fact_risk: 0,
        },
        review_summary: "低风险 fast_chat 本地轻量审核通过".to_string(),
        ..Default::default()
    }
}

pub(crate) fn review_passed(
    review: &DecisionReviewResult,
    runtime: &UserRuntimeParameters,
) -> bool {
    review.approved
        && review.scores.fact_risk < runtime.fact_risk_block_at
        && review.scores.pressure_risk < runtime.pressure_risk_block_at
        && review.scores.human_like >= runtime.human_like_rewrite_below
        && review.scores.emotional_value >= runtime.emotional_value_rewrite_below
        && review.scores.product_accuracy >= runtime.product_accuracy_block_below
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn review_decision(
    state: &AppState,
    contact: &Contact,
    inbound: &ConversationMessage,
    decision: &AgentDecision,
    playbook: Option<&OperationPlaybook>,
    domain_config: Option<&OperationDomainConfig>,
    runtime: &UserRuntimeParameters,
    memory: &OperatingMemory,
    context_pack: &Document,
    operation_knowledge: &[OperationKnowledgeItem],
    knowledge_chunks: &[OperationKnowledgeChunk],
    knowledge_route: &KnowledgeRouteResult,
    review_mode: &str,
    run_id: Option<&str>,
) -> AppResult<DecisionReviewResult> {
    if !decision.should_reply {
        return Ok(DecisionReviewResult {
            approved: true,
            scores: ReviewScores {
                human_like: 10,
                emotional_value: 10,
                product_accuracy: 10,
                relationship_progress: 5,
                conversion_readiness: 5,
                pressure_risk: 0,
                fact_risk: 0,
            },
            review_summary: "无需回复，无发送风险".to_string(),
            ..Default::default()
        });
    }
    let prompt_key = if review_mode == "light" {
        "user.review.light.system"
    } else {
        "user.review.system"
    };
    let system =
        prompts::load_prompt(&state.db, &state.config.default_workspace_id, prompt_key).await?;
    let runtime_text = serde_json::to_string(&runtime.as_document()).unwrap_or_default();
    let memory_card_text = serde_json::to_string(context_pack).unwrap_or_default();
    let memory_text = serde_json::to_string(&mongodb::bson::doc! {
        "memoryCard": context_pack.clone(),
        "relationshipState": memory.relationship_state.clone(),
        "productFit": memory.product_fit.clone(),
        "nextAction": memory.next_action.clone()
    })
    .unwrap_or_default();
    let knowledge_route_text = serde_json::to_string(knowledge_route).unwrap_or_default();
    let user = format!(
        r#"请评审候选回复。
Review 模式: {}
输出 JSON：
{{
  "approved": true,
  "scores": {{
    "humanLike": 8,
    "emotionalValue": 7,
    "productAccuracy": 9,
    "relationshipProgress": 6,
    "conversionReadiness": 6,
    "pressureRisk": 2,
    "factRisk": 1
  }},
  "formulaBreakdown": {{
    "trust": "Credibility + Reliability + Intimacy - SelfOrientation",
    "conversionReadiness": "Motivation × ProductFit × Timing × Trust ÷ Friction",
    "emotionalValue": "Empathy + Validation + Specificity + AutonomySupport - Pressure"
  }},
  "claimAnalysis": {{
    "hasProductClaim": false,
    "requiresProductKnowledge": false,
    "knowledgeSupported": true,
    "reason": "说明候选回复是否涉及我方产品能力、价格、案例、效果、交付、承诺等需要知识库支撑的表述"
  }},
  "risks": [],
  "rewriteInstruction": "",
  "reviewSummary": ""
}}

评审原则：
- 转化平衡：既允许适度推进，也不能伤害信任。
- 禁止虚假稀缺、恐惧营销、编造案例、编造价格、编造承诺。
- 如果不像微信真人、太模板、太销售，要降低 humanLike 或提高 pressureRisk。
- 如果没有基于产品知识却做了产品承诺，要提高 factRisk 和降低 productAccuracy。
- 产品知识为空时，允许关系维护、测试消息和轻量澄清；但任何具体价格、案例、效果保证、产品能力承诺都必须视为事实风险。
- 知识切片只能作为导航；涉及产品能力、案例、价格、效果、交付承诺时，候选回复必须由 verifiedClaims、sourceAnchors 或 evidenceItems 支撑。
- 如果候选回复使用了未验证切片、无 sourceAnchors 的事实、unsupportedClaims 或 needs_review/rejected 内容，应提高 factRisk 并要求改写或拦截。
- claimAnalysis 必须基于语义判断，不要按关键词判断。用户原话中的“AI运营”“自动化”等词不等于产品承诺；只有候选回复在表达我方能提供什么、保证什么、价格/案例/效果/交付能力时，才算需要产品知识支撑。
- 如果候选回复只是承接用户顾虑、表达理解、提出轻量澄清问题，requiresProductKnowledge=false。
- 必须检查候选回复是否违背长期记忆卡片里的 doNotDo、commitments、coreFacts、recentFacts、objections 和 deprecatedFacts；违背时应提高风险并要求改写或拦截。
- 如果 doNotDo 或用户最新消息要求不要连续提问、不要追问、降低打扰，而候选回复仍继续追问或一次问多个问题，应提高 pressureRisk，必要时不通过。
- 如果最近聊天中我方上一轮已经问了某个问题，用户没有回答而是在表达新顾虑，候选回复不应重复同一个问题；重复追问应视为人味和情绪价值不足。
- 如果用户提出清单、步骤、准备事项、方案框架，候选回复只说“我发你/我整理给你”但没有实际给出内容或创建资源动作，应降低 Reliability/EmotionalValue 并要求改写。
- 长对话里候选回复不能每轮都只追问。若用户已经给出明确方向，回复应至少包含一个具体判断、可执行建议或小框架，否则应要求改写。
- 如果候选回复暗示未提供来源的过往客户案例、行业经验、个人经历，或使用“完全可以/一定/保证”等绝对化产品能力表述，应提高 factRisk 或要求改写为保守表达。

客户最新消息:
{}

候选回复:
{}

决策:
{}

长期运营记忆:
{}

长期记忆卡片:
{}

运营方法:
{}

用户运营域策略:
{}

硬运行参数:
{}

产品知识:
{}

知识路由:
{}"#,
        review_mode,
        inbound.content,
        decision.reply_text,
        serde_json::to_string(decision).unwrap_or_default(),
        memory_text,
        memory_card_text,
        playbook.map(format_playbook_for_prompt).unwrap_or_default(),
        domain_config
            .map(format_operation_domain_config_for_prompt)
            .unwrap_or_default(),
        runtime_text,
        format_operation_knowledge_for_prompt(operation_knowledge, knowledge_chunks),
        knowledge_route_text
    );
    let value = generate_agent_json(
        state,
        Some(&contact.account_id),
        Some(&contact.wxid),
        run_id,
        prompt_key,
        &system,
        &user,
    )
    .await?;
    let mut review: DecisionReviewResult = serde_json::from_value(value)?;
    let markers = load_product_claim_markers(state).await;
    enforce_decision_guards_with_markers(
        &mut review,
        decision,
        domain_config,
        operation_knowledge,
        knowledge_chunks,
        contact.operation_state.as_deref(),
        &markers,
    );
    review.approved = review_passed(&review, runtime);
    Ok(review)
}

// ─────────────────────────────────────────────────────────────────────────
// agent-autonomy-loop W2 / Task 3.2：`finalize_review_for_send` 最终安全汇总层。
//
// 设计 §4.5 / N3：把 `RawAgentDecision::validate_and_promote` 的 promote_risks、
// `local_decision_review` / `review_decision` 输出的 review、以及 R5 verified
// knowledge 强约束 / R5.3 claim_analysis 缺失 fail-closed / R8 字典 candidate
// 标记 / R2.6 should_hold + holdCategory 校验等所有"硬安全门"汇总到一处，
// 任一硬门触发 SHALL 强制 `decision.should_reply=false` +
// `decision.autonomy_mode="blocked"`，并产出 [`FinalizeOutcome`] 描述本次
// 终态（含 `gateway_status` / `final_review_status` / 待写 `agent_events`）。
//
// 设计原则：
// * **纯函数**：本函数不写库、不调 LLM，仅对 `decision` / `review` 做内存变更；
//   产生的事件以 [`PendingFinalizeEvent`] 形式返回给 task 3.4 的 gateway 主路径
//   持久化（避免在 review.rs 中引入 AppState/db 反向依赖）。
// * **任何上游 `approved=true` SHALL NOT 绕过本函数**：finalize 是发送前的
//   最后一道闸门，调用方在三分支（budget_exceeded / should_run_review / 默认）
//   后 SHALL 一律走本函数（详见 task 3.4）。
// * **顺序**：与 R3.5 → R3.7 → R5.4 → R5.3 → R8 → R2.6 严格一致；前置硬门
//   命中后短路返回，避免后续门叠加噪声；R8 字典 candidate 仅追加 risks，
//   不阻塞；R2.6 holdCategory 校验放在最后保证非法值被矫正前其它路径
//   有机会先决定 status。
// ─────────────────────────────────────────────────────────────────────────

/// `finalize_review_for_send` 输出的 `gateway_status` × `finalReviewStatus` 终态。
///
/// 严格对齐 requirements.md "状态枚举映射表"。`Approved` 表示通过本汇总层
/// （未触发任何硬门，且 `review.approved && decision.should_reply`），允许
/// 进入 R2 single-shot revision 或 outbox enqueue（由 task 3.4 决定）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatewayStatusFinal {
    /// 通过本汇总层。等价于 `gateway_status = "approved"` +
    /// `finalReviewStatus = "approved"`（W2 task 3.4 在 revision 路径下可
    /// 改写为 `revision_applied_approved`）。
    Approved,
    /// R3.5 / R3.6：必填字段 / 枚举非法 → blocked_by_required_field。
    BlockedByRequiredField,
    /// R3.7：预算超额 + needs_review=true → blocked_by_budget。
    BlockedByBudget,
    /// R5.4：requiresProductKnowledge=true 且 verified_chunks=∅ →
    /// blocked_unverified_product_claim。
    BlockedUnverifiedProductClaim,
    /// R5.3.a：claim_analysis 缺失 / 损坏且推断为产品声明 → fail-closed
    /// blocked_by_safety_guard。
    BlockedBySafetyGuard,
    /// R2.6：Review Agent 输出 should_hold=true，按 hold_category 分类。
    Held(String),
}

impl GatewayStatusFinal {
    /// 映射到 `agent_run_logs.status / gateway_result.gatewayStatus` 落库字面量。
    pub(crate) fn gateway_status_str(&self) -> String {
        match self {
            GatewayStatusFinal::Approved => "approved".to_string(),
            GatewayStatusFinal::BlockedByRequiredField => "blocked_by_required_field".to_string(),
            GatewayStatusFinal::BlockedByBudget => "blocked_by_budget".to_string(),
            GatewayStatusFinal::BlockedUnverifiedProductClaim => {
                "blocked_unverified_product_claim".to_string()
            }
            GatewayStatusFinal::BlockedBySafetyGuard => "blocked_by_safety_guard".to_string(),
            GatewayStatusFinal::Held(category) => category.clone(),
        }
    }

    /// 映射到 `agent_run_logs.final_review_status` 落库字面量（与 R9.2 严格枚举对齐）。
    pub(crate) fn final_review_status_str(&self) -> String {
        // gateway_status 与 finalReviewStatus 在所有 finalize 终态下一一对应；
        // R2 revision 应用后 task 3.4 会把 `Approved` 改写为
        // `revision_applied_approved`，本函数不参与该改写。
        self.gateway_status_str()
    }
}

/// finalize 阶段产生但尚未写库的 `agent_events` 条目。
///
/// 由调用方（task 3.4 gateway 主路径）调用 [`write_event_for_account`] 持久化。
/// 把事件先聚合再批量写，便于单元测试断言事件 kind / details 而无需
/// mock AppState / Mongo。
///
/// 注意：`Document` 不实现 `Eq`，故本结构仅 `PartialEq`，供单元测试断言使用。
#[derive(Debug, Clone, PartialEq)]
pub struct PendingFinalizeEvent {
    pub kind: String,
    pub status: String,
    pub summary: String,
    pub details: Document,
}

/// `finalize_review_for_send` 完整输出。
///
/// 调用方典型用法（task 3.4）：
/// ```ignore
/// let outcome = finalize_review_for_send(raw_review, &mut decision, &runtime, ...);
/// for event in &outcome.pending_events {
///     write_event_for_account(state, ..., &event.kind, &event.status,
///                              &event.summary, Some(event.details.clone())).await?;
/// }
/// match outcome.status {
///     GatewayStatusFinal::Approved => /* 进入 outbox enqueue */,
///     _ => /* 写 finalReviewStatus，不发送 */,
/// }
/// ```
#[derive(Debug, Clone)]
pub struct FinalizeOutcome {
    /// finalize 后的 review（risks 已聚合 promote_risks + finalize 阶段追加）。
    pub review: DecisionReviewResult,
    /// 终态枚举（见 [`GatewayStatusFinal`]）。
    pub status: GatewayStatusFinal,
    /// 待写 `agent_events` 列表。
    pub pending_events: Vec<PendingFinalizeEvent>,
}

/// agent-autonomy-loop W2 / Task 3.2（R3.5 / R3.7 / R5.3 / R5.4 / R5.7 / R2.6 / R8）：
/// 最终安全汇总层。
///
/// 详见模块上方的长 doc-comment。本函数 SHALL 是**纯函数**（仅修改入参引用
/// 与构造返回值），不调用 LLM、不写库；事件以 [`PendingFinalizeEvent`] 形式
/// 返回，由 task 3.4 gateway 主路径在持有 `&AppState` 时持久化。
///
/// 参数：
/// * `review`：上游 `local_decision_review` / `review_decision` 输出的评审结
///   果（已通过 `enforce_decision_guards`，但尚未做 R5.3 fail-closed 推断 /
///   R5.4 verified_chunks 校验 / R2.6 holdCategory 矫正）。
/// * `decision`：候选回复决策；finalize 触发硬门时 SHALL 把 `should_reply`
///   强制 false、`autonomy_mode` 强制 `"blocked"`。
/// * `_runtime`：运行时硬参数，本期保留参数位以匹配 design.md §4.5 签名，
///   后续 task 3.4 / W3 接入 taxonomy / R8 时使用。
/// * `_contact`：当前 contact，本期保留参数位（同上，task 3.4 / R8 使用）。
/// * `knowledge_chunks`：当前 run 已加载的知识切片，用于 R5.4
///   verified_chunks 计算与 R5.7 safe_claims 反向门。
/// * `markers`：`enforce_string_fact_risk_guard` 的产品声明标记词集合，用于
///   R5.3.a fail-closed 推断。
/// * `promote_risks`：来自 [`super::types::RawAgentDecision::validate_and_promote`]
///   的协议违规标签（如 `missing_required_field:* / invalid_enum_value:* /
///   invalid_type:* / decision_phase_invalid:* /
///   insufficient_detail_in_critical_turn:*`）。
/// ISSUE-003 (R13)：判断 inbound 文本中是否有任何产品 / 价格 / 承诺类
/// marker 命中（白名单豁免后），用于 R5.3.a fail-closed 软化判定。
///
/// 返回 true 表示 inbound 完全无产品意图（极简问候 / 闲聊），此时若仅 LLM
/// reply_text 自发输出含 marker（trigger=string_marker_hit），认为是 LLM
/// 输出模板偶发误命中，降级到 R5.3.b risks-only 不 block。
///
/// inbound_text 为空字符串时返回 false（保持 fail-closed 安全侧默认）。
pub(crate) fn inbound_has_no_product_marker(
    inbound_text: &str,
    markers: &ProductClaimMarkers,
) -> bool {
    if inbound_text.is_empty() {
        return false;
    }
    let hits = markers.scan(inbound_text);
    let real_hit = hits
        .iter()
        .any(|hit| !markers.passes_whitelist(inbound_text, hit));
    !real_hit
}

pub fn finalize_review_for_send(
    review: DecisionReviewResult,
    decision: &mut AgentDecision,
    _runtime: &UserRuntimeParameters,
    _contact: &Contact,
    knowledge_chunks: &[OperationKnowledgeChunk],
    markers: &ProductClaimMarkers,
    promote_risks: Vec<String>,
    inbound_text: &str,
) -> FinalizeOutcome {
    let mut review = review;
    let mut pending_events: Vec<PendingFinalizeEvent> = Vec::new();

    // 把 promote_risks 合并进 review.risks，去重保序，避免后续 has_protocol_violation
    // / contains 检查漏掉这些上游标签。
    extend_risks_unique(&mut review.risks, promote_risks.iter().cloned());

    // ── R3.5 / R3.6：必填字段 / 枚举非法 → blocked_by_required_field ──
    //
    // 凡 promote_risks 中含 `missing_required_field:* / invalid_enum_value:* /
    // invalid_type:* / decision_phase_invalid:* /
    // insufficient_detail_in_critical_turn:*` 任一标签，SHALL 强制 blocked。
    if has_protocol_violation(&promote_risks) {
        review.approved = false;
        decision.should_reply = false;
        decision.autonomy_mode = "blocked".to_string();
        let mut details = Document::new();
        details.insert(
            "violations",
            promote_risks
                .iter()
                .filter(|r| is_protocol_violation_tag(r))
                .cloned()
                .collect::<Vec<_>>(),
        );
        pending_events.push(PendingFinalizeEvent {
            kind: "autonomy_field_violation".to_string(),
            status: "blocked".to_string(),
            summary: "自治协议必填 / 枚举校验失败：本次决策被强制 blocked".to_string(),
            details,
        });
        review.final_review_status = "blocked_by_required_field".to_string();
        return FinalizeOutcome {
            review,
            status: GatewayStatusFinal::BlockedByRequiredField,
            pending_events,
        };
    }

    // ── R3.7：预算超额 + needs_review=true → blocked_by_budget ──
    //
    // `local_decision_review` 在 R3.7 路径下已把 risks 设为
    // `["budget_exceeded_no_review"]`。本层只负责把 autonomy_mode 强制
    // `"blocked"` 并切到对应终态（autonomy_mode 在 local 阶段未改写以保持
    // 单一职责，详见 task 3.1 doc-comment）。
    if review.risks.iter().any(|r| r == "budget_exceeded_no_review") {
        review.approved = false;
        decision.should_reply = false;
        decision.autonomy_mode = "blocked".to_string();
        pending_events.push(PendingFinalizeEvent {
            kind: "budget_exceeded_no_review".to_string(),
            status: "blocked".to_string(),
            summary: "预算超额且 needs_review=true：本次决策被强制 blocked".to_string(),
            details: Document::new(),
        });
        review.final_review_status = "blocked_by_budget".to_string();
        return FinalizeOutcome {
            review,
            status: GatewayStatusFinal::BlockedByBudget,
            pending_events,
        };
    }

    // ── R5.4：verified knowledge 强约束 ──
    //
    // 仅当 `claim_analysis` 显式声明 `requiresProductKnowledge=true` 时触发；
    // claim_analysis 缺失 / 损坏走 R5.3 推断分支（下方）。
    let claim_analysis = &review.claim_analysis.clone();
    let claim_malformed = claim_analysis_is_malformed(claim_analysis);
    let requires_product_knowledge =
        !claim_malformed && claim_requires_product_knowledge(claim_analysis);

    if requires_product_knowledge {
        let verified_chunks = compute_verified_chunks(&decision.used_knowledge_ids, knowledge_chunks);
        if verified_chunks.is_empty() {
            review.approved = false;
            review.scores.fact_risk = review.scores.fact_risk.max(6);
            extend_risks_unique(
                &mut review.risks,
                std::iter::once("product_claim_without_verified_knowledge".to_string()),
            );
            decision.should_reply = false;
            decision.autonomy_mode = "blocked".to_string();
            let mut details = Document::new();
            details.insert(
                "used_knowledge_ids",
                decision.used_knowledge_ids.clone(),
            );
            details.insert("knowledge_chunk_total", knowledge_chunks.len() as i64);
            pending_events.push(PendingFinalizeEvent {
                kind: "product_claim_blocked".to_string(),
                status: "blocked".to_string(),
                summary: "产品声明缺少 verified knowledge 支撑：本次决策被强制 blocked"
                    .to_string(),
                details,
            });
            review.final_review_status = "blocked_unverified_product_claim".to_string();
            return FinalizeOutcome {
                review,
                status: GatewayStatusFinal::BlockedUnverifiedProductClaim,
                pending_events,
            };
        }

        // R5.7 safe_claims 反向门：不阻塞，仅 risks 标记。
        let unverified =
            compute_unverified_safe_claims(&decision.safe_claims_used, &verified_chunks);
        if !unverified.is_empty() {
            append_unverified_safe_claim_risks(&mut review.risks, &unverified);
        }
    }

    // ── R5.3.a / R5.3.b：claim_analysis 缺失 / 损坏 fail-closed 推断 ──
    if claim_malformed {
        // 无论 fail-closed 是否触发，都 SHALL 追加 `claim_analysis_malformed`
        // 留痕（R5.3 末段）。
        extend_risks_unique(
            &mut review.risks,
            std::iter::once("claim_analysis_malformed".to_string()),
        );

        if let Some(trigger) = infer_product_claim_trigger(decision, markers) {
            // ISSUE-003 (R13)：S1 happy 极简 inbound 偶发误伤软化路径。
            // 仅当 trigger=string_marker_hit (LLM 自发输出含 marker 但 inbound
            // 完全无产品/价格/承诺意图) 时降级到 R5.3.b risks-only。
            // knowledge_need / used_knowledge_ids 是 LLM 自我声明，不软化。
            let softened = trigger == "string_marker_hit"
                && inbound_has_no_product_marker(inbound_text, markers);
            if softened {
                let mut details = Document::new();
                details.insert("original_trigger", trigger.to_string());
                details.insert("inbound_text_len", inbound_text.chars().count() as i64);
                pending_events.push(PendingFinalizeEvent {
                    kind: "claim_analysis_malformed_softened".to_string(),
                    status: "warning".to_string(),
                    summary:
                        "claim_analysis 缺失但 inbound 无产品意图 + 仅 reply_text 误命中 marker → 降级 R5.3.b（仅 risks 不 block）"
                            .to_string(),
                    details,
                });
                extend_risks_unique(
                    &mut review.risks,
                    std::iter::once(format!("claim_malformed_softened:{trigger}")),
                );
                // 落入 R5.3.b 路径：不 fail-closed，让下游 should_hold / approved
                // 自然判定。
            } else {
                // R5.3.a：fail-closed → blocked_by_safety_guard
                review.approved = false;
                review.scores.fact_risk = review.scores.fact_risk.max(6);
                decision.should_reply = false;
                decision.autonomy_mode = "blocked".to_string();
                let mut details = Document::new();
                details.insert("triggered_by", trigger.to_string());
                details.insert("knowledge_need", decision.knowledge_need.clone());
                details.insert(
                    "used_knowledge_ids_len",
                    decision.used_knowledge_ids.len() as i64,
                );
                pending_events.push(PendingFinalizeEvent {
                    kind: "claim_analysis_malformed_fail_closed".to_string(),
                    status: "blocked".to_string(),
                    summary:
                        "claim_analysis 缺失 / 损坏，推断为产品声明 → fail-closed blocked_by_safety_guard"
                            .to_string(),
                    details,
                });
                review.final_review_status = "blocked_by_safety_guard".to_string();
                return FinalizeOutcome {
                    review,
                    status: GatewayStatusFinal::BlockedBySafetyGuard,
                    pending_events,
                };
            }
        }
        // R5.3.b：claim_analysis 缺失但非产品声明 → 仅 risks 标记，不 block。
    }

    // ── R8 字典 candidate 标记（占位，不阻塞）──
    //
    // 真正的 taxonomy::check_value / upsert_candidate 在 W3 task 4.7 接入；
    // 本层不引入 W3 模块依赖，仅保留接口位（任何上游已经 push 的
    // `taxonomy_candidate:*` / `taxonomy_deprecated_value:*` risks SHALL NOT
    // 强制 review.approved=false，与 R8.4 约束一致）。

    // ── R2.6：should_hold + holdCategory 校验 ──
    //
    // 任何 should_hold=true 路径在 finalize 终态前都 SHALL 经过本校验，避免
    // 上游写入 `held_for_human / human_required` 等暗示人工接管的取值穿透
    // 到落库或前端显示（违反 R2.7 业务语义保护）。
    let assertion = assert_hold_category_valid(&mut review);
    if let HoldCategoryAssertion::Coerced { original } = &assertion {
        let mut details = Document::new();
        details.insert("original", original.clone());
        details.insert("coerced_to", HOLD_CATEGORY_HELD_BY_AI_POLICY.to_string());
        pending_events.push(PendingFinalizeEvent {
            kind: EVENT_AUTONOMY_HOLD_CATEGORY_INVALID.to_string(),
            status: "warning".to_string(),
            summary: format!(
                "Review Agent 输出非法 hold_category=\"{original}\"，强制改写为 held_by_ai_policy"
            ),
            details,
        });
    }

    if review.should_hold {
        // hold_category 已被 assert_hold_category_valid 矫正到合法三选一。
        let category = review.hold_category.clone();
        debug_assert!(
            matches!(
                category.as_str(),
                HOLD_CATEGORY_HELD_BY_AI_POLICY
                    | HOLD_CATEGORY_BLOCKED_BY_SAFETY_GUARD
                    | HOLD_CATEGORY_AI_WAITING_FOR_MORE_CONTEXT
            ),
            "assert_hold_category_valid SHALL 把 hold_category 矫正到三选一"
        );
        decision.should_reply = false;
        // hold 路径不强制 autonomy_mode = blocked：`held_by_ai_policy /
        // ai_waiting_for_more_context` 是 AI 主动暂缓，可保留 assisted；
        // `blocked_by_safety_guard` 由 R5 路径触发时已强制 blocked。
        review.final_review_status = category.clone();
        return FinalizeOutcome {
            review,
            status: GatewayStatusFinal::Held(category),
            pending_events,
        };
    }

    // ── 默认：approved 通过 ──
    if review.approved && decision.should_reply {
        review.final_review_status = "approved".to_string();
        FinalizeOutcome {
            review,
            status: GatewayStatusFinal::Approved,
            pending_events,
        }
    } else {
        // approved=false 但未触发任何硬门（例如 review_passed 阈值不够）；
        // 不发送，但也不阻断后续 R2 revision 路径（task 3.4 决定）；这里
        // 用 held_by_ai_policy 作为兜底分类，避免空字符串穿透到 finalReviewStatus。
        review.final_review_status = HOLD_CATEGORY_HELD_BY_AI_POLICY.to_string();
        FinalizeOutcome {
            review,
            status: GatewayStatusFinal::Held(HOLD_CATEGORY_HELD_BY_AI_POLICY.to_string()),
            pending_events,
        }
    }
}

/// 判断 `risks` 中是否包含任何"自治协议违规"标签（R3.5 / R3.6 / R1.5 / R1.10）。
fn has_protocol_violation(risks: &[String]) -> bool {
    risks.iter().any(|r| is_protocol_violation_tag(r))
}

/// 单个 risk 标签是否属于"自治协议违规"语义。
fn is_protocol_violation_tag(risk: &str) -> bool {
    risk.starts_with("missing_required_field:")
        || risk.starts_with("invalid_enum_value:")
        || risk.starts_with("invalid_type:")
        || risk.starts_with("decision_phase_invalid:")
        || risk.starts_with("insufficient_detail_in_critical_turn:")
}

/// 把新 risks 追加到 `risks` 末尾，跳过已存在的字面量（保序去重）。
fn extend_risks_unique<I: IntoIterator<Item = String>>(risks: &mut Vec<String>, iter: I) {
    for tag in iter {
        if !risks.iter().any(|r| r == &tag) {
            risks.push(tag);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// agent-autonomy-loop W2 / Task 3.7：R2 single-shot revision 控制流纯函数。
//
// `gateway::run_user_operation_gateway` 中的 R2 revision 块（约 ~660-960 行）
// 与 `AppState` / `RunBudget` task-local / 异步 LLM 调用 / Mongo 事件写入紧
// 耦合，难以单测。这里把"是否触发 revision"和"如何把 revision 失败映射
// 为 finalize 终态"两段纯逻辑提取出来，便于 task 3.7 的 ≥ 5 例 lib 单元
// 测试覆盖（gateway.rs 仍负责 LLM 调用 / timeout / 事件持久化等副作用，
// 直接 dispatch 到本模块的纯函数）。
//
// 设计原则：
// * 纯函数：本模块决策函数不读取 task-local 状态、不调 LLM、不写库；
//   `budget_exceeded` 由调用方通过 `current_run_budget()` 计算后传入；
// * 与 design.md §4.5 状态映射表一致：revision 触发的 4 类失败终态
//   （revision_skipped_invalid_direction / revision_skipped_budget_exceeded /
//   revision_llm_failure / revision_failed）SHALL 映射到 `finalReviewStatus
//   = "revision_failed"` + `gateway_status = Held(held_by_ai_policy)` +
//   `should_reply = false`；revision 触发本身的"事件 kind"由
//   [`RevisionDecision::Skip`] / [`derive_revision_failure`] 显式返回，
//   gateway.rs 持有 `&AppState` 时 SHALL 写 `agent_events`。
// ─────────────────────────────────────────────────────────────────────────

/// `decide_revision` 输出：是否触发 single-shot revision。
///
/// 设计 §4.5 R2.3 / R2.5 / R2.8 / R2.9：
/// * `NotEligible`：上游 finalize 未通过（status != Approved 或 should_hold=true
///   或 needs_revision=false）→ 不进入 revision 块；
/// * `Skip { reason, event }`：进入 revision 块但被前置条件拦截
///   （revisionDirection 空 / 预算超额）→ 写指定 `agent_events.kind`，
///   终态由 [`derive_revision_failure`] 决定；
/// * `Proceed`：调用 Reply Agent 第二次。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RevisionDecision {
    /// 不触发 revision（finalize 已 hold/blocked，或 review 未要求 revision）。
    NotEligible,
    /// 进入 revision 但被前置条件跳过；`event` 为 `agent_events.kind`，
    /// `reason` 落 `agent_run_logs.revision_reason` 字段。
    Skip {
        reason: &'static str,
        event: &'static str,
    },
    /// 通过所有前置条件，调用方 SHALL 调用 Reply Agent 第二次。
    Proceed,
}

/// agent-autonomy-loop W2 / Task 3.7（R2.3 / R2.5 / R2.8 / R2.9）：纯函数判定
/// 是否触发 single-shot revision。
///
/// 调用方典型用法（gateway.rs）：
/// ```ignore
/// let budget_exceeded = current_run_budget()
///     .map(|b| b.is_exceeded())
///     .unwrap_or(false);
/// match decide_revision(&finalize_status, &review, budget_exceeded) {
///     RevisionDecision::NotEligible => { /* 跳过 R2 块 */ }
///     RevisionDecision::Skip { reason, event } => {
///         let (reason_str, status) = derive_revision_failure(reason);
///         /* 写 agent_events kind=event，落 revision_reason=reason_str */
///     }
///     RevisionDecision::Proceed => { /* 调用 Reply Agent 第二次 */ }
/// }
/// ```
///
/// 参数：
/// * `finalize_status`：第一轮 finalize 终态；只有 `Approved` 才进入 R2；
/// * `review`：finalize 后的 review，读 `needs_revision / should_hold /
///   revision_direction`；
/// * `budget_exceeded`：调用方根据 `RunBudget::is_exceeded()` 计算的快照
///   （task-local，不在纯函数内读取）。
pub(crate) fn decide_revision(
    finalize_status: &GatewayStatusFinal,
    review: &DecisionReviewResult,
    budget_exceeded: bool,
) -> RevisionDecision {
    // R2.3 前置：finalize 未通过 / 已 hold / review 未要求 revision → 不进 R2 块。
    if !matches!(finalize_status, GatewayStatusFinal::Approved) {
        return RevisionDecision::NotEligible;
    }
    if !review.needs_revision {
        return RevisionDecision::NotEligible;
    }
    if review.should_hold {
        return RevisionDecision::NotEligible;
    }

    // R2.5：revisionDirection 空白（含仅空白）→ Skip("revisionDirection_empty")。
    if review.revision_direction.trim().is_empty() {
        return RevisionDecision::Skip {
            reason: "revisionDirection_empty",
            event: "revision_skipped_invalid_direction",
        };
    }

    // R2.8：revision 之前预算超额 → Skip("budget_exceeded_before_revision")。
    if budget_exceeded {
        return RevisionDecision::Skip {
            reason: "budget_exceeded_before_revision",
            event: "revision_skipped_budget_exceeded",
        };
    }

    RevisionDecision::Proceed
}

/// agent-autonomy-loop W2 / Task 3.7（R2.4 / R2.11）：把 revision 失败原因
/// 映射到 `(revision_reason, GatewayStatusFinal)`。
///
/// 所有 revision 失败路径最终 finalReviewStatus 都 SHALL 是 `"revision_failed"`，
/// gateway_status 都 SHALL 是 `Held(held_by_ai_policy)`（与 design.md §4.5
/// 状态映射表一致）；本函数主要确保 gateway.rs 的 4 个 revision 失败分支
/// （invalid_direction / budget_exceeded / llm_error / llm_timeout /
/// post_review_failed）使用同一套终态字面量，避免散落字面量造成漂移。
///
/// 参数 `reason` 接受以下字面量（gateway.rs 中按分支选择）：
/// * `"revisionDirection_empty"` → R2.5 跳过；
/// * `"budget_exceeded_before_revision"` → R2.8 跳过；
/// * `"revision_llm_timeout_30s"` → R2.11 超时；
/// * `"revision_post_review_failed"` → R2.4 第二轮 review 仍 fail；
/// * 任何 `revision_llm_error:*` 前缀 → R2.11 LLM 业务错误；
/// * 其它字符串 → 视为未知失败原因，仍走 `revision_failed` 终态（fail-closed）。
///
/// 返回 `(revision_reason, status)`：调用方 SHALL 把 `revision_reason` 落
/// `agent_run_logs.revision_reason`，把 `status` 作为 finalize_status 写回
/// gateway 主路径。
pub(crate) fn derive_revision_failure(reason: &str) -> (String, GatewayStatusFinal) {
    // 所有 revision 失败终态统一为 Held(held_by_ai_policy)；finalReviewStatus
    // 由调用方在 review.final_review_status 中显式写 "revision_failed"。
    let status = GatewayStatusFinal::Held(HOLD_CATEGORY_HELD_BY_AI_POLICY.to_string());
    (reason.to_string(), status)
}


#[cfg(test)]
mod local_decision_review_tests {
    //! agent-autonomy-loop W2 / Task 3.1：`local_decision_review` 二态语义单元测试。
    //!
    //! 校验三条路径互斥：
    //! 1. budget 超额 + needs_review=true  → approved=false + risks=["budget_exceeded_no_review"]
    //! 2. budget 超额 + needs_review=false → approved=true  + risks 含 "local_review_low_risk_only"
    //! 3. budget 未超额                    → approved=true  + risks 为空（保留旧默认通过）

    use super::super::budget::RunBudget;
    use super::super::types::AgentDecision;
    use super::local_decision_review;

    fn decision_with_needs_review(needs_review: bool) -> AgentDecision {
        AgentDecision {
            needs_review,
            ..AgentDecision::default()
        }
    }

    /// budget 超额且预算超额发生在 token 维度时的二态判定。
    fn exceeded_budget_by_tokens() -> RunBudget {
        let b = RunBudget::new("test-run-tokens", 100, 10, i32::MAX);
        b.record_call(150); // 超额：tokens_used >= token_budget
        assert!(b.is_exceeded());
        b
    }

    /// budget 超额且预算超额发生在 LLM 调用次数维度时的二态判定。
    fn exceeded_budget_by_calls() -> RunBudget {
        let b = RunBudget::new("test-run-calls", 1_000_000, 1, i32::MAX);
        b.record_call(0); // 超额：llm_calls_used >= max_llm_calls
        assert!(b.is_exceeded());
        b
    }

    /// budget 未超额；任何 needs_review 取值都应走默认 approved 路径。
    fn ok_budget() -> RunBudget {
        let b = RunBudget::new("test-run-ok", 1_000_000, 100, i32::MAX);
        assert!(!b.is_exceeded());
        b
    }

    #[test]
    fn budget_exceeded_with_needs_review_blocks_with_single_risk() {
        let decision = decision_with_needs_review(true);
        let budget = exceeded_budget_by_tokens();

        let review = local_decision_review(&decision, &budget);

        assert!(
            !review.approved,
            "needs_review=true + 预算超额：local_decision_review SHALL 返回 approved=false"
        );
        assert_eq!(
            review.risks,
            vec!["budget_exceeded_no_review".to_string()],
            "risks SHALL 恰好为 [\"budget_exceeded_no_review\"]，不附加其它标签"
        );
    }

    #[test]
    fn budget_exceeded_via_llm_calls_with_needs_review_also_blocks() {
        // 验证 R3.7 不区分预算耗尽维度：tokens 维度与 llm_calls 维度都触发 blocked。
        let decision = decision_with_needs_review(true);
        let budget = exceeded_budget_by_calls();

        let review = local_decision_review(&decision, &budget);

        assert!(!review.approved);
        assert_eq!(review.risks, vec!["budget_exceeded_no_review".to_string()]);
    }

    #[test]
    fn budget_exceeded_without_needs_review_passes_with_low_risk_marker() {
        let decision = decision_with_needs_review(false);
        let budget = exceeded_budget_by_tokens();

        let review = local_decision_review(&decision, &budget);

        assert!(
            review.approved,
            "needs_review=false + 预算超额：本地仍允许放行（低风险快速通道）"
        );
        assert!(
            review.risks.contains(&"local_review_low_risk_only".to_string()),
            "risks 必须含 'local_review_low_risk_only' 标记，便于 finalize 阶段识别本路径"
        );
        assert!(
            !review.risks.contains(&"budget_exceeded_no_review".to_string()),
            "低风险路径不应再追加 budget_exceeded_no_review，避免与 R3.7 路径混淆"
        );
    }

    #[test]
    fn budget_not_exceeded_passes_without_extra_risks() {
        // 默认路径回归：未超额时无论 needs_review 取值都返回 approved 且不携带预算相关风险。
        let budget = ok_budget();

        for needs_review in [true, false] {
            let decision = decision_with_needs_review(needs_review);
            let review = local_decision_review(&decision, &budget);

            assert!(
                review.approved,
                "未超额时 SHALL 保留旧默认 approved 行为 (needs_review={needs_review})"
            );
            assert!(
                review.risks.is_empty(),
                "未超额路径不应追加预算/低风险标签 (risks={:?}, needs_review={needs_review})",
                review.risks
            );
        }
    }

    #[test]
    fn knowledge_required_lowers_product_accuracy_score_when_not_blocked() {
        // 保留旧实现的 product_accuracy 启发式：knowledge_need == "required" 时降为 7。
        // 仅在 approved=true 路径生效；blocked 路径所有评分清零，无需校验。
        let budget = ok_budget();

        let mut decision = AgentDecision::default();
        decision.needs_review = false;
        decision.knowledge_need = "required".to_string();
        let review_required = local_decision_review(&decision, &budget);
        assert!(review_required.approved);
        assert_eq!(review_required.scores.product_accuracy, 7);

        decision.knowledge_need = "not_required".to_string();
        let review_not_required = local_decision_review(&decision, &budget);
        assert!(review_not_required.approved);
        assert_eq!(review_not_required.scores.product_accuracy, 10);
    }
}


#[cfg(test)]
mod finalize_review_for_send_tests {
    //! agent-autonomy-loop W2 / Task 3.2：`finalize_review_for_send` 单元测试。
    //!
    //! 覆盖 design.md §4.5 的 6 类硬安全门 + 1 默认通过路径。task 3.6 会在
    //! 后续 commit 中扩展到 ≥ 7 例并接入 PBT。这里先写"最小覆盖核心分支"
    //! 的 6 例，验证函数签名 + 行为约定固定。
    //!
    //! 三类断言：
    //! 1. `decision.should_reply / autonomy_mode` 在硬门触发时强制改写；
    //! 2. `outcome.status` / `review.final_review_status` 与状态映射表一致；
    //! 3. `outcome.pending_events` 含正确 kind（autonomy_field_violation /
    //!    budget_exceeded_no_review / product_claim_blocked /
    //!    claim_analysis_malformed_fail_closed / autonomy_hold_category_invalid）。

    use mongodb::bson::doc;
    use mongodb::bson::Document;

    use crate::models::{AgentStatus, Contact, OperationKnowledgeChunk};

    use super::super::guards::default_product_claim_markers;
    use super::super::runtime::UserRuntimeParameters;
    use super::super::types::{
        AgentDecision, DecisionReviewResult, EVENT_AUTONOMY_HOLD_CATEGORY_INVALID,
        HOLD_CATEGORY_HELD_BY_AI_POLICY,
    };
    use super::{finalize_review_for_send, inbound_has_no_product_marker, GatewayStatusFinal};

    fn dummy_runtime() -> UserRuntimeParameters {
        // UserRuntimeParameters 没有 Default impl；用 from_config(None, &state) 不可
        // 在测试里构造（需要 AppState）。直接手写一个最小占位（与
        // src/agent/runtime.rs 默认值对齐）即可，因为 finalize_review_for_send
        // 当前只读 contact / chunks / markers / promote_risks，runtime 仅作为
        // 参数位（W3 接入 taxonomy 后才会真正使用）。
        UserRuntimeParameters {
            recent_message_limit: 30,
            min_reply_interval_seconds: 30,
            max_daily_touches: 5,
            max_pending_follow_ups: 3,
            follow_up_expires_hours: 72,
            cooldown_after_no_reply_hours: 24,
            fact_risk_block_at: 6,
            pressure_risk_block_at: 6,
            human_like_rewrite_below: 6,
            emotional_value_rewrite_below: 6,
            product_accuracy_block_below: 6,
            operation_state_confidence_full_review_below: 6,
            run_token_budget: 30000,
            run_max_llm_calls: 6,
            simulation_token_budget: 30000,
            reaction_token_budget: 8000,
            reaction_max_llm_calls: 2,
            autonomy_protocol_enabled: true,
            knowledge_routing_mode: "auto_tool_loop".to_string(),
            knowledge_max_tool_loops: 3,
            knowledge_max_tool_calls: 6,
            knowledge_open_slice_max_k: 4,
            knowledge_search_top_k: 8,
            outbox_poll_interval_seconds: 5,
            outbox_lease_seconds: 60,
        }
    }

    fn dummy_contact() -> Contact {
        Contact {
            id: None,
            workspace_id: "default".to_string(),
            account_id: "default".to_string(),
            wxid: "test_wxid".to_string(),
            nickname: None,
            remark: None,
            alias: None,
            agent_status: AgentStatus::Managed,
            human_profile_note: None,
            agent_profile: None,
            memory_summary: None,
            playbook_id: None,
            playbook_version: None,
            tags: Vec::new(),
            customer_stage: None,
            customer_stage_updated_at: None,
            intent_level: None,
            commitments: Vec::new(),
            follow_up_policy: None,
            operation_state: None,
            operation_state_reason: None,
            operation_state_confidence: None,
            operation_state_updated_at: None,
            cooldown_until: None,
            operation_policy: mongodb::bson::Document::new(),
            profile_attributes: mongodb::bson::Document::new(),
            profile_updated_at: None,
            last_message_at: None,
            last_inbound_at: None,
            last_outbound_at: None,
            last_agent_run_at: None,
            custom_agent_instructions: None,
            created_at: mongodb::bson::DateTime::now(),
            updated_at: mongodb::bson::DateTime::now(),
        }
    }

    fn approved_review() -> DecisionReviewResult {
        DecisionReviewResult {
            approved: true,
            ..Default::default()
        }
    }

    fn approved_decision() -> AgentDecision {
        AgentDecision {
            should_reply: true,
            autonomy_mode: "auto".to_string(),
            run_mode: "fast_chat".to_string(),
            risk_level: "low".to_string(),
            knowledge_need: "not_required".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn protocol_violation_in_promote_risks_blocks_with_required_field_status() {
        // R3.5 / R3.6：promote_risks 中含 missing_required_field SHALL 触发
        // blocked_by_required_field，强制 should_reply=false + autonomy_mode=blocked。
        let mut decision = approved_decision();
        let runtime = dummy_runtime();
        let contact = dummy_contact();
        let markers = default_product_claim_markers();

        let outcome = finalize_review_for_send(
            approved_review(),
            &mut decision,
            &runtime,
            &contact,
            &[],
            &markers,
            vec!["missing_required_field:risk_level".to_string()],
            "",
        );

        assert_eq!(outcome.status, GatewayStatusFinal::BlockedByRequiredField);
        assert_eq!(outcome.review.final_review_status, "blocked_by_required_field");
        assert!(!outcome.review.approved);
        assert!(!decision.should_reply);
        assert_eq!(decision.autonomy_mode, "blocked");
        assert!(outcome
            .pending_events
            .iter()
            .any(|e| e.kind == "autonomy_field_violation"));
    }

    #[test]
    fn budget_exceeded_no_review_blocks_with_budget_status() {
        // R3.7：上游 local_decision_review 已把 risks 设为
        // ["budget_exceeded_no_review"]；finalize SHALL 把 autonomy_mode 强制
        // blocked 并切到 BlockedByBudget。
        let mut decision = approved_decision();
        let runtime = dummy_runtime();
        let contact = dummy_contact();
        let markers = default_product_claim_markers();

        let mut review = approved_review();
        review.approved = false;
        review.risks = vec!["budget_exceeded_no_review".to_string()];

        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            &[],
            &markers,
            Vec::new(),
            "",
        );

        assert_eq!(outcome.status, GatewayStatusFinal::BlockedByBudget);
        assert_eq!(outcome.review.final_review_status, "blocked_by_budget");
        assert!(!decision.should_reply);
        assert_eq!(decision.autonomy_mode, "blocked");
        assert!(outcome
            .pending_events
            .iter()
            .any(|e| e.kind == "budget_exceeded_no_review"));
    }

    #[test]
    fn requires_product_knowledge_without_verified_chunks_blocks_unverified_product_claim() {
        // R5.4：claim_analysis.requiresProductKnowledge=true 且 verified_chunks
        // 集合为空（used_knowledge_ids 全部 integrity_status != "verified" 或 chunks
        // 列表为空）→ blocked_unverified_product_claim。
        let mut decision = approved_decision();
        decision.used_knowledge_ids = vec!["507f1f77bcf86cd799439011".to_string()];

        let mut review = approved_review();
        review.claim_analysis = doc! {
            "requiresProductKnowledge": true,
            "knowledgeSupported": false,
        };

        let runtime = dummy_runtime();
        let contact = dummy_contact();
        let markers = default_product_claim_markers();

        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            &[],
            &markers,
            Vec::new(),
            "",
        );

        assert_eq!(
            outcome.status,
            GatewayStatusFinal::BlockedUnverifiedProductClaim
        );
        assert_eq!(
            outcome.review.final_review_status,
            "blocked_unverified_product_claim"
        );
        assert!(outcome.review.scores.fact_risk >= 6);
        assert!(!decision.should_reply);
        assert_eq!(decision.autonomy_mode, "blocked");
        assert!(outcome
            .pending_events
            .iter()
            .any(|e| e.kind == "product_claim_blocked"));
    }

    #[test]
    fn malformed_claim_analysis_with_required_knowledge_fail_closed_blocked_by_safety_guard() {
        // R5.3.a：claim_analysis 缺失 + decision.knowledge_need="required" → fail-closed
        // blocked_by_safety_guard，事件 detail.triggered_by="knowledge_need"。
        let mut decision = approved_decision();
        decision.knowledge_need = "required".to_string();

        // claim_analysis 留空（默认 Document::new()）触发 malformed 路径。
        let review = approved_review();

        let runtime = dummy_runtime();
        let contact = dummy_contact();
        let markers = default_product_claim_markers();

        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            &[],
            &markers,
            Vec::new(),
            "",
        );

        assert_eq!(outcome.status, GatewayStatusFinal::BlockedBySafetyGuard);
        assert_eq!(outcome.review.final_review_status, "blocked_by_safety_guard");
        assert!(outcome
            .review
            .risks
            .iter()
            .any(|r| r == "claim_analysis_malformed"));
        assert!(!decision.should_reply);
        assert_eq!(decision.autonomy_mode, "blocked");
        let event = outcome
            .pending_events
            .iter()
            .find(|e| e.kind == "claim_analysis_malformed_fail_closed")
            .expect("SHALL emit claim_analysis_malformed_fail_closed event");
        assert_eq!(
            event.details.get_str("triggered_by").unwrap_or_default(),
            "knowledge_need"
        );
    }

    #[test]
    fn malformed_claim_analysis_for_chitchat_only_marks_risks_without_blocking() {
        // R5.3.b：claim_analysis 缺失 + 非产品声明（knowledge_need=not_required +
        // used_knowledge_ids 为空 + 无字符串 marker hit）→ 仅追加 risks 标记，
        // 不 block，进入默认 approved 通过分支。
        let mut decision = approved_decision();
        decision.knowledge_need = "not_required".to_string();
        decision.reply_text = "你好，最近怎么样？".to_string();

        let review = approved_review();
        let runtime = dummy_runtime();
        let contact = dummy_contact();
        let markers = default_product_claim_markers();

        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            &[],
            &markers,
            Vec::new(),
            "",
        );

        assert_eq!(outcome.status, GatewayStatusFinal::Approved);
        assert_eq!(outcome.review.final_review_status, "approved");
        assert!(outcome
            .review
            .risks
            .iter()
            .any(|r| r == "claim_analysis_malformed"));
        assert!(decision.should_reply);
        assert_eq!(decision.autonomy_mode, "auto");
        // R5.3.b 路径下 SHALL NOT 写 fail-closed 事件。
        assert!(!outcome
            .pending_events
            .iter()
            .any(|e| e.kind == "claim_analysis_malformed_fail_closed"));
    }

    #[test]
    fn forbidden_hold_category_is_coerced_to_held_by_ai_policy_with_event() {
        // R2.6 / R9.8：hold_category="held_for_human" SHALL 被强制改为
        // held_by_ai_policy，并产出 autonomy_hold_category_invalid 事件，detail.original
        // 含原值；终态 = Held("held_by_ai_policy")。
        let mut decision = approved_decision();
        let mut review = approved_review();
        review.should_hold = true;
        review.hold_reason = "用户明确表示需要时间考虑".to_string();
        review.hold_category = "held_for_human".to_string();

        let runtime = dummy_runtime();
        let contact = dummy_contact();
        let markers = default_product_claim_markers();

        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            &[],
            &markers,
            Vec::new(),
            "",
        );

        match &outcome.status {
            GatewayStatusFinal::Held(category) => {
                assert_eq!(category, HOLD_CATEGORY_HELD_BY_AI_POLICY);
            }
            other => panic!("expected Held(held_by_ai_policy), got {:?}", other),
        }
        assert_eq!(
            outcome.review.final_review_status,
            HOLD_CATEGORY_HELD_BY_AI_POLICY
        );
        assert_eq!(outcome.review.hold_category, HOLD_CATEGORY_HELD_BY_AI_POLICY);
        assert!(!decision.should_reply);
        let event = outcome
            .pending_events
            .iter()
            .find(|e| e.kind == EVENT_AUTONOMY_HOLD_CATEGORY_INVALID)
            .expect("SHALL emit autonomy_hold_category_invalid event");
        assert_eq!(
            event.details.get_str("original").unwrap_or_default(),
            "held_for_human"
        );
    }

    #[test]
    fn requires_product_knowledge_with_verified_chunks_passes_through() {
        // R5.4 / R5.7 happy path：claim_analysis.requiresProductKnowledge=true，
        // used_knowledge_ids 命中 verified chunks → 通过 → final_review_status=approved，
        // 不强制 blocked。safe_claims 反向门若不命中 verified.safe_claims 仅追加 risks。
        let chunk_id = mongodb::bson::oid::ObjectId::new();
        let chunk = OperationKnowledgeChunk {
            id: Some(chunk_id),
            workspace_id: "default".to_string(),
            account_id: None,
            document_id: None,
            item_id: None,
            domain: "default".to_string(),
            knowledge_type: None,
            business_context: None,
            title: "Verified".to_string(),
            summary: None,
            body: Some("body".to_string()),
            routing_card: None,
            applicable_scenes: Vec::new(),
            not_applicable_scenes: Vec::new(),
            safe_claims: vec!["响应及时".to_string()],
            forbidden_claims: Vec::new(),
            evidence_items: Vec::new(),
            source_quote: None,
            source_anchors: Vec::new(),
            integrity_status: Some("verified".to_string()),
            confidence_score: Some(9),
            distortion_risks: Vec::new(),
            unsupported_claims: Vec::new(),
            verified_claims: Vec::new(),
            status: "active".to_string(),
            priority: 0,
            product_tags: Vec::new(),
            trigger_keywords: Vec::new(),
            business_topics: Vec::new(),
            created_at: mongodb::bson::DateTime::now(),
            updated_at: mongodb::bson::DateTime::now(),
        };

        let mut decision = approved_decision();
        decision.used_knowledge_ids = vec![chunk_id.to_hex()];
        decision.safe_claims_used = vec!["响应及时".to_string()];

        let mut review = approved_review();
        review.claim_analysis = doc! {
            "requiresProductKnowledge": true,
            "knowledgeSupported": true,
        };

        let runtime = dummy_runtime();
        let contact = dummy_contact();
        let markers = default_product_claim_markers();

        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            std::slice::from_ref(&chunk),
            &markers,
            Vec::new(),
            "",
        );

        assert_eq!(outcome.status, GatewayStatusFinal::Approved);
        assert_eq!(outcome.review.final_review_status, "approved");
        assert!(decision.should_reply);
        assert_eq!(decision.autonomy_mode, "auto");
        assert!(!outcome
            .review
            .risks
            .iter()
            .any(|r| r.starts_with("safe_claim_not_verified:")));
    }

    // ─────────────────────────────────────────────────────────────────
    // Task 3.6 扩展（gate matrix 完备性）：补齐 design.md §4.5 在原 7 例
    // 中未单独覆盖的分支：
    //   * R5.7 反向门 happy path（有 verified_chunks + safe_claim 不命中
    //     → 仅追加 risks，不阻塞）；
    //   * R5.7 cap-5 overflow 聚合（7 个不命中 → 5 条 + and_more:2）；
    //   * R8.4 taxonomy_candidate 占位 pass-through（不阻塞）；
    //   * R2.6 canonical hold_category 直通（无矫正事件 +
    //     Held(canonical_category) 终态）；
    //   * 默认 fallback：approved=false 但无硬门触发 → Held(held_by_ai_policy)。
    // ─────────────────────────────────────────────────────────────────

    /// 构造一个含 `safe_claims` 集合 `["a", "b"]` 的 verified knowledge chunk。
    fn build_verified_chunk(safe_claims: Vec<String>) -> OperationKnowledgeChunk {
        OperationKnowledgeChunk {
            id: Some(mongodb::bson::oid::ObjectId::new()),
            workspace_id: "default".to_string(),
            account_id: None,
            document_id: None,
            item_id: None,
            domain: "default".to_string(),
            knowledge_type: None,
            business_context: None,
            title: "Verified".to_string(),
            summary: None,
            body: Some("body".to_string()),
            routing_card: None,
            applicable_scenes: Vec::new(),
            not_applicable_scenes: Vec::new(),
            safe_claims,
            forbidden_claims: Vec::new(),
            evidence_items: Vec::new(),
            source_quote: None,
            source_anchors: Vec::new(),
            integrity_status: Some("verified".to_string()),
            confidence_score: Some(9),
            distortion_risks: Vec::new(),
            unsupported_claims: Vec::new(),
            verified_claims: Vec::new(),
            status: "active".to_string(),
            priority: 0,
            product_tags: Vec::new(),
            trigger_keywords: Vec::new(),
            business_topics: Vec::new(),
            created_at: mongodb::bson::DateTime::now(),
            updated_at: mongodb::bson::DateTime::now(),
        }
    }

    #[test]
    fn requires_product_knowledge_with_unsupported_safe_claims_marks_risks_but_passes() {
        // R5.7 反向门：requiresProductKnowledge=true 且 verified_chunks 非空，
        // 但 safe_claims_used 中存在未被 verified.safe_claims 命中的 claim →
        // 仅追加 `safe_claim_not_verified:<claim>` risks，不阻塞 approved。
        let chunk = build_verified_chunk(vec!["响应及时".to_string()]);
        let chunk_id = chunk.id.unwrap();

        let mut decision = approved_decision();
        decision.used_knowledge_ids = vec![chunk_id.to_hex()];
        // 一个命中、一个未命中
        decision.safe_claims_used =
            vec!["响应及时".to_string(), "免费试用".to_string()];

        let mut review = approved_review();
        review.claim_analysis = doc! {
            "requiresProductKnowledge": true,
            "knowledgeSupported": true,
        };

        let runtime = dummy_runtime();
        let contact = dummy_contact();
        let markers = default_product_claim_markers();

        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            std::slice::from_ref(&chunk),
            &markers,
            Vec::new(),
            "",
        );

        assert_eq!(
            outcome.status,
            GatewayStatusFinal::Approved,
            "R5.7 仅是 risks 标记，不应改动终态"
        );
        assert_eq!(outcome.review.final_review_status, "approved");
        assert!(outcome.review.approved);
        assert!(decision.should_reply);
        assert_eq!(decision.autonomy_mode, "auto");
        assert!(
            outcome
                .review
                .risks
                .iter()
                .any(|r| r == "safe_claim_not_verified:免费试用"),
            "未命中 verified.safe_claims 的 claim SHALL 进入 risks (got: {:?})",
            outcome.review.risks
        );
        assert!(
            !outcome
                .review
                .risks
                .iter()
                .any(|r| r == "safe_claim_not_verified:响应及时"),
            "命中 verified.safe_claims 的 claim 不应被标记"
        );
    }

    #[test]
    fn requires_product_knowledge_with_seven_unverified_claims_caps_at_five_with_overflow() {
        // R5.7 cap-5 overflow：7 个不命中 safe_claim → 前 5 条单独追加，剩余
        // 2 条聚合为 `safe_claim_not_verified:and_more:2`，总共恰好 6 条
        // safe_claim_not_verified:* risks。verified_chunks 仅有非匹配的 claim
        // 集合，确保所有 7 个 used safe_claim 都未命中。
        let chunk = build_verified_chunk(vec!["unrelated_claim".to_string()]);
        let chunk_id = chunk.id.unwrap();

        let mut decision = approved_decision();
        decision.used_knowledge_ids = vec![chunk_id.to_hex()];
        decision.safe_claims_used = (1..=7).map(|i| format!("c{i}")).collect();

        let mut review = approved_review();
        review.claim_analysis = doc! {
            "requiresProductKnowledge": true,
            "knowledgeSupported": true,
        };

        let runtime = dummy_runtime();
        let contact = dummy_contact();
        let markers = default_product_claim_markers();

        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            std::slice::from_ref(&chunk),
            &markers,
            Vec::new(),
            "",
        );

        assert_eq!(outcome.status, GatewayStatusFinal::Approved);
        let safe_claim_risks: Vec<&String> = outcome
            .review
            .risks
            .iter()
            .filter(|r| r.starts_with("safe_claim_not_verified:"))
            .collect();
        assert_eq!(
            safe_claim_risks.len(),
            6,
            "5 条单独 + 1 条 and_more 聚合 = 6 (got: {:?})",
            safe_claim_risks
        );
        // 前 5 条按原顺序保留
        for (i, claim) in (1..=5_i32).enumerate() {
            assert_eq!(
                safe_claim_risks[i],
                &format!("safe_claim_not_verified:c{claim}")
            );
        }
        assert_eq!(
            safe_claim_risks[5],
            &"safe_claim_not_verified:and_more:2".to_string()
        );
    }

    #[test]
    fn upstream_taxonomy_candidate_risks_pass_through_without_blocking() {
        // R8.4：上游（W3 task 4.7 接入 taxonomy 后）已经 push 的
        // `taxonomy_candidate:*` / `taxonomy_deprecated_value:*` risks，在
        // finalize 层 SHALL NOT 强制 review.approved=false / autonomy_mode=blocked。
        // 当前 W2 阶段 finalize 不主动调用 taxonomy::check_value，但 R8.4
        // 约束要求即使这些 risks 已经存在也不阻塞，故在此显式回归。
        let mut decision = approved_decision();
        let mut review = approved_review();
        review
            .risks
            .push("taxonomy_candidate:customer_stage:lukewarm_lead".to_string());
        review
            .risks
            .push("taxonomy_deprecated_value:intent_level:legacy_warm".to_string());

        let runtime = dummy_runtime();
        let contact = dummy_contact();
        let markers = default_product_claim_markers();

        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            &[],
            &markers,
            Vec::new(),
            "",
        );

        assert_eq!(outcome.status, GatewayStatusFinal::Approved);
        assert_eq!(outcome.review.final_review_status, "approved");
        assert!(outcome.review.approved);
        assert!(decision.should_reply);
        assert_eq!(decision.autonomy_mode, "auto");
        // 上游 risks 应被原样保留（不被 finalize 清洗）。
        assert!(outcome
            .review
            .risks
            .iter()
            .any(|r| r == "taxonomy_candidate:customer_stage:lukewarm_lead"));
        assert!(outcome
            .review
            .risks
            .iter()
            .any(|r| r == "taxonomy_deprecated_value:intent_level:legacy_warm"));
        // 不应触发任何 hold_category 矫正事件。
        assert!(outcome.pending_events.is_empty());
    }

    #[test]
    fn canonical_hold_category_passes_through_without_coercion_event() {
        // R2.6 happy path：should_hold=true + hold_category="ai_waiting_for_more_context"
        // SHALL 直接进入 Held(canonical) 终态，不产出 autonomy_hold_category_invalid 事件。
        // 同时 should_reply 强制 false；autonomy_mode 保留原值（hold 路径不强制 blocked）。
        let mut decision = approved_decision();
        decision.autonomy_mode = "assisted".to_string();

        let mut review = approved_review();
        review.should_hold = true;
        review.hold_reason = "等待用户补充关键背景信息".to_string();
        review.hold_category = "ai_waiting_for_more_context".to_string();

        let runtime = dummy_runtime();
        let contact = dummy_contact();
        let markers = default_product_claim_markers();

        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            &[],
            &markers,
            Vec::new(),
            "",
        );

        match &outcome.status {
            GatewayStatusFinal::Held(category) => {
                assert_eq!(category, "ai_waiting_for_more_context");
            }
            other => panic!("expected Held(ai_waiting_for_more_context), got {other:?}"),
        }
        assert_eq!(
            outcome.review.final_review_status,
            "ai_waiting_for_more_context"
        );
        assert_eq!(outcome.review.hold_category, "ai_waiting_for_more_context");
        assert!(!decision.should_reply, "Held 路径 SHALL 强制 should_reply=false");
        assert_eq!(
            decision.autonomy_mode, "assisted",
            "canonical hold 路径不应强制 autonomy_mode=blocked，保留 assisted"
        );
        // 合法枚举不应触发矫正事件。
        assert!(
            !outcome
                .pending_events
                .iter()
                .any(|e| e.kind == EVENT_AUTONOMY_HOLD_CATEGORY_INVALID),
            "canonical hold_category 不应产出 autonomy_hold_category_invalid 事件 (got: {:?})",
            outcome.pending_events
        );
    }

    #[test]
    fn approved_false_without_hard_gate_falls_back_to_held_by_ai_policy() {
        // 兜底分支：上游 review.approved=false（例如 review_passed 因
        // human_like 阈值不达标返回 false），但未触发任何硬门（无协议违规、
        // 无 budget_exceeded、无 product_claim、无 should_hold）→ finalize
        // SHALL 走默认兜底 Held("held_by_ai_policy")，避免 final_review_status
        // 落空字符串穿透到落库。
        let mut decision = approved_decision();
        let mut review = approved_review();
        review.approved = false;
        review.scores.human_like = 3;
        review.risks.push("human_like_below_threshold".to_string());

        let runtime = dummy_runtime();
        let contact = dummy_contact();
        let markers = default_product_claim_markers();

        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            &[],
            &markers,
            Vec::new(),
            "",
        );

        match &outcome.status {
            GatewayStatusFinal::Held(category) => {
                assert_eq!(category, HOLD_CATEGORY_HELD_BY_AI_POLICY);
            }
            other => panic!(
                "approved=false + 无硬门触发 SHALL 走 Held(held_by_ai_policy) 兜底，got {other:?}"
            ),
        }
        assert_eq!(
            outcome.review.final_review_status,
            HOLD_CATEGORY_HELD_BY_AI_POLICY,
            "final_review_status 不应为空字符串"
        );
        // 兜底分支不主动改写 should_reply / autonomy_mode：保留上游决策语义
        // （由 task 3.4 在 outbox enqueue 阶段根据 status != Approved 决定不发送）。
        assert!(!outcome.review.approved);
        // 不应产出 finalize 阶段的事件。
        assert!(outcome.pending_events.is_empty());
    }

    // ── ISSUE-003 (R13)：claim_analysis_malformed 软化路径单元测试 ──
    //
    // 仅当 trigger=string_marker_hit AND inbound 无产品意图时降级到 R5.3.b
    // risks-only；其它两种 trigger（knowledge_need / used_knowledge_ids）保持
    // R5.3.a fail-closed 语义不变。

    #[test]
    fn r13_softens_string_marker_hit_when_inbound_innocuous() {
        // claim_analysis 空 + LLM 自发输出含 marker（"100%为您服务"）+ inbound
        // 是简单问候 → 应走 R5.3.b 软化，不 fail-closed。
        let runtime = dummy_runtime();
        let contact = dummy_contact();
        let markers = default_product_claim_markers();
        let mut decision = approved_decision();
        decision.reply_text = "100%为您服务！".to_string();
        decision.knowledge_need = "not_required".to_string();
        decision.used_knowledge_ids = Vec::new();
        let mut review = approved_review();
        review.claim_analysis = Document::new();

        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            &[],
            &markers,
            Vec::new(),
            "你好啊", // inbound 无产品意图
        );

        // 软化：不应是 BlockedBySafetyGuard
        assert!(
            !matches!(outcome.status, GatewayStatusFinal::BlockedBySafetyGuard),
            "string_marker_hit + inbound 无产品意图 SHALL 软化，不进 BlockedBySafetyGuard，got {:?}",
            outcome.status
        );
        // 应留软化痕迹
        assert!(outcome
            .review
            .risks
            .iter()
            .any(|r| r.starts_with("claim_malformed_softened:")));
        assert!(outcome
            .pending_events
            .iter()
            .any(|e| e.kind == "claim_analysis_malformed_softened"));
        // claim_analysis_malformed 标签仍保留（R5.3 末段要求）
        assert!(outcome.review.risks.iter().any(|r| r == "claim_analysis_malformed"));
    }

    #[test]
    fn r13_does_not_soften_when_inbound_has_product_marker() {
        let runtime = dummy_runtime();
        let contact = dummy_contact();
        let markers = default_product_claim_markers();
        let mut decision = approved_decision();
        decision.reply_text = "100%为您服务！".to_string();
        decision.knowledge_need = "not_required".to_string();
        decision.used_knowledge_ids = Vec::new();
        let mut review = approved_review();
        review.claim_analysis = Document::new();

        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            &[],
            &markers,
            Vec::new(),
            "你们百分之百保证年化30吗", // inbound 也含强 marker
        );

        assert_eq!(
            outcome.status,
            GatewayStatusFinal::BlockedBySafetyGuard,
            "inbound 也含产品意图 SHALL 仍 fail-closed"
        );
        assert!(outcome
            .pending_events
            .iter()
            .any(|e| e.kind == "claim_analysis_malformed_fail_closed"));
    }

    #[test]
    fn r13_does_not_soften_when_trigger_is_knowledge_need() {
        let runtime = dummy_runtime();
        let contact = dummy_contact();
        let markers = default_product_claim_markers();
        let mut decision = approved_decision();
        decision.reply_text = "好的".to_string(); // reply 无 marker
        decision.knowledge_need = "required".to_string(); // LLM 主动声明
        decision.used_knowledge_ids = Vec::new();
        let mut review = approved_review();
        review.claim_analysis = Document::new();

        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            &[],
            &markers,
            Vec::new(),
            "你好啊", // inbound 无产品意图
        );

        assert_eq!(
            outcome.status,
            GatewayStatusFinal::BlockedBySafetyGuard,
            "knowledge_need=required SHALL 仍 fail-closed（LLM 自我声明优先）"
        );
    }

    #[test]
    fn r13_does_not_soften_when_trigger_is_used_knowledge_ids() {
        let runtime = dummy_runtime();
        let contact = dummy_contact();
        let markers = default_product_claim_markers();
        let mut decision = approved_decision();
        decision.reply_text = "好的".to_string();
        decision.knowledge_need = "not_required".to_string();
        decision.used_knowledge_ids = vec!["kb_xyz".to_string()];
        let mut review = approved_review();
        review.claim_analysis = Document::new();

        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            &[],
            &markers,
            Vec::new(),
            "你好啊",
        );

        assert_eq!(
            outcome.status,
            GatewayStatusFinal::BlockedBySafetyGuard,
            "used_knowledge_ids 非空 SHALL 仍 fail-closed"
        );
    }

    #[test]
    fn r13_inbound_has_no_product_marker_pure_returns_false_on_empty() {
        // 防御性：inbound_text 为空时 SHALL 返回 false（保持 fail-closed 安全侧）
        let markers = default_product_claim_markers();
        assert!(!inbound_has_no_product_marker("", &markers));
    }

    #[test]
    fn r13_inbound_has_no_product_marker_pure_returns_true_on_innocuous() {
        let markers = default_product_claim_markers();
        assert!(inbound_has_no_product_marker("你好啊", &markers));
        assert!(inbound_has_no_product_marker("最近忙什么呢", &markers));
    }

    #[test]
    fn r13_inbound_has_no_product_marker_pure_returns_false_when_marker_hit() {
        let markers = default_product_claim_markers();
        assert!(!inbound_has_no_product_marker("100%保证年化30", &markers));
    }
}


#[cfg(test)]
mod revision_control_flow_tests {
    //! agent-autonomy-loop W2 / Task 3.7：R2 single-shot revision 控制流单元测试。
    //!
    //! 覆盖 design.md §4.5 状态映射表中 revision 相关 5 条分支：
    //!   1. needs_revision=true + 通过所有前置条件 → `RevisionDecision::Proceed`
    //!      （等价于 gateway.rs 调用 Reply Agent 第二次，调用次数 == 2）；
    //!   2. revisionDirection 仅含空白 → `Skip { reason="revisionDirection_empty",
    //!      event="revision_skipped_invalid_direction" }`；
    //!   3. RunBudget.is_exceeded()=true 优先于其它通过条件 →
    //!      `Skip { reason="budget_exceeded_before_revision",
    //!      event="revision_skipped_budget_exceeded" }`；
    //!   4. `derive_revision_failure` 对四类失败原因
    //!      （revision_post_review_failed / revision_llm_timeout_30s /
    //!      revision_llm_error:* / 未知字符串）SHALL 一致映射到
    //!      `(reason, Held(held_by_ai_policy))`；
    //!   5. finalize_status != Approved / should_hold=true / needs_revision=false
    //!      均走 `NotEligible`（gateway.rs 整段跳过 R2 块）。
    //!
    //! 这些测试 SHALL 是纯函数测试：不依赖 `AppState` / Mongo / LLM / RunBudget
    //! task-local；`budget_exceeded` 由测试直接构造 bool 参数注入。

    use super::super::types::{DecisionReviewResult, HOLD_CATEGORY_HELD_BY_AI_POLICY};
    use super::{
        decide_revision, derive_revision_failure, GatewayStatusFinal, RevisionDecision,
    };

    fn review_with(needs_revision: bool, revision_direction: &str) -> DecisionReviewResult {
        DecisionReviewResult {
            approved: true,
            needs_revision,
            revision_direction: revision_direction.to_string(),
            should_hold: false,
            ..Default::default()
        }
    }

    /// **Validates: Requirement 2.3 / 2.10**
    ///
    /// 通过所有前置条件 → `Proceed`，对应 gateway.rs 调用 Reply Agent 第二次
    /// （等价于"Reply Agent 调用次数 == 2"）。
    #[test]
    fn needs_revision_true_with_valid_direction_proceeds() {
        let review = review_with(true, "把第二句改得更口语化一些");
        let decision = decide_revision(&GatewayStatusFinal::Approved, &review, false);
        assert_eq!(decision, RevisionDecision::Proceed);
    }

    /// **Validates: Requirement 2.4 / 2.11**
    ///
    /// `derive_revision_failure("revision_post_review_failed")` 表示"第二轮 review
    /// 仍 fail" 的失败映射，对应 R2.4。返回 reason 字面量原样保留 + status =
    /// Held(held_by_ai_policy)；调用方据此把 `final_review_status =
    /// "revision_failed"` 与 `should_reply=false` 写回 review。
    #[test]
    fn second_round_failure_maps_to_held_by_ai_policy_with_revision_post_review_failed() {
        let (reason, status) = derive_revision_failure("revision_post_review_failed");
        assert_eq!(reason, "revision_post_review_failed");
        match status {
            GatewayStatusFinal::Held(category) => {
                assert_eq!(category, HOLD_CATEGORY_HELD_BY_AI_POLICY);
            }
            other => panic!(
                "revision 失败 SHALL 一律映射到 Held(held_by_ai_policy)，got {other:?}"
            ),
        }
    }

    /// **Validates: Requirement 2.8**
    ///
    /// `budget_exceeded=true` 在通过所有 review 字段校验后仍 SHALL 短路为
    /// `Skip { event="revision_skipped_budget_exceeded" }`，gateway.rs 据此
    /// 写 `agent_events kind="revision_skipped_budget_exceeded"` 而非进入
    /// 第二次 Reply Agent。
    #[test]
    fn budget_exceeded_before_revision_skips_with_budget_event() {
        let review = review_with(true, "请稍微更具体一点");
        let decision = decide_revision(&GatewayStatusFinal::Approved, &review, true);
        assert_eq!(
            decision,
            RevisionDecision::Skip {
                reason: "budget_exceeded_before_revision",
                event: "revision_skipped_budget_exceeded",
            }
        );
    }

    /// **Validates: Requirement 2.5**
    ///
    /// `revision_direction` 仅含空白（`"   \t\n"`）SHALL 视为空，触发
    /// `revision_skipped_invalid_direction`；budget 取何值都不影响（空白判定
    /// 优先于 budget 判定）。
    #[test]
    fn whitespace_only_revision_direction_skips_with_invalid_direction_event() {
        let review = review_with(true, "   \t\n");
        // budget 取 false 与 true 都应得到同一 Skip（空白判定优先级最高）。
        for budget_exceeded in [false, true] {
            let decision = decide_revision(&GatewayStatusFinal::Approved, &review, budget_exceeded);
            assert_eq!(
                decision,
                RevisionDecision::Skip {
                    reason: "revisionDirection_empty",
                    event: "revision_skipped_invalid_direction",
                },
                "whitespace-only revision_direction 与 budget_exceeded={budget_exceeded} 时 \
                 SHALL 仍走 invalid_direction 分支"
            );
        }
    }

    /// **Validates: Requirement 2.11**
    ///
    /// `derive_revision_failure("revision_llm_timeout_30s")` 对应 30s 超时分支；
    /// gateway.rs 据此写 `revision_llm_failure` 事件并把终态切为
    /// `Held(held_by_ai_policy)` + `final_review_status="revision_failed"`。
    #[test]
    fn llm_timeout_failure_maps_to_held_by_ai_policy() {
        let (reason, status) = derive_revision_failure("revision_llm_timeout_30s");
        assert_eq!(reason, "revision_llm_timeout_30s");
        assert_eq!(
            status,
            GatewayStatusFinal::Held(HOLD_CATEGORY_HELD_BY_AI_POLICY.to_string())
        );
    }

    /// **Validates: Requirement 2.11**
    ///
    /// `revision_llm_error:*` 前缀（业务错误 / JSON 解析失败）也 SHALL 映射
    /// 到 `Held(held_by_ai_policy)`，reason 字面量原样透传到
    /// `agent_run_logs.revision_reason`，便于审计排查根因。
    #[test]
    fn llm_error_failure_preserves_reason_string() {
        let (reason, status) =
            derive_revision_failure("revision_llm_error:reqwest::Error: timeout");
        assert_eq!(reason, "revision_llm_error:reqwest::Error: timeout");
        assert_eq!(
            status,
            GatewayStatusFinal::Held(HOLD_CATEGORY_HELD_BY_AI_POLICY.to_string())
        );
    }

    /// **Validates: Requirement 2.3**
    ///
    /// `finalize_status != Approved`（含 BlockedBy* / Held(*)）→ `NotEligible`，
    /// gateway.rs 据此整段跳过 R2 块；同样 `should_hold=true` / `needs_revision=false`
    /// 也走 `NotEligible`。
    #[test]
    fn ineligible_paths_yield_not_eligible() {
        let review = review_with(true, "valid direction");

        // finalize 已被硬门拦截（任意 blocked_* 终态）→ NotEligible
        for status in [
            GatewayStatusFinal::BlockedByRequiredField,
            GatewayStatusFinal::BlockedByBudget,
            GatewayStatusFinal::BlockedUnverifiedProductClaim,
            GatewayStatusFinal::BlockedBySafetyGuard,
            GatewayStatusFinal::Held(HOLD_CATEGORY_HELD_BY_AI_POLICY.to_string()),
        ] {
            assert_eq!(
                decide_revision(&status, &review, false),
                RevisionDecision::NotEligible,
                "finalize_status={status:?} SHALL 跳过 R2 块"
            );
        }

        // needs_revision=false → NotEligible（即使 direction 非空 + budget OK）
        let no_need = review_with(false, "ignored");
        assert_eq!(
            decide_revision(&GatewayStatusFinal::Approved, &no_need, false),
            RevisionDecision::NotEligible
        );

        // should_hold=true → NotEligible（hold 路径不进 revision）
        let mut hold_review = review_with(true, "valid");
        hold_review.should_hold = true;
        assert_eq!(
            decide_revision(&GatewayStatusFinal::Approved, &hold_review, false),
            RevisionDecision::NotEligible
        );
    }

    /// 兜底：未知 reason 字面量也 SHALL 映射到统一终态（fail-closed），避免
    /// 散落字面量造成 finalReviewStatus 漂移。
    #[test]
    fn unknown_reason_still_falls_back_to_held_by_ai_policy() {
        let (reason, status) = derive_revision_failure("some_future_reason_we_dont_know");
        assert_eq!(reason, "some_future_reason_we_dont_know");
        assert_eq!(
            status,
            GatewayStatusFinal::Held(HOLD_CATEGORY_HELD_BY_AI_POLICY.to_string())
        );
    }
}

// ── P3 性质测试（agent-autonomy-loop W3 / Task 4.13：≥ 64 用例）─────────
//
// **Property 3: 预算超额不发送**
// **Validates: Requirements 3.7, 3.10**
//
// 随机生成 `RunBudget`（is_exceeded=true）+ `decision.needs_review=true`，
// 断言 `local_decision_review` 返回 `approved=false` + 唯一 risk
// `budget_exceeded_no_review`。互斥地，needs_review=false 时返回
// `approved=true` + 含 `local_review_low_risk_only`。
//
// `local_decision_review` 是 `pub(crate)`，因此本性质放在 review.rs 自身
// 的 cfg(test) 模块中实现，与 P1/P6（`tests/autonomy_protocol_pbt.rs`）正交。

#[cfg(test)]
mod p3_pbt {
    use super::super::budget::RunBudget;
    use super::super::types::AgentDecision;
    use super::local_decision_review;
    use proptest::prelude::*;

    fn arbitrary_token_overflow() -> impl Strategy<Value = (i64, i64)> {
        // (token_budget, tokens_to_consume) 满足 tokens_to_consume >= token_budget
        (10i64..=10_000i64).prop_flat_map(|budget| {
            (Just(budget), budget..=budget * 5)
        })
    }

    fn arbitrary_call_overflow() -> impl Strategy<Value = (i32, i32)> {
        // (max_llm_calls, calls_to_record) 满足 calls_to_record >= max_llm_calls
        (1i32..=20i32).prop_flat_map(|cap| (Just(cap), cap..=cap * 3))
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 64,
            max_shrink_iters: 50,
            ..ProptestConfig::default()
        })]

        /// budget 超额（任一维度）+ needs_review=true → approved=false +
        /// risks 恰好为 ["budget_exceeded_no_review"]。
        #[test]
        fn p3_budget_exceeded_with_needs_review_blocks(
            (token_budget, consume) in arbitrary_token_overflow(),
            (call_cap, record_calls) in arbitrary_call_overflow(),
            use_token_path in any::<bool>(),
        ) {
            let budget = if use_token_path {
                let b = RunBudget::new("p3-tokens", token_budget, 100, i32::MAX);
                b.record_call(consume);
                b
            } else {
                let b = RunBudget::new("p3-calls", 1_000_000, call_cap, i32::MAX);
                for _ in 0..record_calls {
                    b.record_call(0);
                }
                b
            };
            prop_assert!(budget.is_exceeded(), "测试预设：budget 必须已超额");

            let decision = AgentDecision {
                needs_review: true,
                ..AgentDecision::default()
            };
            let review = local_decision_review(&decision, &budget);
            prop_assert!(!review.approved, "needs_review=true 路径下应被本地拒绝");
            prop_assert_eq!(
                review.risks.clone(),
                vec!["budget_exceeded_no_review".to_string()],
                "risks 必须恰好为 [budget_exceeded_no_review]，实际 {:?}",
                review.risks
            );
        }

        /// 互斥分支：budget 超额 + needs_review=false → approved=true +
        /// risks 含 `local_review_low_risk_only`，便于 finalize 走低风险通道。
        #[test]
        fn p3_budget_exceeded_without_needs_review_passes(
            (token_budget, consume) in arbitrary_token_overflow(),
        ) {
            let budget = RunBudget::new("p3-low", token_budget, 100, i32::MAX);
            budget.record_call(consume);
            prop_assert!(budget.is_exceeded());
            let decision = AgentDecision {
                needs_review: false,
                ..AgentDecision::default()
            };
            let review = local_decision_review(&decision, &budget);
            prop_assert!(review.approved);
            prop_assert!(review
                .risks
                .iter()
                .any(|r| r == "local_review_low_risk_only"));
            prop_assert!(!review
                .risks
                .iter()
                .any(|r| r == "budget_exceeded_no_review"));
        }
    }
}


// ── P2 性质测试（agent-autonomy-loop W3 / Task 4.15：≥ 64 用例）─────────
//
// **Property 2: Single-Shot Revision 上限**
// **Validates: Requirements 2.3, 2.4, 2.8**
//
// 性质 1：在任意 (`needs_revision`, `revision_direction`, `should_hold`,
//        `budget_exceeded`, `finalize_status`) 组合下，`decide_revision`
//        SHALL 返回 NotEligible / Skip / Proceed 三态之一，永不 panic。
// 性质 2：当 `should_hold=true` 时永不返回 Proceed（单 run 不二次调 Reply）。
// 性质 3：`budget_exceeded=true` + 通过其它前置 → 必返回
//        `Skip { event="revision_skipped_budget_exceeded" }`（不进 LLM）。
// 性质 4：`revision_direction` 仅含空白 + 通过其它前置 → 必返回
//        `Skip { event="revision_skipped_invalid_direction" }`。

#[cfg(test)]
mod p2_pbt {
    use super::super::types::DecisionReviewResult;
    use super::{decide_revision, GatewayStatusFinal, RevisionDecision};
    use proptest::prelude::*;

    fn arbitrary_finalize_status() -> impl Strategy<Value = GatewayStatusFinal> {
        prop_oneof![
            Just(GatewayStatusFinal::Approved),
            Just(GatewayStatusFinal::BlockedByRequiredField),
            Just(GatewayStatusFinal::BlockedByBudget),
            Just(GatewayStatusFinal::BlockedUnverifiedProductClaim),
            Just(GatewayStatusFinal::Held("held_by_ai_policy".to_string())),
        ]
    }

    fn arbitrary_revision_direction() -> impl Strategy<Value = String> {
        prop_oneof![
            Just(String::new()),
            Just("   ".to_string()),
            Just("\t\n  ".to_string()),
            "[a-z ]{1,40}".prop_map(String::from),
        ]
    }

    fn build_review(
        needs_revision: bool,
        direction: String,
        should_hold: bool,
    ) -> DecisionReviewResult {
        DecisionReviewResult {
            approved: !should_hold,
            needs_revision,
            revision_direction: direction,
            should_hold,
            ..Default::default()
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 64,
            max_shrink_iters: 100,
            ..ProptestConfig::default()
        })]

        /// P2-a：组合枚举下决策永不 panic 且返回值在 {NotEligible, Skip, Proceed}。
        #[test]
        fn p2_decide_revision_total_function(
            status in arbitrary_finalize_status(),
            needs_revision in any::<bool>(),
            direction in arbitrary_revision_direction(),
            should_hold in any::<bool>(),
            budget_exceeded in any::<bool>(),
        ) {
            let review = build_review(needs_revision, direction, should_hold);
            let outcome = decide_revision(&status, &review, budget_exceeded);
            // 编译期已经保证只能是这 3 个 variant；这里断言不 panic 即可。
            let _ = matches!(
                outcome,
                RevisionDecision::NotEligible
                    | RevisionDecision::Skip { .. }
                    | RevisionDecision::Proceed
            );
        }

        /// P2-b：should_hold=true → 永不 Proceed（避免在 hold 状态下二次调 Reply Agent）。
        #[test]
        fn p2_should_hold_never_proceeds(
            status in arbitrary_finalize_status(),
            needs_revision in any::<bool>(),
            direction in arbitrary_revision_direction(),
            budget_exceeded in any::<bool>(),
        ) {
            let review = build_review(needs_revision, direction, /*should_hold=*/ true);
            let outcome = decide_revision(&status, &review, budget_exceeded);
            prop_assert_ne!(outcome, RevisionDecision::Proceed,
                "should_hold=true 时 SHALL NOT 触发 revision");
        }

        /// P2-c：budget_exceeded=true + finalize=Approved + needs_revision=true +
        /// non-empty direction + !should_hold → 必为 Skip(revision_skipped_budget_exceeded)。
        #[test]
        fn p2_budget_exceeded_skips_revision(
            direction in "[a-z]{5,40}".prop_map(String::from),
        ) {
            let review = build_review(true, direction, false);
            let outcome = decide_revision(&GatewayStatusFinal::Approved, &review, true);
            prop_assert_eq!(
                outcome,
                RevisionDecision::Skip {
                    reason: "budget_exceeded_before_revision",
                    event: "revision_skipped_budget_exceeded",
                }
            );
        }

        /// P2-d：revision_direction 仅含空白 + finalize=Approved + needs_revision=true +
        /// !should_hold + !budget_exceeded → 必为
        /// Skip(revision_skipped_invalid_direction)（空白判定优先于 budget）。
        #[test]
        fn p2_blank_direction_skipped_invalid(
            blank in prop_oneof![
                Just("".to_string()),
                Just(" ".to_string()),
                Just("   ".to_string()),
                Just("\t\n".to_string()),
            ],
        ) {
            let review = build_review(true, blank, false);
            let outcome = decide_revision(&GatewayStatusFinal::Approved, &review, false);
            prop_assert_eq!(
                outcome,
                RevisionDecision::Skip {
                    reason: "revisionDirection_empty",
                    event: "revision_skipped_invalid_direction",
                }
            );
        }
    }
}
