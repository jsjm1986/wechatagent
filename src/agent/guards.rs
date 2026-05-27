//! 决策守卫 — 状态机迁移合法性 + planner 同步辅助。
//!
//! 销售域守卫（fact-risk / pressure-risk / product_accuracy / safe_claims /
//! routing_card / taxonomy guards 等）已在 2026-05-25 知识库清理中删除，方法论
//! 切换为 wiki + 3 闸（knowledge_grounding / hallucination / run_budget），新闸
//! 在 commit 3 引入。本模块只剩下与 `operation_domain_configs` 状态机字典对齐
//! 的纯函数。

use mongodb::bson::Document;

use crate::models::{OperationDomainConfig, OperationStatePolicy};

use super::types::{AgentDecision, RunPlannerResult};

pub(crate) fn normalize_decision_state(
    decision: &mut AgentDecision,
    domain_config: Option<&OperationDomainConfig>,
) {
    let Some(current) = decision
        .operation_state
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    if operation_state_exists(domain_config, current) {
        return;
    }
    if let Some(key) = operation_state_key_by_name(domain_config, current) {
        decision.operation_state = Some(key);
    }
}

// W1 / R3.6 / N1：本函数不再填默认；缺失字段由 validate_and_promote 校验。
//
// 这里保留的是 `memory_write_score` 与 planner.memory_change_importance 的非
// 枚举性同步：Agent 输出了 `operating_memory_update` 但未填 write_score 时按
// planner 估计回填，供 `write_memory_candidates` 区分 pending / completed。
pub(crate) fn normalize_decision_runtime(decision: &mut AgentDecision, planner: &RunPlannerResult) {
    if decision.memory_write_score == 0 && !decision.operating_memory_update.is_empty() {
        decision.memory_write_score = planner.memory_change_importance;
    }
}

pub(crate) fn planner_from_decision(decision: &AgentDecision, reason: &str) -> RunPlannerResult {
    let risk_level = if decision.risk_level.trim().is_empty() {
        "medium".to_string()
    } else {
        decision.risk_level.clone()
    };
    let knowledge_required = decision_requires_knowledge(decision);
    RunPlannerResult {
        risk_level: risk_level.clone(),
        context_needs_refresh: false,
        memory_change_importance: decision.memory_write_score.clamp(0, 10),
        knowledge_required,
        review_mode: if decision.needs_review || risk_level == "high" || knowledge_required {
            "full".to_string()
        } else {
            "light".to_string()
        },
        reason: reason.to_string(),
        ..Default::default()
    }
}

pub(crate) fn decision_requires_knowledge(decision: &AgentDecision) -> bool {
    matches!(
        decision.knowledge_need.trim(),
        "required" | "insufficient" | "knowledge_required"
    )
}

pub(crate) fn operation_state_exists(
    domain_config: Option<&OperationDomainConfig>,
    key: &str,
) -> bool {
    let states = operation_states(domain_config);
    states.is_empty()
        || states
            .iter()
            .any(|state| state.get_str("key").ok() == Some(key))
}

pub(crate) fn operation_state_key_by_name(
    domain_config: Option<&OperationDomainConfig>,
    name: &str,
) -> Option<String> {
    operation_states(domain_config)
        .into_iter()
        .find(|state| state.get_str("name").ok() == Some(name))
        .and_then(|state| state.get_str("key").ok().map(ToString::to_string))
}

pub(crate) fn operation_states(domain_config: Option<&OperationDomainConfig>) -> Vec<Document> {
    domain_config
        .and_then(|config| config.state_machine.get_array("states").ok())
        .map(|states| {
            states
                .iter()
                .filter_map(|item| item.as_document().cloned())
                .collect()
        })
        .unwrap_or_default()
}

/// 状态机迁移合法性校验。
///
/// 规则：
/// - 状态机为空（domain_config 缺失）时不做迁移校验，向后兼容老配置；
/// - 目标 state `allowFromAny=true`（如 cooldown）总是合法；
/// - `from` 为空时只有目标 = `new_contact` 合法；
/// - 否则 `from` 必须出现在目标 state 的 `allowedFrom` 列表中。
///
/// 返回 `Some(reason)` 表示拦截理由；返回 `None` 表示通过。
pub fn check_state_transition(
    domain_config: Option<&OperationDomainConfig>,
    from: Option<&str>,
    to: &str,
) -> Option<String> {
    let states = operation_states(domain_config);
    if states.is_empty() {
        return None; // 没有状态机不强校验。
    }
    let target = states
        .iter()
        .find(|state| state.get_str("key").ok() == Some(to))?;
    if target.get_bool("allowFromAny").unwrap_or(false) {
        return None;
    }
    let from = from.map(str::trim).filter(|s| !s.is_empty());
    match from {
        None => {
            if to == "new_contact" {
                None
            } else {
                Some(format!("state_transition_invalid: from=<empty> to={to}"))
            }
        }
        Some(from_key) => {
            let allowed: Vec<&str> = target
                .get_array("allowedFrom")
                .map(|arr| {
                    arr.iter()
                        .filter_map(|item| item.as_str())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if allowed.iter().any(|key| *key == from_key) {
                None
            } else {
                Some(format!("state_transition_invalid: from={from_key} to={to}"))
            }
        }
    }
}

// ── Phase B / B4：operation_state_policies enforcement ────────────────────

/// Phase B / B4：把一个 [`AgentDecision`] 归一到一个 action 类型字符串。
///
/// 当前归一规则（Phase B 范围）：
/// - `should_reply == true` → `"reply"`
/// - `should_reply == false`, follow_up.kind 为 `"silent_followup"` 或 `"proactive_followup"`
///   → `"follow_up"`
/// - `should_reply == false` 且 `cooldown_until` 非空 → `"cooldown"`
/// - 其它 → `"silent"`
///
/// 该字符串与 `operation_state_policies.allowed / forbidden` 数组里的标签**字面量**对齐。
/// 后续 Phase E 引入 `ActionType` enum 时可平滑替换字符串字面量为枚举 to_string。
pub fn classify_decision_action(decision: &AgentDecision) -> &'static str {
    if decision.should_reply {
        return "reply";
    }
    if let Some(fu) = decision.follow_up.as_ref() {
        if fu.needed {
            return "follow_up";
        }
    }
    if decision
        .cooldown_until
        .as_deref()
        .map(str::trim)
        .map(|s| !s.is_empty())
        .unwrap_or(false)
    {
        return "cooldown";
    }
    "silent"
}

/// Phase B / B4：用 `operation_state_policies` 行校验候选 action 是否被允许。
///
/// 拦截规则：
/// 1. policy 缺失（含 `status != "active"`）→ `Ok(())`，向前兼容老部署；
/// 2. `forbidden` 命中 action → `Err(reason)`，优先级最高；
/// 3. `allowed` 非空且不包含 action → `Err(reason)`，白名单收敛模式；
/// 4. 其它 → `Ok(())`。
///
/// `reason` 字符串前缀固定为 `state_action_forbidden:` / `state_action_not_allowed:`，
/// 便于上层 finalize 走 reason 分流。
pub fn enforce_state_action_policy(
    policy: Option<&OperationStatePolicy>,
    action: &str,
) -> Result<(), String> {
    let Some(policy) = policy else { return Ok(()); };
    if policy.status != "active" {
        return Ok(());
    }
    if policy.forbidden.iter().any(|a| a == action) {
        return Err(format!(
            "state_action_forbidden: state={} action={}",
            policy.state_key, action
        ));
    }
    if !policy.allowed.is_empty() && !policy.allowed.iter().any(|a| a == action) {
        return Err(format!(
            "state_action_not_allowed: state={} action={}",
            policy.state_key, action
        ));
    }
    Ok(())
}

#[cfg(test)]
mod policy_tests {
    //! Phase B / B4：`classify_decision_action` + `enforce_state_action_policy` 单测。
    use super::*;
    use crate::models::OperationStatePolicy;
    use crate::agent::types::FollowUpDecision;
    use mongodb::bson::DateTime;

    fn mk_policy(state: &str, allowed: &[&str], forbidden: &[&str]) -> OperationStatePolicy {
        OperationStatePolicy {
            id: None,
            workspace_id: "ws".to_string(),
            domain: "user".to_string(),
            state_key: state.to_string(),
            allowed: allowed.iter().map(|s| s.to_string()).collect(),
            forbidden: forbidden.iter().map(|s| s.to_string()).collect(),
            recommended_pace: None,
            status: "active".to_string(),
            updated_at: DateTime::now(),
            version: 1,
            current_version: true,
            previous_version: None,
            seeded_by: None,
        }
    }

    fn mk_decision_reply() -> AgentDecision {
        let mut d = AgentDecision::default();
        d.should_reply = true;
        d.reply_text = "test".to_string();
        d
    }

    #[test]
    fn classify_reply_when_should_reply_true() {
        let mut d = AgentDecision::default();
        d.should_reply = true;
        assert_eq!(classify_decision_action(&d), "reply");
    }

    #[test]
    fn classify_silent_when_no_signals() {
        let d = AgentDecision::default();
        assert_eq!(classify_decision_action(&d), "silent");
    }

    #[test]
    fn classify_follow_up_when_silent_followup_kind() {
        let mut d = AgentDecision::default();
        d.follow_up = Some(FollowUpDecision {
            needed: true,
            ..Default::default()
        });
        assert_eq!(classify_decision_action(&d), "follow_up");
    }

    #[test]
    fn classify_cooldown_when_should_reply_false_and_cooldown_until_set() {
        let mut d = AgentDecision::default();
        d.cooldown_until = Some("2030-01-01T00:00:00Z".to_string());
        assert_eq!(classify_decision_action(&d), "cooldown");
    }

    #[test]
    fn enforce_passes_when_policy_missing() {
        assert!(enforce_state_action_policy(None, "reply").is_ok());
    }

    #[test]
    fn enforce_passes_when_policy_inactive() {
        let mut p = mk_policy("new_contact", &[], &["reply"]);
        p.status = "draft".to_string();
        assert!(enforce_state_action_policy(Some(&p), "reply").is_ok());
    }

    #[test]
    fn enforce_blocks_when_action_in_forbidden() {
        let p = mk_policy("cooldown", &[], &["reply"]);
        let err = enforce_state_action_policy(Some(&p), "reply").unwrap_err();
        assert!(err.starts_with("state_action_forbidden:"));
        assert!(err.contains("state=cooldown"));
        assert!(err.contains("action=reply"));
    }

    #[test]
    fn enforce_blocks_when_allowlist_set_and_action_missing() {
        let p = mk_policy("warmup", &["follow_up"], &[]);
        let err = enforce_state_action_policy(Some(&p), "reply").unwrap_err();
        assert!(err.starts_with("state_action_not_allowed:"));
    }

    #[test]
    fn enforce_passes_when_allowlist_empty_and_no_forbidden() {
        let p = mk_policy("warmup", &[], &[]);
        assert!(enforce_state_action_policy(Some(&p), "reply").is_ok());
    }

    #[test]
    fn enforce_passes_when_action_in_allowlist() {
        let p = mk_policy("warmup", &["reply", "follow_up"], &[]);
        assert!(enforce_state_action_policy(Some(&p), "reply").is_ok());
    }

    #[test]
    fn forbidden_takes_priority_over_allowed() {
        // 同一 action 同时出现在 allowed + forbidden → forbidden 胜出。
        let p = mk_policy("guarded", &["reply"], &["reply"]);
        let err = enforce_state_action_policy(Some(&p), "reply").unwrap_err();
        assert!(err.starts_with("state_action_forbidden:"));
    }

    #[test]
    fn classify_then_enforce_reply_decision_with_forbidden_state() {
        let d = mk_decision_reply();
        let p = mk_policy("cooldown", &[], &["reply"]);
        let action = classify_decision_action(&d);
        assert_eq!(action, "reply");
        assert!(enforce_state_action_policy(Some(&p), action).is_err());
    }
}

