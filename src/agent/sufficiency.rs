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
                _ => PromptTier::Relational, // 兜底：默认升中档
            };
            TierDecision::Escalate(tier)
        }
        "need_clarification" => TierDecision::Clarify,
        _ => TierDecision::Enough,
    }
}

/// 纯谓词：本轮是否构成「自评乐观偏差」——Reply Agent 自评信息已足（sufficiency=enough）、
/// 却同时本轮确实需要产品知识、且知识路由的覆盖度不足（missing/weak）。
///
/// 设计 §2.2「先观测后判罚，不强拦」：命中只记一条观测 telemetry 供日后校准自评可靠性，
/// **不改变档位决策**（仍走 Enough、回复照常发）。抽成纯谓词让这条判据（三个条件 AND +
/// 正向精确匹配）可被 lib 单测锁死——必须正向匹配 `== "enough"`，**绝不能**用 `!= ...`
/// 否定匹配（会把 not_required 寒暄轮、_=>Enough 兜底的 unknown/空 误记成乐观偏差）。
///
/// coverage 取 `missing` 与 `weak` 两态（`enough`/`not_required` 不算偏差）；
/// `decision_requires_knowledge` 把 not_required 寒暄轮天然挡掉，双保险。
pub(crate) fn is_coverage_optimism(decision: &AgentDecision, knowledge_coverage: &str) -> bool {
    decision.sufficiency.as_str() == "enough"
        && matches!(knowledge_coverage, "missing" | "weak")
        && super::guards::decision_requires_knowledge(decision)
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
    fn test_need_more_context_invalid_tier_falls_back_to_relational() {
        let d = make_decision("need_more_context", "garbage");
        assert_eq!(decide_tier_escalation(&d), TierDecision::Escalate(PromptTier::Relational));
    }

    fn decision_with_need(sufficiency: &str, knowledge_need: &str) -> AgentDecision {
        AgentDecision {
            sufficiency: sufficiency.to_string(),
            knowledge_need: knowledge_need.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn coverage_optimism_hits_on_enough_plus_missing_plus_required() {
        let d = decision_with_need("enough", "required");
        assert!(is_coverage_optimism(&d, "missing"));
    }

    #[test]
    fn coverage_optimism_hits_on_weak_too() {
        // 你选了 missing+weak 都纳入观测——weak 态同样算乐观偏差。
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
}
