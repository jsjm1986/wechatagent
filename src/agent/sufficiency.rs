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

/// 纯谓词：本轮是否构成「确定高危、必须当场升 Full」——自评说够了（enough），但本轮确实
/// 需要产品知识（decision_requires_knowledge）、且知识路由覆盖度为 `missing`（连弱证据都没有）。
///
/// 与 [`is_coverage_optimism`] 正交：missing → 强升（本谓词，硬动作）；weak → 观测（那个谓词，
/// 先观测后判罚）。两者各管一态，互不重叠。必须正向 `== "missing"`，绝不用 `!=`。
pub(crate) fn should_force_full_on_missing(
    decision: &AgentDecision,
    knowledge_coverage: &str,
) -> bool {
    decision.sufficiency.as_str() == "enough"
        && knowledge_coverage == "missing"
        && super::guards::decision_requires_knowledge(decision)
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
/// 但知识覆盖只是 `weak`（有弱证据、未硬到 missing）。missing 已由
/// [`should_force_full_on_missing`] 强升承接，本谓词只盯不硬堵的 weak 灰区。
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
pub(crate) fn should_record_used_knowledge_ids(forced_full: bool, escalated_to_full: bool) -> bool {
    forced_full || escalated_to_full
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
        assert_eq!(decide_tier_escalation(&d), TierDecision::Escalate(PromptTier::Relational));
    }

    #[test]
    fn test_need_more_context_escalates_to_full() {
        let d = make_decision("need_more_context", "full");
        assert_eq!(decide_tier_escalation(&d), TierDecision::Escalate(PromptTier::Full));
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
        assert_eq!(decide_tier_escalation(&d), TierDecision::Escalate(PromptTier::Full));
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
    fn force_full_hits_on_enough_missing_and_needs_knowledge() {
        let d = decision_with_need("enough", "required");
        assert!(should_force_full_on_missing(&d, "missing"));
    }

    #[test]
    fn force_full_skips_weak_and_adequate_coverage() {
        // weak 归观测、不强升；enough/not_required 覆盖足够不强升。
        let d = decision_with_need("enough", "required");
        assert!(!should_force_full_on_missing(&d, "weak"));
        assert!(!should_force_full_on_missing(&d, "enough"));
        assert!(!should_force_full_on_missing(&d, "not_required"));
    }

    #[test]
    fn force_full_skips_when_knowledge_not_needed() {
        // 寒暄轮 knowledge_need=not_required，即便 coverage=missing 也不强升。
        let d = decision_with_need("enough", "not_required");
        assert!(!should_force_full_on_missing(&d, "missing"));
    }

    #[test]
    fn force_full_requires_positive_enough_not_negation() {
        // _=>Enough 兜底的 unknown/空不是"自评够了"，不强升。
        assert!(!should_force_full_on_missing(&decision_with_need("unknown", "required"), "missing"));
        assert!(!should_force_full_on_missing(&decision_with_need("", "required"), "missing"));
        assert!(!should_force_full_on_missing(&decision_with_need("need_more_context", "required"), "missing"));
    }

    #[test]
    fn sufficiency_recognized_three_states_only() {
        assert!(is_sufficiency_recognized(&make_decision("enough", "")));
        assert!(is_sufficiency_recognized(&make_decision("need_more_context", "")));
        assert!(is_sufficiency_recognized(&make_decision("need_clarification", "")));
        assert!(!is_sufficiency_recognized(&make_decision("", "")));
        assert!(!is_sufficiency_recognized(&make_decision("garbage", "")));
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
}
