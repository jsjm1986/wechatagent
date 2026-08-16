//! 渐进式三档 + 充分性自评（2026-06-23）
//!
//! Reply Agent 在每轮决策后自评：本轮信息是否充足？如果不够，需要提升到哪一档
//! prompt？需要澄清什么？
//!
//! 本模块提供纯函数档位判定逻辑，供 gateway 调用。

use super::types::AgentDecision;

/// 渐进式三档 prompt 枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptTier {
    /// 精简档（Tier 1）：仅关系上下文 + 画像
    Lean,
    /// 关系档（Tier 2）：Tier 1 + 关系历史记忆
    Relational,
    /// 完整档（Tier 3）：Tier 2 + 知识库 + 完整 SOP
    Full,
}

/// 档位判定结果：Agent 自评后，gateway 应如何响应？
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TierDecision {
    /// 信息够了，直接进五闸评审
    Enough,
    /// 需升档重生成
    Escalate(PromptTier),
    /// 信息不足需澄清
    Clarify,
}

/// 纯函数：根据充分性自评决定下一步动作。
pub fn decide_tier_escalation(decision: &AgentDecision) -> TierDecision {
    match decision.sufficiency.as_str() {
        "enough" => TierDecision::Enough,
        "need_more_context" => {
            let tier = match decision.missing_tier.as_str() {
                "relational" => PromptTier::Relational,
                "full" => PromptTier::Full,
                // 非法值回落 Full（更保守，宁可多注入，避免复合高价值轮被卡在无知识档）。
                _ => PromptTier::Full,
            };
            TierDecision::Escalate(tier)
        }
        "need_clarification" => TierDecision::Clarify,
        _ => TierDecision::Enough,
    }
}

/// Why a Lean result must be regenerated with the Full business context.
///
/// This chooses only the context tier, never the business action. In particular, a referral reason
/// does not select a card and a routed chunk is authoritative here only when it came from the
/// Knowledge Agent citation path rather than fallback navigation.
pub(crate) fn forced_full_context_reason(
    decision: &AgentDecision,
    has_cited_knowledge_context: bool,
    has_explicit_referral_context: bool,
    has_structured_knowledge_directive: bool,
) -> Option<&'static str> {
    if decision.sufficiency != "enough" {
        return None;
    }
    if super::guards::decision_requires_knowledge(decision) {
        return Some("lean_declared_knowledge_required");
    }
    if has_cited_knowledge_context {
        return Some("knowledge_route_cited_context");
    }
    if has_structured_knowledge_directive {
        return Some("knowledge_route_structured_directive");
    }
    if has_explicit_referral_context {
        return Some("explicit_referral_context_requested");
    }
    None
}

/// 纯谓词：sufficiency 是否落在已知三态（enough / need_more_context / need_clarification）内。
/// false = LLM 输出畸形（空/乱值），decide_tier_escalation 会走 `_=>Enough` 兜底 = 静默降级，
/// 应被观测（块 B 的 ptier_self_assessment_malformed）。
pub(crate) fn is_sufficiency_recognized(decision: &AgentDecision) -> bool {
    matches!(
        decision.sufficiency.as_str(),
        "enough" | "need_more_context" | "need_clarification"
    )
}

/// 纯谓词：本轮是否构成「需观测的自评乐观灰区」——自评说够了（enough）、本轮需产品知识、
/// 但知识覆盖只是 `weak`（有弱证据、未硬到 missing）。Full 上下文是否加载由
/// [`forced_full_context_reason`] 统一裁决；本谓词只保留 weak 灰区观测。
///
/// 命中只记观测 telemetry（先观测后判罚），不改档位决策。正向 `== "weak"`，绝不用 `!=`。
pub(crate) fn is_coverage_optimism(decision: &AgentDecision, knowledge_coverage: &str) -> bool {
    decision.sufficiency.as_str() == "enough"
        && knowledge_coverage == "weak"
        && super::guards::decision_requires_knowledge(decision)
}

/// 纯谓词:本决策是否应记录 `used_knowledge_ids`(知识路由命中的切片 id)。
///
/// 仅当经 **Full** 知识档(`forced_full` 强升 Full / `escalated_to_full` 升档到 Full)时才记——
/// 只有 Full 档 `include_business=true` 真注入并读了切片(decision.rs:318)。Lean-Enough /
/// Clarify(Lean) / Escalate(Relational) 都 `include_business=false`,没读切片;若记路由 id,
/// grounding 硬闸 `compute_verified_chunks` 取 `used ∩ verified` 非空即放行,会架空
/// `blocked_unverified_product_claim` 红线(gates.rs:660)。正向条件,绝不改成无条件赋值。
#[cfg(test)]
pub(crate) fn should_record_used_knowledge_ids(forced_full: bool, escalated_to_full: bool) -> bool {
    forced_full || escalated_to_full
}

/// KB-01：本决策最终应记录的 used_knowledge_ids。
/// Full 档(读了切片)记路由命中 id;非 Full 档(没读切片)一律清空——含 LLM 经
/// carry_through 透传的自报值,不给 grounding 硬闸 `compute_verified_chunks` 留架空口。
#[cfg(test)]
pub(crate) fn resolve_used_knowledge_ids(
    forced_full: bool,
    escalated_to_full: bool,
    route_ids: Vec<String>,
) -> Vec<String> {
    if should_record_used_knowledge_ids(forced_full, escalated_to_full) {
        route_ids
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_decision(sufficiency: &str, missing_tier: &str) -> AgentDecision {
        AgentDecision {
            sufficiency: sufficiency.to_string(),
            missing_tier: missing_tier.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn test_enough_passes_through() {
        let d = make_decision("enough", "");
        assert_eq!(decide_tier_escalation(&d), TierDecision::Enough);
    }

    #[test]
    fn test_need_more_context_escalates_to_relational() {
        let d = make_decision("need_more_context", "relational");
        assert_eq!(
            decide_tier_escalation(&d),
            TierDecision::Escalate(PromptTier::Relational)
        );
    }

    #[test]
    fn test_need_more_context_escalates_to_full() {
        let d = make_decision("need_more_context", "full");
        assert_eq!(
            decide_tier_escalation(&d),
            TierDecision::Escalate(PromptTier::Full)
        );
    }

    #[test]
    fn test_need_clarification_triggers_clarify() {
        let d = make_decision("need_clarification", "");
        assert_eq!(decide_tier_escalation(&d), TierDecision::Clarify);
    }

    #[test]
    fn test_unknown_sufficiency_defaults_to_enough() {
        let d = make_decision("unknown", "");
        assert_eq!(decide_tier_escalation(&d), TierDecision::Enough);
    }

    #[test]
    fn test_empty_sufficiency_defaults_to_enough() {
        let d = make_decision("", "");
        assert_eq!(decide_tier_escalation(&d), TierDecision::Enough);
    }

    #[test]
    fn test_need_more_context_invalid_tier_falls_back_to_full() {
        let d = make_decision("need_more_context", "garbage");
        assert_eq!(
            decide_tier_escalation(&d),
            TierDecision::Escalate(PromptTier::Full)
        );
    }

    fn decision_with_need(sufficiency: &str, knowledge_need: &str) -> AgentDecision {
        AgentDecision {
            sufficiency: sufficiency.to_string(),
            knowledge_need: knowledge_need.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn coverage_optimism_only_weak_not_missing() {
        // 收窄后：missing 归强升、不再算观测乐观；weak 才算。
        let d = decision_with_need("enough", "required");
        assert!(!is_coverage_optimism(&d, "missing"));
        assert!(is_coverage_optimism(&d, "weak"));
    }

    #[test]
    fn coverage_optimism_hits_on_weak_too() {
        // missing+weak 中只有 weak 纳入观测——weak 态算乐观偏差。
        let d = decision_with_need("enough", "insufficient");
        assert!(is_coverage_optimism(&d, "weak"));
        let d2 = decision_with_need("enough", "knowledge_required");
        assert!(is_coverage_optimism(&d2, "weak"));
    }

    #[test]
    fn coverage_optimism_skips_when_coverage_adequate() {
        // enough/not_required 覆盖度不算偏差。
        let d = decision_with_need("enough", "required");
        assert!(!is_coverage_optimism(&d, "enough"));
        assert!(!is_coverage_optimism(&d, "not_required"));
    }

    #[test]
    fn coverage_optimism_skips_when_knowledge_not_needed() {
        // 本轮不需要产品知识（寒暄轮 knowledge_need=not_required）→ 即便 coverage=missing 也不记。
        let d = decision_with_need("enough", "not_required");
        assert!(!is_coverage_optimism(&d, "missing"));
    }

    #[test]
    fn coverage_optimism_requires_positive_enough_match_not_negation() {
        // 防御对抗警告的最大陷阱：绝不能用 !=enough 否定匹配。
        // _=>Enough 兜底的 unknown/空 sufficiency 虽然档位走 Enough，但不是「自评说够了」，
        // 不能记成乐观偏差。这里用正向 == "enough"，故 unknown/空 不命中。
        let unknown = decision_with_need("unknown", "required");
        assert!(!is_coverage_optimism(&unknown, "missing"));
        let empty = decision_with_need("", "required");
        assert!(!is_coverage_optimism(&empty, "missing"));
        // need_more_context / need_clarification 也不是「自评够了」，不记。
        let more = decision_with_need("need_more_context", "required");
        assert!(!is_coverage_optimism(&more, "missing"));
    }

    #[test]
    fn sufficiency_recognized_three_states_only() {
        assert!(is_sufficiency_recognized(&make_decision("enough", "")));
        assert!(is_sufficiency_recognized(&make_decision(
            "need_more_context",
            ""
        )));
        assert!(is_sufficiency_recognized(&make_decision(
            "need_clarification",
            ""
        )));
        assert!(!is_sufficiency_recognized(&make_decision("", "")));
        assert!(!is_sufficiency_recognized(&make_decision("garbage", "")));
    }

    #[test]
    fn full_context_reason_uses_independent_business_signals_without_forcing_actions() {
        let required = decision_with_need("enough", "required");
        assert_eq!(
            forced_full_context_reason(&required, false, false, false),
            Some("lean_declared_knowledge_required")
        );
        let routine = decision_with_need("enough", "not_required");
        assert_eq!(
            forced_full_context_reason(&routine, true, false, false),
            Some("knowledge_route_cited_context")
        );
        assert_eq!(
            forced_full_context_reason(&routine, false, true, false),
            Some("explicit_referral_context_requested")
        );
        assert_eq!(
            forced_full_context_reason(&routine, false, false, true),
            Some("knowledge_route_structured_directive")
        );
        assert_eq!(
            forced_full_context_reason(&routine, false, false, false),
            None
        );
        let already_escalating = decision_with_need("need_more_context", "required");
        assert_eq!(
            forced_full_context_reason(&already_escalating, true, true, true),
            None,
            "the ordinary escalation branch owns non-enough decisions"
        );
    }

    #[test]
    fn used_knowledge_ids_recorded_only_for_full_tier() {
        // 命门:Lean-Enough / Clarify(Lean) / Escalate(Relational) 都没读切片
        // (forced_full=false, escalated_to_full=false),绝不能记路由 id——否则
        // grounding 硬闸把"没读过的切片"误当读过,架空 blocked_unverified_product_claim。
        assert!(
            !should_record_used_knowledge_ids(false, false),
            "非 Full 档不读切片,记路由 id 会架空 grounding 硬闸(红线)"
        );
        // forced_full 强升 Full(自评 enough 但 coverage=missing 且需知识)→ 记。
        assert!(should_record_used_knowledge_ids(true, false));
        // escalated_to_full 升档到 Full → 记。
        assert!(should_record_used_knowledge_ids(false, true));
        // 两者同真(理论不同时发生,防御)→ 记。
        assert!(should_record_used_knowledge_ids(true, true));
    }

    // ── KB-01：非 Full 档清空 used_knowledge_ids，堵 LLM 自报架空硬闸 ──
    #[test]
    fn kb01_lean_tier_clears_self_reported_ids() {
        // 非 Full 档(false,false)：即便传入(经 carry_through 透传的)自报 id，也一律清空。
        let ids = resolve_used_knowledge_ids(false, false, vec!["a".into(), "b".into()]);
        assert!(
            ids.is_empty(),
            "非 Full 档必须清空 used_knowledge_ids(含 LLM 自报)"
        );
    }

    #[test]
    fn kb01_full_tier_keeps_route_ids() {
        // Full 档(forced 或 escalated)：读了切片，保留路由命中 id，不误伤合法背书。
        assert_eq!(
            resolve_used_knowledge_ids(true, false, vec!["id1".into()]),
            vec!["id1".to_string()]
        );
        assert_eq!(
            resolve_used_knowledge_ids(false, true, vec!["id1".into()]),
            vec!["id1".to_string()]
        );
    }

    #[test]
    fn kb01_lean_self_reported_verified_id_cannot_forge_grounding_gate() {
        // 端到端不变量：非 Full 档 + 自报一个真实 verified 语料 id →
        // resolve 清空 → compute_verified_chunks 取 used∩verified 为空 → 硬闸不被架空。
        use crate::models::OperationKnowledgeChunk;
        use mongodb::bson::{oid::ObjectId, DateTime};
        let oid = ObjectId::new();
        let mut chunk = OperationKnowledgeChunk::default();
        chunk.id = Some(oid);
        chunk.integrity_status = Some("verified".into());
        chunk.valid_to = None;
        let self_reported = vec![oid.to_hex()];
        let resolved = resolve_used_knowledge_ids(false, false, self_reported);
        assert!(resolved.is_empty(), "非 Full 档清空自报 id");
        let chunks = [chunk];
        let verified =
            crate::agent::guards::compute_verified_chunks(&resolved, &chunks, DateTime::now());
        assert!(
            verified.is_empty(),
            "非 Full 档不得有 verified 背书——否则架空 blocked_unverified_product_claim"
        );
    }
}
