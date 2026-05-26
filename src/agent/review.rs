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
    OperationPlaybook,
};
use crate::prompts;
use crate::routes::AppState;

use super::budget::RunBudget;
use super::decision::{format_operation_domain_config_for_prompt, format_playbook_for_prompt};
use super::generate_agent_json;
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
                    hallucination_score: 0,
                    knowledge_grounding_score: 0,
                    ..Default::default()
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
                hallucination_score: 0,
                knowledge_grounding_score: if decision.knowledge_need == "required" {
                    7
                } else {
                    10
                },
                ..Default::default()
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
            hallucination_score: 0,
            knowledge_grounding_score: if decision.knowledge_need == "required" {
                7
            } else {
                10
            },
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
        && review.scores.hallucination_score < runtime.fact_risk_block_at
        && review.scores.human_like >= runtime.human_like_rewrite_below
        && review.scores.emotional_value >= runtime.emotional_value_rewrite_below
        && review.scores.knowledge_grounding_score >= runtime.product_accuracy_block_below
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
                hallucination_score: 0,
                knowledge_grounding_score: 10,
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
        format_operation_knowledge_for_prompt(knowledge_chunks),
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
    let _ = (decision, domain_config, knowledge_chunks, contact);
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
/// ISSUE-003 (R13) stub: 真正实现已随旧 sales 守卫一起删除，
/// commit 3 在 wiki 闸架下重写 review.rs 时再补；保留签名避免 callers 编译失败。
pub(crate) fn inbound_has_no_product_marker(_inbound_text: &str) -> bool {
    false
}

pub fn finalize_review_for_send(
    review: DecisionReviewResult,
    decision: &mut AgentDecision,
    _runtime: &UserRuntimeParameters,
    _contact: &Contact,
    _knowledge_chunks: &[OperationKnowledgeChunk],
    promote_risks: Vec<String>,
    _inbound_text: &str,
) -> FinalizeOutcome {
    let mut review = review;
    let mut pending_events: Vec<PendingFinalizeEvent> = Vec::new();

    extend_risks_unique(&mut review.risks, promote_risks.iter().cloned());

    // R3.5 / R3.6：必填字段 / 枚举非法 → blocked_by_required_field
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

    // R3.7：预算超额 + needs_review=true → blocked_by_budget
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

    // commit 3 将以 wiki + 3 闸（knowledge_grounding / hallucination /
    // run_budget）替换旧的 R5 verified-knowledge / safe_claims / claim_analysis
    // 串联硬门。本期保留 protocol violation + budget exceeded 两道闸即可让
    // gateway 主路径继续编译。

    // R2.6：should_hold + holdCategory 校验
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
        review.final_review_status = category.clone();
        return FinalizeOutcome {
            review,
            status: GatewayStatusFinal::Held(category),
            pending_events,
        };
    }

    // 默认：approved 通过
    if review.approved && decision.should_reply {
        review.final_review_status = "approved".to_string();
        FinalizeOutcome {
            review,
            status: GatewayStatusFinal::Approved,
            pending_events,
        }
    } else {
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
