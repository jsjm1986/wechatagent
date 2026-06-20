//! ask_human 策略解析（纯函数）：ask_human_policy 存在则用它；否则回落旧
//! principal_decider/high_risk_escalation_mode 字段映射（字节等价红线④）。无 IO。

use crate::models::{AskHumanPolicy, AskHumanQuietHours, DeciderRef, OperationDomainConfig};

/// 解析后的请示策略（运行时唯一权威；旧字段仅 None 时兜底）。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResolvedAskHumanPolicy {
    pub decider_chain: Vec<DeciderRef>,
    pub escalate_safety_guard: bool,
    pub escalate_unverified_product: bool,
    pub escalate_ai_policy_hold: bool,
    pub escalate_stuck: bool,
    pub dedupe_window_hours: Option<f64>,
    pub daily_push_cap: Option<u32>,
    pub quiet_hours: Option<AskHumanQuietHours>,
    pub timeout_hours: Option<f64>,
}

/// 解析请示策略。优先 ask_human_policy；None 时回落旧字段映射（字节等价）。
pub(crate) fn resolve_ask_human_policy(config: &OperationDomainConfig) -> ResolvedAskHumanPolicy {
    if let Some(p) = &config.ask_human_policy {
        return ResolvedAskHumanPolicy {
            decider_chain: p.decider_chain.clone(),
            escalate_safety_guard: p.escalate_safety_guard,
            escalate_unverified_product: p.escalate_unverified_product,
            escalate_ai_policy_hold: p.escalate_ai_policy_hold,
            escalate_stuck: p.escalate_stuck,
            dedupe_window_hours: p.dedupe_window_hours,
            daily_push_cap: p.daily_push_cap,
            quiet_hours: p.quiet_hours.clone(),
            timeout_hours: p.timeout_hours,
        };
    }
    let all_mode = config.high_risk_escalation_mode.as_deref() == Some("all");
    let chain = config
        .principal_decider
        .clone()
        .map(|w| vec![DeciderRef { wxid: w, display_name: None }])
        .unwrap_or_default();
    ResolvedAskHumanPolicy {
        decider_chain: chain,
        escalate_safety_guard: true,
        escalate_unverified_product: true,
        escalate_ai_policy_hold: all_mode,
        escalate_stuck: true,
        dedupe_window_hours: None,
        daily_push_cap: None,
        quiet_hours: None,
        timeout_hours: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::OperationDomainConfig;

    fn base_config() -> OperationDomainConfig {
        OperationDomainConfig {
            id: None,
            workspace_id: "ws1".into(),
            domain: "user_operations".into(),
            name: "n".into(),
            goal: "g".into(),
            methodology: "m".into(),
            workflow: "w".into(),
            tool_policy: "t".into(),
            automation_policy: "a".into(),
            review_policy: "r".into(),
            runtime_parameters: Default::default(),
            state_machine: Default::default(),
            status: "active".into(),
            updated_at: mongodb::bson::DateTime::now(),
            version: 1,
            current_version: true,
            previous_version: None,
            seeded_by: None,
            principal_decider: None,
            high_risk_escalation_mode: None,
            ask_human_policy: None,
        }
    }

    #[test]
    fn legacy_none_maps_to_decision_only_defaults() {
        let cfg = base_config();
        let r = resolve_ask_human_policy(&cfg);
        assert!(r.escalate_safety_guard);
        assert!(r.escalate_unverified_product);
        assert!(r.escalate_stuck);
        assert!(!r.escalate_ai_policy_hold);
        assert!(r.decider_chain.is_empty());
        assert_eq!(r.timeout_hours, None);
    }

    #[test]
    fn legacy_all_mode_enables_ai_policy_hold() {
        let mut cfg = base_config();
        cfg.high_risk_escalation_mode = Some("all".into());
        cfg.principal_decider = Some("boss".into());
        let r = resolve_ask_human_policy(&cfg);
        assert!(r.escalate_ai_policy_hold);
        assert_eq!(r.decider_chain.len(), 1);
        assert_eq!(r.decider_chain[0].wxid, "boss");
    }

    #[test]
    fn ask_human_policy_takes_precedence_over_legacy() {
        let mut cfg = base_config();
        cfg.high_risk_escalation_mode = Some("all".into());
        cfg.ask_human_policy = Some(AskHumanPolicy {
            decider_chain: vec![DeciderRef { wxid: "alice".into(), display_name: Some("决策人A".into()) }],
            escalate_safety_guard: true,
            escalate_unverified_product: false,
            escalate_ai_policy_hold: false,
            escalate_stuck: true,
            dedupe_window_hours: Some(6.0),
            daily_push_cap: Some(10),
            quiet_hours: None,
            timeout_hours: Some(24.0),
        });
        let r = resolve_ask_human_policy(&cfg);
        assert!(!r.escalate_unverified_product);
        assert_eq!(r.decider_chain[0].wxid, "alice");
        assert_eq!(r.timeout_hours, Some(24.0));
    }
}
