//! ask_human 策略解析（纯函数）：ask_human_policy 存在则用它；否则回落旧
//! principal_decider/high_risk_escalation_mode 字段映射（字节等价红线④）。无 IO。

use crate::models::{AskHumanQuietHours, DeciderRef, OperationDomainConfig};

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

/// 静默时段判定：now 落在 [start,end) 内（按 tz_offset 折算小时）。支持跨午夜（start>end）。
pub(crate) fn in_quiet_hours(qh: &AskHumanQuietHours, now_ms: i64) -> bool {
    let shifted = now_ms + (qh.tz_offset_hours as i64) * 3600 * 1000;
    let hour = ((shifted / (3600 * 1000)) % 24 + 24) % 24;
    let h = hour as u8;
    if qh.start_hour <= qh.end_hour {
        h >= qh.start_hour && h < qh.end_hour
    } else {
        h >= qh.start_hour || h < qh.end_hour // 跨午夜
    }
}

/// 推卡前骚扰门：daily_push_cap / dedupe_window_hours / quiet_hours 任一不满足 → false（不推）。
/// 全 None → true（字节等价，全放行）。
pub(crate) fn push_allowed(
    policy: &ResolvedAskHumanPolicy,
    today_count: u32,
    last_push_ms: Option<i64>,
    now_ms: i64,
) -> bool {
    if let Some(cap) = policy.daily_push_cap {
        if today_count >= cap {
            return false;
        }
    }
    if let (Some(window_h), Some(last)) = (policy.dedupe_window_hours, last_push_ms) {
        let elapsed_h = (now_ms - last) as f64 / (3600.0 * 1000.0);
        if elapsed_h < window_h {
            return false;
        }
    }
    if let Some(qh) = &policy.quiet_hours {
        if in_quiet_hours(qh, now_ms) {
            return false;
        }
    }
    true
}

/// 超时转备选：当前决策人在链中、已等待 age_hours 超过 timeout_hours，返回链中下一位。
/// timeout_hours=None（无限等待）/ 未超时 / 已是链尾 → None。
pub(crate) fn next_decider_on_timeout<'a>(
    policy: &'a ResolvedAskHumanPolicy,
    current_wxid: &str,
    age_hours: f64,
) -> Option<&'a DeciderRef> {
    let timeout = policy.timeout_hours?;
    if age_hours < timeout {
        return None;
    }
    let idx = policy.decider_chain.iter().position(|d| d.wxid == current_wxid)?;
    policy.decider_chain.get(idx + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AskHumanPolicy, OperationDomainConfig};

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
            assist_mode_enabled: None,
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

    fn resolved_with(daily_cap: Option<u32>, dedupe_h: Option<f64>) -> ResolvedAskHumanPolicy {
        ResolvedAskHumanPolicy {
            decider_chain: vec![],
            escalate_safety_guard: true,
            escalate_unverified_product: true,
            escalate_ai_policy_hold: false,
            escalate_stuck: true,
            dedupe_window_hours: dedupe_h,
            daily_push_cap: daily_cap,
            quiet_hours: None,
            timeout_hours: None,
        }
    }

    #[test]
    fn push_allowed_none_config_always_true() {
        // 无 cap、无 dedupe、无 quiet → 字节等价（全放行）。
        let p = resolved_with(None, None);
        assert!(push_allowed(&p, 999, Some(0), 1_000));
    }

    #[test]
    fn push_blocked_when_daily_cap_reached() {
        let p = resolved_with(Some(3), None);
        assert!(push_allowed(&p, 2, None, 1_000));   // 未达上限
        assert!(!push_allowed(&p, 3, None, 1_000));  // 达上限
    }

    #[test]
    fn push_blocked_within_dedupe_window() {
        let p = resolved_with(None, Some(6.0)); // 6h 窗
        let now = 10 * 3600 * 1000i64;
        let recent = now - 3600 * 1000; // 1h 前推过 → 窗内 → 拦
        assert!(!push_allowed(&p, 0, Some(recent), now));
        let old = now - 7 * 3600 * 1000; // 7h 前 → 超窗 → 放行
        assert!(push_allowed(&p, 0, Some(old), now));
    }

    #[test]
    fn next_decider_picks_following_after_timeout() {
        let mut p = resolved_with(None, None);
        p.timeout_hours = Some(24.0);
        p.decider_chain = vec![
            DeciderRef { wxid: "a".into(), display_name: None },
            DeciderRef { wxid: "b".into(), display_name: None },
        ];
        // 当前 a，已等 25h > 24h → 转 b
        assert_eq!(next_decider_on_timeout(&p, "a", 25.0).map(|d| d.wxid.as_str()), Some("b"));
        // 未超时 → None
        assert_eq!(next_decider_on_timeout(&p, "a", 10.0), None);
        // 已是链尾 b → None（继续等）
        assert_eq!(next_decider_on_timeout(&p, "b", 99.0), None);
    }

    #[test]
    fn next_decider_none_when_timeout_unset() {
        let mut p = resolved_with(None, None);
        p.decider_chain = vec![
            DeciderRef { wxid: "a".into(), display_name: None },
            DeciderRef { wxid: "b".into(), display_name: None },
        ];
        // timeout_hours=None → 无限等待，永不转
        assert_eq!(next_decider_on_timeout(&p, "a", 9999.0), None);
    }

    #[test]
    fn in_quiet_hours_cross_midnight_and_tz() {
        use crate::models::AskHumanQuietHours;
        // 跨午夜窗 22:00–06:00, tz=0（UTC）。now=23:00 → 窗内；now=12:00 → 窗外。
        let qh = AskHumanQuietHours { start_hour: 22, end_hour: 6, tz_offset_hours: 0 };
        let h23 = 23 * 3600 * 1000i64;       // 23:00 UTC
        let h12 = 12 * 3600 * 1000i64;       // 12:00 UTC
        assert!(in_quiet_hours(&qh, h23));   // 跨午夜窗内
        assert!(!in_quiet_hours(&qh, h12));  // 窗外
        // tz=+8：UTC 18:00 → 本地 02:00（在 22–06 窗内）。
        let qh8 = AskHumanQuietHours { start_hour: 22, end_hour: 6, tz_offset_hours: 8 };
        let utc18 = 18 * 3600 * 1000i64;     // UTC 18:00 → 本地 02:00
        assert!(in_quiet_hours(&qh8, utc18));
        // 非跨午夜窗 09:00–17:00, tz=0：now=12:00 窗内、now=20:00 窗外。
        let day = AskHumanQuietHours { start_hour: 9, end_hour: 17, tz_offset_hours: 0 };
        assert!(in_quiet_hours(&day, h12));
        assert!(!in_quiet_hours(&day, 20 * 3600 * 1000i64));
    }
}
