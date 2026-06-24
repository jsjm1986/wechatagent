//! 渐进式三档 + 充分性自评（2026-06-23）
//!
//! Reply Agent 在每轮决策后自评：本轮信息是否充足？如果不够，需要提升到哪一档
//! prompt？需要澄清什么？
//!
//! 本模块提供纯函数档位判定逻辑，供 gateway 调用。
//!
//! `#![allow(dead_code)]`：本任务（三档计划 Task 1）只落地数据结构 + 纯函数 + 单测，
//! gateway 接线在后续 Task（2-7）完成，故公开项暂无生产调用点，靠 module 级
//! allow 静默 dead_code，待接线后移除。

#![allow(dead_code)]

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

/// 纯函数：根据充分性自评 + coverage 兜底观测，决定下一步动作。
pub fn decide_tier_escalation(
    decision: &AgentDecision,
    knowledge_coverage: &str,
) -> TierDecision {
    match decision.sufficiency.as_str() {
        "enough" => {
            // TODO: coverage 兜底观测（先观测后判罚，不强拦）
            let _ = knowledge_coverage;
            TierDecision::Enough
        }
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
        assert_eq!(decide_tier_escalation(&d, "enough"), TierDecision::Enough);
    }

    #[test]
    fn test_need_more_context_escalates_to_relational() {
        let d = make_decision("need_more_context", "relational");
        assert_eq!(decide_tier_escalation(&d, "enough"), TierDecision::Escalate(PromptTier::Relational));
    }

    #[test]
    fn test_need_more_context_escalates_to_full() {
        let d = make_decision("need_more_context", "full");
        assert_eq!(decide_tier_escalation(&d, "enough"), TierDecision::Escalate(PromptTier::Full));
    }

    #[test]
    fn test_need_clarification_triggers_clarify() {
        let d = make_decision("need_clarification", "");
        assert_eq!(decide_tier_escalation(&d, "missing"), TierDecision::Clarify);
    }

    #[test]
    fn test_unknown_sufficiency_defaults_to_enough() {
        let d = make_decision("unknown", "");
        assert_eq!(decide_tier_escalation(&d, "enough"), TierDecision::Enough);
    }

    #[test]
    fn test_empty_sufficiency_defaults_to_enough() {
        let d = make_decision("", "");
        assert_eq!(decide_tier_escalation(&d, "enough"), TierDecision::Enough);
    }

    #[test]
    fn test_need_more_context_invalid_tier_falls_back_to_relational() {
        let d = make_decision("need_more_context", "garbage");
        assert_eq!(decide_tier_escalation(&d, "enough"), TierDecision::Escalate(PromptTier::Relational));
    }
}
