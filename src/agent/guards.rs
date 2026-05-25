//! 决策守卫 — 状态机迁移合法性 + planner 同步辅助。
//!
//! 销售域守卫（fact-risk / pressure-risk / product_accuracy / safe_claims /
//! routing_card / taxonomy guards 等）已在 2026-05-25 知识库清理中删除，方法论
//! 切换为 wiki + 3 闸（knowledge_grounding / hallucination / run_budget），新闸
//! 在 commit 3 引入。本模块只剩下与 `operation_domain_configs` 状态机字典对齐
//! 的纯函数。

use mongodb::bson::Document;

use crate::models::OperationDomainConfig;

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
