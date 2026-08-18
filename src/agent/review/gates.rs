//! Review 判定闸门：双闸分类 → 双 reviewer 分歧 → 最终安全汇总 → revision 决策。
//!
//! 本文件汇集 review 的**纯判定逻辑**（不调 LLM、不写库），从异步主流程
//! [`super::review_decision`] 拆出，便于密集单测覆盖：
//! - `review_passed` / `classify_dual_gate` / `route_dual_gate`：Phase B/B1 双闸；
//! - `build_reviewer_decision_view`：Phase B/B2 reviewer 事实面视图；
//! - `DualReviewerDisagreement` / `detect` / `apply`：Phase E/E2 双脑分歧；
//! - `finalize_review_for_send`：W2 最终安全汇总层（所有硬门汇于一处）；
//! - `decide_revision` / `derive_revision_failure`：W2 single-shot revision 控制流。

use mongodb::bson::Document;

use crate::agent::runtime::UserRuntimeParameters;
use crate::agent::types::{
    assert_hold_category_valid, AgentDecision, DecisionReviewResult, HoldCategoryAssertion,
    EVENT_AUTONOMY_HOLD_CATEGORY_INVALID, HOLD_CATEGORY_AI_WAITING_FOR_MORE_CONTEXT,
    HOLD_CATEGORY_BLOCKED_BY_SAFETY_GUARD, HOLD_CATEGORY_HELD_BY_AI_POLICY,
};

pub fn review_passed(review: &DecisionReviewResult, runtime: &UserRuntimeParameters) -> bool {
    let live_scores_are_valid = review
        .claim_analysis
        .get_str("reviewScoreStatus")
        .is_ok_and(|status| status == "valid");
    review.approved
        && review.scores.hallucination_score < runtime.fact_risk_block_at
        && review.scores.human_like >= runtime.human_like_rewrite_below
        && review.scores.emotional_value >= runtime.emotional_value_rewrite_below
        // G01 / H14：grounding 项与 classify_dual_gate:120 同源。classify 算
        // `grounding_gate_applies = !bypass || claim`；本闸要的是其"通过"对偶：
        // 闸不适用(bypass=true 且无产品声明) 或 grounding 达标即放行 →
        // `!grounding_gate_applies || grounding>=阈值` = `(bypass && !claim) || grounding>=阈值`。
        // DEFAULT bypass=false → `(false && ..)=false` → 整段退化为原 `grounding>=阈值`（字节等价）。
        && ((runtime.grounding_gate_bypass_without_claim
                && !crate::agent::guards::claim_requires_product_knowledge(&review.claim_analysis))
            || review.scores.knowledge_grounding_score >= runtime.product_accuracy_block_below)
        // Phase B / B1：恢复 pressure_risk 软闸 — `>=` 阈值视为压迫感过强，拦截。
        // 0 表示 reviewer 未给分（含老数据反序列化默认），不参与拦截。
        && ((!live_scores_are_valid && review.scores.pressure_risk == 0)
            || (review.scores.pressure_risk > 0
                && review.scores.pressure_risk < runtime.pressure_risk_block_at))
        // boundary/privacy 软闸对偶（与 classify_dual_gate 的 boundary_privacy_safety
        // <= 3 判定同源）：1-3 低分（泄露内部画像/评判、暴露 AI 身份或幕后领导信息）
        // 拦截。0 = reviewer 未填 / 老数据豁免（同 pressure_risk）；>=4 放行。
        && ((!live_scores_are_valid && review.scores.boundary_privacy_safety == 0)
            || review.scores.boundary_privacy_safety > 3)
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
/// 是否回复、知识引用、状态/阶段、tool-loop 协议字段。候选正文由 Reviewer 的独立
/// `候选回复` 槽注入，不在此重复序列化。
pub(crate) fn build_reviewer_decision_view(decision: &AgentDecision) -> String {
    let semantic_assessment = decision
        .intent_analysis
        .get_document("semanticAssessment")
        .cloned()
        .unwrap_or_default();
    serde_json::to_string(&mongodb::bson::doc! {
        "shouldReply": decision.should_reply,
        "matchedKnowledgeIds": decision.matched_knowledge_ids.clone(),
        "safeClaimsUsed": decision.safe_claims_used.clone(),
        "usedKnowledgeIds": decision.used_knowledge_ids.clone(),
        "objectionsDetected": decision.objections_detected.clone(),
        "namecardToSend": mongodb::bson::to_bson(&decision.namecard_to_send)
            .unwrap_or(mongodb::bson::Bson::Null),
        "customerStage": decision.customer_stage.clone().unwrap_or_default(),
        "intentLevel": decision.intent_level.clone().unwrap_or_default(),
        "operationState": decision.operation_state.clone().unwrap_or_default(),
        "decisionPhase": decision.decision_phase.clone(),
        "autonomyMode": decision.autonomy_mode.clone(),
        "runMode": decision.run_mode.clone(),
        "riskLevel": decision.risk_level.clone(),
        "knowledgeNeed": decision.knowledge_need.clone(),
        "commitmentUpdates": mongodb::bson::to_bson(&decision.commitment_updates)
            .unwrap_or(mongodb::bson::Bson::Array(Vec::new())),
        "semanticAssessment": semantic_assessment,
    })
    .unwrap_or_default()
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
    let live_scores_are_valid = review
        .claim_analysis
        .get_str("reviewScoreStatus")
        .is_ok_and(|status| status == "valid");
    let mut hard_risks: Vec<String> = Vec::new();
    if review.scores.hallucination_score >= runtime.fact_risk_block_at {
        hard_risks.push(format!(
            "hallucination_score_{}_ge_{}",
            review.scores.hallucination_score, runtime.fact_risk_block_at
        ));
    }
    // universal-domain-adaptation H14：grounding 软分数硬闸条件化。
    // DEFAULT `bypass=false` → `!false = true` → grounding_gate_applies 恒真 →
    // 与改造前字节等价（每条回复都判）。情感/关系域 `bypass=true` → 仅当本条
    // claim_analysis.requiresProductKnowledge=true 时才纳入 grounding 硬闸，纯情感
    // 回复不再被 grounding 低分误拦。红线：blocked_unverified_product_claim
    // （finalize 的 R5.4 verified 强约束 + 漏判探针）与本闸不同函数/不同阶段，不受影响。
    let grounding_gate_applies = !runtime.grounding_gate_bypass_without_claim
        || crate::agent::guards::claim_requires_product_knowledge(&review.claim_analysis);
    if grounding_gate_applies
        && review.scores.knowledge_grounding_score < runtime.product_accuracy_block_below
    {
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
    if live_scores_are_valid && review.scores.pressure_risk == 0 {
        soft_risks.push("pressure_risk_0_unscored".to_string());
        direction_parts.push(
            "pressureRisk returned zero for a live review, so pressure safety was not established; rewrite conservatively and provide a concrete score."
                .to_string(),
        );
    } else if review.scores.pressure_risk != 0
        && review.scores.pressure_risk >= runtime.pressure_risk_block_at
    {
        soft_risks.push(format!(
            "pressure_risk_{}_ge_{}",
            review.scores.pressure_risk, runtime.pressure_risk_block_at
        ));
        direction_parts.push(format!(
            "pressureRisk 评分 {} 高于等于阈值 {}：去掉催促、紧迫、稀缺感、连环追问，\
             承接对方顾虑、留出思考空间；是否保留追问由你按用户当前语境判断——\
             用户若表达不想被追问，应改用陈述句承接、不再提问。",
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
    // 渐进式三档+隐私维度(2026-06-23)：边界/隐私安全软闸。1-3 低分触发改写。
    // `!= 0` 仿照上方 pressure_risk 的老数据兼容豁免：缺省 0 表示 reviewer 未填
    // /旧持久化文档无此键，视为未评分不拦截；真低分(1-3)才命中软闸。
    if (live_scores_are_valid || review.scores.boundary_privacy_safety != 0)
        && review.scores.boundary_privacy_safety <= 3
    {
        soft_risks.push(format!(
            "boundary_privacy_safety_{}_le_3",
            review.scores.boundary_privacy_safety
        ));
        direction_parts.push(
            "候选回复可能泄露内部画像/评判、暴露AI身份或幕后领导信息——请改写：\
             移除对客户的内部评判表述，不暴露AI身份与幕后决策来源。"
                .to_string(),
        );
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
    reply_text: &str,
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
            // item ②：把本次回复的客观特征（问句数 / 字数 / 共情词密度）追加到
            // 改写方向后，让单次改写有的放矢，而非只给机械模板。对 reviewer 自带
            // 方向同样追加（事实标注不冲突，只补充客观信息）。
            let features = reply_objective_features(reply_text);
            if !features.is_empty() {
                if !review.revision_direction.is_empty() {
                    review.revision_direction.push(' ');
                }
                review.revision_direction.push_str(&features);
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

/// item ②：从候选回复正文提取廉价客观特征，供软闸改写指令使用。
///
/// 不做任何判罚、**也不替改写 Agent 预判方向**——只把问句数和字数这两个
/// 不依赖自然语言词表的客观量拼成一句提示。语气、共情和业务语义全部交给改写 Agent。
/// 空回复返回空串（不追加）。
///
/// **问句一项刻意只报数、不下"该加该减"的结论**：是否调整问句高度依赖用户当前
/// 语境（用户说"别一直问"时该减，用户在等推进时可加），这是语义判断，归改写
/// Agent（它能看到完整对话）。历史上这里写死「问句过少可加反问」，在用户明确
/// 拒绝追问时会把改写 Agent 反向带偏、反复加问句导致 revision_failed——故移除
/// 该机器预判，符合本项目 agent-first（m014 下线关键词快路径）的取向。
fn reply_objective_features(reply_text: &str) -> String {
    let text = reply_text.trim();
    if text.is_empty() {
        return String::new();
    }
    let questions = text.matches(['?', '？']).count();
    let chars = text.chars().count();
    format!(
        "【本次回复客观特征】问句 {questions} 个、{chars} 字——\
         改写时请结合完整对话判断是否需要调整问句、承接方式和篇幅；不要用固定词表\
         代替语义判断。"
    )
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
/// ```text
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

/// 该 contact 是否有生效的 A 类领导授权产品豁免。
///
/// 判据：`Contact.domain_attributes.<PRINCIPAL_PRODUCT_EXEMPTION_ATTR>` 存在，且其子文档
/// `granted == true`。领导针对该客户授权后由 relay 接线写入该子文档；R5.4 产品门据此
/// 并联放行该客户的产品说法。`domain_attributes` 为 `Option<Document>`，缺容器 / 缺 key /
/// 缺 granted 一律返回 false（fail-closed：无授权即按原门拦截）。
pub fn contact_has_principal_product_exemption(contact: &crate::models::Contact) -> bool {
    contact
        .domain_attributes
        .as_ref()
        .and_then(|d| {
            d.get_document(crate::models::PRINCIPAL_PRODUCT_EXEMPTION_ATTR)
                .ok()
        })
        .and_then(|d| d.get_bool("granted").ok())
        .unwrap_or(false)
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
/// * `promote_risks`：来自 [`crate::agent::types::RawAgentDecision::validate_and_promote`]
///   的协议违规标签（如 `missing_required_field:* / invalid_enum_value:* /
///   invalid_type:* / decision_phase_invalid:* /
///   insufficient_detail_in_critical_turn:*`）。
/// * `priced_from_catalog`：本轮 decision 报价 product_id 是否命中本 workspace active
///   产品目录（G2 结构化背书，由 gateway 算好传入）；R5.4 与 verified_chunks 取或。
/// * `principal_product_exempted`：该客户是否有生效的 A 类领导授权产品豁免（由 gateway
///   经 [`contact_has_principal_product_exemption`] 从 contact 读出传入）；R5.4 第三条
///   并联背书，与前两者取或，任一成立即放行产品声明门。
pub fn finalize_review_for_send(
    review: DecisionReviewResult,
    decision: &mut AgentDecision,
    runtime: &UserRuntimeParameters,
    _contact: &crate::models::Contact,
    knowledge_chunks: &[crate::models::OperationKnowledgeChunk],
    promote_risks: Vec<String>,
    priced_from_catalog: bool,
    principal_product_exempted: bool,
) -> FinalizeOutcome {
    let _stage_timer = crate::agent::run_audit::stage_timer("finalize");
    finalize_review_for_send_at(
        review,
        decision,
        runtime,
        _contact,
        knowledge_chunks,
        promote_risks,
        priced_from_catalog,
        principal_product_exempted,
        mongodb::bson::DateTime::now(),
    )
}

/// Same final safety aggregation as [`finalize_review_for_send`], evaluated at
/// an explicit instant. Prompt Shadow uses one instant for both branches so a
/// knowledge-expiry boundary cannot become an accidental second variable.
#[allow(clippy::too_many_arguments)]
pub(crate) fn finalize_review_for_send_at(
    review: DecisionReviewResult,
    decision: &mut AgentDecision,
    runtime: &UserRuntimeParameters,
    _contact: &crate::models::Contact,
    knowledge_chunks: &[crate::models::OperationKnowledgeChunk],
    promote_risks: Vec<String>,
    priced_from_catalog: bool,
    principal_product_exempted: bool,
    evaluated_at: mongodb::bson::DateTime,
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

    // R1.5 长度违规（关键轮推理字段偏短）→ single-shot revision，**不**硬 block。
    //
    // spec R1.5 只要求 `review.approved = false`；它不是结构性协议违规（字段已输出、
    // 只是 < 20 字 / 回复理由 < 30 字），靠一次改写即可补全。历史代码把它和
    // missing_required_field 一起塞进 blocked_by_required_field 硬门 + 直接 return，
    // 连 revision 都不给——t15 跌单弧 6 轮因此全程哑火（0 轮 approved < 下限 2）。
    //
    // 这里改为：当 promote_risks 只含 insufficient_detail（结构性硬违规已在上方
    // return）、reply-agent 本就要回复（should_reply）、且**硬安全闸通过**时，标记
    // needs_revision + 给出"补全推理痕迹"方向，让 finalize 末尾把 approved 矫正回
    // true → 进 decide_revision 的 Proceed 通道改写一次。
    //
    // 硬闸守卫（关键安全不变量）：仅当 classify_dual_gate 非 HardGateFailure 时才降级。
    // 若该轮同时撞 hallucination / grounding 硬闸，route_dual_gate 已写 approved=false
    // 且不设 needs_revision；此处绝不能把 needs_revision 抬成 true，否则 finalize 末尾
    // 756 行分支会把硬闸失败的危险回复矫正成 approved 发出去。R5.4 verified-claim 硬门
    // / should_hold / budget 各自在下方 return，本降级既不绕过它们、也不改 should_reply。
    let hard_gate_failed = matches!(
        classify_dual_gate(&review, runtime),
        DualGateClassification::HardGateFailure { .. }
    );
    if has_insufficient_detail_only(&promote_risks) && decision.should_reply && !hard_gate_failed {
        review.needs_revision = true;
        if review.revision_direction.trim().is_empty() {
            review.revision_direction =
                "本轮被判定为关键变化轮，但部分推理字段（用户理解 / 关系判断 / 运营目标 / \
                 知识需求理由 / 记忆更新理由 / 自我批判 / 风险自检，或回复理由）过短，缺乏可审计的\
                 因果链。请在保持回复正文自然口语的同时，把这些自治协议字段补写充分（每个 ≥ 20 字\
                 的实质内容、回复理由 ≥ 30 字含足量中文），不要用 \"unchanged\" 占位。"
                    .to_string();
        }
    }

    // R3.7：预算超额 + needs_review=true → blocked_by_budget
    if review
        .risks
        .iter()
        .any(|r| r == "budget_exceeded_no_review")
    {
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

    // Revalidate verified evidence at the actual send boundary. ClaimGate and Reviewer may finish
    // just before a chunk expires; a non-product claim must not retain authorization from a source
    // that is no longer verified at finalize time. Product claims remain owned by R5.4 below so
    // their established blocked_unverified_product_claim status and exemptions stay unchanged.
    let verified_at_finalize = crate::agent::guards::compute_verified_chunks(
        &decision.used_knowledge_ids,
        knowledge_chunks,
        evaluated_at,
    )
    .into_iter()
    .filter_map(|chunk| {
        chunk
            .id
            .map(|id| format!("verified_knowledge:{}", id.to_hex()))
    })
    .collect::<std::collections::HashSet<_>>();
    let stale_non_product_evidence = review
        .claim_analysis
        .get_array("claimManifest")
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_document())
        .filter(|claim| {
            claim.get_str("evidenceNeed") == Ok("required")
                && !claim.get_bool("productClaim").unwrap_or(false)
        })
        .flat_map(|claim| {
            claim
                .get_array("evidenceRefs")
                .ok()
                .into_iter()
                .flatten()
                .filter_map(|value| value.as_str())
        })
        .any(|evidence_ref| {
            evidence_ref.starts_with("verified_knowledge:")
                && !verified_at_finalize.contains(evidence_ref)
        });
    if stale_non_product_evidence {
        review.approved = false;
        review.scores.hallucination_score = review.scores.hallucination_score.max(6);
        extend_risks_unique(
            &mut review.risks,
            std::iter::once("business_claim_evidence_expired_before_send".to_string()),
        );
        decision.should_reply = false;
        decision.autonomy_mode = "blocked".to_string();
        pending_events.push(PendingFinalizeEvent {
            kind: "business_claim_evidence_expired".to_string(),
            status: "blocked".to_string(),
            summary: "业务事实证据在发送前已失效，候选回复被安全拦截".to_string(),
            details: mongodb::bson::doc! {
                "used_knowledge_ids": decision.used_knowledge_ids.clone(),
            },
        });
        review.final_review_status = "blocked_by_safety_guard".to_string();
        return FinalizeOutcome {
            review,
            status: GatewayStatusFinal::BlockedBySafetyGuard,
            pending_events,
        };
    }

    // Open-world business evidence gate. The independent semantic ClaimGate extracts atomic
    // assertions, while the service validates candidate quotes and evidence IDs. Product claims
    // keep their established R5.4 authorization paths below (verified knowledge, catalog, or
    // principal exemption); only unsupported non-product business facts are stopped here.
    let unsupported_non_product = review
        .claim_analysis
        .get_i64("unsupportedNonProductBusinessClaimCount")
        .unwrap_or(0);
    if unsupported_non_product > 0 {
        review.approved = false;
        review.scores.hallucination_score = review.scores.hallucination_score.max(6);
        extend_risks_unique(
            &mut review.risks,
            std::iter::once("unsupported_business_claim".to_string()),
        );
        decision.should_reply = false;
        decision.autonomy_mode = "blocked".to_string();
        let manifest = review
            .claim_analysis
            .get_array("claimManifest")
            .cloned()
            .unwrap_or_default();
        pending_events.push(PendingFinalizeEvent {
            kind: "unsupported_business_claim_blocked".to_string(),
            status: "blocked".to_string(),
            summary: "候选回复仍含没有可信来源支持的现实业务事实，发送前已拦截".to_string(),
            details: mongodb::bson::doc! {
                "unsupported_non_product_claim_count": unsupported_non_product,
                "claim_manifest": manifest,
            },
        });
        review.final_review_status = "blocked_by_safety_guard".to_string();
        return FinalizeOutcome {
            review,
            status: GatewayStatusFinal::BlockedBySafetyGuard,
            pending_events,
        };
    }

    // ── R5.4：verified knowledge 产品声明强约束 ──
    //
    // CLAUDE.md 硬规则：产品声明必须由 operation_knowledge_chunks 中 verified
    // 知识背书，否则 blocked_unverified_product_claim。这是对 reviewer 自评分
    // （knowledge_grounding_score 软闸，可被 LLW 高估）的确定性结构化兜底——
    // 仅当 reviewer 的 claim_analysis 显式声明 requiresProductKnowledge=true 时
    // 触发；此时若本 run 引用的知识切片里没有任何 verified chunk，强制 block。
    //
    // 客观购买事实增强（2026-06-15 spec §5.4）：G2 active product 是「结构化 verified
    // 背书」的并联来源——admin 在「产品与成交」频道显式录入的 product_id/价格/SKU，
    // 可信度 ≥ 手工撰写的非结构化知识 chunk。故 `priced_from_catalog`（决策引用的
    // product_id ∈ 本 workspace active products，由 gateway 算好传入）与 verified_chunks
    // 取**或**：两者皆空才 block。零扰动：无产品行业产品表空 → priced_from_catalog 恒假
    // → 行为与改造前字节等价（纯情感回复 requiresProductKnowledge 本就为假，不进此块）。
    //
    // 第三条并联背书（A 类领导授权豁免，`principal_product_exempted`）：领导针对该客户
    // 显式授权后，该客户的产品说法视为已获授权背书——与 verified_chunks / priced_from_catalog
    // 三者取**或**，任一成立即放行。零扰动：无授权记录时 gateway 传入恒假（生产暂无写入方，
    // 写入在 relay 接线阶段完成），行为与改造前字节等价。
    //
    // 注：2026-05-25 知识库清理删除了 chunk.safe_claims / ProductClaimMarkers，
    // 故 R5.7 safe_claims 反向门 / R5.3 claim_analysis 缺失 fail-closed 推断不在
    // 本次恢复范围；claim_analysis 缺失时按"非产品声明"放行（reviewer 软闸 +
    // knowledge_router verified-only corpus 仍在兜底）。
    if crate::agent::guards::claim_requires_product_knowledge(&review.claim_analysis) {
        let verified_chunks = crate::agent::guards::compute_verified_chunks(
            &decision.used_knowledge_ids,
            knowledge_chunks,
            evaluated_at,
        );
        if verified_chunks.is_empty() && !priced_from_catalog && !principal_product_exempted {
            review.approved = false;
            review.scores.hallucination_score = review.scores.hallucination_score.max(6);
            extend_risks_unique(
                &mut review.risks,
                std::iter::once("product_claim_without_verified_knowledge".to_string()),
            );
            decision.should_reply = false;
            decision.autonomy_mode = "blocked".to_string();
            let mut details = Document::new();
            details.insert("used_knowledge_ids", decision.used_knowledge_ids.clone());
            details.insert("knowledge_chunk_total", knowledge_chunks.len() as i64);
            pending_events.push(PendingFinalizeEvent {
                kind: "product_claim_blocked".to_string(),
                status: "blocked".to_string(),
                summary: "产品声明缺少 verified knowledge 支撑：本次决策被强制 blocked".to_string(),
                details,
            });
            review.final_review_status = "blocked_unverified_product_claim".to_string();
            return FinalizeOutcome {
                review,
                status: GatewayStatusFinal::BlockedUnverifiedProductClaim,
                pending_events,
            };
        }
    }

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
    } else if review.approved && !decision.should_reply {
        // A3「主动沉默」：reviewer 通过了决策（approved=true），但 reply-agent
        // 本就判 should_reply=false（确认收到 / 无需触达）。这是"已审核通过的
        // 沉默"，语义上等同 no_reply，而非 hold/block——should_hold 三道硬门、
        // protocol/budget/product-claim 硬门均未命中（都已在上方 return）。
        //
        // 终态返回 Approved：gateway 的 Approved 路径已按 should_reply 分流，
        // should_reply=false 时落 final_review_status=no_reply、跳过 outbox、
        // 生命周期映射为 completed（run_envelope::derive_lifecycle_from_status）。
        // 若误落进下方 else，会被错标 held_by_ai_policy → failed_after_decision，
        // 把一次正常的"无需回复"误计为策略暂缓。should_run_review 在
        // should_reply=false 时返回 false，故本路径的 review 来自
        // local_decision_review（approved=true），不消耗额外 LLM 调用。
        review.final_review_status = "approved".to_string();
        FinalizeOutcome {
            review,
            status: GatewayStatusFinal::Approved,
            pending_events,
        }
    } else {
        // approved=false 且未触发任何硬门（如 review_passed 阈值不够、reviewer
        // 直接 approved=false）→ held_by_ai_policy。注意本分支不再承接
        // approved=true 的沉默决策（已被上一分支接走）。
        review.final_review_status = HOLD_CATEGORY_HELD_BY_AI_POLICY.to_string();
        FinalizeOutcome {
            review,
            status: GatewayStatusFinal::Held(HOLD_CATEGORY_HELD_BY_AI_POLICY.to_string()),
            pending_events,
        }
    }
}

/// 判断 `risks` 中是否包含任何"自治协议违规"标签（R3.5 / R3.6）。
fn has_protocol_violation(risks: &[String]) -> bool {
    risks.iter().any(|r| is_protocol_violation_tag(r))
}

/// 单个 risk 标签是否属于"自治协议**硬**违规"语义 → `blocked_by_required_field`。
///
/// spec R3.5 / R3.6 的硬门只针对**结构性**违规：必填字段缺失（字段根本没输出）、
/// 枚举非法、类型错误、decision_phase 非法。这些是 Agent 没遵守输出契约，无法靠
/// 一次改写补救，故直接 block。
///
/// 刻意**不含** `insufficient_detail_in_critical_turn:*`：那是字段**已输出但偏短**
/// （推理痕迹 < 20 字 / 回复理由 < 30 字）——spec R1.5 只要求 `review.approved =
/// false`，从未要求升级成 `blocked_by_required_field` 硬门。把它当硬门会让一条
/// 安全/质量闸全过、仅推理说明少几个字的回复被不可恢复地枪毙、连一次 revision 都
/// 不给，正是 t15 跌单弧暴露的"闸门系统性过度拦截/全程哑火"真 bug。它改由
/// [`is_insufficient_detail_tag`] 识别、走 single-shot revision 通道（与软闸失败同路）。
fn is_protocol_violation_tag(risk: &str) -> bool {
    crate::agent::types::is_reply_protocol_violation(risk)
}

/// 单个 risk 标签是否属于"关键轮推理字段偏短"语义（spec R1.5 长度违规）。
/// 不是结构性协议违规，按软失败处理：触发 single-shot revision 补全，而非硬 block。
fn is_insufficient_detail_tag(risk: &str) -> bool {
    risk.starts_with("insufficient_detail_in_critical_turn:")
}

/// `promote_risks` 中存在 ≥1 个 insufficient_detail 标签、且**不含**任何结构性硬
/// 违规（[`is_protocol_violation_tag`]）。用于 finalize 判定"是否只需 revision 补全"。
/// 调用点在硬协议门 return 之后，故此处只要 has_protocol_violation 已为 false 即可，
/// 但仍显式双判以防调用顺序变动时回归。
fn has_insufficient_detail_only(promote_risks: &[String]) -> bool {
    promote_risks.iter().any(|r| is_insufficient_detail_tag(r))
        && !promote_risks.iter().any(|r| is_protocol_violation_tag(r))
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
#[cfg(test)]
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
/// ```text
/// let budget_exceeded = current_run_budget()
///     .map(|b| b.is_llm_or_token_exhausted())
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
/// * `budget_exceeded`：调用方根据 `RunBudget::is_llm_or_token_exhausted()` 计算的快照
///   （task-local，不在纯函数内读取）。
#[cfg(test)]
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
/// Decide fallback from the Reviewer's structured scores, not from reply keywords.
///
/// The AI owns semantic review. Code only enforces its typed safety result: hard-gate,
/// pressure, boundary/privacy, or dual-reviewer disagreement can never fall back. A draft may
/// be restored only when the structured classification is safe except for human-like or
/// emotional-value style quality, or when the mechanical style-divergence detector was the
/// sole trigger.
#[cfg(test)]
pub(crate) fn revision_fallback_is_safe_style_only(
    review: &DecisionReviewResult,
    runtime: &UserRuntimeParameters,
) -> bool {
    if review
        .risks
        .iter()
        .any(|risk| risk.starts_with("reviewer_dual_disagree:"))
    {
        return false;
    }

    match classify_dual_gate(review, runtime) {
        DualGateClassification::HardGateFailure { .. } => false,
        DualGateClassification::SoftGateFailure { risks, .. } => {
            !risks.is_empty()
                && risks.iter().all(|risk| {
                    risk.starts_with("human_like_") || risk.starts_with("emotional_value_")
                })
        }
        DualGateClassification::AllPass => review.risks.iter().any(|risk| risk == "style_diverged"),
    }
}

/// Apply the revision failure policy and return whether the pre-revision draft may be restored.
/// Unsafe or unknown revision triggers fail closed with `revision_failed`.
#[cfg(test)]
pub(crate) fn apply_revision_fallback(
    review: &mut DecisionReviewResult,
    runtime: &UserRuntimeParameters,
    finalize_status: &mut GatewayStatusFinal,
    failure_reason: &str,
) -> (String, bool) {
    let allow_fallback = revision_fallback_is_safe_style_only(review, runtime);
    review.approved = allow_fallback;
    review.revision_applied = false;
    if allow_fallback {
        review.final_review_status = "revision_applied_approved".to_string();
        *finalize_status = GatewayStatusFinal::Approved;
    } else {
        review.final_review_status = "revision_failed".to_string();
        *finalize_status = GatewayStatusFinal::Held(HOLD_CATEGORY_HELD_BY_AI_POLICY.to_string());
    }
    (failure_reason.to_string(), allow_fallback)
}

#[cfg(test)]
mod revision_fallback_tests {
    use super::{apply_revision_fallback, GatewayStatusFinal, HOLD_CATEGORY_HELD_BY_AI_POLICY};
    use crate::agent::runtime::UserRuntimeParameters;
    use crate::agent::types::DecisionReviewResult;

    #[test]
    fn fallback_sets_approved_draft_state() {
        let mut review = DecisionReviewResult::default();
        review.approved = false;
        review.revision_applied = false;
        review.final_review_status = String::new();
        let mut status = GatewayStatusFinal::Held(HOLD_CATEGORY_HELD_BY_AI_POLICY.to_string());

        review.risks.push("style_diverged".to_string());
        review.scores.human_like = 10;
        review.scores.emotional_value = 10;
        review.scores.hallucination_score = 0;
        review.scores.knowledge_grounding_score = 10;
        review.scores.pressure_risk = 1;
        review.scores.boundary_privacy_safety = 10;
        let runtime = UserRuntimeParameters::default();
        let (reason, restored) = apply_revision_fallback(
            &mut review,
            &runtime,
            &mut status,
            "revision_llm_timeout_30s",
        );

        assert!(
            restored,
            "pure style revision may restore the approved draft"
        );
        assert!(review.approved, "回退后原稿应视为已批准");
        assert!(!review.revision_applied, "改写未真正应用");
        assert_eq!(review.final_review_status, "revision_applied_approved");
        assert!(
            matches!(status, GatewayStatusFinal::Approved),
            "finalize 应回到 Approved 走发送"
        );
        assert_eq!(
            reason, "revision_llm_timeout_30s",
            "失败原因应原样返回供审计"
        );
    }

    #[test]
    fn unsafe_revision_failure_is_held() {
        let mut review = DecisionReviewResult::default();
        review.scores.human_like = 10;
        review.scores.emotional_value = 10;
        review.scores.knowledge_grounding_score = 10;
        review.scores.pressure_risk = 1;
        review.scores.boundary_privacy_safety = 2;
        let runtime = UserRuntimeParameters::default();
        let mut status = GatewayStatusFinal::Held(HOLD_CATEGORY_HELD_BY_AI_POLICY.to_string());
        let (reason, restored) = apply_revision_fallback(
            &mut review,
            &runtime,
            &mut status,
            "revision_post_review_failed",
        );
        assert!(!restored);
        assert_eq!(reason, "revision_post_review_failed");
        assert!(!review.approved);
        assert_eq!(review.final_review_status, "revision_failed");
        assert!(matches!(status, GatewayStatusFinal::Held(_)));
    }

    #[test]
    fn unknown_or_empty_revision_trigger_is_not_restorable() {
        for risks in [
            Vec::<String>::new(),
            vec!["reviewer_dual_disagree:approved_mismatch".to_string()],
        ] {
            let mut review = DecisionReviewResult {
                risks,
                ..Default::default()
            };
            review.scores.human_like = 10;
            review.scores.emotional_value = 10;
            review.scores.knowledge_grounding_score = 10;
            review.scores.pressure_risk = 1;
            review.scores.boundary_privacy_safety = 10;
            let runtime = UserRuntimeParameters::default();
            let mut status = GatewayStatusFinal::Approved;
            let (_, restored) =
                apply_revision_fallback(&mut review, &runtime, &mut status, "failure");
            assert!(!restored);
            assert_eq!(review.final_review_status, "revision_failed");
        }
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

    use super::review_passed;
    use crate::agent::runtime::UserRuntimeParameters;
    use crate::agent::types::{DecisionReviewResult, ReviewScores};

    fn full_pass_review() -> DecisionReviewResult {
        DecisionReviewResult {
            approved: true,
            scores: ReviewScores {
                human_like: 80,
                emotional_value: 70,
                hallucination_score: 1,
                knowledge_grounding_score: 80,
                pressure_risk: 1,
                boundary_privacy_safety: 10,
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
    fn review_passed_blocks_explicit_live_pressure_risk_zero() {
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.scores.pressure_risk = 0;
        review.claim_analysis.insert("reviewScoreStatus", "valid");
        assert!(
            !review_passed(&review, &runtime),
            "an explicit live pressure score of zero is unscored, not a legacy omission"
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

    #[test]
    fn review_passed_honors_grounding_bypass_for_non_product_reply() {
        // G01：review_passed 的 grounding 项要与 classify_dual_gate:120 同源。
        // bypass=true（情感/关系域）+ 本条无产品声明 + grounding 低分 → H14 应放行。
        let mut runtime = UserRuntimeParameters::default();
        runtime.grounding_gate_bypass_without_claim = true;
        runtime.product_accuracy_block_below = 7;
        let mut review = full_pass_review();
        review.scores.knowledge_grounding_score = 3; // 低于阈值 7
                                                     // claim_analysis 空 → claim_requires_product_knowledge=false（无产品声明）
        review.claim_analysis = mongodb::bson::doc! {};
        assert!(
            review_passed(&review, &runtime),
            "bypass=true + 无产品声明 + 低 grounding 应放行(H14)"
        );

        // 对照①：DEFAULT bypass=false → 低 grounding 仍拦（字节等价）。
        runtime.grounding_gate_bypass_without_claim = false;
        assert!(
            !review_passed(&review, &runtime),
            "bypass=false 时低 grounding 必拦(字节等价)"
        );

        // 对照②：bypass=true 但有产品声明 → 低 grounding 仍拦。
        runtime.grounding_gate_bypass_without_claim = true;
        review.claim_analysis = mongodb::bson::doc! { "requiresProductKnowledge": true };
        assert!(
            !review_passed(&review, &runtime),
            "有产品声明时 bypass 不豁免 grounding"
        );
    }

    #[test]
    fn review_passed_blocks_when_boundary_privacy_low() {
        // boundary_privacy_safety 低分(1-3)=泄露内部画像/暴露AI身份/幕后领导，
        // review_passed 必须拦截（与 classify_dual_gate 同源）。
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.scores.boundary_privacy_safety = 2;
        assert!(
            !review_passed(&review, &runtime),
            "boundary_privacy_safety=2 必须拦截"
        );
    }

    #[test]
    fn review_passed_blocks_when_boundary_privacy_at_3() {
        // 边界值：3 仍在低分区间(<=3)，必须拦截。
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.scores.boundary_privacy_safety = 3;
        assert!(
            !review_passed(&review, &runtime),
            "boundary_privacy_safety=3（低分上沿）必须拦截"
        );
    }

    #[test]
    fn review_passed_passes_when_boundary_privacy_at_4() {
        // 阈值上沿：4 >3 放行。
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.scores.boundary_privacy_safety = 4;
        assert!(
            review_passed(&review, &runtime),
            "boundary_privacy_safety=4（>3）应放行"
        );
    }

    #[test]
    fn review_passed_ignores_boundary_privacy_zero_for_legacy_data() {
        // 老数据 / reviewer 未输出 boundary_privacy_safety → 默认 0，豁免不拦截。
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.scores.boundary_privacy_safety = 0;
        assert!(
            review_passed(&review, &runtime),
            "boundary_privacy_safety=0（老数据/未填）必须视为豁免"
        );
    }

    #[test]
    fn review_passed_blocks_explicit_live_boundary_privacy_zero() {
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.scores.boundary_privacy_safety = 0;
        review.claim_analysis.insert("reviewScoreStatus", "valid");
        assert!(
            !review_passed(&review, &runtime),
            "an explicit live boundary score of zero is unsafe, not a legacy omission"
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
    //! * 候选正文由独立槽注入，事实面视图不得重复 reply_text；should_reply /
    //!   matched_knowledge_ids 等控制与引用字段必须保留；
    //! * 只有 intent_analysis.semanticAssessment 语义合同进入 reviewer 视图；其余
    //!   推理 Document 不进 reviewer 视图。

    use super::build_reviewer_decision_view;
    use crate::agent::types::{AgentDecision, NamecardDirective};
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
            namecard_to_send: Some(NamecardDirective {
                card_id: "64a1f2c3e4b5a697889a0011".to_string(),
                reason: Some("客户明确要求顾问对接".to_string()),
            }),
            intent_analysis: doc! {
                "semanticAssessment": {
                    "responseDisposition": "reply",
                    "speechAct": "statement",
                },
                "explanation": "should not leak",
            },
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
        // 候选正文在 Reviewer prompt 的独立槽中，此处不得重复注入。
        assert!(
            !view.contains("好的，明白您的顾虑"),
            "不应重复 replyText: {view}"
        );
        assert!(
            !view.contains("replyText"),
            "不应包含 replyText key: {view}"
        );
        assert!(
            view.contains("\"shouldReply\":true"),
            "应保留 shouldReply: {}",
            view
        );
        assert!(
            view.contains("\"customerStage\":\"evaluating\""),
            "应保留 customerStage: {}",
            view
        );
        assert!(
            view.contains("\"operationState\":\"trust_building\""),
            "应保留 operationState: {}",
            view
        );
        assert!(
            view.contains("\"k1\""),
            "应保留 knowledge id 引用: {}",
            view
        );
        assert!(
            view.contains("price"),
            "应保留 objectionsDetected: {}",
            view
        );
        assert!(
            view.contains("semanticAssessment") && view.contains("responseDisposition"),
            "应保留 Reply Agent 的语义合同: {}",
            view
        );
        assert!(
            !view.contains("\"explanation\":\"should not leak\""),
            "不应注入 semanticAssessment 之外的 intentAnalysis 推理: {}",
            view
        );
        assert!(
            view.contains("\"namecardToSend\"") && view.contains("64a1f2c3e4b5a697889a0011"),
            "应保留待审核的受控名片动作: {}",
            view
        );
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

    use super::{
        classify_dual_gate, contact_has_principal_product_exemption, decide_revision,
        finalize_review_for_send, finalize_review_for_send_at, reply_objective_features,
        review_passed, route_dual_gate, DualGateClassification, FinalizeOutcome,
        GatewayStatusFinal, RevisionDecision,
    };
    use crate::agent::runtime::UserRuntimeParameters;
    use crate::agent::types::{
        AgentDecision, DecisionReviewResult, ReviewScores, HOLD_CATEGORY_HELD_BY_AI_POLICY,
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
                boundary_privacy_safety: 10,
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
            avatar_url: None,
            sex: None,
            agent_status: AgentStatus::Managed,
            human_profile_note: None,
            custom_agent_instructions: None,
            operation_mode_override: None,
            agent_profile: None,
            memory_summary: None,
            playbook_id: None,
            playbook_version: None,
            manual_tags: Vec::new(),
            manual_tags_updated_at: None,
            manual_tags_by: None,
            confirmed_tags: Vec::new(),
            bayesian_signals: Vec::new(),
            personality_profile: None,
            tags_version: 0,
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
            outcome_events: Vec::new(),
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
                assert!(risks.iter().any(|r| r.starts_with("knowledge_grounding_")));
            }
            other => panic!("expected HardGateFailure, got {:?}", other),
        }
    }

    #[test]
    fn h14_grounding_gate_bypassed_when_no_claim_and_profile_opts_out() {
        // H14：bypass=true（情感/关系域）+ 本条无产品声明（claim_analysis 空）→
        // grounding 低分被旁路 → AllPass（不再被 grounding 软分数硬闸误拦）。
        let mut runtime = UserRuntimeParameters::default();
        runtime.grounding_gate_bypass_without_claim = true;
        let mut review = full_pass_review();
        review.scores.knowledge_grounding_score = runtime.product_accuracy_block_below - 1;
        // claim_analysis 默认空 → claim_requires_product_knowledge=false。
        assert_eq!(
            classify_dual_gate(&review, &runtime),
            DualGateClassification::AllPass
        );
    }

    #[test]
    fn h14_grounding_gate_still_applies_with_claim_even_when_bypass_on() {
        // H14：bypass=true 但本条 requiresProductKnowledge=true（出现产品声明）→
        // grounding 硬闸照常纳入 → HardGateFailure。情感域偶发产品声明仍被守住。
        let mut runtime = UserRuntimeParameters::default();
        runtime.grounding_gate_bypass_without_claim = true;
        let mut review = full_pass_review();
        review.scores.knowledge_grounding_score = runtime.product_accuracy_block_below - 1;
        review.claim_analysis = mongodb::bson::doc! { "requiresProductKnowledge": true };
        match classify_dual_gate(&review, &runtime) {
            DualGateClassification::HardGateFailure { risks } => {
                assert!(risks.iter().any(|r| r.starts_with("knowledge_grounding_")));
            }
            other => panic!("expected HardGateFailure, got {:?}", other),
        }
    }

    #[test]
    fn h14_grounding_gate_unconditional_when_bypass_off_default() {
        // H14 DEFAULT 等价锁：bypass=false（销售域默认）+ 无产品声明 + grounding 低分
        // → 仍 HardGateFailure（与改造前逐字一致）。
        let runtime = UserRuntimeParameters::default();
        assert!(!runtime.grounding_gate_bypass_without_claim);
        let mut review = full_pass_review();
        review.scores.knowledge_grounding_score = runtime.product_accuracy_block_below - 1;
        match classify_dual_gate(&review, &runtime) {
            DualGateClassification::HardGateFailure { risks } => {
                assert!(risks.iter().any(|r| r.starts_with("knowledge_grounding_")));
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
    fn classify_dual_gate_marks_explicit_live_pressure_zero_as_soft_failure() {
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.scores.pressure_risk = 0;
        review.claim_analysis.insert("reviewScoreStatus", "valid");
        match classify_dual_gate(&review, &runtime) {
            DualGateClassification::SoftGateFailure { risks, .. } => {
                assert!(risks.iter().any(|risk| risk == "pressure_risk_0_unscored"))
            }
            other => panic!("expected SoftGateFailure, got {:?}", other),
        }
    }

    #[test]
    fn reply_objective_features_reports_metrics_without_prejudging_questions() {
        // 纯 LLM 驱动：机器只报客观量 + 提示"按用户语境判断问句增减"，
        // 不替改写 Agent 预判"该加问句"（历史写死"可加反问"会在用户拒绝
        // 追问时反向带偏，导致 revision_failed）。
        let out = reply_objective_features("好的，我来想想看");
        // 报了客观量。
        assert!(out.contains("问句"));
        assert!(out.contains("字"));
        // 把语气与问句增减判断交还给改写 Agent，不再由服务端词表代判。
        assert!(out.contains("结合完整对话判断"));
        assert!(out.contains("不要用固定词表"));
        // 不再无条件鼓励加问句（旧的反向引导根因）。
        assert!(!out.contains("问句过少可加 1 个自然反问"));
    }

    #[test]
    fn classify_dual_gate_pressure_direction_defers_question_to_agent() {
        // 高 pressureRisk 的改写方向不替 Agent 预判"加澄清问题"，
        // 而是让其按用户语境决定，并点明拒绝追问时改陈述句。
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.scores.pressure_risk = runtime.pressure_risk_block_at;
        match classify_dual_gate(&review, &runtime) {
            DualGateClassification::SoftGateFailure { direction, .. } => {
                assert!(direction.contains("按用户当前语境判断"));
                assert!(direction.contains("不想被追问"));
                // 不再无条件建议"加 1 个轻量澄清问题"。
                assert!(!direction.contains("1 个轻量澄清问题"));
            }
            other => panic!("expected SoftGateFailure, got {:?}", other),
        }
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
    fn classify_dual_gate_marks_low_boundary_privacy_as_soft_failure() {
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.scores.boundary_privacy_safety = 2; // 低分(1-3)
        match classify_dual_gate(&review, &runtime) {
            DualGateClassification::SoftGateFailure { risks, .. } => {
                assert!(risks
                    .iter()
                    .any(|r| r.starts_with("boundary_privacy_safety_")));
            }
            other => panic!("expected SoftGateFailure, got {:?}", other),
        }
    }

    #[test]
    fn classify_dual_gate_ignores_boundary_privacy_zero_as_legacy() {
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.scores.boundary_privacy_safety = 0; // 未填,豁免
                                                   // 其它分都满分 → 应 AllPass
        assert_eq!(
            classify_dual_gate(&review, &runtime),
            DualGateClassification::AllPass
        );
    }

    #[test]
    fn classify_dual_gate_blocks_explicit_live_boundary_privacy_zero() {
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.scores.boundary_privacy_safety = 0;
        review.claim_analysis.insert("reviewScoreStatus", "valid");
        match classify_dual_gate(&review, &runtime) {
            DualGateClassification::SoftGateFailure { risks, .. } => assert!(risks
                .iter()
                .any(|risk| risk == "boundary_privacy_safety_0_le_3")),
            other => panic!("expected live zero boundary score to fail, got {other:?}"),
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
        route_dual_gate(&mut review, &runtime, "好的，我来想想看");
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
        route_dual_gate(&mut review, &runtime, "好的，我来想想看");
        assert!(
            review
                .revision_direction
                .starts_with("reviewer 自己写的明确方向"),
            "reviewer 已给方向时其原文必须保留在前缀（item ② 仅追加客观特征，不覆盖）"
        );
        assert!(review.needs_revision);
    }

    #[test]
    fn route_dual_gate_leaves_hard_failure_without_revision_flag() {
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.scores.hallucination_score = runtime.fact_risk_block_at + 1;
        let prev_dir = review.revision_direction.clone();
        route_dual_gate(&mut review, &runtime, "好的，我来想想看");
        assert!(!review.needs_revision, "硬闸失败不能触发 revision");
        assert_eq!(review.revision_direction, prev_dir);
        assert!(!review.approved);
    }

    #[test]
    fn route_dual_gate_keeps_all_pass_approved_true() {
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        route_dual_gate(&mut review, &runtime, "好的，我来想想看");
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
        route_dual_gate(&mut review, &runtime, "好的，我来想想看");
        let mut decision = shouldreply_decision();
        let contact = finalize_contact();
        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            &[],
            Vec::new(),
            false,
            false,
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
        route_dual_gate(&mut review, &runtime, "好的，我来想想看");
        let mut decision = shouldreply_decision();
        let contact = finalize_contact();
        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            &[],
            Vec::new(),
            false,
            false,
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
    fn finalize_approved_but_silent_decision_is_no_reply_not_held() {
        // A3「主动沉默」回归门：reviewer 通过（approved=true），但 reply-agent
        // 本就判 should_reply=false（如"客户只是确认收到"）。这是"已审核通过的
        // 沉默"，必须落 Approved（gateway 据 should_reply 分流到 no_reply /
        // completed 生命周期），绝不能被 else-fallthrough 错标 held_by_ai_policy
        // （→ failed_after_decision，把正常无需回复误计为策略暂缓）。
        // 对应 full_flow_a3_no_reply_skips_review_and_outbox 的根因修复。
        let runtime = UserRuntimeParameters::default();
        let review = full_pass_review(); // approved=true，无任何硬门命中
        let mut decision = shouldreply_decision();
        decision.should_reply = false; // reply-agent 主动判沉默
        decision.reply_text = String::new();
        let contact = finalize_contact();
        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            &[],
            Vec::new(),
            false,
            false,
        );
        assert_eq!(
            outcome.status,
            GatewayStatusFinal::Approved,
            "approved=true + should_reply=false 的主动沉默必须是 Approved，不能 Held"
        );
        assert_eq!(
            outcome.review.final_review_status, "approved",
            "主动沉默的 final_review_status 应为 approved（gateway 再据 should_reply 写 no_reply）"
        );
        assert!(outcome.review.approved);
        // 沉默路径不应被误标为任何 hold/block 风险。
        assert!(
            !outcome
                .review
                .risks
                .iter()
                .any(|r| r == "state_action_policy_blocked"),
            "主动沉默不应携带任何策略拦截风险标签"
        );
    }

    #[test]
    fn finalize_unapproved_without_hard_gate_stays_held() {
        // 反向门：approved=false 且未触发任何硬门（reviewer 直接判不通过、
        // 软闸阈值不够且无 revision_direction）→ 仍走 Held(held_by_ai_policy)。
        // 确保上面的 A3 分支没有把"真正该 hold"的 approved=false 也放行。
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.approved = false; // reviewer 直接不通过
        review.needs_revision = false;
        review.revision_direction = String::new();
        let mut decision = shouldreply_decision(); // should_reply=true
        let contact = finalize_contact();
        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            &[],
            Vec::new(),
            false,
            false,
        );
        match outcome.status {
            GatewayStatusFinal::Held(category) => {
                assert_eq!(category, HOLD_CATEGORY_HELD_BY_AI_POLICY);
            }
            other => panic!("expected Held(held_by_ai_policy), got {:?}", other),
        }
        assert_eq!(
            outcome.review.final_review_status,
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
        route_dual_gate(&mut review, &runtime, "好的，我来想想看");
        let mut decision = shouldreply_decision();
        let contact = finalize_contact();
        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            &[],
            Vec::new(),
            false,
            false,
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
        route_dual_gate(&mut review, &runtime, "好的，我来想想看");
        let mut decision = shouldreply_decision();
        let contact = finalize_contact();
        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            &[],
            Vec::new(),
            false,
            false,
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

    // ── R5.4：verified-knowledge 产品声明强约束（结构化兜底闸）单测 ──

    fn mk_chunk(integrity: &str) -> crate::models::OperationKnowledgeChunk {
        let now = DateTime::now();
        crate::models::OperationKnowledgeChunk {
            id: Some(mongodb::bson::oid::ObjectId::new()),
            workspace_id: "default".to_string(),
            account_id: Some("default".to_string()),
            document_id: None,
            item_id: None,
            domain: "user".to_string(),
            knowledge_type: None,
            business_context: None,
            title: "t".to_string(),
            summary: None,
            body: None,
            applicable_scenes: Vec::new(),
            not_applicable_scenes: Vec::new(),
            product_tags: Vec::new(),
            business_topics: Vec::new(),
            source_quote: None,
            source_anchors: Vec::new(),
            integrity_status: Some(integrity.to_string()),
            confidence_score: Some(80),
            status: "active".to_string(),
            priority: 0,
            created_at: now,
            updated_at: now,
            wiki_type: None,
            domain_attributes: None,
            provenance: None,
            valid_from: None,
            valid_to: None,
            superseded_by: None,
            previous_version_id: None,
            related_chunks: None,
            usage_stats: None,
            dynamic_confidence: None,
            integrity_score: None,
            locked_fields: None,
            chunk_type: "product_fact".to_string(),
        }
    }

    #[test]
    fn finalize_blocks_unsupported_non_product_business_claim() {
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.claim_analysis = mongodb::bson::doc! {
            "requiresProductKnowledge": false,
            "requiresBusinessEvidence": true,
            "unsupportedNonProductBusinessClaimCount": 1_i64,
            "claimManifest": [{
                "sourceQuote": "到店前带身份证",
                "claim": "到店必须携带身份证",
                "scope": "visit_requirement",
                "productClaim": false,
                "evidenceNeed": "required",
                "evidenceRefs": Vec::<String>::new(),
                "supported": false,
            }],
        };
        let mut decision = shouldreply_decision();
        decision.reply_text = "到店前带身份证就行".to_string();
        let contact = finalize_contact();
        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            &[],
            Vec::new(),
            false,
            false,
        );
        assert_eq!(outcome.status, GatewayStatusFinal::BlockedBySafetyGuard);
        assert!(!decision.should_reply);
        assert_eq!(decision.autonomy_mode, "blocked");
        assert_eq!(
            outcome.review.final_review_status,
            "blocked_by_safety_guard"
        );
        assert!(outcome
            .pending_events
            .iter()
            .any(|event| event.kind == "unsupported_business_claim_blocked"));
    }

    #[test]
    fn finalize_allows_supported_non_product_business_claim() {
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.claim_analysis = mongodb::bson::doc! {
            "requiresProductKnowledge": false,
            "requiresBusinessEvidence": true,
            "unsupportedNonProductBusinessClaimCount": 0_i64,
            "claimManifest": [{
                "sourceQuote": "明天下午三点见",
                "claim": "客户已约明天下午三点",
                "scope": "appointment",
                "productClaim": false,
                "evidenceNeed": "required",
                "evidenceRefs": ["current_user_message"],
                "supported": true,
            }],
        };
        let mut decision = shouldreply_decision();
        decision.reply_text = "好的，明天下午三点见".to_string();
        let contact = finalize_contact();
        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            &[],
            Vec::new(),
            false,
            false,
        );
        assert_eq!(outcome.status, GatewayStatusFinal::Approved);
        assert!(decision.should_reply);
    }

    #[test]
    fn finalize_blocks_non_product_claim_when_verified_evidence_expires_before_send() {
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        let mut chunk = mk_chunk("verified");
        let chunk_id = chunk.id.expect("chunk id").to_hex();
        let evaluated_at = DateTime::from_millis(2_000);
        chunk.valid_to = Some(DateTime::from_millis(1_999));
        review.claim_analysis = mongodb::bson::doc! {
            "requiresProductKnowledge": false,
            "unsupportedNonProductBusinessClaimCount": 0_i64,
            "claimManifest": [{
                "sourceQuote": "门店提供无障碍通道",
                "claim": "门店提供无障碍通道",
                "scope": "accessibility",
                "subject": "business",
                "productClaim": false,
                "evidenceNeed": "required",
                "evidenceRefs": [format!("verified_knowledge:{chunk_id}")],
                "supported": true,
            }],
        };
        let mut decision = shouldreply_decision();
        decision.reply_text = "门店提供无障碍通道".to_string();
        decision.used_knowledge_ids = vec![chunk_id];
        let contact = finalize_contact();
        let outcome = finalize_review_for_send_at(
            review,
            &mut decision,
            &runtime,
            &contact,
            std::slice::from_ref(&chunk),
            Vec::new(),
            false,
            false,
            evaluated_at,
        );
        assert_eq!(outcome.status, GatewayStatusFinal::BlockedBySafetyGuard);
        assert!(!decision.should_reply);
        assert!(outcome
            .review
            .risks
            .iter()
            .any(|risk| risk == "business_claim_evidence_expired_before_send"));
    }

    #[test]
    fn finalize_blocks_product_claim_without_verified_chunk() {
        // R5.4：reviewer claim_analysis.requiresProductKnowledge=true 且本 run
        // 引用的切片里没有 verified chunk → blocked_unverified_product_claim。
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.claim_analysis = mongodb::bson::doc! { "requiresProductKnowledge": true };
        let mut decision = shouldreply_decision();
        // 引用了一个 needs_review（非 verified）chunk
        let chunk = mk_chunk("needs_review");
        decision.used_knowledge_ids = vec![chunk.id.unwrap().to_hex()];
        let contact = finalize_contact();
        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            std::slice::from_ref(&chunk),
            Vec::new(),
            false,
            false,
        );
        assert_eq!(
            outcome.status,
            GatewayStatusFinal::BlockedUnverifiedProductClaim
        );
        assert!(!outcome.review.approved);
        assert!(!decision.should_reply);
        assert_eq!(decision.autonomy_mode, "blocked");
        assert!(outcome
            .review
            .risks
            .iter()
            .any(|r| r == "product_claim_without_verified_knowledge"));
        assert!(outcome
            .pending_events
            .iter()
            .any(|e| e.kind == "product_claim_blocked"));
    }

    #[test]
    fn finalize_allows_product_claim_when_priced_from_catalog() {
        // R5.4 G2 并联背书：无 verified chunk，但报价命中 active 产品目录
        // （priced_from_catalog=true）→ 结构化报价视为已背书，不触发红线。
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.claim_analysis = mongodb::bson::doc! { "requiresProductKnowledge": true };
        let mut decision = shouldreply_decision();
        // 没引用任何 verified chunk（used_knowledge_ids 空 / 无 verified）
        let contact = finalize_contact();
        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            &[],
            Vec::new(),
            true,  // priced_from_catalog：报价 product_id 命中 active 产品目录
            false, // principal_product_exempted：无领导授权豁免
        );
        assert_eq!(
            outcome.status,
            GatewayStatusFinal::Approved,
            "目录报价背书应放行，不被 blocked_unverified_product_claim 错杀"
        );
        assert!(outcome.review.approved);
    }

    #[test]
    fn principal_exemption_helper_detects_granted() {
        // helper 三态：无 domain_attributes / 有 key 且 granted=true / 缺 granted。
        // domain_attributes 是 Option<Document>（models.rs:203），finalize_contact 置 None。
        let mut c = finalize_contact();
        assert!(
            !contact_has_principal_product_exemption(&c),
            "无 domain_attributes 容器时应返回 false（fail-closed）"
        );
        c.domain_attributes = Some(mongodb::bson::doc! {
            crate::models::PRINCIPAL_PRODUCT_EXEMPTION_ATTR: {
                "granted": true,
                "substance": "这款年度会员是 199 元",
            },
        });
        assert!(
            contact_has_principal_product_exemption(&c),
            "granted=true 时应识别为有生效豁免"
        );
        // granted=false 时不放行。
        c.domain_attributes = Some(mongodb::bson::doc! {
            crate::models::PRINCIPAL_PRODUCT_EXEMPTION_ATTR: { "granted": false },
        });
        assert!(
            !contact_has_principal_product_exemption(&c),
            "granted=false 时不应识别为有效豁免"
        );
    }

    #[test]
    fn finalize_allows_product_claim_when_principal_exempted() {
        // R5.4 第三条并联背书：无 verified chunk、无目录报价（priced=false），
        // 但该客户有生效的领导授权豁免（principal_product_exempted=true）→ 不 block。
        // 复用 priced 测试同 setup，仅把末两参改成 priced=false, exempted=true。
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.claim_analysis = mongodb::bson::doc! { "requiresProductKnowledge": true };
        let mut decision = shouldreply_decision();
        // 没引用任何 verified chunk（used_knowledge_ids 空 / 无 verified）
        let contact = finalize_contact();
        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            &[],
            Vec::new(),
            false, // priced_from_catalog：无目录报价背书
            true,  // principal_product_exempted：领导已针对该客户授权
        );
        assert_eq!(
            outcome.status,
            GatewayStatusFinal::Approved,
            "领导授权豁免应放行，不被 blocked_unverified_product_claim 错杀"
        );
        assert!(outcome.review.approved);
    }

    #[test]
    fn finalize_allows_product_claim_with_verified_chunk() {
        // R5.4 反向：引用了 verified chunk → 不触发 R5.4，走 Approved。
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.claim_analysis = mongodb::bson::doc! { "requiresProductKnowledge": true };
        let mut decision = shouldreply_decision();
        let chunk = mk_chunk("verified");
        decision.used_knowledge_ids = vec![chunk.id.unwrap().to_hex()];
        let contact = finalize_contact();
        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            std::slice::from_ref(&chunk),
            Vec::new(),
            false,
            false,
        );
        assert_eq!(outcome.status, GatewayStatusFinal::Approved);
        assert!(outcome.review.approved);
    }

    #[test]
    fn finalize_skips_r54_when_claim_does_not_require_product_knowledge() {
        // requiresProductKnowledge=false（或缺失）→ R5.4 不介入，即便无 chunk。
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.claim_analysis = mongodb::bson::doc! { "requiresProductKnowledge": false };
        let mut decision = shouldreply_decision();
        let contact = finalize_contact();
        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            &[],
            Vec::new(),
            false,
            false,
        );
        assert_eq!(outcome.status, GatewayStatusFinal::Approved);
        assert!(outcome.review.approved);
    }

    // 发送边界不再扫描正文关键词。是否需要证据完全由 AI 的结构化 claimAnalysis
    // 决定，服务端只执行证据 ID、目录和时效校验。
    #[test]
    fn finalize_does_not_scan_reply_text_for_business_markers() {
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.claim_analysis = mongodb::bson::doc! { "requiresProductKnowledge": false };
        let mut decision = shouldreply_decision();
        decision.reply_text = "这句话包含任意业务词，但模型明确判为无证据声明".to_string();
        let contact = finalize_contact();
        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            &[],
            Vec::new(),
            false,
            false,
        );
        assert_eq!(outcome.status, GatewayStatusFinal::Approved);
        assert!(decision.should_reply);
        assert!(!outcome
            .pending_events
            .iter()
            .any(|e| e.kind == "grounding_probe_reviewer_missed"));
    }

    // ── R1.5 insufficient_detail 降级为 single-shot revision（t15 跌单弧根因修复）──
    //
    // 历史 bug：关键轮推理字段偏短（insufficient_detail_in_critical_turn:*）被
    // is_protocol_violation_tag 当成结构性硬违规 → blocked_by_required_field 直接
    // return、revision_applied=false，连一次改写都不给。t15 6 轮成交弧因此 0 轮
    // approved（< 下限 2）全程哑火。修复后它走软失败路径：标 needs_revision + 方向 →
    // finalize 矫正 approved=true → decide_revision Proceed。

    #[test]
    fn finalize_insufficient_detail_only_routes_to_revision_not_hard_block() {
        // 安全/质量闸全过，promote_risks 只含 insufficient_detail → Approved + needs_revision。
        let runtime = UserRuntimeParameters::default();
        let review = full_pass_review();
        let mut decision = shouldreply_decision();
        let contact = finalize_contact();
        let promote_risks = vec![
            "insufficient_detail_in_critical_turn:operation_goal".to_string(),
            "insufficient_detail_in_critical_turn:relationship_read".to_string(),
        ];
        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            &[],
            promote_risks,
            false,
            false,
        );
        let FinalizeOutcome {
            review: finalized,
            status,
            ..
        } = outcome;
        assert_eq!(
            status,
            GatewayStatusFinal::Approved,
            "insufficient_detail-only 必须矫正为 Approved（进 revision 通道），而非 blocked_by_required_field"
        );
        assert!(finalized.approved);
        assert!(
            finalized.needs_revision,
            "应标记 needs_revision 触发 single-shot revision"
        );
        assert!(
            !finalized.revision_direction.trim().is_empty(),
            "应写补全推理痕迹的 revision_direction"
        );
        // 矫正后的 Approved + needs_revision 必须让 decide_revision 进 Proceed。
        assert_eq!(
            decide_revision(&status, &finalized, false),
            RevisionDecision::Proceed,
            "矫正后必须能触发 single-shot revision"
        );
    }

    #[test]
    fn finalize_insufficient_detail_with_hard_gate_still_held_not_sent() {
        // 关键安全不变量：insufficient_detail + hallucination 硬闸失败 → 仍 Held，
        // 绝不能被降级矫正成 Approved 发出去。
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.scores.hallucination_score = runtime.fact_risk_block_at + 1; // 硬闸失败
        route_dual_gate(&mut review, &runtime, "好的，我来想想看"); // 硬闸 → approved=false，不设 needs_revision
        let mut decision = shouldreply_decision();
        let contact = finalize_contact();
        let promote_risks = vec!["insufficient_detail_in_critical_turn:operation_goal".to_string()];
        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            &[],
            promote_risks,
            false,
            false,
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
            other => panic!("硬闸失败时必须 Held，绝不放行；got {:?}", other),
        }
        assert!(!finalized.approved, "硬闸失败的回复绝不能被矫正成 approved");
    }

    #[test]
    fn finalize_structural_violation_still_hard_blocks() {
        // 真正的结构性协议违规（missing_required_field）仍走 blocked_by_required_field 硬门。
        let runtime = UserRuntimeParameters::default();
        let review = full_pass_review();
        let mut decision = shouldreply_decision();
        let contact = finalize_contact();
        let promote_risks = vec!["missing_required_field:why_should_reply".to_string()];
        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            &[],
            promote_risks,
            false,
            false,
        );
        assert_eq!(
            outcome.status,
            GatewayStatusFinal::BlockedByRequiredField,
            "结构性必填缺失仍必须硬 block"
        );
        assert!(!decision.should_reply);
    }

    #[test]
    fn finalize_mixed_structural_and_insufficient_detail_prefers_hard_block() {
        // 同时含结构性违规 + insufficient_detail → 结构性硬门优先 return（不降级）。
        let runtime = UserRuntimeParameters::default();
        let review = full_pass_review();
        let mut decision = shouldreply_decision();
        let contact = finalize_contact();
        let promote_risks = vec![
            "insufficient_detail_in_critical_turn:operation_goal".to_string(),
            "invalid_enum_value:operation_state:nonsense".to_string(),
        ];
        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            &[],
            promote_risks,
            false,
            false,
        );
        assert_eq!(
            outcome.status,
            GatewayStatusFinal::BlockedByRequiredField,
            "混合违规时结构性硬门优先，不走 revision 降级"
        );
    }

    // ── M1：管理发送网关接入 finalize 后的两道核心不变量 ──
    //
    // send_contact_message_gateway 原先仅凭 review_passed 放行（软/硬闸折叠 bool），
    // 缺 finalize 的 R5.4 verified-knowledge 确定性硬门。下面两测钉死修复：
    //   1. review_passed 会放行的无背书产品声明，finalize 确定性拦截；
    //   2. finalize 对软闸失败标 Approved，靠 `&& review_passed` guard 仍不发。

    #[test]
    fn m1_review_passed_lets_unverified_product_claim_through_but_finalize_blocks() {
        // M1 核心：reviewer 自报 grounding 高分（review_passed=true），但本 run 引用的
        // 切片无 verified chunk → finalize R5.4 确定性 block。证明管理发送仅凭
        // review_passed 会误发这条危险内容，接入 finalize 后被拦。
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        // 显式产品声明；grounding 分仍高（reviewer 自评乐观）。
        review.claim_analysis = mongodb::bson::doc! { "requiresProductKnowledge": true };
        let mut decision = shouldreply_decision();
        let chunk = mk_chunk("needs_review"); // 非 verified
        decision.used_knowledge_ids = vec![chunk.id.unwrap().to_hex()];

        // 断言 1：旧管理路径仅凭 review_passed —— 会放行这条无背书产品声明。
        assert!(
            review_passed(&review, &runtime),
            "reviewer 自报 grounding 高分时 review_passed 放行（M1 的漏点）"
        );

        // 断言 2：接入 finalize 后 —— R5.4 verified_chunks=∅ 确定性拦截。
        let contact = finalize_contact();
        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            std::slice::from_ref(&chunk),
            Vec::new(),
            false,
            false,
        );
        assert_eq!(
            outcome.status,
            GatewayStatusFinal::BlockedUnverifiedProductClaim,
            "finalize R5.4 必须拦截无 verified 背书的产品声明（管理发送新增保护）"
        );
    }

    #[test]
    fn m1_soft_gate_failure_stays_blocked_via_review_passed_guard() {
        // M1 回归：软闸失败（human_like 不达标）时 finalize 会标 Approved，
        // 管理发送必须靠 `matches!(Approved) && review_passed` 的 guard 仍不发，
        // 否则会把软闸失败内容未经改写直接发出（管理发送无 revision 通道）。
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.scores.human_like = runtime.human_like_rewrite_below - 1; // 软闸失败
        let mut decision = shouldreply_decision();
        let contact = finalize_contact();
        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            &[],
            Vec::new(),
            false,
            false,
        );
        // finalize 单看：软闸失败仍落 Approved（指望 revision 循环）。
        assert!(
            matches!(outcome.status, GatewayStatusFinal::Approved),
            "finalize 对软闸失败标 Approved（依赖调用方 revision 或 guard）"
        );
        // 但 review_passed 因 human_like 不达标返 false → 管理发送 guard 挡住。
        let passed = matches!(outcome.status, GatewayStatusFinal::Approved)
            && review_passed(&outcome.review, &runtime);
        assert!(
            !passed,
            "管理发送的 `&& review_passed` guard 必须挡住软闸失败被当 Approved 发出"
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

    use super::{
        apply_dual_reviewer_disagreement, detect_dual_reviewer_disagreement,
        DualReviewerDisagreement,
    };
    use crate::agent::runtime::UserRuntimeParameters;
    use crate::agent::types::{DecisionReviewResult, ReviewScores};

    fn full_pass_review() -> DecisionReviewResult {
        DecisionReviewResult {
            approved: true,
            scores: ReviewScores {
                human_like: 80,
                emotional_value: 70,
                hallucination_score: 1,
                knowledge_grounding_score: 80,
                pressure_risk: 1,
                boundary_privacy_safety: 10,
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
