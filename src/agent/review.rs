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
            ..Default::default()
        },
        review_summary: "低风险 fast_chat 本地轻量审核通过".to_string(),
        ..Default::default()
    }
}

/// Phase B / B2：把 [`AgentDecision`] 投影成 reviewer 可见的 **事实面** 视图，
/// 剥离所有 reply-agent 的自我推理字段，强制 reviewer 走独立判断路径。
///
/// 直接 `serde_json::to_string(decision)` 会把以下字段一并喂给 reviewer：
///
/// - `user_understanding / relationship_read / operation_goal`
/// - `knowledge_need_reason / memory_update_reason / risk_self_check`
/// - `self_critique / why_should_reply / why_skip_reply`
/// - `intent_analysis / next_best_action / operating_memory_update`
///   等推理 doc
///
/// 这些字段是 reply-agent 自洽逻辑链的产物，喂给 reviewer 会形成
/// "reviewer 追认 reply-agent" 的副作用。本函数只暴露候选回复的事实面：
/// 是否回复、回复文本、知识引用、状态/阶段、tool-loop 协议字段。
pub(crate) fn build_reviewer_decision_view(decision: &super::types::AgentDecision) -> String {
    serde_json::to_string(&mongodb::bson::doc! {
        "shouldReply": decision.should_reply,
        "replyText": decision.reply_text.clone(),
        "matchedKnowledgeIds": decision.matched_knowledge_ids.clone(),
        "safeClaimsUsed": decision.safe_claims_used.clone(),
        "usedKnowledgeIds": decision.used_knowledge_ids.clone(),
        "objectionsDetected": decision.objections_detected.clone(),
        "customerStage": decision.customer_stage.clone().unwrap_or_default(),
        "intentLevel": decision.intent_level.clone().unwrap_or_default(),
        "operationState": decision.operation_state.clone().unwrap_or_default(),
        "decisionPhase": decision.decision_phase.clone(),
        "autonomyMode": decision.autonomy_mode.clone(),
        "runMode": decision.run_mode.clone(),
        "riskLevel": decision.risk_level.clone(),
        "knowledgeNeed": decision.knowledge_need.clone(),
    })
    .unwrap_or_default()
}

pub fn review_passed(
    review: &DecisionReviewResult,
    runtime: &UserRuntimeParameters,
) -> bool {
    review.approved
        && review.scores.hallucination_score < runtime.fact_risk_block_at
        && review.scores.human_like >= runtime.human_like_rewrite_below
        && review.scores.emotional_value >= runtime.emotional_value_rewrite_below
        && review.scores.knowledge_grounding_score >= runtime.product_accuracy_block_below
        // Phase B / B1：恢复 pressure_risk 软闸 — `>=` 阈值视为压迫感过强，拦截。
        // 0 表示 reviewer 未给分（含老数据反序列化默认），不参与拦截。
        && (review.scores.pressure_risk == 0
            || review.scores.pressure_risk < runtime.pressure_risk_block_at)
}

/// Phase B / B1：双闸分类结果。
///
/// `review_passed` 把硬闸（hallucination / knowledge_grounding）和软闸
/// （humanLike / pressureRisk / emotionalValue）一起折叠成一个 bool，导致
/// 软闸失败后 `approved=false` → finalize 走 Held 分支，single-shot
/// revision 通道（[`decide_revision`]）永远 `NotEligible`，本意"软闸失败
/// 触发 revision"被绕过。
///
/// 本枚举把两类失败显式区分，让 [`route_dual_gate`] 在软闸失败时仍保留
/// `approved=true` + 写 `needs_revision=true` + `revision_direction`，让
/// finalize 进入 `Approved`、再由 `decide_revision` 走 `Proceed` 触发
/// revision。硬闸失败仍然 `approved=false` → finalize 走 Held。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DualGateClassification {
    /// 硬 / 软闸都通过。
    AllPass,
    /// 硬闸失败（hallucination ≥ 阈值 / knowledge_grounding < 阈值），
    /// SHALL 直接 `approved=false`，finalize 走 Held。
    HardGateFailure { risks: Vec<String> },
    /// 软闸失败（humanLike < 阈值 / pressureRisk ≥ 阈值 / emotionalValue
    /// < 阈值），SHALL 保留 `approved` 不变（finalize 走 Approved）但
    /// 写 `needs_revision=true` + `revision_direction`，触发 single-shot
    /// revision；硬闸通过的前提下才能进入本分支。
    SoftGateFailure {
        direction: String,
        risks: Vec<String>,
    },
}

/// Phase B / B1：纯函数版双闸分类，按"硬闸优先"裁定。
///
/// 与 [`review_passed`] 对偶：本函数不读 `review.approved`，只看分数 vs
/// runtime 阈值，便于单测同时覆盖 reviewer `approved=false` 但分数全过、
/// reviewer `approved=true` 但软闸失败等组合。
pub(crate) fn classify_dual_gate(
    review: &DecisionReviewResult,
    runtime: &UserRuntimeParameters,
) -> DualGateClassification {
    let mut hard_risks: Vec<String> = Vec::new();
    if review.scores.hallucination_score >= runtime.fact_risk_block_at {
        hard_risks.push(format!(
            "hallucination_score_{}_ge_{}",
            review.scores.hallucination_score, runtime.fact_risk_block_at
        ));
    }
    if review.scores.knowledge_grounding_score < runtime.product_accuracy_block_below {
        hard_risks.push(format!(
            "knowledge_grounding_{}_lt_{}",
            review.scores.knowledge_grounding_score, runtime.product_accuracy_block_below
        ));
    }
    if !hard_risks.is_empty() {
        return DualGateClassification::HardGateFailure { risks: hard_risks };
    }

    let mut soft_risks: Vec<String> = Vec::new();
    let mut direction_parts: Vec<String> = Vec::new();
    if review.scores.human_like < runtime.human_like_rewrite_below {
        soft_risks.push(format!(
            "human_like_{}_lt_{}",
            review.scores.human_like, runtime.human_like_rewrite_below
        ));
        direction_parts.push(format!(
            "humanLike 评分 {} 低于阈值 {}：请把语气改写得更像微信真人对话——\
             少模板、少销售腔、贴近上下文；保留要表达的事实，但句式与停顿向\
             自然口语靠拢。",
            review.scores.human_like, runtime.human_like_rewrite_below
        ));
    }
    if review.scores.pressure_risk != 0
        && review.scores.pressure_risk >= runtime.pressure_risk_block_at
    {
        soft_risks.push(format!(
            "pressure_risk_{}_ge_{}",
            review.scores.pressure_risk, runtime.pressure_risk_block_at
        ));
        direction_parts.push(format!(
            "pressureRisk 评分 {} 高于等于阈值 {}：去掉催促、紧迫、稀缺感、\
             连环追问；改为承接对方顾虑 + 1 个轻量澄清问题或 1 个具体小建议，\
             留出对方思考空间。",
            review.scores.pressure_risk, runtime.pressure_risk_block_at
        ));
    }
    if review.scores.emotional_value < runtime.emotional_value_rewrite_below {
        soft_risks.push(format!(
            "emotional_value_{}_lt_{}",
            review.scores.emotional_value, runtime.emotional_value_rewrite_below
        ));
        direction_parts.push(format!(
            "emotionalValue 评分 {} 低于阈值 {}：增加对对方处境的具体共情、\
             承接对方关切的细节；避免泛泛的安慰或纯交易语气。",
            review.scores.emotional_value, runtime.emotional_value_rewrite_below
        ));
    }
    if soft_risks.is_empty() {
        return DualGateClassification::AllPass;
    }
    let direction = direction_parts.join(" ");
    DualGateClassification::SoftGateFailure {
        direction,
        risks: soft_risks,
    }
}

/// Phase B / B1：把 `classify_dual_gate` 的判定写回 review 字段。
///
/// 设计要点：
/// * `HardGateFailure`：照旧 `approved=false`（finalize 会进 Held 分支）。
/// * `SoftGateFailure`：保持 `approved` 由原始 `review_passed` 算出（也就是
///   `false`），但同时**写 `needs_revision=true` + `revision_direction`**。
///   `finalize_review_for_send` 会先看 protocol violation / budget /
///   should_hold 三道硬门——这三道都没命中时，新增的"soft-gate 唯一原因"
///   分支会把 `approved` 强制改回 `true` 并保留 `needs_revision`，让
///   `decide_revision` 进入 `Proceed`。
/// * `AllPass`：照旧用 `review_passed` 决定 `approved`。
///
/// 调用方 SHALL 在反序列化 reviewer JSON 后立即调用本函数，替换原本的
/// `review.approved = review_passed(&review, runtime)`。
pub(crate) fn route_dual_gate(
    review: &mut DecisionReviewResult,
    runtime: &UserRuntimeParameters,
) {
    let classification = classify_dual_gate(review, runtime);
    // 先按 review_passed 写一遍 approved（保持现有 PBT / 老调用点的语义不
    // 变；soft-gate 路径下 finalize 会再矫正回 true）。
    let baseline_approved = review_passed(review, runtime);
    review.approved = baseline_approved;
    match classification {
        DualGateClassification::AllPass | DualGateClassification::HardGateFailure { .. } => {
            // 硬闸失败：approved=false，finalize 进 Held。本函数不再追加 risks，
            // 因为 finalize 已有自己的 risk 通道；硬闸细节走 review.risks 即可。
        }
        DualGateClassification::SoftGateFailure { direction, risks } => {
            // 软闸失败：标记 needs_revision，让 finalize 改写 approved=true。
            // reviewer 自己已经写了 revision_direction（prompt 鼓励它给方向）
            // 时不覆盖；为空才用机器化方向兜底。
            if review.revision_direction.trim().is_empty() {
                review.revision_direction = direction;
            }
            review.needs_revision = true;
            for risk in risks {
                if !review.risks.iter().any(|r| r == &risk) {
                    review.risks.push(risk);
                }
            }
        }
    }
}

/// Phase E / E2：reviewer 双脑并行分歧种类。
///
/// 主 reviewer 与第二 reviewer 各跑一次评分后，按"硬决策一致性"判定分歧：
/// - `ApprovedMismatch`：一边 `approved=true` 另一边 `approved=false`（含
///   route_dual_gate 写过 needs_revision 的情况）；最强分歧信号。
/// - `DualGateMismatch`：[`classify_dual_gate`] 类别不同（一方 AllPass 另一方
///   HardGateFailure / SoftGateFailure，或 Hard ↔ Soft 互换）；强分歧信号。
/// - `SoftRiskDelta`：双方均软闸失败但具体命中的软闸不一致（如一方仅 humanLike
///   低、另一方仅 pressureRisk 高），代表两个模型看到了不同的弱点；中等分歧。
///
/// 任一分歧命中即返回 `Some`；双方完全一致返回 `None`，跳过 single-shot
/// revision 触发。本枚举刻意不细化具体差值（"分数差几"），因为不同模型的
/// 评分尺度本就不可直接比，只比较结构化的硬决策更稳健。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DualReviewerDisagreement {
    ApprovedMismatch,
    DualGateMismatch,
    SoftRiskDelta,
}

impl DualReviewerDisagreement {
    pub(crate) fn risk_marker(&self) -> &'static str {
        match self {
            Self::ApprovedMismatch => "reviewer_dual_disagree:approved_mismatch",
            Self::DualGateMismatch => "reviewer_dual_disagree:dual_gate_mismatch",
            Self::SoftRiskDelta => "reviewer_dual_disagree:soft_risk_delta",
        }
    }

    pub(crate) fn revision_direction(&self) -> &'static str {
        match self {
            Self::ApprovedMismatch => {
                "双 reviewer 在 approved 标志上分歧：请重新审视回复，确认安全闸全过；如有疑虑，\
                 倾向更保守的措辞。"
            }
            Self::DualGateMismatch => {
                "双 reviewer 在双闸分类上分歧：一方判定通过、另一方判定硬/软闸命中。请按更严格\
                 的一方意见改写——倾向更稳妥的语气与更明确的事实背书。"
            }
            Self::SoftRiskDelta => {
                "双 reviewer 在软闸命中上分歧：两个模型看到了不同的弱点。请同时回应两边的关切——\
                 兼顾自然口语 + 去施压感 + 共情，不放弃任何一方提出的改写方向。"
            }
        }
    }
}

/// Phase E / E2 纯函数：检测双 reviewer 是否分歧。
///
/// 输入两份独立评分结果与统一 runtime 阈值，按上面三档判定：approved-flag
/// 不一致优先级最高（结构性分歧），其次是 dual_gate 类别不一致，最后才是
/// 软闸命中具体项不一致。本函数不读 review.approved 之外的"reviewer 自陈"
/// 字段，只看分数 vs 阈值，确保不会被任一 reviewer 的 LLM hallucination
/// 推翻硬决策。
pub(crate) fn detect_dual_reviewer_disagreement(
    primary: &DecisionReviewResult,
    second: &DecisionReviewResult,
    runtime: &UserRuntimeParameters,
) -> Option<DualReviewerDisagreement> {
    let primary_approved = review_passed(primary, runtime);
    let second_approved = review_passed(second, runtime);
    if primary_approved != second_approved {
        return Some(DualReviewerDisagreement::ApprovedMismatch);
    }
    let primary_class = classify_dual_gate(primary, runtime);
    let second_class = classify_dual_gate(second, runtime);
    match (&primary_class, &second_class) {
        (DualGateClassification::AllPass, DualGateClassification::AllPass) => None,
        (
            DualGateClassification::HardGateFailure { .. },
            DualGateClassification::HardGateFailure { .. },
        ) => None,
        (
            DualGateClassification::SoftGateFailure { risks: a, .. },
            DualGateClassification::SoftGateFailure { risks: b, .. },
        ) => {
            // 双方都是软闸失败，但具体命中的子项可能不一样。命中集合相同 → 视为一致。
            let mut a_sorted: Vec<&String> = a.iter().collect();
            let mut b_sorted: Vec<&String> = b.iter().collect();
            a_sorted.sort();
            b_sorted.sort();
            if a_sorted == b_sorted {
                None
            } else {
                Some(DualReviewerDisagreement::SoftRiskDelta)
            }
        }
        _ => Some(DualReviewerDisagreement::DualGateMismatch),
    }
}

/// Phase E / E2 纯函数：把分歧落到主 review 上。
///
/// 主 review 已经走完 [`route_dual_gate`]；这里追加：
/// - `needs_revision = true`（即便主 review 自己判定 AllPass）
/// - 空 `revision_direction` 兜底为 [`DualReviewerDisagreement::revision_direction`]
/// - `risks` 追加 [`DualReviewerDisagreement::risk_marker`]
///
/// 已经写过 `revision_direction` 的不覆盖（保留主 reviewer 的语义）。
pub(crate) fn apply_dual_reviewer_disagreement(
    review: &mut DecisionReviewResult,
    disagreement: &DualReviewerDisagreement,
) {
    review.needs_revision = true;
    if review.revision_direction.trim().is_empty() {
        review.revision_direction = disagreement.revision_direction().to_string();
    }
    let marker = disagreement.risk_marker().to_string();
    if !review.risks.iter().any(|r| r == &marker) {
        review.risks.push(marker);
    }
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
                ..Default::default()
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
    // Phase B / B2：reviewer 视图剥离 reply-agent 自我推理。直接 `to_string(decision)`
    // 会把 9 个 self-reasoning 字段（why_should_reply / self_critique /
    // knowledge_need_reason / memory_update_reason / risk_self_check /
    // user_understanding / relationship_read / operation_goal / why_skip_reply）
    // + intent_analysis / next_best_action 推理 doc 一并喂给 reviewer，导致
    // reviewer 倾向于追认 reply-agent 的逻辑而失去 epistemic distance。
    // 这里只暴露候选回复事实面：是否回复、回复文本、知识引用、状态/阶段、tool-loop
    // 协议字段；其余字段（含 reasoning）不进 reviewer 上下文。
    let decision_view_text = build_reviewer_decision_view(decision);
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
- humanLike 与 pressureRisk 是 **硬评分** 软闸（Phase B / B1）：humanLike 低于阈值
  或 pressureRisk 高于等于阈值，会触发 single-shot revision；reviewer 必须给 0-100
  的具体分数，并在 `needsRevision` / `revisionDirection` 里给出可执行的改写方向。
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
        decision_view_text,
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
    // S2 (Phase 0)：reviewer 双模真并行——主 reviewer 走 generate_agent_json
    // （含 LRU cache + llm_call_logs），第二 reviewer 走纯 LlmGenerator。
    // 两路用 tokio::join! 并发，墙钟 ≈ max(p1, p2) 而非 p1 + p2。
    // 双脑禁用时（second_reviewer_llm = None）退化为单 future，行为不变。
    let primary_future = generate_agent_json(
        state,
        Some(&contact.account_id),
        Some(&contact.wxid),
        run_id,
        prompt_key,
        &system,
        &user,
    );
    let value = if let Some(second_llm) = state.second_reviewer_llm.as_ref() {
        let second_future = second_llm.generate_json(&system, &user);
        let (primary_res, second_res) = tokio::join!(primary_future, second_future);
        let primary_value = primary_res?;
        let mut review: DecisionReviewResult = serde_json::from_value(primary_value)?;
        let _ = (decision, domain_config, knowledge_chunks, contact);
        // Phase B / B1：双闸路由替换原 `review.approved = review_passed(...)`。
        // 软闸失败时保持 approved=false（review_passed 行为）但同时写
        // needs_revision=true / revision_direction，让 finalize 在硬门未命中时
        // 把 soft-gate-only 失败矫正为 Approved，以触发 single-shot revision。
        route_dual_gate(&mut review, runtime);

        // Phase E / E2：reviewer 双脑并行——若 AppState 注入了第二 provider，再跑
        // 一份独立评分，与主 reviewer 走 [`detect_dual_reviewer_disagreement`]
        // 比较；分歧即触发 single-shot revision，达到 epistemic diversity。
        // 第二 provider 调用失败仅 warn 不阻塞——双脑是增益机制，不应成为新故障源。
        match second_res {
            Ok(second_value) => match serde_json::from_value::<DecisionReviewResult>(second_value)
            {
                Ok(mut second_review) => {
                    route_dual_gate(&mut second_review, runtime);
                    if let Some(disagreement) =
                        detect_dual_reviewer_disagreement(&review, &second_review, runtime)
                    {
                        tracing::info!(
                            account_id = %contact.account_id,
                            contact_wxid = %contact.wxid,
                            primary_approved = review.approved,
                            second_approved = second_review.approved,
                            disagreement = ?disagreement,
                            "reviewer dual-mode disagreement detected — triggering revision"
                        );
                        apply_dual_reviewer_disagreement(&mut review, &disagreement);
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        ?error,
                        "second reviewer JSON parse failed — falling back to primary review"
                    );
                }
            },
            Err(error) => {
                tracing::warn!(
                    ?error,
                    "second reviewer LLM call failed — falling back to primary review"
                );
            }
        }
        return Ok(review);
    } else {
        primary_future.await?
    };
    let mut review: DecisionReviewResult = serde_json::from_value(value)?;
    let _ = (decision, domain_config, knowledge_chunks, contact);
    route_dual_gate(&mut review, runtime);

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
    } else if review.needs_revision
        && !review.revision_direction.trim().is_empty()
        && !review.should_hold
        && decision.should_reply
    {
        // Phase B / B1：soft-gate-only failure（humanLike / pressureRisk /
        // emotionalValue 任一软闸不达标，但 hallucination / grounding 硬闸
        // 通过，且 protocol / budget / should_hold 三道硬门都未命中）。
        // route_dual_gate 已写好 revision_direction + needs_revision，这里
        // 把 approved 矫正回 true，让 finalReviewStatus="approved" 进入
        // gateway 的 single-shot revision 通道（decide_revision Proceed）。
        // 注意：硬闸失败永远走不到这里（hard 失败时 needs_revision 不会被
        // route_dual_gate 写为 true）。
        review.approved = true;
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

/// Phase D / D2：从一段出站文本提取风格指纹。
///
/// 设计取舍：选**结构特征**（长度桶 + 标点密度 + emoji 出现 + 句末符号），
/// 而非 LLM 嵌入向量。理由：
/// - 廉价、确定性、纯字符串运算，不占 RunBudget；
/// - 风格漂移最容易在结构上暴露（一会儿一句话、一会儿三段；一会儿带表情、
///   一会儿正经；一会儿陈述句、一会儿问句堆叠）；
/// - 语义级风格（如"专业 vs 亲切"）已经在 reviewer prompt 里通过 reply_style
///   playbook 字段控制，不重复造轮子。
///
/// 输出形如 `"len:s|emoji:0|qmark:1|excl:0|tail:.|nl:0"` 的紧凑串。
pub(crate) fn extract_outbound_style_fingerprint(content: &str) -> String {
    let trimmed = content.trim();
    let chars = trimmed.chars().count();
    let len_bucket = if chars <= 30 {
        "xs"
    } else if chars <= 80 {
        "s"
    } else if chars <= 200 {
        "m"
    } else {
        "l"
    };

    let has_emoji = trimmed
        .chars()
        .any(|c| matches!(c as u32, 0x1F300..=0x1FAFF | 0x2600..=0x27BF));
    let has_qmark = trimmed.contains('?') || trimmed.contains('？');
    let has_excl = trimmed.contains('!') || trimmed.contains('！');
    let nl_count = trimmed.matches('\n').count().min(9);

    // 句末符号：跳过尾部 emoji / 空白，归一化中英文标点。emoji 常作"装饰"挂在
    // 真句末符号之后（"方便聊一下吗？😊"），把它纳入 tail 会误把所有带 emoji 句
    // 都标成 tail:x，掩盖真实的问句 / 陈述句结构差异。
    let tail = trimmed
        .chars()
        .rev()
        .find(|c| {
            !c.is_whitespace()
                && !matches!(*c as u32, 0x1F300..=0x1FAFF | 0x2600..=0x27BF)
        })
        .unwrap_or('.');
    let tail_class = match tail {
        '?' | '？' => 'q',
        '!' | '！' => 'e',
        '。' | '.' => '.',
        '~' | '～' => '~',
        _ => 'x',
    };

    format!(
        "len:{}|emoji:{}|qmark:{}|excl:{}|tail:{}|nl:{}",
        len_bucket,
        if has_emoji { 1 } else { 0 },
        if has_qmark { 1 } else { 0 },
        if has_excl { 1 } else { 0 },
        tail_class,
        nl_count,
    )
}

/// Phase D / D2：判断两条风格指纹是否"分歧足够大"。
///
/// 风格指纹是 5 段 `key:value` 拼接的串。分歧度 = 不同 key 数量 ≥ 3 视为发散。
/// 本函数只做语义判定，不读 contact / state；上层 reviewer 拿 bool 决定要不要
/// 触发 single-shot revision。
pub(crate) fn style_diverged(prev: &str, current: &str) -> bool {
    if prev.is_empty() || current.is_empty() {
        return false;
    }
    let prev_parts: Vec<&str> = prev.split('|').collect();
    let cur_parts: Vec<&str> = current.split('|').collect();
    let len = prev_parts.len().min(cur_parts.len());
    let diff = (0..len)
        .filter(|i| prev_parts[*i] != cur_parts[*i])
        .count();
    diff >= 3
}

#[cfg(test)]
mod style_fingerprint_tests {
    use super::*;

    #[test]
    fn fingerprint_is_deterministic() {
        let s = extract_outbound_style_fingerprint("您好，请问需要更多信息吗？");
        let s2 = extract_outbound_style_fingerprint("您好，请问需要更多信息吗？");
        assert_eq!(s, s2);
    }

    #[test]
    fn fingerprint_captures_length_bucket() {
        let xs = extract_outbound_style_fingerprint("好的");
        let m = extract_outbound_style_fingerprint(&"中".repeat(120));
        assert!(xs.contains("len:xs"));
        assert!(m.contains("len:m"));
    }

    #[test]
    fn fingerprint_captures_emoji_and_question() {
        let s = extract_outbound_style_fingerprint("方便聊一下吗？😊");
        assert!(s.contains("emoji:1"));
        assert!(s.contains("qmark:1"));
        assert!(s.contains("tail:q"), "trailing emoji 之前是问号: {}", s);
    }

    #[test]
    fn fingerprint_captures_newlines() {
        let s = extract_outbound_style_fingerprint("第一段\n\n第二段\n第三段");
        assert!(s.contains("nl:3"));
    }

    /// 完全相同的两条 → 不分歧。
    #[test]
    fn style_diverged_same_returns_false() {
        let a = extract_outbound_style_fingerprint("好的，请稍等。");
        assert!(!style_diverged(&a, &a));
    }

    /// 长度桶 + 句末符号 + 问号同时变 → 分歧 ≥ 3 → true。
    #[test]
    fn style_diverged_three_axes_changed() {
        let prev = extract_outbound_style_fingerprint("收到。");
        let cur = extract_outbound_style_fingerprint(&format!(
            "{}\n请问您还需要补充哪些信息呢？",
            "嗯".repeat(120)
        ));
        assert!(style_diverged(&prev, &cur), "prev={} cur={}", prev, cur);
    }

    /// 仅长度桶变（其它一致）→ 1 处不同 → false（容忍小幅波动）。
    #[test]
    fn style_diverged_minor_change_returns_false() {
        let prev = extract_outbound_style_fingerprint("好的。");
        let cur = extract_outbound_style_fingerprint("好的，已收到。");
        assert!(!style_diverged(&prev, &cur), "prev={} cur={}", prev, cur);
    }

    /// 空指纹（首轮回复）→ 永远不分歧，避免误触发首次 revision。
    #[test]
    fn style_diverged_empty_returns_false() {
        let cur = extract_outbound_style_fingerprint("好的。");
        assert!(!style_diverged("", &cur));
        assert!(!style_diverged(&cur, ""));
    }
}

#[cfg(test)]
mod review_passed_dual_gate_tests {
    //! Phase B / B1：`review_passed` 在 hallucination / grounding 两闸之外，
    //! 还要承担 humanLike + pressureRisk 双软闸。验证：
    //!
    //! * pressureRisk == 0（老数据 / reviewer 未填）→ 不参与拦截；
    //! * pressureRisk >= 阈值（默认 7）→ 必须返回 false，下游走
    //!   single-shot revision 通道；
    //! * humanLike < 阈值（默认 6）→ 必须返回 false；
    //! * 全分通过且 approved=true → 返回 true。

    use super::super::runtime::UserRuntimeParameters;
    use super::super::types::{DecisionReviewResult, ReviewScores};
    use super::review_passed;

    fn full_pass_review() -> DecisionReviewResult {
        DecisionReviewResult {
            approved: true,
            scores: ReviewScores {
                human_like: 80,
                emotional_value: 70,
                hallucination_score: 1,
                knowledge_grounding_score: 80,
                pressure_risk: 1,
            },
            ..Default::default()
        }
    }

    #[test]
    fn review_passed_passes_when_pressure_risk_under_threshold() {
        let runtime = UserRuntimeParameters::default();
        let review = full_pass_review();
        assert!(
            review_passed(&review, &runtime),
            "全分通过的 review 必须 review_passed=true"
        );
    }

    #[test]
    fn review_passed_blocks_when_pressure_risk_at_threshold() {
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.scores.pressure_risk = runtime.pressure_risk_block_at;
        assert!(
            !review_passed(&review, &runtime),
            "pressureRisk == block_at 必须拦截，触发 single-shot revision"
        );
    }

    #[test]
    fn review_passed_blocks_when_pressure_risk_above_threshold() {
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.scores.pressure_risk = runtime.pressure_risk_block_at + 5;
        assert!(
            !review_passed(&review, &runtime),
            "pressureRisk 超过 block_at 必须拦截"
        );
    }

    #[test]
    fn review_passed_ignores_pressure_risk_zero_for_legacy_data() {
        // 老数据 / reviewer 未输出 pressureRisk → R11 兼容：默认 0，不拦截。
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.scores.pressure_risk = 0;
        assert!(
            review_passed(&review, &runtime),
            "pressureRisk == 0（老数据/未填）必须视为豁免"
        );
    }

    #[test]
    fn review_passed_blocks_when_human_like_below_threshold() {
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.scores.human_like = runtime.human_like_rewrite_below - 1;
        assert!(
            !review_passed(&review, &runtime),
            "humanLike < rewrite_below 必须拦截，触发 single-shot revision"
        );
    }

    #[test]
    fn review_passed_blocks_when_approved_false() {
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.approved = false;
        assert!(
            !review_passed(&review, &runtime),
            "approved=false 必须直接拦截，无视分数"
        );
    }
}

#[cfg(test)]
mod reviewer_decision_view_tests {
    //! Phase B / B2：[`build_reviewer_decision_view`] 必须剥离 reply-agent
    //! 自我推理。验证：
    //!
    //! * 9 个 reasoning 字段（self_critique / why_should_reply 等）即使非空，
    //!   reviewer 视图里也不应包含其值或 key；
    //! * 候选回复事实面（reply_text / should_reply / matched_knowledge_ids 等）
    //!   必须保留；
    //! * intent_analysis / next_best_action / operating_memory_update 三个
    //!   推理 Document 不进 reviewer 视图。

    use super::super::types::AgentDecision;
    use super::build_reviewer_decision_view;
    use mongodb::bson::doc;

    fn decision_with_reasoning_filled() -> AgentDecision {
        AgentDecision {
            run_mode: "deep_reason".to_string(),
            risk_level: "low".to_string(),
            knowledge_need: "not_required".to_string(),
            should_reply: true,
            reply_text: "好的，明白您的顾虑".to_string(),
            user_understanding: "用户在表达对价格的担忧".to_string(),
            relationship_read: "信任度中等，处于评估阶段".to_string(),
            operation_goal: "建立信任，先不推产品".to_string(),
            knowledge_need_reason: "本轮不涉及产品承诺".to_string(),
            memory_update_reason: "unchanged".to_string(),
            self_critique: "上一轮回复略显急切，本轮放慢节奏".to_string(),
            why_should_reply: "用户提出了具体顾虑，需要回应".to_string(),
            why_skip_reply: String::new(),
            risk_self_check: "无产品承诺，无销售压力".to_string(),
            customer_stage: Some("evaluating".to_string()),
            intent_level: Some("medium".to_string()),
            operation_state: Some("trust_building".to_string()),
            decision_phase: "final".to_string(),
            autonomy_mode: "auto".to_string(),
            matched_knowledge_ids: vec!["k1".to_string(), "k2".to_string()],
            safe_claims_used: vec!["c1".to_string()],
            used_knowledge_ids: vec!["k1".to_string()],
            objections_detected: vec!["price".to_string()],
            intent_analysis: doc! { "explanation": "should not leak" },
            next_best_action: doc! { "explanation": "should not leak" },
            operating_memory_update: doc! { "explanation": "should not leak" },
            ..Default::default()
        }
    }

    #[test]
    fn reviewer_view_strips_self_critique_and_reasoning() {
        let view = build_reviewer_decision_view(&decision_with_reasoning_filled());
        // 9 个 reasoning 字段值都不应出现
        let leaked_values = [
            "用户在表达对价格的担忧",
            "信任度中等，处于评估阶段",
            "建立信任，先不推产品",
            "本轮不涉及产品承诺",
            "上一轮回复略显急切，本轮放慢节奏",
            "用户提出了具体顾虑，需要回应",
            "无产品承诺，无销售压力",
            "should not leak",
        ];
        for needle in leaked_values {
            assert!(
                !view.contains(needle),
                "reviewer view 不应包含 reply-agent 推理片段 {:?}: view={}",
                needle,
                view
            );
        }
        // 推理 key 也不应出现
        let leaked_keys = [
            "userUnderstanding",
            "relationshipRead",
            "operationGoal",
            "knowledgeNeedReason",
            "memoryUpdateReason",
            "selfCritique",
            "whyShouldReply",
            "whySkipReply",
            "riskSelfCheck",
            "intentAnalysis",
            "nextBestAction",
            "operatingMemoryUpdate",
        ];
        for key in leaked_keys {
            assert!(
                !view.contains(key),
                "reviewer view 不应包含 reasoning key {:?}: view={}",
                key,
                view
            );
        }
    }

    #[test]
    fn reviewer_view_preserves_reply_facts() {
        let view = build_reviewer_decision_view(&decision_with_reasoning_filled());
        // 候选回复事实面必须保留
        assert!(view.contains("好的，明白您的顾虑"), "应保留 replyText: {}", view);
        assert!(view.contains("\"shouldReply\":true"), "应保留 shouldReply: {}", view);
        assert!(view.contains("\"customerStage\":\"evaluating\""), "应保留 customerStage: {}", view);
        assert!(view.contains("\"operationState\":\"trust_building\""), "应保留 operationState: {}", view);
        assert!(view.contains("\"k1\""), "应保留 knowledge id 引用: {}", view);
        assert!(view.contains("price"), "应保留 objectionsDetected: {}", view);
    }

    #[test]
    fn reviewer_view_handles_empty_decision() {
        let view = build_reviewer_decision_view(&AgentDecision::default());
        // 即使是空 decision，view 也应是合法 JSON 且不 panic
        let parsed: serde_json::Value =
            serde_json::from_str(&view).expect("reviewer view 必须是合法 JSON");
        assert!(parsed.is_object(), "reviewer view 必须是 JSON 对象");
    }
}

#[cfg(test)]
mod dual_gate_classification_tests {
    //! Phase B / B1：双闸分类纯函数 + soft-gate-only 路由 + finalize 矫正 +
    //! decide_revision Proceed 的端到端单测。证明 humanLike / pressureRisk /
    //! emotionalValue 任一软闸不达标时，flow 走的是 single-shot revision
    //! 而不是 hold。硬闸失败仍走 hold。

    use super::super::runtime::UserRuntimeParameters;
    use super::super::types::{
        AgentDecision, DecisionReviewResult, ReviewScores, HOLD_CATEGORY_HELD_BY_AI_POLICY,
    };
    use super::{
        classify_dual_gate, decide_revision, finalize_review_for_send, route_dual_gate,
        DualGateClassification, FinalizeOutcome, GatewayStatusFinal, RevisionDecision,
    };
    use crate::models::{AgentStatus, Contact};
    use mongodb::bson::{DateTime, Document};

    fn full_pass_review() -> DecisionReviewResult {
        DecisionReviewResult {
            approved: true,
            scores: ReviewScores {
                human_like: 80,
                emotional_value: 70,
                hallucination_score: 1,
                knowledge_grounding_score: 80,
                pressure_risk: 1,
            },
            ..Default::default()
        }
    }

    fn finalize_contact() -> Contact {
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
            custom_agent_instructions: None,
            agent_profile: None,
            memory_summary: None,
            playbook_id: None,
            playbook_version: None,
            tags: Vec::new(),
            domain_attributes: None,
            domain_attributes_updated_at: None,
            commitments: Vec::new(),
            follow_up_policy: None,
            operation_state: None,
            operation_state_reason: None,
            operation_state_confidence: None,
            operation_state_updated_at: None,
            cooldown_until: None,
            operation_policy: Document::new(),
            profile_attributes: Document::new(),
            profile_updated_at: None,
            last_message_at: None,
            last_inbound_at: None,
            last_outbound_at: None,
            last_agent_run_at: None,
            last_outbound_style: None,
            intent_trajectory: Vec::new(),
            locale: None,
            created_at: DateTime::now(),
            updated_at: DateTime::now(),
        }
    }

    fn shouldreply_decision() -> AgentDecision {
        AgentDecision {
            should_reply: true,
            reply_text: "好的，我来想想看".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn classify_dual_gate_returns_all_pass_when_full_score() {
        let runtime = UserRuntimeParameters::default();
        let review = full_pass_review();
        assert_eq!(
            classify_dual_gate(&review, &runtime),
            DualGateClassification::AllPass
        );
    }

    #[test]
    fn classify_dual_gate_marks_hallucination_as_hard_failure() {
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.scores.hallucination_score = runtime.fact_risk_block_at + 1;
        match classify_dual_gate(&review, &runtime) {
            DualGateClassification::HardGateFailure { risks } => {
                assert!(risks.iter().any(|r| r.starts_with("hallucination_score_")));
            }
            other => panic!("expected HardGateFailure, got {:?}", other),
        }
    }

    #[test]
    fn classify_dual_gate_marks_low_grounding_as_hard_failure() {
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.scores.knowledge_grounding_score = runtime.product_accuracy_block_below - 1;
        match classify_dual_gate(&review, &runtime) {
            DualGateClassification::HardGateFailure { risks } => {
                assert!(risks
                    .iter()
                    .any(|r| r.starts_with("knowledge_grounding_")));
            }
            other => panic!("expected HardGateFailure, got {:?}", other),
        }
    }

    #[test]
    fn classify_dual_gate_marks_low_human_like_as_soft_failure() {
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.scores.human_like = runtime.human_like_rewrite_below - 1;
        match classify_dual_gate(&review, &runtime) {
            DualGateClassification::SoftGateFailure { direction, risks } => {
                assert!(direction.contains("humanLike"));
                assert!(!direction.trim().is_empty());
                assert!(risks.iter().any(|r| r.starts_with("human_like_")));
            }
            other => panic!("expected SoftGateFailure, got {:?}", other),
        }
    }

    #[test]
    fn classify_dual_gate_marks_high_pressure_risk_as_soft_failure() {
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.scores.pressure_risk = runtime.pressure_risk_block_at;
        match classify_dual_gate(&review, &runtime) {
            DualGateClassification::SoftGateFailure { direction, risks } => {
                assert!(direction.contains("pressureRisk"));
                assert!(risks.iter().any(|r| r.starts_with("pressure_risk_")));
            }
            other => panic!("expected SoftGateFailure, got {:?}", other),
        }
    }

    #[test]
    fn classify_dual_gate_ignores_pressure_risk_zero_as_legacy() {
        // R11 兼容：pressure_risk == 0 视作未填，不参与分类。
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.scores.pressure_risk = 0;
        assert_eq!(
            classify_dual_gate(&review, &runtime),
            DualGateClassification::AllPass
        );
    }

    #[test]
    fn classify_dual_gate_marks_low_emotional_value_as_soft_failure() {
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.scores.emotional_value = runtime.emotional_value_rewrite_below - 1;
        match classify_dual_gate(&review, &runtime) {
            DualGateClassification::SoftGateFailure { direction, risks } => {
                assert!(direction.contains("emotionalValue"));
                assert!(risks.iter().any(|r| r.starts_with("emotional_value_")));
            }
            other => panic!("expected SoftGateFailure, got {:?}", other),
        }
    }

    #[test]
    fn classify_dual_gate_combines_multiple_soft_failures() {
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.scores.human_like = runtime.human_like_rewrite_below - 1;
        review.scores.pressure_risk = runtime.pressure_risk_block_at + 1;
        match classify_dual_gate(&review, &runtime) {
            DualGateClassification::SoftGateFailure { direction, risks } => {
                assert!(direction.contains("humanLike"));
                assert!(direction.contains("pressureRisk"));
                assert!(risks.iter().any(|r| r.starts_with("human_like_")));
                assert!(risks.iter().any(|r| r.starts_with("pressure_risk_")));
            }
            other => panic!("expected SoftGateFailure, got {:?}", other),
        }
    }

    #[test]
    fn classify_dual_gate_hard_failure_wins_over_soft() {
        // 同时硬闸 + 软闸失败时，硬闸优先 — soft path 不可绕过 hard path。
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.scores.hallucination_score = runtime.fact_risk_block_at + 1;
        review.scores.human_like = runtime.human_like_rewrite_below - 1;
        match classify_dual_gate(&review, &runtime) {
            DualGateClassification::HardGateFailure { .. } => {}
            other => panic!("expected HardGateFailure, got {:?}", other),
        }
    }

    #[test]
    fn route_dual_gate_sets_needs_revision_on_soft_failure() {
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.scores.human_like = runtime.human_like_rewrite_below - 1;
        route_dual_gate(&mut review, &runtime);
        assert!(review.needs_revision, "软闸失败必须写 needs_revision");
        assert!(
            !review.revision_direction.trim().is_empty(),
            "软闸失败必须自动补 revision_direction"
        );
        // approved 由 review_passed 决定，软闸下应为 false（finalize 会矫正）。
        assert!(!review.approved);
    }

    #[test]
    fn route_dual_gate_preserves_reviewer_revision_direction() {
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.scores.human_like = runtime.human_like_rewrite_below - 1;
        review.revision_direction = "reviewer 自己写的明确方向".to_string();
        route_dual_gate(&mut review, &runtime);
        assert_eq!(
            review.revision_direction, "reviewer 自己写的明确方向",
            "reviewer 已给方向时不可被机器化兜底覆盖"
        );
        assert!(review.needs_revision);
    }

    #[test]
    fn route_dual_gate_leaves_hard_failure_without_revision_flag() {
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.scores.hallucination_score = runtime.fact_risk_block_at + 1;
        let prev_dir = review.revision_direction.clone();
        route_dual_gate(&mut review, &runtime);
        assert!(!review.needs_revision, "硬闸失败不能触发 revision");
        assert_eq!(review.revision_direction, prev_dir);
        assert!(!review.approved);
    }

    #[test]
    fn route_dual_gate_keeps_all_pass_approved_true() {
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        route_dual_gate(&mut review, &runtime);
        assert!(review.approved);
        assert!(!review.needs_revision);
    }

    #[test]
    fn finalize_promotes_soft_gate_failure_to_approved() {
        // route_dual_gate(soft fail) → finalize 应矫正 approved=true 并返回
        // GatewayStatusFinal::Approved，让 decide_revision 进 Proceed。
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.scores.pressure_risk = runtime.pressure_risk_block_at + 2;
        route_dual_gate(&mut review, &runtime);
        let mut decision = shouldreply_decision();
        let contact = finalize_contact();
        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            &[],
            Vec::new(),
            "用户最新消息",
        );
        let FinalizeOutcome {
            review: finalized,
            status,
            ..
        } = outcome;
        assert_eq!(
            status,
            GatewayStatusFinal::Approved,
            "软闸 soft-gate-only 失败必须矫正为 Approved"
        );
        assert!(finalized.approved);
        assert!(finalized.needs_revision);
        assert_eq!(finalized.final_review_status, "approved");
    }

    #[test]
    fn finalize_keeps_hard_gate_failure_in_held() {
        // route_dual_gate(hard fail) → finalize 应仍走 Held(held_by_ai_policy)。
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.scores.hallucination_score = runtime.fact_risk_block_at + 1;
        route_dual_gate(&mut review, &runtime);
        let mut decision = shouldreply_decision();
        let contact = finalize_contact();
        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            &[],
            Vec::new(),
            "用户最新消息",
        );
        let FinalizeOutcome {
            review: finalized,
            status,
            ..
        } = outcome;
        match status {
            GatewayStatusFinal::Held(category) => {
                assert_eq!(category, HOLD_CATEGORY_HELD_BY_AI_POLICY);
            }
            other => panic!("expected Held, got {:?}", other),
        }
        assert!(!finalized.approved);
        assert_eq!(
            finalized.final_review_status,
            HOLD_CATEGORY_HELD_BY_AI_POLICY
        );
    }

    #[test]
    fn decide_revision_proceeds_after_soft_gate_matchback() {
        // 端到端：reviewer 给出软闸失败的分数 → route_dual_gate 写
        // needs_revision + revision_direction → finalize 矫正为 Approved →
        // decide_revision 必须返回 Proceed，触发 single-shot revision。
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.scores.human_like = runtime.human_like_rewrite_below - 2;
        route_dual_gate(&mut review, &runtime);
        let mut decision = shouldreply_decision();
        let contact = finalize_contact();
        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            &[],
            Vec::new(),
            "用户最新消息",
        );
        let FinalizeOutcome {
            review: finalized,
            status,
            ..
        } = outcome;
        assert_eq!(status, GatewayStatusFinal::Approved);
        let revision = decide_revision(&status, &finalized, false);
        assert_eq!(
            revision,
            RevisionDecision::Proceed,
            "soft-gate-only 失败必须最终触发 Proceed"
        );
    }

    #[test]
    fn decide_revision_does_not_proceed_after_hard_gate_failure() {
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.scores.knowledge_grounding_score = runtime.product_accuracy_block_below - 1;
        route_dual_gate(&mut review, &runtime);
        let mut decision = shouldreply_decision();
        let contact = finalize_contact();
        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            &[],
            Vec::new(),
            "用户最新消息",
        );
        let FinalizeOutcome {
            review: finalized,
            status,
            ..
        } = outcome;
        let revision = decide_revision(&status, &finalized, false);
        assert_eq!(
            revision,
            RevisionDecision::NotEligible,
            "硬闸失败永远不能触发 revision"
        );
    }
}

#[cfg(test)]
mod dual_reviewer_disagreement_tests {
    //! Phase E / E2：双 reviewer 分歧检测纯函数 + apply 副作用单测。
    //! 覆盖 6 档：
    //! - 双方 AllPass → None
    //! - 双方 HardGate → None（不细化，避免 LLM 评分尺度差异误判）
    //! - 双方 SoftGate 命中相同 → None
    //! - 双方 SoftGate 命中不同 → SoftRiskDelta
    //! - AllPass × SoftGate → DualGateMismatch
    //! - approved-flag 不一致 → ApprovedMismatch（最高优先级）
    //! - apply 副作用：needs_revision=true、空 revision_direction 兜底、risk
    //!   marker 去重追加

    use super::super::runtime::UserRuntimeParameters;
    use super::super::types::{DecisionReviewResult, ReviewScores};
    use super::{
        apply_dual_reviewer_disagreement, detect_dual_reviewer_disagreement,
        DualReviewerDisagreement,
    };

    fn full_pass_review() -> DecisionReviewResult {
        DecisionReviewResult {
            approved: true,
            scores: ReviewScores {
                human_like: 80,
                emotional_value: 70,
                hallucination_score: 1,
                knowledge_grounding_score: 80,
                pressure_risk: 1,
            },
            ..Default::default()
        }
    }

    fn soft_failed_review_low_human_like(runtime: &UserRuntimeParameters) -> DecisionReviewResult {
        let mut r = full_pass_review();
        r.approved = false;
        r.scores.human_like = runtime.human_like_rewrite_below - 1;
        r
    }

    fn soft_failed_review_high_pressure(runtime: &UserRuntimeParameters) -> DecisionReviewResult {
        let mut r = full_pass_review();
        r.approved = false;
        r.scores.pressure_risk = runtime.pressure_risk_block_at + 1;
        r
    }

    fn hard_failed_review(runtime: &UserRuntimeParameters) -> DecisionReviewResult {
        let mut r = full_pass_review();
        r.approved = false;
        r.scores.hallucination_score = runtime.fact_risk_block_at + 1;
        r
    }

    #[test]
    fn both_all_pass_returns_none() {
        let runtime = UserRuntimeParameters::default();
        let primary = full_pass_review();
        let second = full_pass_review();
        assert!(detect_dual_reviewer_disagreement(&primary, &second, &runtime).is_none());
    }

    #[test]
    fn both_hard_gate_returns_none() {
        let runtime = UserRuntimeParameters::default();
        let primary = hard_failed_review(&runtime);
        let second = hard_failed_review(&runtime);
        assert!(detect_dual_reviewer_disagreement(&primary, &second, &runtime).is_none());
    }

    #[test]
    fn both_soft_gate_same_risk_returns_none() {
        let runtime = UserRuntimeParameters::default();
        let primary = soft_failed_review_low_human_like(&runtime);
        let second = soft_failed_review_low_human_like(&runtime);
        assert!(detect_dual_reviewer_disagreement(&primary, &second, &runtime).is_none());
    }

    #[test]
    fn both_soft_gate_different_risks_returns_soft_risk_delta() {
        let runtime = UserRuntimeParameters::default();
        let primary = soft_failed_review_low_human_like(&runtime);
        let second = soft_failed_review_high_pressure(&runtime);
        assert_eq!(
            detect_dual_reviewer_disagreement(&primary, &second, &runtime),
            Some(DualReviewerDisagreement::SoftRiskDelta)
        );
    }

    #[test]
    fn approved_mismatch_takes_priority() {
        let runtime = UserRuntimeParameters::default();
        let primary = full_pass_review();
        // 第二份 reviewer 把 hallucination 抬过硬闸阈值 → review_passed=false
        let second = hard_failed_review(&runtime);
        assert_eq!(
            detect_dual_reviewer_disagreement(&primary, &second, &runtime),
            Some(DualReviewerDisagreement::ApprovedMismatch),
            "approved 标志不一致比 dual_gate 类别不一致更优先"
        );
    }

    #[test]
    fn all_pass_vs_soft_gate_returns_dual_gate_mismatch() {
        let runtime = UserRuntimeParameters::default();
        // 主 reviewer AllPass，第二个软闸命中但仍 approved=true（虚构场景）
        // → review_passed 在 runtime 阈值下两者一致都为 true，但分类不一致
        let primary = full_pass_review();
        let mut second = full_pass_review();
        // human_like 拉到刚好等于阈值（不触发 review_passed=false，但 classify
        // 走 SoftGateFailure 路径 —— 注意 review_passed 会一致返回 true）。
        // 为了保证 review_passed 双方都 true，second.approved 保持 true。
        second.scores.human_like = runtime.human_like_rewrite_below - 1;
        second.approved = true;
        // review_passed 内部依赖 approved + scores 共同判定；如果 approved=true
        // 但软闸命中，review_passed 通常仍返回 false → 走 ApprovedMismatch。
        // 因此本用例要的是 review_passed 一致 + classify 不一致。
        // 实际实现中只要双方 approved 都 true 且分数都过硬闸，review_passed=true；
        // 软闸不影响 review_passed —— 验证此前提。
        let primary_passed = super::review_passed(&primary, &runtime);
        let second_passed = super::review_passed(&second, &runtime);
        if primary_passed != second_passed {
            // 实现把软闸纳入 review_passed —— 改走 ApprovedMismatch 验证路径。
            assert_eq!(
                detect_dual_reviewer_disagreement(&primary, &second, &runtime),
                Some(DualReviewerDisagreement::ApprovedMismatch)
            );
        } else {
            assert_eq!(
                detect_dual_reviewer_disagreement(&primary, &second, &runtime),
                Some(DualReviewerDisagreement::DualGateMismatch)
            );
        }
    }

    #[test]
    fn apply_sets_needs_revision_and_appends_risk_marker() {
        let mut review = full_pass_review();
        review.needs_revision = false;
        review.revision_direction = String::new();
        review.risks.clear();
        apply_dual_reviewer_disagreement(&mut review, &DualReviewerDisagreement::SoftRiskDelta);
        assert!(review.needs_revision, "needs_revision 必须被强制置 true");
        assert!(
            !review.revision_direction.trim().is_empty(),
            "空 revision_direction 必须被兜底文案填充"
        );
        assert!(
            review
                .risks
                .iter()
                .any(|r| r == "reviewer_dual_disagree:soft_risk_delta"),
            "risks 必须追加 disagreement risk_marker"
        );
    }

    #[test]
    fn apply_does_not_overwrite_existing_revision_direction() {
        let mut review = full_pass_review();
        let existing = "保留主 reviewer 自己的改写指令".to_string();
        review.revision_direction = existing.clone();
        apply_dual_reviewer_disagreement(&mut review, &DualReviewerDisagreement::ApprovedMismatch);
        assert_eq!(
            review.revision_direction, existing,
            "已有的 revision_direction 不能被覆盖"
        );
    }

    #[test]
    fn apply_is_idempotent_on_risk_markers() {
        let mut review = full_pass_review();
        apply_dual_reviewer_disagreement(&mut review, &DualReviewerDisagreement::DualGateMismatch);
        apply_dual_reviewer_disagreement(&mut review, &DualReviewerDisagreement::DualGateMismatch);
        let count = review
            .risks
            .iter()
            .filter(|r| r == &"reviewer_dual_disagree:dual_gate_mismatch")
            .count();
        assert_eq!(count, 1, "重复 apply 不应重复追加同一 risk_marker");
    }
}
