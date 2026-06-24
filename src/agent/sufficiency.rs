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

/// 档位判定结果：Agent 自评后，gateway 应如何响应？
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TierDecision {
    /// 信息充足，可直接回复（sufficiency == "enough"）
    Enough,
    /// 信息不足，需提升 prompt 档位后重跑（sufficiency == "need_more_context"）
    Escalate { target_tier: PromptTier },
    /// 需要向客户澄清（sufficiency == "need_clarification"）
    Clarify { intent: String },
}

/// 渐进式三档 prompt 枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptTier {
    /// 基础档（Tier 1）：仅关系上下文 + 画像
    Basic,
    /// 关系档（Tier 2）：Tier 1 + 关系历史记忆
    Relational,
    /// 完整档（Tier 3）：Tier 2 + 知识库 + 完整 SOP
    Full,
}

/// 根据 Agent 自评结果，判定档位提升策略。
///
/// # 参数
/// - `decision`: Reply Agent 输出的决策（含 sufficiency / missing_tier / clarification_intent）
/// - `current_tier`: 本轮运行时的 prompt 档位
///
/// # 返回
/// - `TierDecision::Enough`: 信息充足，可直接回复
/// - `TierDecision::Escalate { target_tier }`: 需提升到更高档位重跑
/// - `TierDecision::Clarify { intent }`: 需向客户澄清
///
/// # 逻辑
/// 1. `sufficiency == "enough"` → Enough
/// 2. `sufficiency == "need_clarification"` → Clarify（无论 missing_tier）
/// 3. `sufficiency == "need_more_context"` → 解析 missing_tier 并提升档位：
///    - "relational" → 提升到 Relational（如果当前是 Basic）
///    - "full" → 提升到 Full（如果当前不是 Full）
///    - 其他 / 空 / 已在目标档 → Enough（兜底不死循环）
pub fn decide_tier_escalation(
    decision: &AgentDecision,
    current_tier: PromptTier,
) -> TierDecision {
    let sufficiency = decision.sufficiency.trim();
    let missing_tier = decision.missing_tier.trim();

    match sufficiency {
        "enough" => TierDecision::Enough,
        "need_clarification" => TierDecision::Clarify {
            intent: decision.clarification_intent.clone(),
        },
        "need_more_context" => {
            match missing_tier {
                "relational" => {
                    // 需要关系档：如果当前是 Basic，提升到 Relational；否则已满足
                    if current_tier == PromptTier::Basic {
                        TierDecision::Escalate {
                            target_tier: PromptTier::Relational,
                        }
                    } else {
                        TierDecision::Enough
                    }
                }
                "full" => {
                    // 需要完整档：如果当前不是 Full，提升到 Full；否则已满足
                    if current_tier != PromptTier::Full {
                        TierDecision::Escalate {
                            target_tier: PromptTier::Full,
                        }
                    } else {
                        TierDecision::Enough
                    }
                }
                _ => {
                    // missing_tier 无效 / 空 / "none" → 兜底为 Enough，避免死循环
                    TierDecision::Enough
                }
            }
        }
        _ => {
            // sufficiency 无效值 → 兜底为 Enough（容错，避免死循环）
            TierDecision::Enough
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enough_returns_enough() {
        let decision = AgentDecision {
            sufficiency: "enough".to_string(),
            ..Default::default()
        };
        let result = decide_tier_escalation(&decision, PromptTier::Basic);
        assert_eq!(result, TierDecision::Enough);
    }

    #[test]
    fn test_need_clarification_returns_clarify() {
        let decision = AgentDecision {
            sufficiency: "need_clarification".to_string(),
            clarification_intent: "请问您具体想了解哪方面？".to_string(),
            ..Default::default()
        };
        let result = decide_tier_escalation(&decision, PromptTier::Basic);
        assert_eq!(
            result,
            TierDecision::Clarify {
                intent: "请问您具体想了解哪方面？".to_string()
            }
        );
    }

    #[test]
    fn test_need_more_context_relational_escalates_from_basic() {
        let decision = AgentDecision {
            sufficiency: "need_more_context".to_string(),
            missing_tier: "relational".to_string(),
            ..Default::default()
        };
        let result = decide_tier_escalation(&decision, PromptTier::Basic);
        assert_eq!(
            result,
            TierDecision::Escalate {
                target_tier: PromptTier::Relational
            }
        );
    }

    #[test]
    fn test_need_more_context_relational_no_escalate_from_relational() {
        let decision = AgentDecision {
            sufficiency: "need_more_context".to_string(),
            missing_tier: "relational".to_string(),
            ..Default::default()
        };
        let result = decide_tier_escalation(&decision, PromptTier::Relational);
        assert_eq!(result, TierDecision::Enough);
    }

    #[test]
    fn test_need_more_context_full_escalates_from_basic() {
        let decision = AgentDecision {
            sufficiency: "need_more_context".to_string(),
            missing_tier: "full".to_string(),
            ..Default::default()
        };
        let result = decide_tier_escalation(&decision, PromptTier::Basic);
        assert_eq!(
            result,
            TierDecision::Escalate {
                target_tier: PromptTier::Full
            }
        );
    }

    #[test]
    fn test_invalid_missing_tier_returns_enough() {
        let decision = AgentDecision {
            sufficiency: "need_more_context".to_string(),
            missing_tier: "invalid_value".to_string(),
            ..Default::default()
        };
        let result = decide_tier_escalation(&decision, PromptTier::Basic);
        assert_eq!(result, TierDecision::Enough);
    }
}
